use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct GammaEvent {
    pub slug: String,
    pub title: String,
    pub closed: bool,
    pub markets: Vec<GammaMarket>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GammaMarket {
    #[serde(rename = "conditionId")]
    pub condition_id: String,
    #[serde(rename = "clobTokenIds")]
    pub clob_token_ids_raw: String,
    pub outcomes: String,
    #[serde(rename = "endDate")]
    pub end_date: String,
}

impl GammaMarket {
    pub fn clob_token_ids(&self) -> Vec<String> {
        serde_json::from_str(&self.clob_token_ids_raw).unwrap_or_default()
    }
    pub fn outcomes_list(&self) -> Vec<String> {
        serde_json::from_str(&self.outcomes).unwrap_or_default()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct OrderBookLevel {
    pub price: String,
    pub size: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BookEvent {
    pub asset_id: String,
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
}

#[derive(Debug, Deserialize, Clone)]
pub struct PriceChangeEvent {
    pub price_changes: Vec<PriceChangeLevel>,
    pub timestamp: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LastTradePriceEvent {
    pub price: String,
    pub size: String,
    pub side: String,
    pub timestamp: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BestBidAskEvent {
    pub asset_id: String,
    pub timestamp: String,
}

#[derive(Debug, Clone)]
pub struct OrderBook {
    pub bids: std::collections::BTreeMap<ordered_float::OrderedFloat<f64>, f64>,
    pub asks: std::collections::BTreeMap<ordered_float::OrderedFloat<f64>, f64>,
    pub last_update_ms: u64,
}

impl OrderBook {
    pub fn new() -> Self {
        Self {
            bids: Default::default(),
            asks: Default::default(),
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
        let size = level.size.parse::<f64>().unwrap_or(0.0);
        let book = if level.side == "BUY" {
            &mut self.bids
        } else {
            &mut self.asks
        };
        if size <= 0.0 {
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

#[derive(Debug, Clone)]
pub struct Trade {
    pub price: f64,
    pub size: f64,
    pub side: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ChainlinkTick {
    pub ts_ms: u64,
    pub price: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ChainlinkCandle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub tick_count: usize,
    pub window_start_ms: u64,
    pub window_end_ms: u64,
}

impl ChainlinkCandle {
    pub fn is_valid(&self) -> bool {
        self.tick_count > 0 && self.open > 0.0
    }
}

#[derive(Debug, Clone)]
pub struct MarketInfo {
    pub slug: String,
    pub title: String,
    pub end_date_ms: u64,
    pub token_up: String,
    pub token_down: String,
}

#[derive(Debug)]
pub struct AppState {
    pub market: Option<MarketInfo>,
    pub book_up: OrderBook,
    pub book_down: OrderBook,
    pub btc_price_chainlink: Option<f64>,
    pub btc_price_binance: Option<f64>,
    pub chainlink_timestamp_ms: u64,
    pub last_ws_clob_ms: u64,
    pub last_ws_rtds_ms: u64,
    pub clob_latency_ms: i64,
    pub rtds_latency_ms: i64,
    pub chainlink_ticks: std::collections::VecDeque<ChainlinkTick>,
    pub chainlink_candle: ChainlinkCandle,
    pub recent_trades: std::collections::VecDeque<Trade>,
    pub rtds_tick_count_window: u64,
    pub clob_event_count_window: u64,
    pub recording_enabled: bool,
    pub recording_file: String,
    pub recording_lines: u64,
    pub recording_bytes: u64,
    pub last_record_write_ms: u64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            market: None,
            book_up: OrderBook::new(),
            book_down: OrderBook::new(),
            btc_price_chainlink: None,
            btc_price_binance: None,
            chainlink_timestamp_ms: 0,
            last_ws_clob_ms: 0,
            last_ws_rtds_ms: 0,
            clob_latency_ms: 0,
            rtds_latency_ms: 0,
            chainlink_ticks: std::collections::VecDeque::with_capacity(1024),
            chainlink_candle: ChainlinkCandle::default(),
            recent_trades: std::collections::VecDeque::with_capacity(100),
            rtds_tick_count_window: 0,
            clob_event_count_window: 0,
            recording_enabled: true,
            recording_file: String::new(),
            recording_lines: 0,
            recording_bytes: 0,
            last_record_write_ms: 0,
        }
    }
}

impl AppState {
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
            } else {
                candle.high = candle.high.max(tick.price);
                candle.low = candle.low.min(tick.price);
                candle.close = tick.price;
            }
            candle.tick_count += 1;
        }
        self.chainlink_candle = candle;
    }

    pub fn reset_for_new_market(&mut self) {
        self.book_up = OrderBook::new();
        self.book_down = OrderBook::new();
        self.chainlink_ticks.clear();
        self.chainlink_candle = ChainlinkCandle::default();
        self.recent_trades.clear();
        self.rtds_tick_count_window = 0;
        self.clob_event_count_window = 0;
        self.recording_lines = 0;
        self.recording_bytes = 0;
    }
}
