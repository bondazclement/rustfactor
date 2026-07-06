//! pm-dash — interface de pilotage web locale (http://127.0.0.1:7777).
//!
//! LECTURE SEULE vis-à-vis du bot : tout est reconstruit depuis les
//! artefacts qu'il produit (journaux NDJSON, run.log, calibration.json).
//! Aucune connexion au processus — l'interface peut vivre ou mourir sans
//! que le trading le sache. Les indicateurs affichés (σ, strike, p, EV)
//! sont calculés par le MÊME code que le bot (pm-core / pm-strategy),
//! jamais réimplémentés.
//!
//! Seule écriture autorisée : `config.toml`, validée par le parseur du
//! bot, avec sauvegarde `.bak` — appliquée au prochain (re)démarrage.

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use pm_core::math::student_t_cdf;
use pm_core::strike::{compute_strike, StrikePolicy, DEFAULT_CONFIDENCE_GAP_MS};
use pm_core::vol::{VolConfig, VolEstimator};
use pm_core::ResolutionTick;
use pm_strategy::calib::CalibTable;
use pm_strategy::config::BotConfig;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const PAGE: &str = include_str!("page.html");
/// Fenêtre de relecture du journal (octets) — ~10 min de trafic CLOB+RTDS.
const TAIL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone)]
struct App {
    base: Arc<PathBuf>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let base = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    anyhow::ensure!(
        base.join("data_v2").exists() || base.join("config.exemple.toml").exists(),
        "lancez pm-dash depuis la racine du projet (ou passez-la en argument)"
    );
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    rt.block_on(serve(App { base: Arc::new(base) }))
}

async fn serve(app: App) -> Result<()> {
    let router = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/api/etat", get(api_etat))
        .route("/api/series", get(api_series))
        .route("/api/calibration", get(api_calibration))
        .route("/api/config", get(api_config_get).post(api_config_post))
        .route("/api/mode", post(api_mode))
        .route("/api/avance", post(api_avance))
        .with_state(app);
    let addr = "127.0.0.1:7777";
    tracing::info!("pm-dash → http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

// ─── Lecture des artefacts du bot ────────────────────────────────────────

fn dernier_run(base: &Path) -> Option<PathBuf> {
    let mut runs: Vec<_> = std::fs::read_dir(base.join("data_v2"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("camp_") || n.starts_with("run_"))
        })
        .collect();
    runs.sort();
    runs.pop()
}

fn dernier_journal(run: &Path) -> Option<PathBuf> {
    let mut js: Vec<_> = std::fs::read_dir(run)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "ndjson")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("journal_"))
        })
        .collect();
    js.sort();
    js.pop()
}

/// Queue du journal, coupée à la première ligne complète.
fn lire_queue(path: &Path) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return vec![];
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(TAIL_BYTES);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return vec![];
    }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        return vec![];
    }
    let skip = if start > 0 { 1 } else { 0 };
    buf.lines().skip(skip).map(String::from).collect()
}

#[derive(Debug, Default, Clone, Serialize)]
struct Fenetre {
    slug: String,
    t0_ms: u64,
    end_ms: u64,
    token_up: String,
    token_down: String,
}

#[derive(Debug, Default, Clone)]
struct Journal {
    ticks: Vec<ResolutionTick>,
    /// (recv_ms, bid, ask) par côté de la fenêtre courante.
    up: Vec<(u64, f64, f64)>,
    down: Vec<(u64, f64, f64)>,
    fenetres: Vec<Fenetre>,
    spot_binance: Option<(u64, f64)>,
    /// Dernière trame reçue par flux (recv_ms) — pour les chronos de l'UI.
    dernier_oracle_ms: u64,
    dernier_relais_ms: u64,
    dernier_clob_ms: u64,
    dernier_binance_ms: u64,
}

/// Cache incrémental : seuls les octets AJOUTÉS depuis le dernier appel
/// sont relus/parsés (l'UI interroge toutes les 2 s ; reparser 64 Mo à
/// chaque fois brûlait un cœur pour rien).
#[derive(Default)]
struct CacheJournal {
    path: PathBuf,
    offset: u64,
    j: Journal,
}

static CACHE: std::sync::Mutex<Option<CacheJournal>> = std::sync::Mutex::new(None);

fn parser_lignes(j: &mut Journal, lignes: &[String]) {
    for l in lignes {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        let stream = v.get("stream").and_then(|s| s.as_str()).unwrap_or("");
        let recv = v.get("recv_ms").and_then(|r| r.as_u64()).unwrap_or(0);
        let Some(raw) = v.get("raw").and_then(|r| r.as_str()) else { continue };
        match stream {
            "rtds" if raw.contains("btc/usd") => {
                j.dernier_oracle_ms = recv;
                if let Ok(m) = serde_json::from_str::<serde_json::Value>(raw) {
                    let p = &m["payload"];
                    if let (Some(ts), Some(px)) = (p["timestamp"].as_u64(), p["value"].as_f64()) {
                        j.ticks.push(ResolutionTick {
                            recv_ms: recv,
                            source_ts_ms: ts,
                            message_ts_ms: ts,
                            price: px,
                        });
                    }
                }
            }
            "rtds" if raw.contains("btcusdt") => j.dernier_relais_ms = recv,
            "rtds" => {}
            "binance" => {
                j.dernier_binance_ms = recv;
                if let Ok(m) = serde_json::from_str::<serde_json::Value>(raw) {
                    if let (Some(ts), Some(px)) = (
                        m["E"].as_u64(),
                        m["p"].as_str().and_then(|s| s.parse::<f64>().ok()),
                    ) {
                        j.spot_binance = Some((ts, px));
                    }
                }
            }
            "gamma" => {
                if let Ok(w) = serde_json::from_str::<serde_json::Value>(raw) {
                    j.fenetres.push(Fenetre {
                        slug: w["slug"].as_str().unwrap_or("").into(),
                        t0_ms: w["start_ms"].as_u64().unwrap_or(0),
                        end_ms: w["end_ms"].as_u64().unwrap_or(0),
                        token_up: w["token_up"].as_str().unwrap_or("").into(),
                        token_down: w["token_down"].as_str().unwrap_or("").into(),
                    });
                }
            }
            "clob" => {
                j.dernier_clob_ms = recv;
                if !raw.contains("price_changes") {
                    continue;
                }
                let Some(courante) = j.fenetres.last() else { continue };
                let (tu, td) = (courante.token_up.clone(), courante.token_down.clone());
                let Ok(m) = serde_json::from_str::<serde_json::Value>(raw) else { continue };
                for ch in m["price_changes"].as_array().into_iter().flatten() {
                    let asset = ch["asset_id"].as_str().unwrap_or("");
                    let (Some(b), Some(a)) = (
                        ch["best_bid"].as_str().and_then(|x| x.parse().ok()),
                        ch["best_ask"].as_str().and_then(|x| x.parse().ok()),
                    ) else {
                        continue;
                    };
                    if asset == tu {
                        j.up.push((recv, b, a));
                    } else if asset == td {
                        j.down.push((recv, b, a));
                    }
                }
            }
            _ => {}
        }
    }
    // Fenêtre glissante : 45 min de ticks (strike + vol), 15 min de carnets.
    let horizon = j.ticks.last().map(|t| t.source_ts_ms).unwrap_or(0);
    j.ticks.retain(|t| t.source_ts_ms + 2_700_000 >= horizon);
    let hb = j.up.last().map(|x| x.0).max(j.down.last().map(|x| x.0)).unwrap_or(0);
    j.up.retain(|x| x.0 + 900_000 >= hb);
    j.down.retain(|x| x.0 + 900_000 >= hb);
    let nf = j.fenetres.len().saturating_sub(12);
    j.fenetres.drain(..nf);
}

fn charger_journal(base: &Path) -> Journal {
    use std::io::{Read, Seek, SeekFrom};
    let mut guard = CACHE.lock().unwrap();
    let Some(run) = dernier_run(base) else { return Journal::default() };
    let Some(path) = dernier_journal(&run) else { return Journal::default() };
    let taille = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let repartir = match guard.as_ref() {
        Some(c) => c.path != path || taille < c.offset,
        None => true,
    };
    if repartir {
        let mut c = CacheJournal { path: path.clone(), offset: taille.saturating_sub(TAIL_BYTES), j: Journal::default() };
        // la première fenêtre courante exige les métadonnées gamma : elles
        // sont en tête de journal, hors de la queue → relire tout le fichier
        // pour les gamma seulement si nécessaire (fichiers gamma rares).
        if c.offset > 0 {
            if let Ok(mut f) = std::fs::File::open(&path) {
                let mut tete = String::new();
                let _ = f.by_ref().take(4 * 1024 * 1024).read_to_string(&mut tete);
                let gammas: Vec<String> = tete.lines().filter(|l| l.contains("\"gamma\"")).map(String::from).collect();
                parser_lignes(&mut c.j, &gammas);
            }
        }
        *guard = Some(c);
    }
    let c = guard.as_mut().unwrap();
    if taille > c.offset {
        if let Ok(mut f) = std::fs::File::open(&path) {
            if f.seek(SeekFrom::Start(c.offset)).is_ok() {
                let mut buf = String::new();
                if f.read_to_string(&mut buf).is_ok() {
                    let complet = buf.ends_with('\n');
                    let mut lignes: Vec<String> = buf.lines().map(String::from).collect();
                    if c.offset > 0 && repartir {
                        lignes.drain(..1.min(lignes.len())); // ligne partielle
                    }
                    if !complet {
                        // garder la ligne incomplète pour le prochain appel
                        if let Some(l) = lignes.pop() {
                            c.offset = taille - l.len() as u64;
                        }
                    } else {
                        c.offset = taille;
                    }
                    parser_lignes(&mut c.j, &lignes);
                }
            }
        }
    }
    c.j.clone()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Extrait les événements utiles de run.log (déjà en clair, sans couleurs).
fn lire_run_log(base: &Path) -> (Vec<serde_json::Value>, Vec<serde_json::Value>, f64, u32, u32, bool) {
    let Some(run) = dernier_run(base) else {
        return (vec![], vec![], 0.0, 0, 0, false);
    };
    let Ok(log) = std::fs::read_to_string(run.join("run.log")) else {
        return (vec![], vec![], 0.0, 0, 0, false);
    };
    let reel = log.contains("MODE RÉEL ARMÉ");
    let strip = |l: &str| -> String {
        let mut out = String::with_capacity(l.len());
        let mut esc = false;
        for c in l.chars() {
            if esc {
                if c == 'm' {
                    esc = false;
                }
            } else if c == '\u{1b}' {
                esc = true;
            } else {
                out.push(c);
            }
        }
        out
    };
    // "TAKER: UP (z=5.13 p=0.990 ask=0.950 avg=0.965 tau=114s dist=107$ mode=frontiere)"
    let champ = |l: &str, k: &str| -> Option<String> {
        let i = l.find(k)? + k.len();
        Some(l[i..].split([' ', ')', 's', '$', '"']).next().unwrap_or("").to_string())
    };
    let heure = |l: &str| l.get(11..19).unwrap_or("").to_string();
    let mut entrees = vec![];
    let mut reglements = vec![];
    let mut pnl = 0.0;
    let (mut conf, mut contra) = (0u32, 0u32);
    for l in log.lines() {
        let l = strip(l);
        if let Some(i) = l.find("TAKER: ") {
            let x = &l[i..];
            entrees.push(serde_json::json!({
                "heure": heure(&l),
                "cote": if x.contains("TAKER: UP") { "UP" } else { "DOWN" },
                "z": champ(x, "z="), "p": champ(x, "p="), "ask": champ(x, "ask="),
                "avg": champ(x, "avg="), "tau": champ(x, "tau="),
                "dist": champ(x, "dist="), "mode": champ(x, "mode="),
            }));
        } else if let Some(i) = l.find("RÈGLEMENT ") {
            let x = &l[i..];
            let avec_position = !x.contains("taker=0 maker_fills=0");
            let pnl_fen: f64 = ["up=", "down="].iter()
                .filter_map(|k| champ(x, k).and_then(|v| v.parse::<f64>().ok()))
                .sum();
            reglements.push(serde_json::json!({
                "heure": heure(&l),
                "slug": x.split(' ').nth(1).unwrap_or(""),
                "issue": if x.contains("issue=Up") { "Up" } else if x.contains("issue=Down") { "Down" } else { "?" },
                "strike": champ(x, "strike=Some(\""),
                "position": avec_position,
                "pnl": pnl_fen,
            }));
            if let Some(j) = l.find("PnL cumulé=") {
                pnl = l[j + "PnL cumulé=".len()..].trim().parse().unwrap_or(pnl);
            }
        } else if let Some(i) = l.find("ENTRÉE EXÉCUTÉE: ") {
            if let Some(e) = entrees.last_mut() {
                let x = &l[i..];
                e["statut"] = serde_json::json!("exécutée");
                e["parts"] = serde_json::json!(champ(x, "EXÉCUTÉE: "));
                e["prix_reel"] = serde_json::json!(champ(x, "@ "));
            }
        } else if l.contains("ENTRÉE NON EXÉCUTÉE") || l.contains("ÉCHEC ORDRE") || l.contains("ORDRE REFUSÉ") {
            if let Some(e) = entrees.last_mut() {
                if e.get("statut").is_none() {
                    e["statut"] = serde_json::json!("non exécutée");
                }
            }
        } else if l.contains("CONFIRME") {
            conf += 1;
        } else if l.contains("CONTREDIT") {
            contra += 1;
        }
    }
    let n = reglements.len().saturating_sub(12);
    (entrees, reglements.split_off(n.min(reglements.len())), pnl, conf, contra, reel)
}

/// Presets du curseur de confiance (docs/LIGNE_EFFICIENCE.md).
/// (dist $, τ s, prix max, marge EV)
const PRESETS: [(&str, f64, f64, f64, f64); 3] = [
    ("prudent", 70.0, 120.0, 0.96, 0.015),
    ("standard", 70.0, 120.0, 0.98, 0.010),
    ("agressif", 40.0, 120.0, 0.98, 0.005),
];

fn preset_courant(cfg: &BotConfig) -> &'static str {
    if cfg.taker.zones_frontiere != 0 {
        return "avancé";
    }
    for (nom, d, t, p, m) in PRESETS {
        if (cfg.taker.dist_frontiere_usd - d).abs() < 1e-9
            && (cfg.taker.tau_frontiere_s - t).abs() < 1e-9
            && (cfg.taker.prix_max_frontiere - p).abs() < 1e-9
            && (cfg.taker.marge_ev - m).abs() < 1e-9
        {
            return nom;
        }
    }
    "personnalisé"
}

// ─── API ─────────────────────────────────────────────────────────────────

async fn api_etat(State(app): State<App>) -> Json<serde_json::Value> {
    let base = app.base.clone();
    let v = tokio::task::spawn_blocking(move || etat(&base))
        .await
        .unwrap_or_else(|e| serde_json::json!({"erreur": e.to_string()}));
    Json(v)
}

fn etat(base: &Path) -> serde_json::Value {
    let j = charger_journal(base);
    let (entrees, reglements, pnl, conf, contra, reel) = lire_run_log(base);
    let cfg = BotConfig::charger_ou_defaut(&base.join("config.toml")).unwrap_or_default();
    let calib = CalibTable::charger_ou_defaut(&base.join("data_v2/calibration.json"));
    let now = now_ms();
    let w = j.fenetres.last();
    let dernier_tick = j.ticks.last();
    // σ EWMA — même estimateur que le bot.
    let mut vol = VolEstimator::new(VolConfig::default());
    for t in &j.ticks {
        vol.push(t);
    }
    let sigma = vol.ewma_sigma_per_sqrt_s();
    // strike de la fenêtre courante — même code que le bot.
    let strike = w.and_then(|w| {
        let c = compute_strike(&j.ticks, w.t0_ms, StrikePolicy::LastAtOrBefore, DEFAULT_CONFIDENCE_GAP_MS);
        c.value.map(|v| (v, c.confidence))
    });
    let (spot, spot_age_s) = dernier_tick
        .map(|t| (t.price, (now.saturating_sub(t.source_ts_ms)) as f64 / 1000.0))
        .unwrap_or((0.0, f64::NAN));
    let tau = w.map(|w| (w.end_ms.saturating_sub(now)) as f64 / 1000.0).unwrap_or(0.0);
    let dist = strike.map(|(k, _)| spot - k).unwrap_or(0.0);
    let z = match (strike, sigma) {
        (Some((k, _)), Some(s)) if spot > 0.0 && k > 0.0 && tau > 0.0 => {
            (spot / k).ln() / (s.max(cfg.modele.sigma_floor_per_sqrt_s) * tau.max(1.0).sqrt())
        }
        _ => 0.0,
    };
    let p_brute = student_t_cdf(z, cfg.modele.student_nu);
    let p_prior = p_brute.max(1.0 - p_brute);
    let p_cal_fav = calib.p_win(dist.abs(), tau, p_prior);
    let p_cal_up = if dist >= 0.0 { p_cal_fav } else { 1.0 - p_cal_fav };
    let (bid_up, ask_up) = j.up.last().map(|&(_, b, a)| (b, a)).unwrap_or((f64::NAN, f64::NAN));
    let (bid_down, ask_down) = j.down.last().map(|&(_, b, a)| (b, a)).unwrap_or((f64::NAN, f64::NAN));
    let ask_fav = if dist >= 0.0 { ask_up } else { ask_down };
    let frais = cfg.taker.fee_rate * ask_fav * (1.0 - ask_fav);
    let ev = p_cal_fav - ask_fav - frais - cfg.taker.cost_buffer;
    let dans_frontiere = dist.abs() >= cfg.taker.dist_frontiere_usd
        && tau <= cfg.taker.tau_frontiere_s
        && tau >= cfg.taker.min_tau_s;
    serde_json::json!({
        "maintenant_ms": now,
        "fenetre": w,
        "tau_s": tau,
        "spot": spot,
        "spot_age_s": spot_age_s,
        "spot_binance": j.spot_binance,
        "strike": strike.map(|(v, _)| v),
        "strike_confidence": strike.map(|(_, c)| c),
        "dist_usd": dist,
        "sigma_par_sqrt_s": sigma,
        "z": z,
        "p_brute_up": p_brute,
        "p_calibree_up": p_cal_up,
        "effectif_bac": calib.effectif(dist.abs(), tau),
        "carnets": {"bid_up": bid_up, "ask_up": ask_up, "bid_down": bid_down, "ask_down": ask_down},
        "ev_favori": ev,
        "dans_frontiere": dans_frontiere,
        "frontiere": {"dist_usd": cfg.taker.dist_frontiere_usd, "tau_s": cfg.taker.tau_frontiere_s,
                       "prix_max": cfg.taker.prix_max_frontiere, "marge_ev": cfg.taker.marge_ev},
        "flux": {
            "oracle_ms": (j.dernier_oracle_ms > 0).then(|| now.saturating_sub(j.dernier_oracle_ms)),
            "relais_ms": (j.dernier_relais_ms > 0).then(|| now.saturating_sub(j.dernier_relais_ms)),
            "clob_ms": (j.dernier_clob_ms > 0).then(|| now.saturating_sub(j.dernier_clob_ms)),
            "binance_ms": (j.dernier_binance_ms > 0).then(|| now.saturating_sub(j.dernier_binance_ms)),
        },
        "bot": {
            "actif": now.saturating_sub(j.dernier_clob_ms.max(j.dernier_oracle_ms)) < 30_000,
            "reel": reel,
            "mode_valeur": cfg.taker.mode_valeur,
            "preset": preset_courant(&cfg),
            "zones_frontiere": cfg.taker.zones_frontiere.to_string(),
            "prix_max_frontiere": cfg.taker.prix_max_frontiere,
            "marge_ev": cfg.taker.marge_ev,
        },
        "pnl_tranche": pnl,
        "resolutions": {"confirmees": conf, "contredites": contra},
        "entrees": entrees,
        "reglements": reglements,
        "calibration": {"fenetres": calib.windows_observed, "observations": calib.total_observations()},
    })
}

async fn api_series(State(app): State<App>) -> Json<serde_json::Value> {
    let base = app.base.clone();
    let v = tokio::task::spawn_blocking(move || series(&base))
        .await
        .unwrap_or_else(|e| serde_json::json!({"erreur": e.to_string()}));
    Json(v)
}

/// Bougies 10 s (OHLC) + σ EWMA échantillonné + séries carnets.
fn series(base: &Path) -> serde_json::Value {
    let j = charger_journal(base);
    let mut bougies: Vec<[f64; 5]> = vec![]; // [t_s, o, h, l, c]
    let mut vol_serie: Vec<[f64; 2]> = vec![];
    let mut vol = VolEstimator::new(VolConfig::default());
    for t in &j.ticks {
        vol.push(t);
        let cell = (t.source_ts_ms / 10_000 * 10) as f64;
        match bougies.last_mut() {
            Some(b) if b[0] == cell => {
                b[2] = b[2].max(t.price);
                b[3] = b[3].min(t.price);
                b[4] = t.price;
            }
            _ => {
                bougies.push([cell, t.price, t.price, t.price, t.price]);
                if let Some(s) = vol.ewma_sigma_per_sqrt_s() {
                    vol_serie.push([cell, s]);
                }
            }
        }
    }
    let ech = |v: &[(u64, f64, f64)]| -> Vec<[f64; 3]> {
        let mut out: Vec<[f64; 3]> = vec![];
        for &(t, b, a) in v {
            let cell = (t / 2_000 * 2) as f64;
            match out.last_mut() {
                Some(x) if x[0] == cell => {
                    x[1] = b;
                    x[2] = a;
                }
                _ => out.push([cell, b, a]),
            }
        }
        out
    };
    serde_json::json!({
        "bougies": bougies,
        "vol": vol_serie,
        "up": ech(&j.up),
        "down": ech(&j.down),
        "fenetres": j.fenetres,
    })
}

async fn api_calibration(State(app): State<App>) -> Json<serde_json::Value> {
    let t = CalibTable::charger_ou_defaut(&app.base.join("data_v2/calibration.json"));
    Json(serde_json::json!({
        "dist_bins": pm_strategy::calib::DIST_BINS,
        "tau_bins": pm_strategy::calib::TAU_BINS,
        "cells": t.cells,
        "prior_strength": t.prior_strength,
        "fenetres": t.windows_observed,
    }))
}

async fn api_config_get(State(app): State<App>) -> Json<serde_json::Value> {
    let path = app.base.join("config.toml");
    let effective = BotConfig::charger_ou_defaut(&path)
        .map(|c| c.en_toml())
        .unwrap_or_else(|e| format!("# erreur: {e}"));
    Json(serde_json::json!({
        "existe": path.exists(),
        "toml": effective,
    }))
}

async fn api_config_post(
    State(app): State<App>,
    body: String,
) -> (StatusCode, Json<serde_json::Value>) {
    // Validation par LE parseur du bot (clés inconnues = refus).
    match toml::from_str::<BotConfig>(&body) {
        Ok(_) => {
            let path = app.base.join("config.toml");
            if path.exists() {
                let _ = std::fs::copy(&path, app.base.join("config.toml.bak"));
            }
            match std::fs::write(&path, &body) {
                Ok(()) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "ok": true,
                        "note": "écrit dans config.toml (sauvegarde .bak) — appliqué au prochain (re)démarrage du bot"
                    })),
                ),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"ok": false, "erreur": e.to_string()})),
                ),
            }
        }
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"ok": false, "erreur": format!("TOML invalide : {e}")})),
        ),
    }
}

/// Applique un preset du curseur de confiance : réécrit les 4 clefs de la
/// frontière dans config.toml (le reste de la config effective est
/// conservé). Même sémantique que l'éditeur : appliqué au prochain
/// (re)démarrage.
async fn api_mode(
    State(app): State<App>,
    body: String,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some((_, d, t, p, m)) = PRESETS.iter().find(|(n, ..)| *n == body.trim()) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"ok": false, "erreur": "preset inconnu (prudent|standard|agressif)"})),
        );
    };
    let path = app.base.join("config.toml");
    let mut cfg = match BotConfig::charger_ou_defaut(&path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok": false, "erreur": e.to_string()})),
            )
        }
    };
    cfg.taker.dist_frontiere_usd = *d;
    cfg.taker.tau_frontiere_s = *t;
    cfg.taker.prix_max_frontiere = *p;
    cfg.taker.marge_ev = *m;
    cfg.taker.zones_frontiere = 0; // un preset = retour à la boîte
    if path.exists() {
        let _ = std::fs::copy(&path, app.base.join("config.toml.bak"));
    }
    match std::fs::write(&path, cfg.en_toml()) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "preset": body.trim(),
                "note": "frontière mise à jour dans config.toml — appliqué au prochain (re)démarrage"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "erreur": e.to_string()})),
        ),
    }
}

/// MODE AVANCÉ : applique un masque de cellules (choisi sur la table de
/// calibration dans l'UI) + prix max + marge. Corps JSON :
/// {"zones": "u64", "prix_max": f64, "marge_ev": f64}
async fn api_avance(
    State(app): State<App>,
    body: String,
) -> (StatusCode, Json<serde_json::Value>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
        return (StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"ok": false, "erreur": "JSON invalide"})));
    };
    let zones: u64 = v["zones"].as_str().and_then(|z| z.parse().ok()).unwrap_or(0);
    let prix_max = v["prix_max"].as_f64().unwrap_or(0.98);
    let marge = v["marge_ev"].as_f64().unwrap_or(0.01);
    if zones == 0 {
        return (StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"ok": false, "erreur": "aucune cellule sélectionnée"})));
    }
    if !(0.5..=0.99).contains(&prix_max) || !(0.0..=0.5).contains(&marge) {
        return (StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"ok": false, "erreur": "prix_max ∈ [0,50;0,99] et marge_ev ∈ [0;0,5] requis"})));
    }
    let path = app.base.join("config.toml");
    let mut cfg = match BotConfig::charger_ou_defaut(&path) {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR,
                          Json(serde_json::json!({"ok": false, "erreur": e.to_string()}))),
    };
    cfg.taker.zones_frontiere = zones;
    cfg.taker.prix_max_frontiere = prix_max;
    cfg.taker.marge_ev = marge;
    if path.exists() {
        let _ = std::fs::copy(&path, app.base.join("config.toml.bak"));
    }
    match std::fs::write(&path, cfg.en_toml()) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok": true,
            "note": format!("mode avancé écrit ({} cellule(s), prix ≤ {prix_max}, marge {marge}) — appliqué au prochain (re)démarrage",
                            zones.count_ones())}))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR,
                   Json(serde_json::json!({"ok": false, "erreur": e.to_string()}))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_invalide_est_refusee() {
        assert!(toml::from_str::<BotConfig>("[taker]\nbankrol = 1.0\n").is_err());
        assert!(toml::from_str::<BotConfig>("[taker]\nbankroll = 500.0\n").is_ok());
    }

    #[test]
    fn queue_journal_sur_fichier_absent() {
        assert!(lire_queue(Path::new("/nonexistent/x.ndjson")).is_empty());
    }

    #[test]
    fn etat_sans_donnees_ne_panique_pas() {
        let dir = std::env::temp_dir().join("pmdash_test");
        std::fs::create_dir_all(dir.join("data_v2")).unwrap();
        let v = etat(&dir);
        assert!(v.get("tau_s").is_some());
        std::fs::remove_dir_all(&dir).ok();
    }
}
