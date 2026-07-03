# Low-Latency Baseline For BTC Up/Down 5m

This note captures the practical bottlenecks observed before the Rust refactor.

## Why the Python collector is not enough

- `collector/runner.py` samples at `sample_interval_ms` (default 500 ms), so multiple events can be collapsed between snapshots.
- `collector/polymarket_ws.py` keeps only `last_trade_price` per token and ignores the high-rate orderbook deltas.
- `collector/csv_writer.py` flushes every row, adding avoidable write-path overhead in the hot loop.
- The pipeline is snapshot-centric instead of event-driven.

## Measured stream behavior (from direct WS tests)

- RTDS Chainlink (`crypto_prices_chainlink`, `btc/usd`) behaves as point updates, not prebuilt candlesticks.
- Reliable live behavior was observed using unfiltered subscribe + client-side symbol filter:
  - about 0.9-1.0 BTC ticks per second
  - about 1.0-2.3 s observed lag between payload timestamp and local receive timestamp
- CLOB stream for the active Up/Down market is much denser than RTDS BTC:
  - `price_change` in the hundreds per second
  - `best_bid_ask`, `book`, `last_trade_price` all active

## Refactor consequence

- Resolution-aligned BTC volatility must be reconstructed from raw Chainlink ticks.
- Microstructure features for taker training must come from full CLOB event ingestion, not sparse snapshots.
- The collector hot path should be event-driven, low-allocation, and separated from persistence.
