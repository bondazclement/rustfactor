//! pm-bot — orchestrateur : acquisition → état → stratégies → exécution.
//!
//! Mode par défaut : PAPER (DryRunGateway). Les ordres sont journalisés,
//! jamais envoyés. Le passage en réel exige la feature `live` de pm-execution
//! et une validation préalable sur archives (voir docs/ARCHITECTURE.md).
//!
//! ```text
//! pm-bot [--out DIR] [--no-taker] [--maker]
//! ```
//!
//! Le maker est DÉSACTIVÉ par défaut : le backtest du 2026-07-04 (24
//! fenêtres) montre qu'il perd sur toute la grille de calibration
//! (sélection adverse des bids au repos) — re-design nécessaire avant
//! réactivation. Le taker, lui, est positif sur toute la grille.

use anyhow::Result;
use pm_acquisition::{clob, gamma::GammaClient, rtds, watchdog::Watchdog, Bus, Recorder};
use pm_core::book::OrderBook;
use pm_core::strike::{compute_strike, StrikePolicy, DEFAULT_CONFIDENCE_GAP_MS};
use pm_core::vol::{VolConfig, VolEstimator};
use pm_core::{BusEvent, ClobEvent, MarketWindow, ResolutionTick};
use pm_execution::risk::{RiskConfig, RiskGate};
use pm_execution::{DryRunGateway, OrderAck, OrderGateway, OrderRequest, OrderSide, TimeInForce};

/// Passerelle effective : paper par défaut ; réelle (sous garde-fous
/// de risque) uniquement avec `--features live` + `--live` + PM_LIVE_ARME=oui.
enum Passerelle {
    Paper(RiskGate<DryRunGateway>),
    #[cfg(feature = "live")]
    Reelle(RiskGate<pm_execution::live::LiveGateway>),
}

impl Passerelle {
    async fn post_order(&self, req: OrderRequest) -> anyhow::Result<OrderAck> {
        match self {
            Passerelle::Paper(g) => g.post_order(req).await,
            #[cfg(feature = "live")]
            Passerelle::Reelle(g) => g.post_order(req).await,
        }
    }

    fn declencher_arret(&self, raison: &str) {
        match self {
            Passerelle::Paper(g) => g.declencher_arret(raison),
            #[cfg(feature = "live")]
            Passerelle::Reelle(g) => g.declencher_arret(raison),
        }
    }

    fn signaler_pnl(&self, pnl: f64) {
        match self {
            Passerelle::Paper(g) => g.signaler_pnl(pnl),
            #[cfg(feature = "live")]
            Passerelle::Reelle(g) => g.signaler_pnl(pnl),
        }
    }
}
use pm_strategy::maker::{Inventory, MakerContext, MakerStrategy, QuoteAction};
use pm_strategy::paper::{PaperBroker, RestingQuote};
use pm_strategy::taker::TakerStrategy;
use pm_strategy::{MarketSnapshot, ProbModel};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time;

#[derive(Debug, Clone)]
struct Args {
    out_dir: PathBuf,
    config_path: Option<PathBuf>,
    taker_enabled: bool,
    maker_enabled: bool,
    /// Exécution RÉELLE (exige la feature `live`, PM_LIVE_ARME=oui, et les
    /// clés d'environnement). Sans tout ça : refus au démarrage.
    live: bool,
    // Surcharges CLI (appliquées APRÈS le fichier de config).
    max_entry_price: Option<f64>,
    min_abs_z: Option<f64>,
    bankroll: Option<f64>,
    max_notional: Option<f64>,
    kelly_fraction: Option<f64>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        out_dir: PathBuf::from("./data_v2"),
        config_path: None,
        taker_enabled: true,
        maker_enabled: false,
        live: false,
        max_entry_price: None,
        min_abs_z: None,
        bankroll: None,
        max_notional: None,
        kelly_fraction: None,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--out" => {
                i += 1;
                args.out_dir = PathBuf::from(
                    argv.get(i)
                        .ok_or_else(|| anyhow::anyhow!("--out: valeur manquante"))?,
                );
            }
            "--no-taker" => args.taker_enabled = false,
            "--maker" => args.maker_enabled = true,
            "--live" => args.live = true,
            "--config" => {
                i += 1;
                args.config_path = Some(PathBuf::from(
                    argv.get(i).ok_or_else(|| anyhow::anyhow!("--config: chemin manquant"))?,
                ));
            }
            "--bankroll" => {
                i += 1;
                args.bankroll = Some(argv.get(i).ok_or_else(|| anyhow::anyhow!("--bankroll: valeur manquante"))?.parse()?);
            }
            "--max-notional" => {
                i += 1;
                args.max_notional = Some(argv.get(i).ok_or_else(|| anyhow::anyhow!("--max-notional: valeur manquante"))?.parse()?);
            }
            "--kelly" => {
                i += 1;
                args.kelly_fraction = Some(argv.get(i).ok_or_else(|| anyhow::anyhow!("--kelly: valeur manquante"))?.parse()?);
            }
            "--max-entry" => {
                i += 1;
                args.max_entry_price = Some(
                    argv.get(i)
                        .ok_or_else(|| anyhow::anyhow!("--max-entry: valeur manquante"))?
                        .parse()?,
                );
            }
            "--min-z" => {
                i += 1;
                args.min_abs_z = Some(
                    argv.get(i)
                        .ok_or_else(|| anyhow::anyhow!("--min-z: valeur manquante"))?
                        .parse()?,
                );
            }
            "--no-maker" => args.maker_enabled = false,
            "--help" | "-h" => {
                println!("pm-bot [--out DIR] [--config FICHIER.toml] [--no-taker] [--maker]");
                println!("       [--max-entry X] [--min-z X] [--bankroll X] [--max-notional X] [--kelly X]");
                println!("Sans --config, ./config.toml est chargé s'il existe. Toutes les clefs :");
                println!("voir config.exemple.toml et docs/CONFIGURATION.md.");
                std::process::exit(0);
            }
            other => anyhow::bail!("argument inconnu: {other}"),
        }
        i += 1;
    }
    Ok(args)
}

/// Le build live embarque DEUX fournisseurs crypto rustls (ring via nos
/// WebSockets, aws-lc-rs via le SDK Polymarket) : rustls exige alors un
/// choix explicite AVANT la première connexion TLS, sinon panique
/// (constaté au premier lancement réel du 06/07 21:50).
#[cfg(feature = "live")]
fn installer_crypto() {
    if rustls::crypto::ring::default_provider().install_default().is_err() {
        tracing::warn!("fournisseur crypto rustls déjà installé");
    }
}
#[cfg(not(feature = "live"))]
fn installer_crypto() {}

fn now_ms() -> u64 {
    pm_acquisition::now_ms()
}

/// État interne de la fenêtre courante.
struct Engine {
    window: Option<MarketWindow>,
    /// Historique des ticks de résolution (rétention 40 min, partagé entre
    /// fenêtres : indispensable pour le strike ET la vol des 30 dernières min).
    ticks: VecDeque<ResolutionTick>,
    vol: VolEstimator,
    book_up: OrderBook,
    book_down: OrderBook,
    strike_frozen: bool,
    strike: Option<pm_core::strike::StrikeComputation>,
    /// Fenêtres réglées, en attente de confirmation market_resolved :
    /// (slug, issue estimée up?, token_up, token_down).
    settled: Vec<(String, Option<bool>, String, String)>,
    retention_ms: u64,
    /// Une résolution officielle a contredit notre estimation (✗) :
    /// le moteur déclenche le kill-switch au prochain tour de boucle.
    contradiction: bool,
}

impl Engine {
    fn new_avec(vol_cfg: VolConfig, retention_s: u64) -> Self {
        Self {
            window: None,
            ticks: VecDeque::with_capacity(8192),
            vol: VolEstimator::new(vol_cfg),
            book_up: OrderBook::new(),
            book_down: OrderBook::new(),
            strike_frozen: false,
            strike: None,
            settled: Vec::new(),
            retention_ms: retention_s * 1000,
            contradiction: false,
        }
    }

    /// Règle la fenêtre précédente dans le broker paper : issue estimée par
    /// « dernier tick ≤ fin » vs strike (confirmée ensuite par market_resolved).
    fn settle_previous(&mut self, broker: &mut PaperBroker) -> Option<bool> {
        let Some(prev) = self.window.clone() else {
            return None;
        };
        let strike = self.strike.as_ref().and_then(|s| s.value);
        let final_tick = self
            .ticks
            .iter()
            .filter(|t| t.source_ts_ms <= prev.end_ms)
            .max_by_key(|t| t.source_ts_ms)
            .copied();
        // Pas d'issue estimée si le strike n'est pas fiable (fenêtre de
        // démarrage/panne) : un ✗ doit rester un vrai signal d'alerte.
        let strike_fiable = self.strike.as_ref().is_some_and(|s| s.confidence >= 0.8);
        let up_won = match (strike, final_tick) {
            (Some(k), Some(t)) if strike_fiable => Some(t.price > k),
            _ => None,
        };
        let report = broker.settle_window(
            &prev.slug,
            &prev.token_up,
            &prev.token_down,
            strike,
            up_won,
            up_won.map(|u| {
                if u {
                    "Up (estimé)".into()
                } else {
                    "Down (estimé)".into()
                }
            }),
        );
        self.settled.push((
            prev.slug.clone(),
            up_won,
            prev.token_up.clone(),
            prev.token_down.clone(),
        ));
        tracing::info!(
            "RÈGLEMENT {} | strike={:?} issue={} | PnL up={:+.2} down={:+.2} | taker={} maker_fills={} | PnL cumulé={:+.2}",
            report.slug,
            report.strike.map(|v| format!("{v:.2}")),
            report.outcome.as_deref().unwrap_or("inconnue"),
            report.pnl_up,
            report.pnl_down,
            report.taker_entries,
            report.maker_fills,
            broker.total_pnl(),
        );
        up_won
    }

    fn on_window(&mut self, w: MarketWindow) {
        tracing::info!("fenêtre active: {} [{} → {}]", w.slug, w.start_ms, w.end_ms);
        self.book_up = OrderBook::new();
        self.book_down = OrderBook::new();
        self.strike_frozen = false;
        self.strike = None;
        self.window = Some(w);
        self.refresh_strike();
    }

    fn on_resolution_tick(&mut self, t: ResolutionTick) {
        self.ticks.push_back(t);
        // Rétention configurable ([moteur] retention_ticks_s) sur l'horloge source.
        let cutoff = t.source_ts_ms.saturating_sub(self.retention_ms);
        while self.ticks.front().is_some_and(|x| x.source_ts_ms < cutoff) {
            self.ticks.pop_front();
        }
        self.vol.push(&t);
        if !self.strike_frozen {
            self.refresh_strike();
        }
    }

    /// Recalcule le strike tant qu'aucun tick ≥ T0 n'a été vu ; dès qu'un tick
    /// après la frontière existe, `LastAtOrBefore` ne peut plus changer → gel.
    fn refresh_strike(&mut self) {
        let Some(w) = &self.window else { return };
        let ticks: Vec<ResolutionTick> = self.ticks.iter().copied().collect();
        let comp = compute_strike(
            &ticks,
            w.start_ms,
            StrikePolicy::LastAtOrBefore,
            DEFAULT_CONFIDENCE_GAP_MS,
        );
        if comp.after.is_some() {
            self.strike_frozen = true;
            tracing::info!(
                "strike gelé: {:?} (confidence={:.3}, gap={:?} ms)",
                comp.value,
                comp.confidence,
                comp.used_gap_ms
            );
        }
        self.strike = Some(comp);
    }

    fn on_clob(&mut self, ev: &ClobEvent, recv_ms: u64) {
        let Some(w) = &self.window else { return };
        match ev {
            ClobEvent::Book {
                asset_id,
                ts_ms,
                bids,
                asks,
            } => {
                let book = if *asset_id == w.token_up {
                    &mut self.book_up
                } else if *asset_id == w.token_down {
                    &mut self.book_down
                } else {
                    return;
                };
                book.apply_snapshot(bids, asks, *ts_ms, recv_ms);
            }
            ClobEvent::PriceChange { ts_ms, changes } => {
                for ch in changes {
                    if ch.asset_id == w.token_up {
                        self.book_up.apply_delta(ch, *ts_ms, recv_ms);
                    } else if ch.asset_id == w.token_down {
                        self.book_down.apply_delta(ch, *ts_ms, recv_ms);
                    }
                }
            }
            ClobEvent::MarketResolved {
                slug,
                winning_asset_id,
                winning_outcome,
                ..
            } => {
                // Le slug est parfois vide sur ces événements (observé au run
                // v2) : on matche par token id, toujours présent.
                let hit = self
                    .settled
                    .iter()
                    .find(|(_, _, up, down)| up == winning_asset_id || down == winning_asset_id);
                match hit {
                    Some((wslug, Some(est_up), up_token, _)) => {
                        let official_up = winning_asset_id == up_token;
                        if *est_up == official_up {
                            tracing::info!(
                                "résolution officielle {wslug}: {winning_outcome} — CONFIRME notre estimation ✓"
                            );
                        } else {
                            tracing::error!(
                                "résolution officielle {wslug}: {winning_outcome} — CONTREDIT notre estimation ✗ (à investiguer)"
                            );
                            self.contradiction = true;
                        }
                    }
                    Some((wslug, None, _, _)) => tracing::info!(
                        "résolution officielle {wslug}: {winning_outcome} (pas d'estimation locale)"
                    ),
                    None => tracing::info!(
                        "résolution officielle {slug}: {winning_outcome} (fenêtre non suivie)"
                    ),
                }
            }
            _ => {}
        }
    }

    fn snapshot(&self, now_ms: u64, any_feed_stale: bool) -> Option<MarketSnapshot> {
        let w = self.window.as_ref()?;
        if now_ms < w.start_ms || now_ms >= w.end_ms {
            return None; // fenêtre pas encore ouverte / déjà résolue
        }
        let strike = self.strike.clone()?;
        let last = self.ticks.back()?;
        Some(MarketSnapshot {
            now_ms,
            t0_ms: w.start_ms,
            t_end_ms: w.end_ms,
            strike,
            spot: last.price,
            spot_source_ts_ms: last.source_ts_ms,
            sigma_per_sqrt_s: self.vol.ewma_sigma_per_sqrt_s(),
            drift_per_s: self.vol.realized_drift_per_s(120, last.source_ts_ms),
            book_up: self.book_up.clone(),
            book_down: self.book_down.clone(),
            any_feed_stale,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    installer_crypto();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = parse_args()?;
    tracing::info!(
        "pm-bot démarre (PAPER mode) — archives: {}",
        args.out_dir.display()
    );

    let bus = Bus::default();
    let recorder = Recorder::spawn(args.out_dir.clone());
    // Configuration : fichier (--config, sinon ./config.toml) puis surcharges CLI.
    let cfg_path = args
        .config_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("./config.toml"));
    let mut cfg = pm_strategy::config::BotConfig::charger_ou_defaut(&cfg_path)?;
    if cfg_path.exists() {
        tracing::info!("configuration chargée depuis {}", cfg_path.display());
    } else {
        tracing::info!("aucun fichier de config ({}) — défauts calibrés", cfg_path.display());
    }
    if let Some(v) = args.max_entry_price { cfg.taker.max_entry_price = v; }
    if let Some(v) = args.min_abs_z { cfg.taker.min_abs_z = v; }
    if let Some(v) = args.bankroll { cfg.taker.bankroll = v; }
    if let Some(v) = args.max_notional { cfg.taker.max_notional = v; }
    if let Some(v) = args.kelly_fraction { cfg.taker.kelly_fraction = v; }
    // Transparence totale : la config EFFECTIVE est journalisée et archivée.
    for line in cfg.en_toml().lines() {
        tracing::info!("[config] {line}");
    }
    recorder.record("meta", cfg.en_toml(), now_ms());

    let arme = std::env::var("PM_LIVE_ARME").map(|v| v == "oui").unwrap_or(false);
    let gateway = if args.live {
        #[cfg(feature = "live")]
        {
            anyhow::ensure!(arme, "--live exige PM_LIVE_ARME=oui (double opt-in)");
            let lg = pm_execution::live::LiveGateway::depuis_env().await?;
            let solde = lg.solde_collateral().await?;
            // Plafonds de risque : défauts micro, surchargés par env.
            let env_f = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
            let risk = RiskConfig {
                max_notional_par_ordre: env_f("PM_MAX_ORDRE_USD", 10.0),
                max_ordres_session: env_f("PM_MAX_ORDRES", 20.0) as u32,
                perte_max_session: env_f("PM_PERTE_MAX_USD", 20.0),
                // ≥ limit_price max du taker (avg+0.01, plafonné 0.99) : le
                // RiskGate borne la folie, pas la stratégie légitime.
                prix_max: 0.99,
            };
            tracing::warn!(
                "MODE RÉEL ARMÉ — solde {solde:.2} $ | plafonds : {:.2} $/ordre, {} ordres, perte max {:.2} $",
                risk.max_notional_par_ordre, risk.max_ordres_session, risk.perte_max_session
            );
            anyhow::ensure!(solde > 1.0, "solde collatéral insuffisant ({solde:.2} $)");
            anyhow::ensure!(
                risk.perte_max_session <= solde,
                "perte max ({:.2} $) > solde ({solde:.2} $) — ajustez PM_PERTE_MAX_USD",
                risk.perte_max_session
            );
            Passerelle::Reelle(RiskGate::new(lg, risk, true))
        }
        #[cfg(not(feature = "live"))]
        {
            anyhow::bail!("--live exige une compilation avec --features live");
        }
    } else {
        // Paper : mêmes garde-fous de risque (testés en continu), armés.
        Passerelle::Paper(RiskGate::new(DryRunGateway::new(), RiskConfig {
            max_notional_par_ordre: cfg.taker.max_notional,
            max_ordres_session: 10_000,
            perte_max_session: f64::INFINITY,
            prix_max: 0.99,
        }, true))
    };

    let watchdog = Watchdog::new(cfg.moteur.watchdog_stale_ms);
    // Flux de résolution : connexion continue, indépendante des fenêtres.
    tokio::spawn(rtds::run(bus.clone(), recorder.clone()));
    // Capture Binance directe (archivage seul — étude lead-lag, cf. binance.rs).
    tokio::spawn(pm_acquisition::binance::run(recorder.clone()));
    tokio::spawn(watchdog.clone().run(bus.clone()));

    // Découverte des fenêtres + rotation du flux CLOB.
    {
        let bus = bus.clone();
        let recorder = recorder.clone();
        let lookahead = cfg.moteur.gamma_lookahead;
        let clob_grace_ms = cfg.moteur.clob_grace_s * 1000;
        tokio::spawn(async move {
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .user_agent("pm-bot/0.1")
                .build()
                .expect("client http");
            let gamma = GammaClient::new(http);
            let mut current_slug = String::new();
            loop {
                match gamma.find_active_window(now_ms() / 1000, lookahead).await {
                    Ok(w) => {
                        if w.slug != current_slug {
                            current_slug = w.slug.clone();
                            // Journalise la fenêtre (méta) et bascule le CLOB.
                            recorder.record(
                                "gamma",
                                serde_json::to_string(&w).unwrap_or_default(),
                                now_ms(),
                            );
                            bus.publish(BusEvent::WindowChanged(w.clone()));
                            // Connexions chevauchantes : l'ancienne tâche CLOB vit
                            // jusqu'à market_resolved (ou sa deadline) — c'est elle
                            // qui capture la résolution officielle.
                            tokio::spawn(clob::run_for_tokens(
                                bus.clone(),
                                recorder.clone(),
                                w.token_up.clone(),
                                w.token_down.clone(),
                                w.end_ms + clob_grace_ms,
                            ));
                        }
                        let wait = w.end_ms.saturating_sub(now_ms()) + 2_000;
                        time::sleep(Duration::from_millis(wait.min(310_000))).await;
                    }
                    Err(e) => {
                        tracing::warn!("découverte gamma: {e:#}");
                        time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }

    // Boucle moteur : état + décisions.
    let mut engine = Engine::new_avec(cfg.volatilite, cfg.moteur.retention_ticks_s);
    let taker = TakerStrategy::new(cfg.taker);
    let maker = MakerStrategy::new(cfg.maker);
    let model = ProbModel::new(cfg.modele);
    // Calibration auto-apprise : persistée, mise à jour à chaque règlement.
    let calib_path = std::env::var("PM_CALIB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| args.out_dir.join("calibration.json"));
    let mut calib = pm_strategy::calib::CalibTable::charger_ou_defaut(&calib_path);
    let mut pending = pm_strategy::calib::FenetrePending::default();
    tracing::info!(
        "calibration: {:.0} observations / {} fenêtres ({})",
        calib.total_observations(), calib.windows_observed, calib_path.display()
    );
    let mut rx = bus.subscribe();
    let mut decide_tick = time::interval(Duration::from_millis(cfg.moteur.decision_step_ms));
    decide_tick.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    let mut broker = PaperBroker::new();

    // Cycle de décision : partagé entre le déclencheur ÉVÉNEMENTIEL (chaque
    // tick d'oracle — latence minimale, audit du 06/07 : la minuterie seule
    // gaspillait jusqu'à 250 ms) et la minuterie de repli (changements de
    // carnet entre deux ticks).
    macro_rules! cycle_decision {
        () => {{
                if engine.contradiction {
                    engine.contradiction = false;
                    gateway.declencher_arret("contradiction de résolution ✗ — chaîne de données suspecte");
                }
                let stale = watchdog.is_stale("rtds") || watchdog.is_stale("clob");
                let Some(snap) = engine.snapshot(now_ms(), stale) else { continue };
                let mut est = model.estimate(&snap);
                if est.reliable {
                    pending.observer(est.dist_usd, est.tau_s);
                }
                model.calibrer(&mut est, &calib);

                if args.taker_enabled {
                    if let Some(d) = taker.decide(&snap, &est) {
                        let w = engine.window.as_ref().unwrap();
                        let token = if d.buy_up { &w.token_up } else { &w.token_down };
                        // Une seule entrée par fenêtre et par token (anti-répétition).
                        if broker.can_take(token) {
                            tracing::info!("TAKER: {} ({})", if d.buy_up { "UP" } else { "DOWN" }, d.reason);
                            // La position n'est comptée que sur EXÉCUTION
                            // réelle confirmée (leçon du micro-test 06/07 :
                            // 1 décision sur 3 exécutée ; erreurs avalées).
                            match gateway.post_order(OrderRequest {
                                token_id: token.clone(),
                                side: OrderSide::Buy,
                                price: d.limit_price,
                                size: d.size,
                                tif: TimeInForce::Fak,
                                tag: format!("taker edge={:.3}", d.edge),
                            }).await {
                                Ok(ack) if ack.taille_executee > 0.0 => {
                                    let prix = ack.prix_reel.unwrap_or(d.avg_price);
                                    tracing::info!(
                                        "ENTRÉE EXÉCUTÉE: {:.2} parts @ {:.4} ({})",
                                        ack.taille_executee, prix, ack.detail
                                    );
                                    broker.fill_taker_avec_frais(token, prix, ack.taille_executee, cfg.taker.fee_rate);
                                }
                                Ok(ack) => tracing::warn!(
                                    "ENTRÉE NON EXÉCUTÉE: {} — aucune position prise",
                                    if ack.detail.is_empty() { "FAK tué (0 part)".into() } else { ack.detail.clone() }
                                ),
                                Err(e) => tracing::error!("ÉCHEC ORDRE: {e:#} — aucune position prise"),
                            }
                        }
                    }
                }
                if args.maker_enabled {
                    let w = engine.window.as_ref().unwrap().clone();
                    for is_up in [true, false] {
                        let token = if is_up { &w.token_up } else { &w.token_down };
                        let other = if is_up { &w.token_down } else { &w.token_up };
                        let pos = broker.position(token);
                        let ctx = MakerContext {
                            inv: Inventory { position: pos.size, avg_entry: pos.avg_entry },
                            other_position: broker.position(other).size,
                            window_frozen: broker.is_window_frozen(),
                        };
                        let d = maker.decide_token(&snap, &est, is_up, ctx);
                        if d.stop_triggered {
                            broker.freeze_window();
                        }
                        let mut new_bid: Option<RestingQuote> = None;
                        let mut new_ask: Option<RestingQuote> = None;
                        for action in &d.actions {
                            match action {
                                QuoteAction::Bid { price, size } => {
                                    new_bid = Some(RestingQuote { price: *price, size: *size });
                                }
                                QuoteAction::Ask { price, size } => {
                                    new_ask = Some(RestingQuote { price: *price, size: *size });
                                }
                                QuoteAction::ExitNow { limit_price, size } => {
                                    let _ = gateway.post_order(OrderRequest {
                                        token_id: token.clone(), side: OrderSide::Sell,
                                        price: *limit_price, size: *size, tif: TimeInForce::Fak,
                                        tag: format!("maker exit {}", d.reason),
                                    }).await;
                                    broker.exit_now(token, *limit_price);
                                }
                            }
                        }
                        // Remplacement idempotent : on ne journalise que les changements.
                        if broker.set_quotes(token, new_bid, new_ask) {
                            tracing::info!(
                                "MAKER {} quotes: bid={:?} ask={:?} ({})",
                                if is_up { "UP" } else { "DOWN" },
                                new_bid.map(|q| (q.price, q.size)),
                                new_ask.map(|q| (q.price, q.size)),
                                d.reason
                            );
                        }
                    }
                }
        }};
    }

    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Ok(BusEvent::WindowChanged(w)) => {
                        #[cfg(feature = "live")]
                        if let Passerelle::Reelle(g) = &gateway {
                            g.interieur().prechauffer(&[w.token_up.clone(), w.token_down.clone()]).await;
                        }
                        let pnl_avant = broker.total_pnl();
                        if let Some(up_won) = engine.settle_previous(&mut broker) {
                            calib.regler_fenetre(&pending, up_won);
                            if let Err(e) = calib.sauver(&calib_path) {
                                tracing::warn!("sauvegarde calibration: {e:#}");
                            }
                        }
                        pending.clear();
                        gateway.signaler_pnl(broker.total_pnl() - pnl_avant);
                        engine.on_window(w);
                    }
                    Ok(BusEvent::Resolution(t)) => {
                        watchdog.touch("rtds");
                        engine.on_resolution_tick(t);
                        // Décision immédiate sur information fraîche (événementiel).
                        cycle_decision!();
                    }
                    Ok(BusEvent::Fast(_)) => watchdog.touch("rtds_fast"),
                    Ok(BusEvent::Clob(ev)) => {
                        watchdog.touch("clob");
                        if let ClobEvent::LastTrade { asset_id, price, side, .. } = &ev {
                            let (bought, sold) = broker.on_market_trade(asset_id, *price, *side);
                            if bought || sold {
                                tracing::info!(
                                    "PAPER fill maker {} sur trade @{:.3} (achat={} vente={})",
                                    &asset_id[..8.min(asset_id.len())], price, bought, sold
                                );
                            }
                        }
                        engine.on_clob(&ev, now_ms());
                    }
                    Ok(BusEvent::FeedStale { stream, silent_ms }) => {
                        tracing::warn!("FEED STALE: {stream} silencieux {silent_ms} ms");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("bus en retard: {n} événements perdus côté moteur");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = decide_tick.tick() => {
                cycle_decision!();
            }
        }
    }
    Ok(())
}
