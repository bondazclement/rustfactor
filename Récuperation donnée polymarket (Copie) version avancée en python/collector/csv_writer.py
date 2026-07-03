from __future__ import annotations

import csv
from datetime import datetime, timezone
from pathlib import Path
from typing import TextIO


CSV_HEADER = [
    "timestamp_ms",
    "readable_ts",  # jj/mm/aaaa -- hh:mm:ss:ms
    "window_epoch",
    "window_slug",
    "strike_price",
    "strike_status",
    "strike_source",
    "strike_confidence",
    "strike_age_ms",
    "strike_before_ts_ms",
    "strike_before_price",
    "strike_after_ts_ms",
    "strike_after_price",
    "strike_gap_ms",
    "strike_note",
    "up_last_price",
    "down_last_price",
    "btc_display",  # Displayed BTC spot (same logic as UI)
    "btc_display_source",  # rtds_chainlink | rtds_usdt_fallback | rtds_chainlink_stale
    "btc_display_age_ms",  # age of displayed source
    "btc_spot_chainlink",  # Polymarket RTDS crypto_prices_chainlink btc/usd
    "btc_fast_usdt",  # Polymarket RTDS crypto_prices btcusdt (display only)
    "btc_inst_vol_coinbase_1m",
    "btc_inst_vol_source",
    "btc_inst_vol_candle_count",
    "btc_inst_vol_age_s",
    "btc_inst_vol_status",
    "btc_rt_vol_ewma",
    "btc_rt_vol_sigma_30s",
    "btc_rt_vol_sigma_120s",
    "btc_rt_candle_range_rel_1m",
    "btc_rt_candle_log_ret_open_1m",
    "btc_rt_trade_count_60s",
    "btc_rt_age_ms",
    "btc_rt_status",
]


class WindowCsvWriter:
    def __init__(self, data_dir: Path) -> None:
        self._data_dir = data_dir
        self._data_dir.mkdir(parents=True, exist_ok=True)
        self._current_epoch: int | None = None
        self._file: TextIO | None = None
        self._writer: csv.writer | None = None

    def rotate_if_needed(self, window_epoch: int) -> None:
        if self._current_epoch == window_epoch:
            return
        self.close()
        path = self._data_dir / f"btc_{window_epoch}.csv"
        self._file = path.open("a", newline="", encoding="utf-8")
        self._writer = csv.writer(self._file)
        if path.stat().st_size == 0:
            self._writer.writerow(CSV_HEADER)
            self._file.flush()
        self._current_epoch = window_epoch

    def write_row(self, row: list[object]) -> None:
        if not self._writer or not self._file:
            raise RuntimeError("CSV writer is not initialized.")
        self._writer.writerow(row)
        self._file.flush()

    def format_ts(self, ts_ms: int) -> str:
        dt = datetime.fromtimestamp(ts_ms / 1000, tz=timezone.utc)
        return dt.strftime("%d/%m/%Y -- %H:%M:%S") + f":{int(ts_ms % 1000):03d}"

    def close(self) -> None:
        if self._file:
            self._file.close()
            self._file = None
            self._writer = None
            self._current_epoch = None
