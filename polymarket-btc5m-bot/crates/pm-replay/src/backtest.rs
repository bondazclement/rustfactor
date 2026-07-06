//! pm-backtest — rejoue les journaux NDJSON v2 contre les stratégies.
//!
//! Reproduit la sémantique du moteur live (pm-bot) : mêmes types, mêmes
//! parseurs, même PaperBroker, horloge simulée = `recv_ms` des trames,
//! évaluation des décisions toutes les 250 ms de temps journal.
//!
//! ```text
//! pm-backtest --journal j1.ndjson [j2 ...] [--no-taker|--no-maker]
//!             [--tp X] [--stop X] [--margin X] [--tau-open S] [--tau-flat S]
//!             [--quote-size N] [--max-z X] [--quiet]
//! pm-backtest --journal ... --grid       # balayage de calibration maker
//! ```

use anyhow::{bail, Context, Result};
use pm_core::book::OrderBook;
use pm_core::strike::{compute_strike, StrikeComputation, StrikePolicy, DEFAULT_CONFIDENCE_GAP_MS};
use pm_core::vol::{VolConfig, VolEstimator};
use pm_core::{BusEvent, ClobEvent, MarketWindow, ResolutionTick};
use pm_replay::v2::load_bus_events;
use pm_strategy::calib::{CalibTable, FenetrePending};
use pm_strategy::maker::{Inventory, MakerConfig, MakerContext, MakerStrategy, QuoteAction};
use pm_strategy::paper::{PaperBroker, RestingQuote, WindowReport};
use pm_strategy::taker::{TakerConfig, TakerStrategy};
use pm_strategy::{MarketSnapshot, ProbModel};
use std::collections::VecDeque;
use std::path::PathBuf;

const DECISION_STEP_MS: u64 = 250;
const STALE_MS: u64 = 6_000;

#[derive(Debug, Clone)]
struct Args {
    journals: Vec<PathBuf>,
    taker_enabled: bool,
    maker_enabled: bool,
    grid: bool,
    taker_grid: bool,
    quiet: bool,
    no_drift: bool,
    drift_cap: Option<f64>,
    gauss: bool,
    calib_out: Option<PathBuf>,
    calib_in: Option<PathBuf>,
    config_path: Option<PathBuf>,
    maker_cfg: MakerConfig,
    taker_cfg: TakerConfig,
}

fn parse_args() -> Result<Args> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut a = Args {
        journals: vec![],
        taker_enabled: true,
        maker_enabled: true,
        grid: false,
        taker_grid: false,
        quiet: false,
        no_drift: false,
        drift_cap: None,
        gauss: false,
        calib_out: None,
        calib_in: None,
        config_path: None,
        maker_cfg: MakerConfig::default(),
        taker_cfg: TakerConfig::default(),
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--journal" => {
                i += 1;
                while i < argv.len() && !argv[i].starts_with("--") {
                    a.journals.push(PathBuf::from(&argv[i]));
                    i += 1;
                }
                continue;
            }
            "--no-taker" => a.taker_enabled = false,
            "--no-maker" => a.maker_enabled = false,
            "--grid" => a.grid = true,
            "--taker-grid" => a.taker_grid = true,
            "--max-entry" => {
                i += 1;
                a.taker_cfg.max_entry_price = argv[i].parse()?;
            }
            "--kelly" => {
                i += 1;
                a.taker_cfg.kelly_fraction = argv[i].parse()?;
            }
            "--min-z" => {
                i += 1;
                a.taker_cfg.min_abs_z = argv[i].parse()?;
            }
            "--min-edge" => {
                i += 1;
                a.taker_cfg.min_edge = argv[i].parse()?;
            }
            "--quiet" => a.quiet = true,
            "--no-drift" => a.no_drift = true,
            "--gauss" => a.gauss = true,
            "--calib-out" => {
                i += 1;
                a.calib_out = Some(PathBuf::from(&argv[i]));
            }
            "--calib-in" => {
                i += 1;
                a.calib_in = Some(PathBuf::from(&argv[i]));
            }
            "--config" => {
                i += 1;
                a.config_path = Some(PathBuf::from(&argv[i]));
            }
            "--drift-cap" => {
                i += 1;
                a.drift_cap = Some(argv[i].parse()?);
            }
            "--tp" => {
                i += 1;
                a.maker_cfg.take_profit = argv[i].parse()?;
            }
            "--stop" => {
                i += 1;
                a.maker_cfg.stop_loss = argv[i].parse()?;
            }
            "--margin" => {
                i += 1;
                a.maker_cfg.edge_margin = argv[i].parse()?;
            }
            "--tau-open" => {
                i += 1;
                a.maker_cfg.min_tau_open_s = argv[i].parse()?;
            }
            "--tau-flat" => {
                i += 1;
                a.maker_cfg.min_tau_flat_s = argv[i].parse()?;
            }
            "--quote-size" => {
                i += 1;
                a.maker_cfg.quote_size = argv[i].parse()?;
            }
            "--max-z" => {
                i += 1;
                a.maker_cfg.max_abs_z_quote = argv[i].parse()?;
            }
            other => bail!("argument inconnu: {other}"),
        }
        i += 1;
    }
    if a.journals.is_empty() {
        bail!("--journal requis");
    }
    // Fichier de config = base ; les flags individuels restent prioritaires.
    let cfg_path = a
        .config_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("./config.toml"));
    if cfg_path.exists() {
        let cfg = pm_strategy::config::BotConfig::charger(&cfg_path)?;
        // Les valeurs du fichier remplacent les défauts UNIQUEMENT si le flag
        // correspondant n'a pas été passé (les flags ont déjà écrit dans a.*).
        let flags: Vec<String> = std::env::args().collect();
        let passed = |f: &str| flags.iter().any(|x| x == f);
        if !passed("--max-entry") && !passed("--kelly") && !passed("--min-z") && !passed("--min-edge") {
            a.taker_cfg = cfg.taker;
        }
        if !passed("--tp") && !passed("--stop") && !passed("--margin")
            && !passed("--tau-open") && !passed("--tau-flat")
            && !passed("--quote-size") && !passed("--max-z")
        {
            a.maker_cfg = cfg.maker;
        }
        eprintln!("config chargée: {}", cfg_path.display());
    }
    Ok(a)
}

/// État moteur — miroir de pm-bot::Engine (glue identique, hors réseau).
struct Engine {
    window: Option<MarketWindow>,
    ticks: VecDeque<ResolutionTick>,
    vol: VolEstimator,
    book_up: OrderBook,
    book_down: OrderBook,
    strike_frozen: bool,
    strike: Option<StrikeComputation>,
    last_rtds_recv_ms: u64,
    last_clob_recv_ms: u64,
}

impl Engine {
    fn new() -> Self {
        Self {
            window: None,
            ticks: VecDeque::with_capacity(8192),
            vol: VolEstimator::new(VolConfig::default()),
            book_up: OrderBook::new(),
            book_down: OrderBook::new(),
            strike_frozen: false,
            strike: None,
            last_rtds_recv_ms: 0,
            last_clob_recv_ms: 0,
        }
    }

    fn settle_previous(&mut self, broker: &mut PaperBroker) -> Option<WindowReport> {
        let prev = self.window.clone()?;
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
        Some(broker.settle_window(
            &prev.slug,
            &prev.token_up,
            &prev.token_down,
            strike,
            up_won,
            up_won.map(|u| if u { "Up".into() } else { "Down".into() }),
        ))
    }

    fn on_window(&mut self, w: MarketWindow) {
        self.book_up = OrderBook::new();
        self.book_down = OrderBook::new();
        self.strike_frozen = false;
        self.strike = None;
        self.window = Some(w);
        self.refresh_strike();
    }

    fn on_resolution_tick(&mut self, t: ResolutionTick) {
        self.ticks.push_back(t);
        let cutoff = t.source_ts_ms.saturating_sub(2_400_000);
        while self.ticks.front().is_some_and(|x| x.source_ts_ms < cutoff) {
            self.ticks.pop_front();
        }
        self.vol.push(&t);
        self.last_rtds_recv_ms = t.recv_ms;
        if !self.strike_frozen {
            self.refresh_strike();
        }
    }

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
        }
        self.strike = Some(comp);
    }

    fn on_clob(&mut self, ev: &ClobEvent, recv_ms: u64) {
        self.last_clob_recv_ms = recv_ms;
        let Some(w) = &self.window else { return };
        match ev {
            ClobEvent::Book {
                asset_id,
                ts_ms,
                bids,
                asks,
            } => {
                if *asset_id == w.token_up {
                    self.book_up.apply_snapshot(bids, asks, *ts_ms, recv_ms);
                } else if *asset_id == w.token_down {
                    self.book_down.apply_snapshot(bids, asks, *ts_ms, recv_ms);
                }
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
            _ => {}
        }
    }

    fn snapshot(&self, now_ms: u64) -> Option<MarketSnapshot> {
        let w = self.window.as_ref()?;
        if now_ms < w.start_ms || now_ms >= w.end_ms {
            return None;
        }
        let strike = self.strike.clone()?;
        let last = self.ticks.back()?;
        let stale = now_ms.saturating_sub(self.last_rtds_recv_ms) > STALE_MS
            || now_ms.saturating_sub(self.last_clob_recv_ms) > STALE_MS;
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
            any_feed_stale: stale,
        })
    }
}

#[derive(Debug, Default, Clone)]
struct BacktestResult {
    reports: Vec<WindowReport>,
    taker_entries: u32,
    maker_fills: u32,
    /// Score de Brier des p_up calibrés (échantillonnés 1×/s) — plus bas
    /// = mieux calibré. Référence : marché ≈ 0,17 (docs/ETUDE_MODELE.md).
    brier: Option<f64>,
    brier_n: u64,
    calib_observations: f64,
    calib_windows: u64,
    /// Table apprise pendant le run (exportable via --calib-out).
    calib_table: Option<CalibTable>,
}

impl BacktestResult {
    fn total_pnl(&self) -> f64 {
        self.reports.iter().map(|r| r.pnl_up + r.pnl_down).sum()
    }
}

fn run_backtest(
    events: &[(u64, BusEvent)],
    taker_enabled: bool,
    maker_enabled: bool,
    maker_cfg: MakerConfig,
    taker_cfg: TakerConfig,
    no_drift: bool,
    drift_cap: Option<f64>,
    gauss: bool,
    calib_init: Option<CalibTable>,
) -> BacktestResult {
    let taker = TakerStrategy::new(taker_cfg);
    let maker = MakerStrategy::new(maker_cfg);
    let mut prob_cfg = pm_strategy::model::ProbConfig::default();
    if no_drift {
        // Martingale pure : n'extrapole jamais la tendance récente.
        prob_cfg.drift_snr_min = f64::INFINITY;
    }
    if let Some(c) = drift_cap {
        prob_cfg.max_drift_z = c;
    }
    if gauss {
        prob_cfg.dist = pm_strategy::model::Dist::Gauss;
        prob_cfg.calibration = false;
    }
    let model = ProbModel::new(prob_cfg);
    let mut engine = Engine::new();
    let mut broker = PaperBroker::new();
    let mut next_decision_ms = 0u64;
    // Calibration en ligne (walk-forward honnête : seules les fenêtres déjà
    // réglées informent la décision courante) + score de Brier.
    let mut calib = calib_init.unwrap_or_default();
    let mut pending = FenetrePending::default();
    let mut p_samples: Vec<f64> = Vec::new(); // p_up échantillonné 1×/s
    let mut last_sample_s = 0u64;
    let mut brier_sum = 0.0f64;
    let mut brier_n = 0u64;
    let mut flush = |engine: &mut Engine, broker: &mut PaperBroker,
                     calib: &mut CalibTable, pending: &mut FenetrePending,
                     p_samples: &mut Vec<f64>, brier_sum: &mut f64, brier_n: &mut u64| {
        if let Some(report) = engine.settle_previous(broker) {
            if let Some(out) = report.outcome.as_deref() {
                let up_won = out == "Up";
                calib.regler_fenetre(pending, up_won);
                let y = if up_won { 1.0 } else { 0.0 };
                for p in p_samples.iter() {
                    *brier_sum += (p - y) * (p - y);
                    *brier_n += 1;
                }
            }
        }
        pending.clear();
        p_samples.clear();
    };

    for (recv_ms, ev) in events {
        match ev {
            BusEvent::WindowChanged(w) => {
                flush(&mut engine, &mut broker, &mut calib, &mut pending,
                      &mut p_samples, &mut brier_sum, &mut brier_n);
                engine.on_window(w.clone());
            }
            BusEvent::Resolution(t) => engine.on_resolution_tick(*t),
            BusEvent::Clob(cev) => {
                if let ClobEvent::LastTrade {
                    asset_id,
                    price,
                    side,
                    ..
                } = cev
                {
                    broker.on_market_trade(asset_id, *price, *side);
                }
                engine.on_clob(cev, *recv_ms);
            }
            _ => {}
        }

        // Décisions au pas de 250 ms de temps journal (comme le live).
        if *recv_ms < next_decision_ms {
            continue;
        }
        next_decision_ms = recv_ms + DECISION_STEP_MS;
        let Some(snap) = engine.snapshot(*recv_ms) else {
            continue;
        };
        let mut est = model.estimate(&snap);
        if est.reliable {
            pending.observer(est.dist_usd, est.tau_s);
        }
        model.calibrer(&mut est, &calib);
        if est.reliable && recv_ms / 1000 != last_sample_s {
            last_sample_s = recv_ms / 1000;
            p_samples.push(est.p_up);
        }
        let w = engine.window.clone().unwrap();

        if taker_enabled {
            if let Some(d) = taker.decide(&snap, &est) {
                let token = if d.buy_up { &w.token_up } else { &w.token_down };
                if broker.can_take(token) {
                    broker.fill_taker_avec_frais(token, d.avg_price, d.size, taker.cfg.fee_rate);
                }
            }
        }
        if maker_enabled {
            for is_up in [true, false] {
                let token = if is_up { &w.token_up } else { &w.token_down };
                let other = if is_up { &w.token_down } else { &w.token_up };
                let pos = broker.position(token);
                let ctx = MakerContext {
                    inv: Inventory {
                        position: pos.size,
                        avg_entry: pos.avg_entry,
                    },
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
                            new_bid = Some(RestingQuote {
                                price: *price,
                                size: *size,
                            })
                        }
                        QuoteAction::Ask { price, size } => {
                            new_ask = Some(RestingQuote {
                                price: *price,
                                size: *size,
                            })
                        }
                        QuoteAction::ExitNow { limit_price, .. } => {
                            broker.exit_now(token, *limit_price)
                        }
                    }
                }
                broker.set_quotes(token, new_bid, new_ask);
            }
        }
    }
    flush(&mut engine, &mut broker, &mut calib, &mut pending,
          &mut p_samples, &mut brier_sum, &mut brier_n);

    let mut res = BacktestResult {
        reports: broker.reports.clone(),
        brier: if brier_n > 0 { Some(brier_sum / brier_n as f64) } else { None },
        brier_n,
        calib_observations: calib.total_observations(),
        calib_windows: calib.windows_observed,
        calib_table: Some(calib),
        ..Default::default()
    };
    for r in &res.reports {
        res.taker_entries += r.taker_entries;
        res.maker_fills += r.maker_fills;
    }
    res
}

fn print_result(label: &str, res: &BacktestResult, quiet: bool) {
    if !quiet {
        println!("--- {label} ---");
        for r in &res.reports {
            println!(
                "  {} | strike={} issue={} | up={:+.2} down={:+.2} | taker={} fills={}",
                r.slug,
                r.strike
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "N/A".into()),
                r.outcome.as_deref().unwrap_or("?"),
                r.pnl_up,
                r.pnl_down,
                r.taker_entries,
                r.maker_fills
            );
        }
    }
    println!(
        "{label}: PnL total = {:+.2} $ | fenêtres={} | entrées taker={} | fills maker={}",
        res.total_pnl(),
        res.reports.len(),
        res.taker_entries,
        res.maker_fills
    );
    if let Some(b) = res.brier {
        println!(
            "{label}: Brier(p calibré) = {:.4} sur {} échantillons (réf. marché ≈ 0.17) | calibration: {:.0} obs / {} fenêtres",
            b, res.brier_n, res.calib_observations, res.calib_windows
        );
    }
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let paths: Vec<&std::path::Path> = args.journals.iter().map(PathBuf::as_path).collect();
    let events = load_bus_events(&paths).context("chargement journaux")?;
    eprintln!(
        "{} événements chargés depuis {} journal(aux)",
        events.len(),
        paths.len()
    );

    if args.grid {
        // Attribution : taker seul d'abord (référence fixe).
        let taker_only = run_backtest(
            &events,
            true,
            false,
            MakerConfig::default(),
            TakerConfig::default(),
            args.no_drift,
            args.drift_cap, args.gauss,
        args.calib_in.as_deref().map(CalibTable::charger_ou_defaut),
    );
        print_result("taker seul", &taker_only, true);

        let mut rows: Vec<(String, f64, u32)> = vec![];
        for tp in [0.04, 0.06, 0.08] {
            for stop in [0.04, 0.06, 0.10] {
                for margin in [0.03, 0.05, 0.08] {
                    for tau_open in [60.0, 120.0] {
                        for tau_flat in [45.0, 90.0] {
                            let cfg = MakerConfig {
                                take_profit: tp,
                                stop_loss: stop,
                                edge_margin: margin,
                                min_tau_open_s: tau_open,
                                min_tau_flat_s: tau_flat,
                                ..Default::default()
                            };
                            let r = run_backtest(
                                &events,
                                false,
                                true,
                                cfg,
                                TakerConfig::default(),
                                args.no_drift,
                                args.drift_cap, args.gauss,
        args.calib_in.as_deref().map(CalibTable::charger_ou_defaut),
    );
                            rows.push((
                                format!(
                                    "tp={tp:.2} stop={stop:.2} margin={margin:.2} tauO={tau_open:.0} tauF={tau_flat:.0}"
                                ),
                                r.total_pnl(),
                                r.maker_fills,
                            ));
                        }
                    }
                }
            }
        }
        rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        println!(
            "\n=== GRID maker seul (top 10 / {} configs) ===",
            rows.len()
        );
        for (label, pnl, fills) in rows.iter().take(10) {
            println!("  {label} → PnL {pnl:+.2} $ (fills={fills})");
        }
        println!("=== pires 3 ===");
        for (label, pnl, fills) in rows.iter().rev().take(3) {
            println!("  {label} → PnL {pnl:+.2} $ (fills={fills})");
        }
        return Ok(());
    }

    if args.taker_grid {
        let mut rows: Vec<(String, f64, u32)> = vec![];
        for max_entry in [0.80, 0.85, 0.88, 0.92, 1.0] {
            for kelly in [0.10, 0.15, 0.25] {
                for min_z in [2.0, 2.5, 3.0] {
                    for min_edge in [0.05, 0.06, 0.08] {
                        let cfg = TakerConfig {
                            max_entry_price: max_entry,
                            kelly_fraction: kelly,
                            min_abs_z: min_z,
                            min_edge,
                            ..Default::default()
                        };
                        let r = run_backtest(
                            &events,
                            true,
                            false,
                            MakerConfig::default(),
                            cfg,
                            args.no_drift,
                            args.drift_cap, args.gauss,
        args.calib_in.as_deref().map(CalibTable::charger_ou_defaut),
    );
                        rows.push((
                            format!(
                                "maxpx={max_entry:.2} kelly={kelly:.2} z≥{min_z:.1} edge≥{min_edge:.2}"
                            ),
                            r.total_pnl(),
                            r.taker_entries,
                        ));
                    }
                }
            }
        }
        rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        println!("=== GRID taker seul (top 15 / {} configs) ===", rows.len());
        for (label, pnl, n) in rows.iter().take(15) {
            println!("  {label} → PnL {pnl:+.2} $ (entrées={n})");
        }
        println!("=== pires 3 ===");
        for (label, pnl, n) in rows.iter().rev().take(3) {
            println!("  {label} → PnL {pnl:+.2} $ (entrées={n})");
        }
        return Ok(());
    }

    let res = run_backtest(
        &events,
        args.taker_enabled,
        args.maker_enabled,
        args.maker_cfg,
        args.taker_cfg,
        args.no_drift,
        args.drift_cap, args.gauss,
        args.calib_in.as_deref().map(CalibTable::charger_ou_defaut),
    );
    print_result("backtest", &res, args.quiet);
    if let (Some(path), Some(table)) = (&args.calib_out, &res.calib_table) {
        table.sauver(path)?;
        eprintln!("table de calibration → {}", path.display());
    }
    Ok(())
}
