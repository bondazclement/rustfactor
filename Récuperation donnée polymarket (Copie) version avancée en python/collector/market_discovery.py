from __future__ import annotations

import json
import re
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any
from urllib.parse import urlparse

import httpx
from loguru import logger

SLUG_RE = re.compile(r"^btc-updown-5m-(\d+)$")


def normalize_slug(slug_or_url: str) -> str:
    """Extract btc-updown-5m-<epoch> from a raw slug or full Polymarket URL."""
    s = slug_or_url.strip()
    if "://" in s or s.startswith("//"):
        parsed = urlparse(s)
        path = (parsed.path or "").strip("/")
        parts = [p for p in path.split("/") if p]
        slug = parts[-1] if parts else s
    else:
        slug = s
    slug = slug.split("?")[0].split("#")[0].strip().strip("/")
    return slug


@dataclass(slots=True)
class ActiveWindow:
    window_epoch: int
    slug: str
    event_id: str
    up_token_id: str
    down_token_id: str
    title: str
    event_start_utc: datetime
    resolution_source: str | None


class GammaMarketDiscovery:
    def __init__(self, gamma_base_url: str) -> None:
        self._base = gamma_base_url.rstrip("/")
        self._client = httpx.AsyncClient(timeout=20.0)

    async def close(self) -> None:
        await self._client.aclose()

    async def fetch_by_slug(self, slug: str) -> ActiveWindow:
        slug = normalize_slug(slug)
        resp = await self._client.get(f"{self._base}/events", params={"slug": slug})
        resp.raise_for_status()
        items = resp.json()
        if not items:
            raise RuntimeError(f"Market with slug '{slug}' not found.")
        return self._parse_event(items[0])

    async def fetch_event_optional(self, slug: str) -> dict[str, Any] | None:
        resp = await self._client.get(f"{self._base}/events", params={"slug": slug})
        resp.raise_for_status()
        items = resp.json()
        return items[0] if items else None

    async def fetch_open_window_for_epoch(self, epoch: int) -> ActiveWindow:
        """Pick an open market for the current 5m slot: try epoch, then +300 / -300."""
        for ep in (epoch, epoch + 300, epoch - 300):
            if ep <= 0:
                continue
            slug = f"btc-updown-5m-{ep}"
            event = await self.fetch_event_optional(slug)
            if not event:
                continue
            market = (event.get("markets") or [None])[0]
            if not market or market.get("closed") is True:
                continue
            return self._parse_event(event)
        raise RuntimeError(
            f"No open btc-updown-5m market for slot {epoch} (tried +300/-300)."
        )

    async def fetch_active_window(self) -> ActiveWindow:
        """Resolve the active 5m window via time-aligned slug + Gamma (no generic list scan)."""
        now = int(time.time())
        return await self.fetch_open_window_for_epoch((now // 300) * 300)

    def _parse_event(self, event: dict[str, Any]) -> ActiveWindow:
        slug = event.get("slug", "")
        m = SLUG_RE.match(slug)
        epoch = int(m.group(1)) if m else 0

        market = (event.get("markets") or [{}])[0]
        outcomes = _parse_json_list(market.get("outcomes"))
        token_ids = _parse_json_list(market.get("clobTokenIds"))

        if len(outcomes) != 2 or len(token_ids) != 2:
            raise RuntimeError(f"Invalid outcomes/token IDs for event={slug}")

        outcome_to_token = {str(outcomes[i]).strip().lower(): str(token_ids[i]) for i in range(2)}
        up_token = outcome_to_token.get("up")
        down_token = outcome_to_token.get("down")

        if not up_token or not down_token:
            raise RuntimeError(f"Unable to map UP/DOWN outcomes for event={slug}")

        est_raw = market.get("eventStartTime")
        if est_raw:
            event_start = _parse_event_start(est_raw)
        else:
            event_start = datetime.fromtimestamp(float(epoch), tz=timezone.utc)

        res_src = market.get("resolutionSource") or event.get("resolutionSource")

        return ActiveWindow(
            window_epoch=epoch,
            slug=slug,
            event_id=str(event.get("id", "")),
            up_token_id=up_token,
            down_token_id=down_token,
            title=event.get("title", ""),
            event_start_utc=event_start,
            resolution_source=str(res_src) if res_src else None,
        )


def _parse_event_start(raw: Any) -> datetime:
    if not raw:
        return datetime.now(timezone.utc)
    if isinstance(raw, datetime):
        return raw if raw.tzinfo else raw.replace(tzinfo=timezone.utc)
    s = str(raw).strip()
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    dt = datetime.fromisoformat(s)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(timezone.utc)


def _parse_json_list(raw: Any) -> list[Any]:
    if raw is None:
        return []
    if isinstance(raw, list):
        return raw
    if isinstance(raw, str):
        try:
            value = json.loads(raw)
            return value if isinstance(value, list) else []
        except json.JSONDecodeError:
            return []
    return []
