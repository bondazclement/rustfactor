from __future__ import annotations

import asyncio
import json
import math
import statistics
import time
from collections import deque
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Literal

import httpx
import websockets

SourceMode = Literal["auto", "exchange", "advanced"]


@dataclass(slots=True)
class VolatilitySnapshot:
    inst_vol: float = 0.0
    source: str = "none"
    candle_count: int = 0
    latest_candle_start_s: int | None = None
    updated_ms: int | None = None
    status: str = "initializing"
    last_error: str | None = None
    rt_vol_ewma: float = 0.0
    rt_vol_sigma_30s: float = 0.0
    rt_vol_sigma_120s: float = 0.0
    rt_candle_range_rel_1m: float = 0.0
    rt_candle_log_ret_open_1m: float = 0.0
    rt_trade_count_60s: int = 0
    rt_last_trade_ts_ms: int | None = None
    rt_status: str = "initializing"

    @property
    def age_s(self) -> int | None:
        if self.latest_candle_start_s is None:
            return None
        return max(0, int(time.time()) - int(self.latest_candle_start_s))

    @property
    def rt_age_ms(self) -> int | None:
        if self.rt_last_trade_ts_ms is None:
            return None
        return max(0, int(time.time() * 1000) - int(self.rt_last_trade_ts_ms))


class CoinbaseCandleVolatility:
    def __init__(
        self,
        *,
        product_id: str = "BTC-USD",
        source_mode: SourceMode = "auto",
        poll_interval_s: float = 10.0,
        window_candles: int = 60,
        ws_url: str = "wss://ws-feed.exchange.coinbase.com",
        rt_enabled: bool = True,
        rt_bar_ms: int = 200,
        rt_ewma_halflife_s: float = 12.0,
        rt_window_fast_s: int = 30,
        rt_window_slow_s: int = 120,
        rt_stale_after_ms: int = 6000,
    ) -> None:
        self._product_id = product_id
        self._source_mode: SourceMode = source_mode
        self._poll_interval_s = poll_interval_s
        self._window_candles = window_candles
        self._ws_url = ws_url
        self._rt_enabled = rt_enabled
        self._rt_bar_ms = max(50, int(rt_bar_ms))
        self._rt_ewma_halflife_s = max(0.5, float(rt_ewma_halflife_s))
        self._rt_window_fast_s = max(5, int(rt_window_fast_s))
        self._rt_window_slow_s = max(self._rt_window_fast_s, int(rt_window_slow_s))
        self._rt_stale_after_ms = max(1000, int(rt_stale_after_ms))
        self._snapshot = VolatilitySnapshot()
        self._lock = asyncio.Lock()
        self._stop = asyncio.Event()
        self._rest_task: asyncio.Task[None] | None = None
        self._ws_task: asyncio.Task[None] | None = None
        self._http = httpx.AsyncClient(timeout=15.0)
        self._ws_connected = False

        self._rt_bars: deque[tuple[int, float]] = deque()
        self._rt_returns: deque[tuple[int, float]] = deque()
        self._rt_trade_ts: deque[int] = deque()
        self._rt_ewma_var: float | None = None
        self._rt_current_bucket_ms: int | None = None
        self._rt_current_bucket_price: float | None = None
        self._minute_start_ms: int | None = None
        self._minute_open: float | None = None
        self._minute_high: float | None = None
        self._minute_low: float | None = None
        self._minute_close: float | None = None

    async def start(self) -> None:
        self._stop.clear()
        self._rest_task = asyncio.create_task(self._run_rest(), name="coinbase-volatility-rest-loop")
        if self._rt_enabled:
            self._ws_task = asyncio.create_task(self._run_ws(), name="coinbase-volatility-ws-loop")
        else:
            await self._set_rt_status("disabled")

    async def stop(self) -> None:
        self._stop.set()
        tasks: list[asyncio.Task[None]] = []
        if self._rest_task:
            self._rest_task.cancel()
            tasks.append(self._rest_task)
            self._rest_task = None
        if self._ws_task:
            self._ws_task.cancel()
            tasks.append(self._ws_task)
            self._ws_task = None
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        await self._http.aclose()

    async def update_config(
        self,
        *,
        source_mode: str | None = None,
        poll_interval_s: float | None = None,
        window_candles: int | None = None,
    ) -> None:
        async with self._lock:
            if source_mode is not None:
                self._source_mode = self._normalize_source_mode(source_mode)
            if poll_interval_s is not None:
                self._poll_interval_s = max(1.0, float(poll_interval_s))
            if window_candles is not None:
                self._window_candles = max(5, int(window_candles))

    async def snapshot(self) -> VolatilitySnapshot:
        async with self._lock:
            rt_status = self._derive_rt_status(
                current=self._snapshot.rt_status,
                rt_last_trade_ts_ms=self._snapshot.rt_last_trade_ts_ms,
            )
            return VolatilitySnapshot(
                inst_vol=self._snapshot.inst_vol,
                source=self._snapshot.source,
                candle_count=self._snapshot.candle_count,
                latest_candle_start_s=self._snapshot.latest_candle_start_s,
                updated_ms=self._snapshot.updated_ms,
                status=self._snapshot.status,
                last_error=self._snapshot.last_error,
                rt_vol_ewma=self._snapshot.rt_vol_ewma,
                rt_vol_sigma_30s=self._snapshot.rt_vol_sigma_30s,
                rt_vol_sigma_120s=self._snapshot.rt_vol_sigma_120s,
                rt_candle_range_rel_1m=self._snapshot.rt_candle_range_rel_1m,
                rt_candle_log_ret_open_1m=self._snapshot.rt_candle_log_ret_open_1m,
                rt_trade_count_60s=self._snapshot.rt_trade_count_60s,
                rt_last_trade_ts_ms=self._snapshot.rt_last_trade_ts_ms,
                rt_status=rt_status,
            )

    async def _run_rest(self) -> None:
        while not self._stop.is_set():
            try:
                await self._refresh_once()
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                await self._set_error(f"refresh_error:{exc}")

            async with self._lock:
                wait_s = max(1.0, float(self._poll_interval_s))
            await asyncio.sleep(wait_s)

    async def _run_ws(self) -> None:
        backoff_s = 1.0
        max_backoff_s = 15.0
        while not self._stop.is_set():
            try:
                async with websockets.connect(self._ws_url, ping_interval=20, ping_timeout=20) as ws:
                    await ws.send(
                        json.dumps(
                            {
                                "type": "subscribe",
                                "product_ids": [self._product_id],
                                "channels": ["matches"],
                            }
                        )
                    )
                    self._ws_connected = True
                    await self._set_rt_status("ok")
                    backoff_s = 1.0
                    while not self._stop.is_set():
                        raw = await asyncio.wait_for(ws.recv(), timeout=15)
                        data = json.loads(raw)
                        if data.get("type") == "match":
                            ts_ms = self._decode_match_time_ms(data.get("time"))
                            price = self._to_float(data.get("price"))
                            if ts_ms is not None and price is not None and price > 0:
                                await self._ingest_trade(ts_ms, price)
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                self._ws_connected = False
                await self._set_rt_status(f"ws_disconnected:{exc.__class__.__name__}")
                await asyncio.sleep(backoff_s)
                backoff_s = min(max_backoff_s, backoff_s * 1.8)
        self._ws_connected = False

    async def _refresh_once(self) -> None:
        async with self._lock:
            source_mode = self._source_mode
            window_candles = self._window_candles
        order = self._source_order(source_mode)
        errors: list[str] = []
        for source in order:
            try:
                candles = await self._fetch_candles(source, window_candles)
                inst_vol, candle_count, latest_start = self._compute_volatility(
                    candles,
                    window_candles=window_candles,
                )
                await self._set_snapshot(
                    VolatilitySnapshot(
                        inst_vol=inst_vol,
                        source=source,
                        candle_count=candle_count,
                        latest_candle_start_s=latest_start,
                        updated_ms=int(time.time() * 1000),
                        status="ok",
                        last_error=None,
                        rt_vol_ewma=self._snapshot.rt_vol_ewma,
                        rt_vol_sigma_30s=self._snapshot.rt_vol_sigma_30s,
                        rt_vol_sigma_120s=self._snapshot.rt_vol_sigma_120s,
                        rt_candle_range_rel_1m=self._snapshot.rt_candle_range_rel_1m,
                        rt_candle_log_ret_open_1m=self._snapshot.rt_candle_log_ret_open_1m,
                        rt_trade_count_60s=self._snapshot.rt_trade_count_60s,
                        rt_last_trade_ts_ms=self._snapshot.rt_last_trade_ts_ms,
                        rt_status=self._snapshot.rt_status,
                    )
                )
                return
            except Exception as exc:
                errors.append(f"{source}:{exc}")
        await self._set_error(" | ".join(errors) if errors else "unknown_error")

    async def _set_snapshot(self, snapshot: VolatilitySnapshot) -> None:
        async with self._lock:
            self._snapshot = snapshot

    async def _set_error(self, error: str) -> None:
        async with self._lock:
            self._snapshot = VolatilitySnapshot(
                inst_vol=self._snapshot.inst_vol,
                source=self._snapshot.source,
                candle_count=self._snapshot.candle_count,
                latest_candle_start_s=self._snapshot.latest_candle_start_s,
                updated_ms=int(time.time() * 1000),
                status="error",
                last_error=error,
                rt_vol_ewma=self._snapshot.rt_vol_ewma,
                rt_vol_sigma_30s=self._snapshot.rt_vol_sigma_30s,
                rt_vol_sigma_120s=self._snapshot.rt_vol_sigma_120s,
                rt_candle_range_rel_1m=self._snapshot.rt_candle_range_rel_1m,
                rt_candle_log_ret_open_1m=self._snapshot.rt_candle_log_ret_open_1m,
                rt_trade_count_60s=self._snapshot.rt_trade_count_60s,
                rt_last_trade_ts_ms=self._snapshot.rt_last_trade_ts_ms,
                rt_status=self._snapshot.rt_status,
            )

    async def _set_rt_status(self, rt_status: str) -> None:
        async with self._lock:
            self._snapshot.rt_status = rt_status

    async def _ingest_trade(self, ts_ms: int, price: float) -> None:
        self._rt_trade_ts.append(ts_ms)
        minute_start_ms = (ts_ms // 60000) * 60000
        if self._minute_start_ms != minute_start_ms:
            self._minute_start_ms = minute_start_ms
            self._minute_open = price
            self._minute_high = price
            self._minute_low = price
            self._minute_close = price
        else:
            self._minute_close = price
            self._minute_high = price if self._minute_high is None else max(self._minute_high, price)
            self._minute_low = price if self._minute_low is None else min(self._minute_low, price)

        bucket_ms = (ts_ms // self._rt_bar_ms) * self._rt_bar_ms
        if self._rt_current_bucket_ms is None:
            self._rt_current_bucket_ms = bucket_ms
            self._rt_current_bucket_price = price
            await self._publish_rt_snapshot(ts_ms)
            return

        if bucket_ms != self._rt_current_bucket_ms:
            if self._rt_current_bucket_price is not None:
                self._append_bar(self._rt_current_bucket_ms, self._rt_current_bucket_price)
            self._rt_current_bucket_ms = bucket_ms
            self._rt_current_bucket_price = price
        else:
            self._rt_current_bucket_price = price

        await self._publish_rt_snapshot(ts_ms)

    def _append_bar(self, bar_ts_ms: int, close: float) -> None:
        prev_close: float | None = self._rt_bars[-1][1] if self._rt_bars else None
        self._rt_bars.append((bar_ts_ms, close))
        if prev_close is not None and prev_close > 0 and close > 0:
            r = math.log(close / prev_close)
            self._rt_returns.append((bar_ts_ms, r))
            alpha = 1.0 - math.exp(-math.log(2.0) * (self._rt_bar_ms / 1000.0) / self._rt_ewma_halflife_s)
            if self._rt_ewma_var is None:
                self._rt_ewma_var = r * r
            else:
                self._rt_ewma_var = (1.0 - alpha) * self._rt_ewma_var + alpha * (r * r)

        cutoff_bars = bar_ts_ms - (self._rt_window_slow_s + 120) * 1000
        while self._rt_bars and self._rt_bars[0][0] < cutoff_bars:
            self._rt_bars.popleft()

        cutoff_returns = bar_ts_ms - (self._rt_window_slow_s + 120) * 1000
        while self._rt_returns and self._rt_returns[0][0] < cutoff_returns:
            self._rt_returns.popleft()

    async def _publish_rt_snapshot(self, ts_ms: int) -> None:
        cutoff_60 = ts_ms - 60000
        while self._rt_trade_ts and self._rt_trade_ts[0] < cutoff_60:
            self._rt_trade_ts.popleft()

        vals_30 = [r for r_ts, r in self._rt_returns if r_ts >= ts_ms - self._rt_window_fast_s * 1000]
        vals_120 = [r for r_ts, r in self._rt_returns if r_ts >= ts_ms - self._rt_window_slow_s * 1000]
        sigma_30 = float(statistics.pstdev(vals_30)) if len(vals_30) >= 2 else 0.0
        sigma_120 = float(statistics.pstdev(vals_120)) if len(vals_120) >= 2 else 0.0
        ewma = math.sqrt(self._rt_ewma_var) if self._rt_ewma_var is not None else 0.0

        range_rel = 0.0
        log_ret_open = 0.0
        if (
            self._minute_open is not None
            and self._minute_high is not None
            and self._minute_low is not None
            and self._minute_close is not None
        ):
            mid = (self._minute_high + self._minute_low) / 2.0
            if mid > 0:
                range_rel = (self._minute_high - self._minute_low) / mid
            if self._minute_open > 0 and self._minute_close > 0:
                log_ret_open = math.log(self._minute_close / self._minute_open)

        rt_status = self._derive_rt_status(
            current="ok",
            rt_last_trade_ts_ms=ts_ms,
            sample_count=len(vals_30),
        )
        async with self._lock:
            self._snapshot.rt_vol_ewma = ewma
            self._snapshot.rt_vol_sigma_30s = sigma_30
            self._snapshot.rt_vol_sigma_120s = sigma_120
            self._snapshot.rt_candle_range_rel_1m = range_rel
            self._snapshot.rt_candle_log_ret_open_1m = log_ret_open
            self._snapshot.rt_trade_count_60s = len(self._rt_trade_ts)
            self._snapshot.rt_last_trade_ts_ms = ts_ms
            self._snapshot.rt_status = rt_status

    def _derive_rt_status(
        self,
        *,
        current: str,
        rt_last_trade_ts_ms: int | None,
        sample_count: int | None = None,
    ) -> str:
        if not self._rt_enabled:
            return "disabled"
        if not self._ws_connected:
            return current if current.startswith("ws_disconnected") else "ws_disconnected"
        if rt_last_trade_ts_ms is None:
            return "initializing"
        age_ms = int(time.time() * 1000) - int(rt_last_trade_ts_ms)
        if age_ms > self._rt_stale_after_ms:
            return "stale"
        if sample_count is not None and sample_count < 2:
            return "insufficient_samples"
        return "ok"

    def _decode_match_time_ms(self, raw_time: object) -> int | None:
        if not isinstance(raw_time, str) or not raw_time:
            return None
        text = raw_time
        if text.endswith("Z"):
            text = text[:-1] + "+00:00"
        try:
            dt = datetime.fromisoformat(text)
        except ValueError:
            return None
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return int(dt.timestamp() * 1000)

    def _to_float(self, value: object) -> float | None:
        try:
            if value is None:
                return None
            return float(value)
        except (TypeError, ValueError):
            return None

    def _source_order(self, mode: SourceMode) -> list[str]:
        if mode == "exchange":
            return ["exchange", "advanced"]
        if mode == "advanced":
            return ["advanced", "exchange"]
        return ["exchange", "advanced"]

    async def _fetch_candles(self, source: str, window_candles: int) -> list[tuple[int, float]]:
        needed = max(window_candles + 10, 40)
        now = int(time.time())
        start = now - needed * 60
        if source == "exchange":
            url = f"https://api.exchange.coinbase.com/products/{self._product_id}/candles"
            params = {"start": start, "end": now, "granularity": 60}
            r = await self._http.get(url, params=params, headers={"User-Agent": "polymarket-collector/1.0"})
            r.raise_for_status()
            rows = r.json()
            candles: list[tuple[int, float]] = []
            if not isinstance(rows, list):
                raise RuntimeError("exchange_bad_payload")
            for row in rows:
                if not isinstance(row, list) or len(row) < 5:
                    continue
                ts = int(row[0])
                close = float(row[4])
                candles.append((ts, close))
            return candles

        if source == "advanced":
            url = f"https://api.coinbase.com/api/v3/brokerage/market/products/{self._product_id}/candles"
            params = {
                "start": str(start),
                "end": str(now),
                "granularity": "ONE_MINUTE",
                "limit": str(min(350, needed + 20)),
            }
            r = await self._http.get(url, params=params, headers={"User-Agent": "polymarket-collector/1.0"})
            r.raise_for_status()
            payload = r.json()
            rows = payload.get("candles", [])
            if not isinstance(rows, list):
                raise RuntimeError("advanced_bad_payload")
            candles = []
            for row in rows:
                if not isinstance(row, dict):
                    continue
                ts = int(row["start"])
                close = float(row["close"])
                candles.append((ts, close))
            return candles

        raise RuntimeError(f"unknown_source:{source}")

    def _compute_volatility(
        self, candles: list[tuple[int, float]], *, window_candles: int
    ) -> tuple[float, int, int]:
        if not candles:
            raise RuntimeError("empty_candles")

        now = int(time.time())
        current_open_start = (now // 60) * 60
        latest_closed_start = current_open_start - 60
        dedup: dict[int, float] = {}
        for ts, close in candles:
            if ts <= latest_closed_start:
                dedup[int(ts)] = float(close)
        ordered = sorted(dedup.items(), key=lambda x: x[0])
        closes = [price for _, price in ordered]
        starts = [ts for ts, _ in ordered]
        if len(closes) < 3:
            raise RuntimeError("not_enough_closed_candles")

        closes = closes[-window_candles:]
        starts = starts[-window_candles:]
        returns: list[float] = []
        for idx in range(1, len(closes)):
            if closes[idx - 1] <= 0 or closes[idx] <= 0:
                continue
            returns.append(math.log(closes[idx] / closes[idx - 1]))
        if len(returns) < 2:
            raise RuntimeError("not_enough_returns")
        return float(statistics.pstdev(returns)), len(closes), int(starts[-1])

    def _normalize_source_mode(self, source_mode: str) -> SourceMode:
        mode = source_mode.strip().lower()
        if mode not in ("auto", "exchange", "advanced"):
            raise ValueError("coinbase_source_mode must be auto/exchange/advanced")
        return mode  # type: ignore[return-value]
