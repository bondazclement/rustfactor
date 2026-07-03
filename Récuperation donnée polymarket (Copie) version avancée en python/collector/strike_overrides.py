from __future__ import annotations

import json
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass(slots=True)
class StrikeOverride:
    window_epoch: int
    value: float
    source: str
    updated_ms: int


class StrikeOverrideStore:
    def __init__(self, path: Path) -> None:
        self._path = path
        self._cache: dict[int, StrikeOverride] = {}
        self._last_mtime_ns: int = -1

    @property
    def path(self) -> Path:
        return self._path

    def upsert(self, window_epoch: int, value: float, source: str) -> StrikeOverride:
        now_ms = int(time.time() * 1000)
        ov = StrikeOverride(
            window_epoch=window_epoch,
            value=float(value),
            source=source,
            updated_ms=now_ms,
        )
        self.reload(force=True)
        self._cache[window_epoch] = ov
        self._write()
        return ov

    def get(self, window_epoch: int) -> StrikeOverride | None:
        self.reload(force=False)
        return self._cache.get(window_epoch)

    def reload(self, *, force: bool) -> None:
        if force:
            self._load()
            return
        try:
            st = self._path.stat()
            if st.st_mtime_ns == self._last_mtime_ns:
                return
        except FileNotFoundError:
            if self._last_mtime_ns != -1:
                self._cache = {}
                self._last_mtime_ns = -1
            return
        self._load()

    def _load(self) -> None:
        if not self._path.exists():
            self._cache = {}
            self._last_mtime_ns = -1
            return
        raw = json.loads(self._path.read_text(encoding="utf-8"))
        out: dict[int, StrikeOverride] = {}
        windows = raw.get("windows", {}) if isinstance(raw, dict) else {}
        if isinstance(windows, dict):
            for k, v in windows.items():
                try:
                    ep = int(k)
                except (TypeError, ValueError):
                    continue
                if not isinstance(v, dict):
                    continue
                try:
                    out[ep] = StrikeOverride(
                        window_epoch=ep,
                        value=float(v["value"]),
                        source=str(v.get("source", "manual_cli")),
                        updated_ms=int(v.get("updated_ms", 0)),
                    )
                except (KeyError, TypeError, ValueError):
                    continue
        self._cache = out
        self._last_mtime_ns = self._path.stat().st_mtime_ns

    def _write(self) -> None:
        self._path.parent.mkdir(parents=True, exist_ok=True)
        payload: dict[str, Any] = {"windows": {}}
        for ep, ov in sorted(self._cache.items()):
            payload["windows"][str(ep)] = {
                "value": ov.value,
                "source": ov.source,
                "updated_ms": ov.updated_ms,
            }
        self._path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        self._last_mtime_ns = self._path.stat().st_mtime_ns
