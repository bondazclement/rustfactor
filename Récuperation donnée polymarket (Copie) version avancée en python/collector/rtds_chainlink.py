from __future__ import annotations

import asyncio
import json
import random
import time
from typing import Any

import websockets
from loguru import logger


def _decode_price(value: Any) -> float | None:
    try:
        if value is None:
            return None
        x = float(value)
        if x > 1e15:
            x /= 1e18
        return x
    except (TypeError, ValueError):
        return None


class PolymarketRtdsBtcClient:
    """BTC/USD via Polymarket RTDS (crypto_prices_chainlink) — aligns with Polymarket crypto markets."""

    def __init__(self, ws_url: str, vol_window_ticks: int, symbol: str = "btc/usd") -> None:
        self._ws_url = ws_url.rstrip("/")
        self._symbol = symbol
        _ = vol_window_ticks
        self._stop = asyncio.Event()
        self._task: asyncio.Task[None] | None = None
        self._ping_task: asyncio.Task[None] | None = None
        self._last_price: float | None = None
        self._last_ts_ms: int | None = None
        self._last_recv_ts_ms: int | None = None
        self._lock = asyncio.Lock()
        self._tick_buf: list[tuple[int, float]] = []
        self._buf_max = 512

    async def start(self) -> None:
        self._stop.clear()
        self._task = asyncio.create_task(self._run(), name="rtds-btc")

    async def stop(self) -> None:
        self._stop.set()
        if self._ping_task:
            self._ping_task.cancel()
            try:
                await self._ping_task
            except asyncio.CancelledError:
                pass
            self._ping_task = None
        if self._task:
            await self._task
            self._task = None

    @property
    def spot(self) -> float | None:
        return self._last_price

    @property
    def spot_ts_ms(self) -> int | None:
        return self._last_ts_ms

    @property
    def inst_vol(self) -> float:
        # Deprecated: volatility is now computed from Coinbase 1m candles.
        return 0.0

    @property
    def recv_ts_ms(self) -> int | None:
        return self._last_recv_ts_ms

    @property
    def age_ms(self) -> int | None:
        if self._last_ts_ms is None:
            return None
        return max(0, int(time.time() * 1000) - int(self._last_ts_ms))

    async def snapshot_tick_history(self) -> list[tuple[int, float]]:
        async with self._lock:
            return list(self._tick_buf)

    async def _append_tick(self, ts_ms: int, price: float) -> None:
        async with self._lock:
            self._tick_buf.append((ts_ms, price))
            if len(self._tick_buf) > self._buf_max:
                del self._tick_buf[: len(self._tick_buf) - self._buf_max]

    async def _run(self) -> None:
        backoff = 1.0
        refresh_timeout_s = 5.5
        sub = {
            "action": "subscribe",
            "subscriptions": [
                {
                    "topic": "crypto_prices_chainlink",
                    "type": "*",
                    "filters": json.dumps({"symbol": self._symbol}),
                }
            ],
        }
        while not self._stop.is_set():
            try:
                logger.bind(ws_rtds=True).info("Connecting Polymarket RTDS ({})", self._symbol)
                async with websockets.connect(self._ws_url, ping_interval=None) as ws:
                    await ws.send(json.dumps(sub))
                    self._ping_task = asyncio.create_task(self._ping_loop(ws), name="rtds-ping")
                    backoff = 1.0
                    while not self._stop.is_set():
                        started = asyncio.get_running_loop().time()
                        try:
                            raw = await asyncio.wait_for(ws.recv(), timeout=refresh_timeout_s)
                        except asyncio.TimeoutError:
                            # Force a refresh from WS side to reduce stale periods on sparse feeds.
                            await ws.send(json.dumps(sub))
                            logger.bind(ws_rtds=True).debug(
                                "RTDS chainlink refresh subscribe (no tick for {:.1f}s)",
                                refresh_timeout_s,
                            )
                            continue
                        latency_ms = (asyncio.get_running_loop().time() - started) * 1000
                        logger.bind(ws_rtds=True).debug("RTDS recv latency={:.2f}ms", latency_ms)
                        if isinstance(raw, bytes):
                            raw = raw.decode()
                        if raw == "PONG":
                            continue
                        if not raw.strip():
                            continue
                        try:
                            msg = json.loads(raw)
                        except json.JSONDecodeError:
                            logger.bind(ws_rtds=True).debug("RTDS non-json: {!r}", raw[:120])
                            continue
                        await self._handle_message(msg)
            except asyncio.TimeoutError:
                logger.bind(ws_rtds=True).warning("RTDS recv timeout; reconnecting")
            except Exception as exc:
                logger.bind(ws_rtds=True).exception("RTDS error: {}", exc)
            finally:
                if self._ping_task:
                    self._ping_task.cancel()
                    try:
                        await self._ping_task
                    except asyncio.CancelledError:
                        pass
                    self._ping_task = None
            if self._stop.is_set():
                break
            sleep_for = min(backoff + random.random(), 15.0)
            logger.bind(ws_rtds=True).warning("RTDS reconnect in {:.1f}s", sleep_for)
            await asyncio.sleep(sleep_for)
            backoff = min(backoff * 2, 15.0)

    async def _ping_loop(self, ws: Any) -> None:
        try:
            while not self._stop.is_set():
                await asyncio.sleep(5.0)
                await ws.send("PING")
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            logger.bind(ws_rtds=True).debug("RTDS ping stopped: {}", exc)

    async def _handle_message(self, msg: dict[str, Any]) -> None:
        topic = msg.get("topic")
        mtype = msg.get("type")
        payload = msg.get("payload")
        if not isinstance(payload, dict):
            return
        sym = str(payload.get("symbol", "")).lower()
        if sym != self._symbol.lower():
            return

        if topic == "crypto_prices_chainlink" and mtype == "update":
            ts_ms = payload.get("timestamp")
            price = _decode_price(payload.get("value"))
            if ts_ms is not None and price is not None:
                await self._apply_tick(int(ts_ms), float(price))
            return

        if topic == "crypto_prices" and mtype == "subscribe":
            data = payload.get("data")
            if isinstance(data, list):
                for row in data:
                    if not isinstance(row, dict):
                        continue
                    ts_ms = row.get("timestamp")
                    price = _decode_price(row.get("value"))
                    if ts_ms is not None and price is not None:
                        await self._apply_tick(int(ts_ms), float(price))

    async def _apply_tick(self, ts_ms: int, price: float) -> None:
        self._last_price = price
        self._last_ts_ms = ts_ms
        self._last_recv_ts_ms = int(time.time() * 1000)
        await self._append_tick(ts_ms, price)


class PolymarketRtdsBtcUsdtClient:
    """
    Polymarket RTDS crypto_prices / btcusdt — faster ticks than crypto_prices_chainlink.
    Display only (not Polymarket resolution oracle).
    """

    def __init__(self, ws_url: str) -> None:
        self._ws_url = ws_url.rstrip("/")
        self._stop = asyncio.Event()
        self._task: asyncio.Task[None] | None = None
        self._last_price: float | None = None
        self._last_ts_ms: int | None = None
        self._last_recv_ts_ms: int | None = None
        self._ping_task: asyncio.Task[None] | None = None

    async def start(self) -> None:
        self._stop.clear()
        self._task = asyncio.create_task(self._run(), name="rtds-btcusdt")

    async def stop(self) -> None:
        self._stop.set()
        if self._ping_task:
            self._ping_task.cancel()
            try:
                await self._ping_task
            except asyncio.CancelledError:
                pass
            self._ping_task = None
        if self._task:
            await self._task
            self._task = None

    @property
    def spot(self) -> float | None:
        return self._last_price

    @property
    def spot_ts_ms(self) -> int | None:
        return self._last_ts_ms

    @property
    def recv_ts_ms(self) -> int | None:
        return self._last_recv_ts_ms

    @property
    def age_ms(self) -> int | None:
        if self._last_ts_ms is None:
            return None
        return max(0, int(time.time() * 1000) - int(self._last_ts_ms))

    async def _run(self) -> None:
        backoff = 1.0
        sub = {
            "action": "subscribe",
            "subscriptions": [
                {"topic": "crypto_prices", "type": "update", "filters": "btcusdt"},
            ],
        }
        while not self._stop.is_set():
            try:
                logger.bind(ws_rtds=True).info("Connecting Polymarket RTDS (btcusdt fast)")
                async with websockets.connect(self._ws_url, ping_interval=None) as ws:
                    await ws.send(json.dumps(sub))
                    self._ping_task = asyncio.create_task(self._ping_loop(ws), name="rtds-usdt-ping")
                    backoff = 1.0
                    while not self._stop.is_set():
                        raw = await asyncio.wait_for(ws.recv(), timeout=55)
                        if isinstance(raw, bytes):
                            raw = raw.decode()
                        if raw == "PONG":
                            continue
                        if not raw.strip():
                            continue
                        try:
                            msg = json.loads(raw)
                        except json.JSONDecodeError:
                            continue
                        self._handle(msg)
            except asyncio.TimeoutError:
                logger.bind(ws_rtds=True).warning("RTDS btcusdt recv timeout; reconnecting")
            except Exception as exc:
                logger.bind(ws_rtds=True).exception("RTDS btcusdt error: {}", exc)
            finally:
                if self._ping_task:
                    self._ping_task.cancel()
                    try:
                        await self._ping_task
                    except asyncio.CancelledError:
                        pass
                    self._ping_task = None
            if self._stop.is_set():
                break
            await asyncio.sleep(min(backoff + random.random(), 15.0))
            backoff = min(backoff * 2, 15.0)

    async def _ping_loop(self, ws: Any) -> None:
        try:
            while not self._stop.is_set():
                await asyncio.sleep(5.0)
                await ws.send("PING")
        except asyncio.CancelledError:
            raise
        except Exception:
            pass

    def _handle(self, msg: dict[str, Any]) -> None:
        if msg.get("topic") != "crypto_prices" or msg.get("type") != "update":
            return
        p = msg.get("payload")
        if not isinstance(p, dict):
            return
        if str(p.get("symbol", "")).lower() != "btcusdt":
            return
        ts_ms = p.get("timestamp")
        val = _decode_price(p.get("value"))
        if ts_ms is not None and val is not None:
            self._last_price = float(val)
            self._last_ts_ms = int(ts_ms)
            self._last_recv_ts_ms = int(time.time() * 1000)
