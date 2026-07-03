# Low-Latency BTC Collector (Rust)

**Installation et commandes (guide complet en français)** : [`GUIDE_INSTALLATION_COLLECTEUR.md`](GUIDE_INSTALLATION_COLLECTEUR.md).

This collector is event-driven and built for `btc-updown-5m` training datasets.

## Goals

- Capture every RTDS Chainlink `btc/usd` tick used for market resolution.
- Capture high-rate CLOB microstructure (`book`, `price_change`, `best_bid_ask`, `last_trade_price`).
- Keep hot path independent from storage by using async channels and a dedicated writer task.

## Binary

From `Viewer rust rtds/`:

```bash
cargo run --bin low_latency_collector -- collect --out ../data_low_latency --duration-sec 300
```

Validation:

```bash
cargo run --bin low_latency_collector -- validate --out ../data_low_latency --window-epoch <EPOCH>
```

## Architecture

1. `gamma_window_loop`: discovers active window (`btc-updown-5m-<epoch>`) and rotates near 5m boundaries.
2. `rtds_loop`: subscribes to `crypto_prices_chainlink` (unfiltered), client-filters `btc/usd`, emits ticks.
3. `clob_loop`: subscribes to market WS for Up/Down token IDs and emits all relevant event types.
4. `feature_engine_loop`: reconstructs rolling BTC and microstructure features from event stream.
5. `writer_loop`: appends raw events and feature snapshots to NDJSON files with batched flushes.

## Output Layout

For each window epoch:

```
data_low_latency/
  window_<epoch>/
    raw.ndjson
    features.ndjson
```

### `raw.ndjson`

- `window_changed`: active market metadata at rotation.
- `rtds_tick`: raw Chainlink tick with source and receive timestamps.
- `clob_event`: normalized CLOB event payload with event type and timestamps.

### `features.ndjson`

High-frequency derived features:

- Chainlink OHLC over current 5m window
- Chainlink tick count and range
- Best bid/ask and midpoint for Up/Down books
- 10s trade count/notional
- Book depth level counts

## Why this supersedes snapshot collection

- Event-driven ingestion avoids collapsing multiple CLOB updates into one sample.
- Writer is decoupled from sockets to reduce backpressure on ingest tasks.
- Reconstruction is deterministic from raw logs and can be replayed for model QA.

## Practical benchmark workflow

1. Collect one full window:
   - `cargo run --bin low_latency_collector -- collect --out ../data_low_latency --duration-sec 320`
2. Validate counts and lag:
   - `cargo run --bin low_latency_collector -- validate --out ../data_low_latency --window-epoch <EPOCH>`
3. Compare OHLC from `features.ndjson` against the live Polymarket candle observed during that window.
