use serde::Deserialize;

// ─── Gamma API ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct GammaEvent {
    pub id: String,
    pub slug: String,
    pub title: String,
    #[serde(rename = "endDate")]
    pub end_date: String,
    pub active: bool,
    pub closed: bool,
    pub markets: Vec<GammaMarket>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GammaMarket {
    pub id: String,
    #[serde(rename = "conditionId")]
    pub condition_id: String,
    #[serde(rename = "clobTokenIds")]
    pub clob_token_ids_raw: String,
    pub outcomes: String,
    #[serde(rename = "outcomePrices")]
    pub outcome_prices: Option<String>,
    #[serde(rename = "bestBid")]
    pub best_bid: Option<f64>,
    #[serde(rename = "bestAsk")]
    pub best_ask: Option<f64>,
    #[serde(rename = "lastTradePrice")]
    pub last_trade_price: Option<f64>,
    #[serde(rename = "orderPriceMinTickSize")]
    pub tick_size: Option<f64>,
    #[serde(rename = "endDate")]
    pub end_date: String,
    #[serde(rename = "acceptingOrders")]
    pub accepting_orders: Option<bool>,
}

impl GammaMarket {
    pub fn clob_token_ids(&self) -> Vec<String> {
        serde_json::from_str(&self.clob_token_ids_raw).unwrap_or_default()
    }

    pub fn outcomes_list(&self) -> Vec<String> {
        serde_json::from_str(&self.outcomes).unwrap_or_default()
    }
}

// ─── CLOB REST ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct OrderBookLevel {
    pub price: String,
    pub size: String,
}

// ─── CLOB WebSocket events ────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct BookEvent {
    pub asset_id: String,
    pub market: String,
    pub bids: Vec<OrderBookLevel>,
    pub asks: Vec<OrderBookLevel>,
    pub timestamp: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PriceChangeLevel {
    pub asset_id: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub best_bid: Option<String>,
    pub best_ask: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PriceChangeEvent {
    pub market: String,
    pub price_changes: Vec<PriceChangeLevel>,
    pub timestamp: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LastTradePriceEvent {
    pub asset_id: String,
    pub market: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub timestamp: String,
    pub transaction_hash: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BestBidAskEvent {
    pub asset_id: String,
    pub market: String,
    pub best_bid: String,
    pub best_ask: String,
    pub spread: Option<String>,
    pub timestamp: String,
}

// ─── OrderBook ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OrderBook {
    pub bids: std::collections::BTreeMap<ordered_float::OrderedFloat<f64>, f64>,
    pub asks: std::collections::BTreeMap<ordered_float::OrderedFloat<f64>, f64>,
    pub asset_id: String,
    pub last_update_ms: u64,
}

impl OrderBook {
    pub fn new(asset_id: String) -> Self {
        Self {
            bids: Default::default(),
            asks: Default::default(),
            asset_id,
            last_update_ms: 0,
        }
    }

    pub fn apply_snapshot(&mut self, event: &BookEvent) {
        self.bids.clear();
        self.asks.clear();
        for b in &event.bids {
            if let (Ok(p), Ok(s)) = (b.price.parse::<f64>(), b.size.parse::<f64>()) {
                if s > 0.0 {
                    self.bids.insert(ordered_float::OrderedFloat(p), s);
                }
            }
        }
        for a in &event.asks {
            if let (Ok(p), Ok(s)) = (a.price.parse::<f64>(), a.size.parse::<f64>()) {
                if s > 0.0 {
                    self.asks.insert(ordered_float::OrderedFloat(p), s);
                }
            }
        }
        self.last_update_ms = event.timestamp.parse::<u64>().unwrap_or(0);
    }

    pub fn apply_delta(&mut self, level: &PriceChangeLevel) {
        let price = match level.price.parse::<f64>() {
            Ok(p) => ordered_float::OrderedFloat(p),
            Err(_) => return,
        };
        let size: f64 = level.size.parse().unwrap_or(0.0);
        let book = if level.side == "BUY" { &mut self.bids } else { &mut self.asks };
        if size == 0.0 {
            book.remove(&price);
        } else {
            book.insert(price, size);
        }
    }

    pub fn best_bid(&self) -> Option<f64> {
        self.bids.keys().next_back().map(|f| f.0)
    }

    pub fn best_ask(&self) -> Option<f64> {
        self.asks.keys().next().map(|f| f.0)
    }

    pub fn mid(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) => Some((b + a) / 2.0),
            _ => None,
        }
    }
}

// ─── Trades marché Polymarket ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Trade {
    pub price: f64,
    pub size: f64,
    pub side: String,
    pub timestamp_ms: u64,
}

// ─── Ticks Chainlink ─────────────────────────────────────────────────────────

/// Un tick de prix BTC/USD reçu depuis Chainlink via RTDS
#[derive(Debug, Clone)]
pub struct ChainlinkTick {
    pub ts_ms: u64,  // timestamp du tick (epoch ms, depuis Chainlink)
    pub price: f64,  // prix BTC/USD
}

// ─── Bougie BTC (calculée depuis les ticks Chainlink) ────────────────────────

/// Bougie OHLC construite à partir des ticks Chainlink
/// Correspond exactement aux données utilisées pour résoudre l'événement
#[derive(Debug, Clone, Default)]
pub struct ChainlinkCandle {
    pub open: f64,           // prix du premier tick de la fenêtre
    pub high: f64,           // prix max
    pub low: f64,            // prix min
    pub close: f64,          // prix du dernier tick
    pub tick_count: usize,   // nombre de ticks reçus
    pub window_start_ms: u64, // début de la fenêtre 5-min
    pub window_end_ms: u64,  // fin de la fenêtre (= endDate du marché)
    pub first_tick_ms: u64,  // timestamp du premier tick reçu dans la fenêtre
    pub last_tick_ms: u64,   // timestamp du dernier tick reçu
}

impl ChainlinkCandle {
    pub fn is_valid(&self) -> bool {
        self.tick_count > 0 && self.open > 0.0
    }

    pub fn direction(&self) -> Option<bool> {
        if !self.is_valid() { return None; }
        // Up si close >= open (même logique que la résolution Polymarket)
        Some(self.close >= self.open)
    }
}

// ─── Infos marché ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MarketInfo {
    pub slug: String,
    pub title: String,
    pub end_date_ms: u64,
    pub condition_id: String,
    pub token_up: String,
    pub token_down: String,
}

// ─── État global de l'application ────────────────────────────────────────────

#[derive(Debug)]
pub struct AppState {
    pub market: Option<MarketInfo>,
    pub book_up: OrderBook,
    pub book_down: OrderBook,

    // Prix BTC spot
    pub btc_price_chainlink: Option<f64>,   // prix Chainlink (résolution officielle)
    pub btc_price_binance: Option<f64>,     // prix Binance (affichage rapide)
    pub chainlink_timestamp_ms: u64,
    pub binance_timestamp_ms: u64,

    // Buffer de ticks Chainlink (max 512)
    pub chainlink_ticks: std::collections::VecDeque<ChainlinkTick>,

    // Bougie BTC calculée depuis les ticks Chainlink de la fenêtre courante
    pub chainlink_candle: ChainlinkCandle,

    // Trades sur l'orderbook Polymarket (pour l'orderbook, pas pour la bougie BTC)
    pub recent_trades: std::collections::VecDeque<Trade>,

    // Statut WebSocket
    pub last_ws_clob_ms: u64,
    pub last_ws_rtds_ms: u64,
    pub clob_latency_ms: i64,
    pub rtds_latency_ms: i64,
    pub next_slot_ts: u64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            market: None,
            book_up: OrderBook::new(String::new()),
            book_down: OrderBook::new(String::new()),
            btc_price_chainlink: None,
            btc_price_binance: None,
            chainlink_timestamp_ms: 0,
            binance_timestamp_ms: 0,
            chainlink_ticks: std::collections::VecDeque::with_capacity(512),
            chainlink_candle: ChainlinkCandle::default(),
            recent_trades: std::collections::VecDeque::with_capacity(50),
            last_ws_clob_ms: 0,
            last_ws_rtds_ms: 0,
            clob_latency_ms: 0,
            rtds_latency_ms: 0,
            next_slot_ts: 0,
        }
    }
}

impl AppState {
    /// Recalcule la bougie Chainlink à partir du buffer de ticks,
    /// en ne conservant que les ticks dans la fenêtre [window_start_ms, window_end_ms].
    pub fn update_chainlink_candle(&mut self, window_start_ms: u64, window_end_ms: u64) {
        let mut candle = ChainlinkCandle {
            window_start_ms,
            window_end_ms,
            ..Default::default()
        };

        for tick in &self.chainlink_ticks {
            if tick.ts_ms < window_start_ms || tick.ts_ms > window_end_ms {
                continue;
            }
            if candle.tick_count == 0 {
                candle.open = tick.price;
                candle.high = tick.price;
                candle.low = tick.price;
                candle.close = tick.price;
                candle.first_tick_ms = tick.ts_ms;
            } else {
                if tick.price > candle.high { candle.high = tick.price; }
                if tick.price < candle.low { candle.low = tick.price; }
                candle.close = tick.price;
            }
            candle.last_tick_ms = tick.ts_ms;
            candle.tick_count += 1;
        }

        self.chainlink_candle = candle;
    }

    /// Réinitialise tout ce qui dépend du marché courant
    pub fn reset_for_new_market(&mut self, token_up: String, token_down: String) {
        self.book_up = OrderBook::new(token_up);
        self.book_down = OrderBook::new(token_down);
        self.chainlink_ticks.clear();
        self.chainlink_candle = ChainlinkCandle::default();
        self.recent_trades.clear();
    }
}
