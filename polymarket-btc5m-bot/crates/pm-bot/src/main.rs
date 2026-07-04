//! pm-bot — orchestrateur : acquisition → état → stratégies → exécution.
//!
//! Mode par défaut : PAPER (DryRunGateway). Les ordres sont journalisés,
//! jamais envoyés. Le passage en réel exige la feature `live` de pm-execution
//! et une validation préalable sur archives (voir docs/ARCHITECTURE.md).
//!
//! ```text
//! pm-bot [--out DIR] [--no-taker] [--no-maker]
//! ```

use anyhow::Result;
use pm_acquisition::{clob, gamma::GammaClient, rtds, watchdog::Watchdog, Bus, Recorder};
use pm_core::book::OrderBook;
use pm_core::strike::{compute_strike, StrikePolicy, DEFAULT_CONFIDENCE_GAP_MS};
use pm_core::vol::{VolConfig, VolEstimator};
use pm_core::{BusEvent, ClobEvent, MarketWindow, ResolutionTick};
use pm_execution::{DryRunGateway, OrderGateway, OrderRequest, OrderSide, TimeInForce};
use pm_strategy::maker::{Inventory, MakerStrategy, QuoteAction};
use pm_strategy::taker::TakerStrategy;
use pm_strategy::{MarketSnapshot, ProbModel};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time;

#[derive(Debug, Clone)]
struct Args {
    out_dir: PathBuf,
    taker_enabled: bool,
    maker_enabled: bool,
}

fn parse_args() -> Result<Args> {
    let mut args =
        Args { out_dir: PathBuf::from("./data_v2"), taker_enabled: true, maker_enabled: true };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--out" => {
                i += 1;
                args.out_dir = PathBuf::from(
                    argv.get(i).ok_or_else(|| anyhow::anyhow!("--out: valeur manquante"))?,
                );
            }
            "--no-taker" => args.taker_enabled = false,
            "--no-maker" => args.maker_enabled = false,
            "--help" | "-h" => {
                println!("pm-bot [--out DIR] [--no-taker] [--no-maker]");
                std::process::exit(0);
            }
            other => anyhow::bail!("argument inconnu: {other}"),
        }
        i += 1;
    }
    Ok(args)
}

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
        }
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
        // Rétention 40 min sur l'horloge source.
        let cutoff = t.source_ts_ms.saturating_sub(2_400_000);
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
        let comp =
            compute_strike(&ticks, w.start_ms, StrikePolicy::LastAtOrBefore, DEFAULT_CONFIDENCE_GAP_MS);
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
            ClobEvent::Book { asset_id, ts_ms, bids, asks } => {
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
            ClobEvent::MarketResolved { slug, winning_outcome, .. } => {
                tracing::info!("résolution officielle {slug}: {winning_outcome}");
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = parse_args()?;
    tracing::info!("pm-bot démarre (PAPER mode) — archives: {}", args.out_dir.display());

    let bus = Bus::default();
    let recorder = Recorder::spawn(args.out_dir.clone());
    let watchdog = Watchdog::new(6_000);
    let gateway = DryRunGateway::new();

    // Flux de résolution : connexion continue, indépendante des fenêtres.
    tokio::spawn(rtds::run(bus.clone(), recorder.clone()));
    tokio::spawn(watchdog.clone().run(bus.clone()));

    // Découverte des fenêtres + rotation du flux CLOB.
    {
        let bus = bus.clone();
        let recorder = recorder.clone();
        tokio::spawn(async move {
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .user_agent("pm-bot/0.1")
                .build()
                .expect("client http");
            let gamma = GammaClient::new(http);
            let mut clob_task: Option<tokio::task::JoinHandle<()>> = None;
            let mut current_slug = String::new();
            loop {
                match gamma.find_active_window(now_ms() / 1000, 5).await {
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
                            if let Some(h) = clob_task.take() {
                                h.abort();
                            }
                            clob_task = Some(tokio::spawn(clob::run_for_tokens(
                                bus.clone(),
                                recorder.clone(),
                                w.token_up.clone(),
                                w.token_down.clone(),
                            )));
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
    let mut engine = Engine::new();
    let taker = TakerStrategy::new(Default::default());
    let maker = MakerStrategy::new(Default::default());
    let model = ProbModel::default();
    let mut rx = bus.subscribe();
    let mut decide_tick = time::interval(Duration::from_millis(250));
    decide_tick.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    // Inventaire paper (les fills ne sont pas simulés en v1 — cf. ROADMAP).
    let inv_up = Inventory::default();
    let inv_down = Inventory::default();

    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Ok(BusEvent::WindowChanged(w)) => engine.on_window(w),
                    Ok(BusEvent::Resolution(t)) => {
                        watchdog.touch("rtds");
                        engine.on_resolution_tick(t);
                    }
                    Ok(BusEvent::Fast(_)) => watchdog.touch("rtds_fast"),
                    Ok(BusEvent::Clob(ev)) => {
                        watchdog.touch("clob");
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
                let stale = watchdog.is_stale("rtds") || watchdog.is_stale("clob");
                let Some(snap) = engine.snapshot(now_ms(), stale) else { continue };
                let est = model.estimate(&snap);

                if args.taker_enabled {
                    if let Some(d) = taker.decide(&snap, &est) {
                        let w = engine.window.as_ref().unwrap();
                        let token = if d.buy_up { &w.token_up } else { &w.token_down };
                        tracing::info!("TAKER: {} ({})", if d.buy_up { "UP" } else { "DOWN" }, d.reason);
                        let _ = gateway.post_order(OrderRequest {
                            token_id: token.clone(),
                            side: OrderSide::Buy,
                            price: d.limit_price,
                            size: d.size,
                            tif: TimeInForce::Fak,
                            tag: format!("taker edge={:.3}", d.edge),
                        }).await;
                    }
                }
                if args.maker_enabled {
                    let w = engine.window.as_ref().unwrap().clone();
                    for (is_up, inv) in [(true, inv_up), (false, inv_down)] {
                        let d = maker.decide_token(&snap, &est, is_up, inv);
                        let token = if is_up { &w.token_up } else { &w.token_down };
                        for action in d.actions {
                            let req = match action {
                                QuoteAction::Bid { price, size } => OrderRequest {
                                    token_id: token.clone(), side: OrderSide::Buy,
                                    price, size, tif: TimeInForce::Gtc,
                                    tag: format!("maker bid {}", d.reason),
                                },
                                QuoteAction::Ask { price, size } => OrderRequest {
                                    token_id: token.clone(), side: OrderSide::Sell,
                                    price, size, tif: TimeInForce::Gtc,
                                    tag: format!("maker ask {}", d.reason),
                                },
                                QuoteAction::ExitNow { limit_price, size } => OrderRequest {
                                    token_id: token.clone(), side: OrderSide::Sell,
                                    price: limit_price, size, tif: TimeInForce::Fak,
                                    tag: format!("maker exit {}", d.reason),
                                },
                            };
                            let _ = gateway.post_order(req).await;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
