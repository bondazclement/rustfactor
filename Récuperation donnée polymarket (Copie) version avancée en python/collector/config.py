from __future__ import annotations

import os
import re
from dataclasses import dataclass
from pathlib import Path

from dotenv import load_dotenv


def _optional_float(raw: str) -> float | None:
    s = raw.strip()
    if not s:
        return None
    return float(s)


def _resolve_control_socket_path(root: Path, raw: str) -> Path:
    candidate = Path(raw.strip()) if raw.strip() else Path("/tmp/polymarket_btc5m_control.sock")
    if not candidate.is_absolute():
        candidate = root / candidate
    if len(str(candidate)) <= 100:
        return candidate
    # AF_UNIX path limit is usually 108 bytes on Linux.
    safe = re.sub(r"[^a-zA-Z0-9_-]+", "_", root.name)[:24]
    return Path(f"/tmp/{safe}_control.sock")


@dataclass(slots=True)
class Settings:
    gamma_base_url: str
    clob_base_url: str
    polymarket_ws_url: str
    rtds_ws_url: str
    sample_interval_ms: int
    ui_refresh_ms: int
    vol_window_ticks: int
    discovery_interval_seconds: float
    log_level: str
    data_dir: Path
    log_dir: Path
    initial_strike_usd: float | None
    strike_overrides_path: Path
    strike_confidence_threshold: float
    strike_confidence_gap_ms: int
    strike_allow_approx_auto: bool
    strike_exact_tolerance_ms: int
    rtds_fast_btcusdt: bool
    coinbase_vol_window_candles: int
    coinbase_poll_interval_seconds: float
    coinbase_source_mode: str
    coinbase_product_id: str
    coinbase_ws_url: str
    coinbase_rt_enabled: bool
    coinbase_rt_bar_ms: int
    coinbase_rt_ewma_halflife_s: float
    coinbase_rt_window_fast_s: int
    coinbase_rt_window_slow_s: int
    coinbase_rt_stale_after_ms: int
    runtime_control_socket_path: Path

RUNTIME_MUTABLE_FIELDS = (
    "sample_interval_ms",
    "ui_refresh_ms",
    "discovery_interval_seconds",
    "strike_confidence_threshold",
    "strike_confidence_gap_ms",
    "strike_allow_approx_auto",
    "strike_exact_tolerance_ms",
    "rtds_fast_btcusdt",
    "coinbase_vol_window_candles",
    "coinbase_poll_interval_seconds",
    "coinbase_source_mode",
)


def parse_runtime_value(field: str, raw_value: str) -> object:
    name = field.strip().lower()
    raw = raw_value.strip()
    if not raw:
        raise ValueError("empty value")
    if name in ("sample_interval_ms", "ui_refresh_ms", "strike_confidence_gap_ms", "strike_exact_tolerance_ms"):
        value = int(raw)
    elif name in ("discovery_interval_seconds", "strike_confidence_threshold", "coinbase_poll_interval_seconds"):
        value = float(raw)
    elif name in ("strike_allow_approx_auto", "rtds_fast_btcusdt"):
        v = raw.lower()
        if v in ("1", "true", "yes", "on"):
            value = True
        elif v in ("0", "false", "no", "off"):
            value = False
        else:
            raise ValueError("expected boolean (true/false)")
    elif name in ("coinbase_vol_window_candles",):
        value = int(raw)
    elif name in ("coinbase_source_mode",):
        value = raw.lower()
    else:
        raise ValueError(f"unknown runtime field: {field}")
    validate_runtime_value(name, value)
    return value


def validate_runtime_value(field: str, value: object) -> None:
    name = field.strip().lower()
    if name == "sample_interval_ms":
        if not isinstance(value, int) or value < 50:
            raise ValueError("sample_interval_ms must be an integer >= 50")
        return
    if name == "ui_refresh_ms":
        if not isinstance(value, int) or value < 50:
            raise ValueError("ui_refresh_ms must be an integer >= 50")
        return
    if name == "discovery_interval_seconds":
        if not isinstance(value, (float, int)) or float(value) < 0.5:
            raise ValueError("discovery_interval_seconds must be >= 0.5")
        return
    if name == "strike_confidence_threshold":
        if not isinstance(value, (float, int)) or not (0.0 <= float(value) <= 1.0):
            raise ValueError("strike_confidence_threshold must be between 0.0 and 1.0")
        return
    if name == "strike_confidence_gap_ms":
        if not isinstance(value, int) or value < 1:
            raise ValueError("strike_confidence_gap_ms must be an integer >= 1")
        return
    if name == "strike_allow_approx_auto":
        if not isinstance(value, bool):
            raise ValueError("strike_allow_approx_auto must be a boolean")
        return
    if name == "strike_exact_tolerance_ms":
        if not isinstance(value, int) or value < 0:
            raise ValueError("strike_exact_tolerance_ms must be an integer >= 0")
        return
    if name == "rtds_fast_btcusdt":
        if not isinstance(value, bool):
            raise ValueError("rtds_fast_btcusdt must be a boolean")
        return
    if name == "coinbase_vol_window_candles":
        if not isinstance(value, int) or value < 5:
            raise ValueError("coinbase_vol_window_candles must be an integer >= 5")
        return
    if name == "coinbase_poll_interval_seconds":
        if not isinstance(value, (float, int)) or float(value) < 1.0:
            raise ValueError("coinbase_poll_interval_seconds must be >= 1.0")
        return
    if name == "coinbase_source_mode":
        if not isinstance(value, str) or value not in ("auto", "exchange", "advanced"):
            raise ValueError("coinbase_source_mode must be auto/exchange/advanced")
        return
    raise ValueError(f"unknown runtime field: {field}")


def load_settings() -> Settings:
    load_dotenv()
    root = Path.cwd()
    return Settings(
        gamma_base_url=os.getenv("GAMMA_BASE_URL", "https://gamma-api.polymarket.com"),
        clob_base_url=os.getenv("CLOB_BASE_URL", "https://clob.polymarket.com"),
        polymarket_ws_url=os.getenv(
            "POLYMARKET_WS_URL", "wss://ws-subscriptions-clob.polymarket.com/ws/market"
        ),
        rtds_ws_url=os.getenv("RTDS_WS_URL", "wss://ws-live-data.polymarket.com"),
        sample_interval_ms=int(os.getenv("SAMPLE_INTERVAL_MS", "500")),
        ui_refresh_ms=int(os.getenv("UI_REFRESH_MS", "100")),
        vol_window_ticks=int(os.getenv("VOL_WINDOW_TICKS", "120")),
        discovery_interval_seconds=float(os.getenv("DISCOVERY_INTERVAL_SECONDS", "5")),
        log_level=os.getenv("LOG_LEVEL", "INFO").upper(),
        data_dir=root / os.getenv("DATA_DIR", "data_5m"),
        log_dir=root / os.getenv("LOG_DIR", "logs"),
        initial_strike_usd=_optional_float(os.getenv("INITIAL_STRIKE_USD", "")),
        strike_overrides_path=root
        / os.getenv("STRIKE_OVERRIDES_PATH", "data_5m/strike_overrides.json"),
        strike_confidence_threshold=float(os.getenv("STRIKE_CONFIDENCE_THRESHOLD", "0.92")),
        strike_confidence_gap_ms=int(os.getenv("STRIKE_CONFIDENCE_GAP_MS", "1500")),
        strike_allow_approx_auto=os.getenv("STRIKE_ALLOW_APPROX_AUTO", "false").lower()
        in ("1", "true", "yes"),
        strike_exact_tolerance_ms=int(os.getenv("STRIKE_EXACT_TOLERANCE_MS", "0")),
        rtds_fast_btcusdt=os.getenv("RTDS_FAST_BTCUSDT", "true").lower()
        in ("1", "true", "yes"),
        coinbase_vol_window_candles=int(os.getenv("COINBASE_VOL_WINDOW_CANDLES", "60")),
        coinbase_poll_interval_seconds=float(os.getenv("COINBASE_POLL_INTERVAL_SECONDS", "10")),
        coinbase_source_mode=os.getenv("COINBASE_SOURCE_MODE", "auto").strip().lower(),
        coinbase_product_id=os.getenv("COINBASE_PRODUCT_ID", "BTC-USD").strip().upper(),
        coinbase_ws_url=os.getenv("COINBASE_WS_URL", "wss://ws-feed.exchange.coinbase.com").strip(),
        coinbase_rt_enabled=os.getenv("COINBASE_RT_ENABLED", "true").lower()
        in ("1", "true", "yes"),
        coinbase_rt_bar_ms=int(os.getenv("COINBASE_RT_BAR_MS", "200")),
        coinbase_rt_ewma_halflife_s=float(os.getenv("COINBASE_RT_EWMA_HALFLIFE_S", "12")),
        coinbase_rt_window_fast_s=int(os.getenv("COINBASE_RT_WINDOW_FAST_S", "30")),
        coinbase_rt_window_slow_s=int(os.getenv("COINBASE_RT_WINDOW_SLOW_S", "120")),
        coinbase_rt_stale_after_ms=int(os.getenv("COINBASE_RT_STALE_AFTER_MS", "6000")),
        runtime_control_socket_path=_resolve_control_socket_path(
            root, os.getenv("RUNTIME_CONTROL_SOCKET_PATH", "/tmp/polymarket_btc5m_control.sock")
        ),
    )
