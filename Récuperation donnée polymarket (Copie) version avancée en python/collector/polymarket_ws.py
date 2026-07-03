from __future__ import annotations

import asyncio
import json
import random
import time
from dataclasses import dataclass
from typing import Any

import websockets
from loguru import logger


@dataclass(slots=True)
class LastPrice:
    price: float | None = None
    side: str | None = None
    updated_ms: int | None = None


class PolymarketWsClient:
    def __init__(self, ws_url: str, up_token_id: str, down_token_id: str) -> None:
        self._ws_url = ws_url
        self._token_ids = [up_token_id, down_token_id]
        self._state: dict[str, LastPrice] = {up_token_id: LastPrice(), down_token_id: LastPrice()}
        self._lock = asyncio.Lock()
        self._stop = asyncio.Event()
        self._task: asyncio.Task[None] | None = None

    async def start(self) -> None:
        self._stop.clear()
        self._task = asyncio.create_task(self._run(), name="polymarket-ws")

    async def stop(self) -> None:
        self._stop.set()
        if self._task:
            await self._task

    async def snapshot(self) -> dict[str, LastPrice]:
        async with self._lock:
            return {k: LastPrice(v.price, v.side, v.updated_ms) for k, v in self._state.items()}

    async def _run(self) -> None:
        backoff = 1.0
        while not self._stop.is_set():
            try:
                logger.bind(ws_poly=True).info("Connecting Polymarket WS (Market Channel)")
                async with websockets.connect(self._ws_url, ping_interval=10, ping_timeout=10) as ws:
                    # Subscribe to the market channel for last trade prices
                    payload = {
                        "type": "market",
                        "assets_ids": self._token_ids,
                    }
                    await ws.send(json.dumps(payload))
                    logger.bind(ws_poly=True).info("Subscribed to assets {}", self._token_ids)
                    backoff = 1.0
                    while not self._stop.is_set():
                        started = time.perf_counter()
                        raw = await asyncio.wait_for(ws.recv(), timeout=55)
                        latency_ms = (time.perf_counter() - started) * 1000
                        logger.bind(ws_poly=True).debug("WS recv latency={:.2f}ms", latency_ms)
                        msg = json.loads(raw)
                        # The market channel sends a list of events or a single event
                        events = msg if isinstance(msg, list) else [msg]
                        for event in events:
                            await self._handle_event(event)
            except asyncio.TimeoutError:
                logger.bind(ws_poly=True).warning("Polymarket WS recv timeout; reconnecting")
            except Exception as exc:
                logger.bind(ws_poly=True).exception("Polymarket WS error: {}", exc)
            if self._stop.is_set():
                break
            sleep_for = min(backoff + random.random(), 15.0)
            logger.bind(ws_poly=True).warning("Polymarket WS reconnect in {:.1f}s", sleep_for)
            await asyncio.sleep(sleep_for)
            backoff = min(backoff * 2, 15.0)

    async def _handle_event(self, event: dict[str, Any]) -> None:
        if not isinstance(event, dict):
            return
        
        etype = event.get("event_type")
        
        # We focus on last_trade_price for takers
        if etype == "last_trade_price":
            token = event.get("asset_id")
            if token in self._state:
                price = _safe_float(event.get("price"))
                side = str(event.get("side", ""))
                await self._update(token, price, side, event.get("timestamp"))
        
        # fallback: best_bid_ask if no trades yet, or to keep state fresh
        # but the requirement is to focus on takers data. 
        elif etype == "price_change":
            # Some price changes might contain last_trade_price updates too in some WS versions
            # But the user asked for taker data specifically.
            pass

    async def _update(self, token: str, price: float | None, side: str | None, ts: Any) -> None:
        async with self._lock:
            state = self._state[token]
            if price is not None:
                state.price = price
            if side is not None:
                state.side = side
            state.updated_ms = _safe_int(ts)


def _safe_float(v: Any) -> float | None:
    try:
        if v is None:
            return None
        return float(v)
    except (TypeError, ValueError):
        return None


def _safe_int(v: Any) -> int | None:
    try:
        if v is None:
            return None
        return int(v)
    except (TypeError, ValueError):
        return None
