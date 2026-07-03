from __future__ import annotations

import asyncio
import signal
import shlex
import time
from collections import deque
from dataclasses import dataclass
from datetime import datetime, timezone

import httpx
from loguru import logger
from rich.console import Console
from rich.layout import Layout
from rich.live import Live
from rich.panel import Panel
from rich.table import Table
from rich.text import Text

from collector.config import RUNTIME_MUTABLE_FIELDS, Settings, parse_runtime_value
from collector.coinbase_volatility import CoinbaseCandleVolatility, VolatilitySnapshot
from collector.csv_writer import WindowCsvWriter
from collector.logging_setup import setup_logging
from collector.market_discovery import ActiveWindow, GammaMarketDiscovery
from collector.polymarket_ws import PolymarketWsClient
from collector.rtds_chainlink import PolymarketRtdsBtcClient, PolymarketRtdsBtcUsdtClient
from collector.runtime_control import RuntimeControlServer
from collector.strike_resolver import (
    StrikeComputation,
    StrikeEvidence,
    compute_strike_from_ticks,
)
from collector.strike_overrides import StrikeOverrideStore

CHAINLINK_FRESHNESS_MS = 5000


def make_sparkline(data: list[float], width: int = 20) -> str:
    if not data:
        return "N/A"
    chars = " ▂▃▄▅▆▇█"
    min_val = min(data)
    max_val = max(data)
    if max_val == min_val:
        return chars[4] * len(data)
    res = []
    window = list(data)[-width:]
    for v in window:
        n = int((v - min_val) / (max_val - min_val) * (len(chars) - 1))
        res.append(chars[n])
    return "".join(res)


@dataclass(slots=True)
class StrikeState:
    value: float | None = None
    source: str = "none"
    status: str = "pending_manual"  # exact_auto | exact_manual | pending_manual | approx_auto | manual_first
    confidence: float | None = None
    evidence: StrikeEvidence | None = None
    note: str | None = None
    updated_ms: int | None = None

    @property
    def age_ms(self) -> int | None:
        if self.updated_ms is None:
            return None
        return max(0, int(time.time() * 1000) - int(self.updated_ms))


class CollectorRunner:
    def __init__(
        self,
        settings: Settings,
        override_slug: str | None = None,
        cli_initial_strike_usd: float | None = None,
    ) -> None:
        self._settings = settings
        self._override_slug = override_slug
        self._stop_event = asyncio.Event()
        self._gamma = GammaMarketDiscovery(settings.gamma_base_url)
        self._csv = WindowCsvWriter(settings.data_dir)
        self._rtds = PolymarketRtdsBtcClient(settings.rtds_ws_url, settings.vol_window_ticks)
        self._rtds_fast: PolymarketRtdsBtcUsdtClient | None = None
        self._current_window: ActiveWindow | None = None
        self._poly: PolymarketWsClient | None = None
        self._strike = StrikeState()
        self._overrides = StrikeOverrideStore(settings.strike_overrides_path)
        self._runtime_lock = asyncio.Lock()
        self._runtime_staged: dict[str, object] = {}
        self._last_control_message = "Menu separé: lancez 'python -m collector.cli control'."

        self._manual_strike_first: float | None = (
            cli_initial_strike_usd
            if cli_initial_strike_usd is not None
            else settings.initial_strike_usd
        )
        self._consumed_manual_strike = False

        self._btc_history = deque(maxlen=60)
        self._up_history = deque(maxlen=60)
        self._down_history = deque(maxlen=60)
        self._console = Console()
        self._http = httpx.AsyncClient(timeout=15.0)
        self._coinbase_vol = CoinbaseCandleVolatility(
            product_id=settings.coinbase_product_id,
            source_mode=settings.coinbase_source_mode,  # type: ignore[arg-type]
            poll_interval_s=settings.coinbase_poll_interval_seconds,
            window_candles=settings.coinbase_vol_window_candles,
            ws_url=settings.coinbase_ws_url,
            rt_enabled=settings.coinbase_rt_enabled,
            rt_bar_ms=settings.coinbase_rt_bar_ms,
            rt_ewma_halflife_s=settings.coinbase_rt_ewma_halflife_s,
            rt_window_fast_s=settings.coinbase_rt_window_fast_s,
            rt_window_slow_s=settings.coinbase_rt_window_slow_s,
            rt_stale_after_ms=settings.coinbase_rt_stale_after_ms,
        )
        self._control_server = RuntimeControlServer(
            settings.runtime_control_socket_path,
            self._execute_control_command,
            self._runtime_state_payload,
        )
        self._latest_vol = VolatilitySnapshot()

    def _rotate_sleep_seconds(self) -> float:
        now = time.time()
        next_boundary = ((int(now) // 300) + 1) * 300
        until = next_boundary - now
        if until < 12.0:
            return 0.8
        return float(self._settings.discovery_interval_seconds)

    def _display_btc(self) -> tuple[float | None, str, int | None]:
        chain_age = self._rtds.age_ms
        if (
            self._rtds.spot is not None
            and chain_age is not None
            and chain_age <= CHAINLINK_FRESHNESS_MS
        ):
            return self._rtds.spot, "rtds_chainlink", chain_age
        if self._rtds_fast and self._rtds_fast.spot is not None:
            return self._rtds_fast.spot, "rtds_usdt_fallback", self._rtds_fast.age_ms
        return self._rtds.spot, "rtds_chainlink_stale", chain_age

    async def run(self) -> None:
        self._install_signal_handlers()
        logger.info("Collector starting...")

        await self._rtds.start()
        await self._sync_fast_client_state()
        await self._coinbase_vol.start()
        await self._control_server.start()

        try:
            await self._set_or_rotate_window(force=True)

            setup_logging(self._settings.log_dir, self._settings.log_level, console=False)

            layout = self._make_layout()
            refresh_hz = max(4, int(round(1000 / max(40, self._settings.ui_refresh_ms))))
            with Live(layout, refresh_per_second=refresh_hz, console=self._console, transient=False) as live:
                sample_task = asyncio.create_task(self._sampling_loop(), name="sampling-loop")
                rotate_task = asyncio.create_task(self._window_rotate_loop(), name="window-rotate-loop")
                ui_task = asyncio.create_task(self._ui_loop(live), name="ui-loop")

                await self._stop_event.wait()

                sample_task.cancel()
                rotate_task.cancel()
                ui_task.cancel()
                await asyncio.gather(
                    sample_task, rotate_task, ui_task, return_exceptions=True
                )

            logger.info("Collector stopped cleanly.")
        finally:
            setup_logging(self._settings.log_dir, self._settings.log_level, console=True)
            if self._poly:
                await self._poly.stop()
            if self._rtds_fast:
                await self._rtds_fast.stop()
            await self._rtds.stop()
            await self._coinbase_vol.stop()
            await self._control_server.stop()
            await self._http.aclose()
            await self._gamma.close()
            self._csv.close()

    def _make_layout(self) -> Layout:
        layout = Layout()
        layout.split_column(
            Layout(name="header", size=3),
            Layout(name="main", ratio=1),
            Layout(name="footer", size=3),
        )
        layout["main"].split_row(Layout(name="stats", ratio=1), Layout(name="charts", ratio=2))
        return layout

    def _update_layout(self, layout: Layout) -> None:
        win = self._current_window
        slug = win.slug if win else "Searching..."

        target_str = f"${self._strike.value:,.2f}" if self._strike.value is not None else "N/A"
        strike_age = _fmt_age(self._strike.age_ms)
        conf_s = "n/a" if self._strike.confidence is None else f"{self._strike.confidence:.2f}"
        strike_meta = f"{self._strike.status}/{self._strike.source}/conf={conf_s}"
        layout["header"].update(
            Panel(
                Text.assemble(
                    ("Polymarket BTC 5m Tracker ", "bold cyan"),
                    (f"| Window: {slug} ", "green"),
                    (f"| Strike: {target_str} ", "yellow bold"),
                    (f"| {strike_meta} ", "dim"),
                    (f"| age={strike_age}", "dim"),
                ),
                border_style="bright_blue",
            )
        )

        st = Table.grid(expand=True)

        disp, src, disp_age = self._display_btc()
        live_price = "..." if disp is None else f"${disp:,.2f}"
        st.add_row(
            "Displayed BTC spot:",
            f"[bold white]{live_price}[/bold white] [dim]| source={src} age={_fmt_age(disp_age)}[/dim]",
        )

        if self._rtds_fast and self._rtds_fast.spot is not None:
            fu = f"${self._rtds_fast.spot:,.2f}"
            fts = ""
            if self._rtds_fast.spot_ts_ms:
                fts = datetime.fromtimestamp(
                    self._rtds_fast.spot_ts_ms / 1000, tz=timezone.utc
                ).strftime("%H:%M:%S")
            st.add_row(
                "BTC fast (RTDS btcusdt, display):",
                f"[bold white]{fu}[/bold white]"
                + (f" [dim]@ {fts} UTC[/dim]" if fts else "")
                + f" [dim]| age={_fmt_age(self._rtds_fast.age_ms)}[/dim]",
            )

        cl_spot = f"${self._rtds.spot:,.2f}" if self._rtds.spot else "..."
        cl_ts = ""
        if self._rtds.spot_ts_ms:
            cl_ts = datetime.fromtimestamp(
                self._rtds.spot_ts_ms / 1000, tz=timezone.utc
            ).strftime("%H:%M:%S")
        st.add_row(
            "BTC/USD (Polymarket RTDS Chainlink):",
            f"[dim]{cl_spot}[/dim]"
            + (f" [dim]@ {cl_ts} UTC[/dim]" if cl_ts else "")
            + f" [dim]| age={_fmt_age(self._rtds.age_ms)}[/dim]",
        )

        vol_age = "n/a" if self._latest_vol.age_s is None else f"{self._latest_vol.age_s}s"
        rt_age = _fmt_age(self._latest_vol.rt_age_ms)
        st.add_row(
            "Volatilite 1m (Coinbase close-close):",
            f"[magenta]{self._latest_vol.inst_vol:.6f}[/magenta]"
            f" [dim]| source={self._latest_vol.source} candles={self._latest_vol.candle_count}"
            f" age={vol_age} status={self._latest_vol.status}[/dim]",
        )
        st.add_row(
            "Volatilite RT (Coinbase matches):",
            f"[magenta]ewma={self._latest_vol.rt_vol_ewma:.6f}[/magenta]"
            f" [dim]| sigma30={self._latest_vol.rt_vol_sigma_30s:.6f}"
            f" sigma120={self._latest_vol.rt_vol_sigma_120s:.6f}"
            f" range1m={self._latest_vol.rt_candle_range_rel_1m:.5f}"
            f" ret1m={self._latest_vol.rt_candle_log_ret_open_1m:+.5f}"
            f" trades60s={self._latest_vol.rt_trade_count_60s}"
            f" age={rt_age} status={self._latest_vol.rt_status}[/dim]",
        )
        st.add_row(" ")

        up_p = "N/A"
        down_p = "N/A"
        if self._up_history:
            up_p = f"{self._up_history[-1] * 100:.2f}¢"
        if self._down_history:
            down_p = f"{self._down_history[-1] * 100:.2f}¢"

        st.add_row("UP (last trade):", f"[bold green]{up_p}[/bold green]")
        st.add_row("DOWN (last trade):", f"[bold red]{down_p}[/bold red]")

        if win and win.event_start_utc:
            est = win.event_start_utc.strftime("%d/%m/%Y %H:%M:%S UTC")
            st.add_row("Event start (Gamma):", f"[dim]{est}[/dim]")
        if self._strike.status == "pending_manual":
            st.add_row(
                "Action requise:",
                '[bold yellow]Strike à ajouter a posteriori[/bold yellow]',
            )
            if self._strike.note:
                st.add_row("Raison:", f"[dim]{self._strike.note}[/dim]")

        layout["stats"].update(Panel(st, title="[bold]Market status[/bold]", border_style="cyan"))

        ct = Table.grid(expand=True)
        ct.add_row("BTC (display):", make_sparkline(list(self._btc_history), 50))
        ct.add_row("UP:", f"[green]{make_sparkline(list(self._up_history), 50)}[/green]")
        ct.add_row("DOWN:", f"[red]{make_sparkline(list(self._down_history), 50)}[/red]")

        diff = 0.0
        if disp is not None and self._strike.value is not None:
            diff = disp - self._strike.value
        diff_str = f"{'+' if diff >= 0 else ''}{diff:.2f}"
        color = "green" if diff >= 0 else "red"

        ct.add_row(" ")
        ct.add_row("Vs strike (display BTC):", f"[{color}]{diff_str} USD[/{color}]")

        layout["charts"].update(
            Panel(ct, title="[bold]Live view[/bold]", border_style="green")
        )

        now = datetime.now(timezone.utc).strftime("%d/%m/%Y %H:%M:%S UTC")
        layout["footer"].update(
            Panel(
                Text.assemble(
                    (f"Time: {now} ", "dim"),
                    (f"| Samples: {len(self._btc_history)} ", "dim"),
                    ("| Control: python -m collector.cli control", "bold cyan"),
                    (f" | {self._last_control_message} ", "dim"),
                    ("| CTRL+C to stop", "bold red"),
                ),
                border_style="dim",
            )
        )

    async def _window_rotate_loop(self) -> None:
        while not self._stop_event.is_set():
            try:
                await self._set_or_rotate_window(force=False)
            except Exception:
                logger.exception("Window rotation failed")
            await asyncio.sleep(self._rotate_sleep_seconds())

    async def _ui_loop(self, live: Live) -> None:
        while not self._stop_event.is_set():
            self._latest_vol = await self._coinbase_vol.snapshot()
            self._update_layout(live.get_renderable())
            interval = max(0.05, self._settings.ui_refresh_ms / 1000.0)
            await asyncio.sleep(interval)

    async def _set_or_rotate_window(self, force: bool) -> None:
        now = int(time.time())
        target_epoch = (now // 300) * 300

        if self._override_slug:
            new_window = await self._gamma.fetch_by_slug(self._override_slug)
            self._override_slug = None
        else:
            if (
                not force
                and self._current_window is not None
                and self._current_window.window_epoch == target_epoch
            ):
                return
            new_window = await self._gamma.fetch_open_window_for_epoch(target_epoch)

        changed = (
            force
            or self._current_window is None
            or new_window.window_epoch != self._current_window.window_epoch
        )
        if not changed:
            return

        if self._poly:
            await self._poly.stop()

        self._poly = PolymarketWsClient(
            self._settings.polymarket_ws_url,
            new_window.up_token_id,
            new_window.down_token_id,
        )
        await self._poly.start()
        self._csv.rotate_if_needed(new_window.window_epoch)
        self._current_window = new_window

        if self._manual_strike_first is not None and not self._consumed_manual_strike:
            self._set_strike(
                self._manual_strike_first,
                source="manual_first_window",
                status="manual_first",
                confidence=1.0,
                evidence=StrikeEvidence(method="manual", reason="first_window_override"),
            )
            self._consumed_manual_strike = True
            logger.info(
                "Strike from user override (first window only): {}",
                self._strike.value,
            )
        else:
            comp = await self._resolve_strike_without_candlestick(new_window)
            self._apply_computation(comp)
            self._apply_manual_override_for_current_window()

        self._up_history.clear()
        self._down_history.clear()
        logger.info(
            "Window {} | strike={} ({}/{}) conf={} | event_start={}",
            new_window.slug,
            self._strike.value,
            self._strike.status,
            self._strike.source,
            self._strike.confidence,
            new_window.event_start_utc.isoformat(),
        )

    async def _clob_last_trade_price(self, token_id: str) -> float | None:
        try:
            r = await self._http.get(
                f"{self._settings.clob_base_url.rstrip('/')}/last-trade-price",
                params={"token_id": token_id},
            )
            r.raise_for_status()
            data = r.json()
            p = data.get("price")
            return float(p) if p is not None else None
        except Exception as exc:
            logger.debug("CLOB last-trade-price failed for {}: {}", token_id[:16], exc)
            return None

    async def _resolve_strike_without_candlestick(self, window: ActiveWindow) -> StrikeComputation:
        es_ms = int(window.event_start_utc.timestamp() * 1000)
        ticks = await self._rtds.snapshot_tick_history()
        comp = compute_strike_from_ticks(
            ticks,
            es_ms,
            confidence_gap_ms=self._settings.strike_confidence_gap_ms,
            exact_match_tolerance_ms=self._settings.strike_exact_tolerance_ms,
        )
        return comp

    def _set_strike(
        self,
        value: float | None,
        *,
        source: str,
        status: str,
        confidence: float | None = None,
        evidence: StrikeEvidence | None = None,
        note: str | None = None,
    ) -> None:
        self._strike = StrikeState(
            value=value,
            source=source,
            status=status,
            confidence=confidence,
            evidence=evidence,
            note=note,
            updated_ms=int(time.time() * 1000),
        )

    def _apply_computation(self, comp: StrikeComputation) -> None:
        if comp.status == "exact_auto":
            self._set_strike(
                comp.value,
                source=comp.source,
                status="exact_auto",
                confidence=comp.confidence,
                evidence=comp.evidence,
            )
            return

        if (
            comp.status == "approx_auto"
            and self._settings.strike_allow_approx_auto
            and comp.confidence >= self._settings.strike_confidence_threshold
        ):
            self._set_strike(
                comp.value,
                source=comp.source,
                status="approx_auto",
                confidence=comp.confidence,
                evidence=comp.evidence,
            )
            logger.warning(
                "Using RTDS approximate strike (auto-approved) conf={:.2f} source={}",
                comp.confidence,
                comp.source,
            )
            return

        note = "Strike à ajouter a posteriori"
        if comp.evidence and comp.evidence.reason:
            note = f"{note} ({comp.evidence.reason})"
        self._set_strike(
            comp.value,
            source=comp.source,
            status="pending_manual",
            confidence=comp.confidence,
            evidence=comp.evidence,
            note=note,
        )
        logger.warning(
            "Pending manual strike for current window: source={} conf={:.2f} note={}",
            comp.source,
            comp.confidence,
            note,
        )

    def _apply_manual_override_for_current_window(self) -> None:
        if not self._current_window:
            return
        ov = self._overrides.get(self._current_window.window_epoch)
        if ov is None:
            return
        if self._strike.status == "exact_manual" and self._strike.value == ov.value:
            return
        self._set_strike(
            ov.value,
            source=ov.source,
            status="exact_manual",
            confidence=1.0,
            evidence=StrikeEvidence(method="manual_override", reason="a_posteriori"),
            note="Strike manuel applique",
        )
        logger.info(
            "Applied manual strike override for window {}: {}",
            self._current_window.window_epoch,
            ov.value,
        )

    async def _sampling_loop(self) -> None:
        next_tick = time.monotonic()
        while not self._stop_event.is_set():
            interval = max(0.05, self._settings.sample_interval_ms / 1000.0)
            if not self._poly or not self._current_window:
                await asyncio.sleep(0.1)
                continue
            self._apply_manual_override_for_current_window()

            snapshot = await self._poly.snapshot()
            up = snapshot.get(self._current_window.up_token_id)
            down = snapshot.get(self._current_window.down_token_id)

            up_p = up.price if up and up.price is not None else None
            down_p = down.price if down and down.price is not None else None
            if up_p is None:
                up_p = await self._clob_last_trade_price(self._current_window.up_token_id)
            if down_p is None:
                down_p = await self._clob_last_trade_price(self._current_window.down_token_id)

            disp, disp_src, disp_age = self._display_btc()
            self._latest_vol = await self._coinbase_vol.snapshot()
            if disp is not None:
                self._btc_history.append(disp)

            if up_p is not None:
                self._up_history.append(up_p)
            if down_p is not None:
                self._down_history.append(down_p)

            now_ms = int(time.time() * 1000)
            row = [
                now_ms,
                self._csv.format_ts(now_ms),
                self._current_window.window_epoch,
                self._current_window.slug,
                self._strike.value,
                self._strike.status,
                self._strike.source,
                self._strike.confidence,
                self._strike.age_ms,
                self._strike.evidence.before_ts_ms if self._strike.evidence else None,
                self._strike.evidence.before_price if self._strike.evidence else None,
                self._strike.evidence.after_ts_ms if self._strike.evidence else None,
                self._strike.evidence.after_price if self._strike.evidence else None,
                self._strike.evidence.gap_ms if self._strike.evidence else None,
                self._strike.note,
                up_p,
                down_p,
                disp,
                disp_src,
                disp_age,
                self._rtds.spot,
                self._rtds_fast.spot if self._rtds_fast else None,
                self._latest_vol.inst_vol,
                self._latest_vol.source,
                self._latest_vol.candle_count,
                self._latest_vol.age_s,
                self._latest_vol.status,
                self._latest_vol.rt_vol_ewma,
                self._latest_vol.rt_vol_sigma_30s,
                self._latest_vol.rt_vol_sigma_120s,
                self._latest_vol.rt_candle_range_rel_1m,
                self._latest_vol.rt_candle_log_ret_open_1m,
                self._latest_vol.rt_trade_count_60s,
                self._latest_vol.rt_age_ms,
                self._latest_vol.rt_status,
            ]
            self._csv.write_row(row)

            next_tick += interval
            now = time.monotonic()
            if next_tick < now - interval:
                next_tick = now + interval
            sleep_for = max(0.0, next_tick - now)
            await asyncio.sleep(sleep_for)

    async def _sync_fast_client_state(self) -> None:
        if self._settings.rtds_fast_btcusdt and self._rtds_fast is None:
            self._rtds_fast = PolymarketRtdsBtcUsdtClient(self._settings.rtds_ws_url)
            await self._rtds_fast.start()
            logger.info("Fast RTDS btcusdt feed enabled")
        elif not self._settings.rtds_fast_btcusdt and self._rtds_fast is not None:
            await self._rtds_fast.stop()
            self._rtds_fast = None
            logger.info("Fast RTDS btcusdt feed disabled")

    async def _execute_control_command(self, raw: str) -> dict[str, object]:
        cmd = raw.strip()
        if cmd.startswith(":"):
            cmd = cmd[1:]
        try:
            await self._handle_control_command(cmd)
            return {"message": self._last_control_message, "state": self._runtime_state_payload()}
        except Exception as exc:
            logger.exception("Runtime control failed")
            self._last_control_message = f"erreur controle: {exc}"
            return {"message": self._last_control_message, "state": self._runtime_state_payload()}

    async def _handle_control_command(self, raw: str) -> None:
        parts = shlex.split(raw)
        if not parts:
            self._last_control_message = "Commande vide."
            return
        cmd = parts[0].lower()
        args = parts[1:]

        if cmd in ("help", "h", "?"):
            self._last_control_message = (
                "Commandes: params, stage <param> <valeur>, staged, apply [params...], "
                "discard, unstage <param>, set-strike <valeur>, stop"
            )
            return
        if cmd in ("params", "show"):
            self._last_control_message = self._runtime_values_summary()
            return
        if cmd in ("staged",):
            self._last_control_message = self._staged_summary()
            return
        if cmd in ("stage", "set"):
            if len(args) < 2:
                self._last_control_message = "usage: stage <param> <valeur>"
                return
            name = args[0].strip().lower()
            if name not in RUNTIME_MUTABLE_FIELDS:
                self._last_control_message = f"parametre inconnu '{name}'"
                return
            value = parse_runtime_value(name, " ".join(args[1:]))
            self._runtime_staged[name] = value
            self._last_control_message = f"staged: {name}={value!r}"
            return
        if cmd == "unstage":
            if len(args) != 1:
                self._last_control_message = "usage: unstage <param>"
                return
            name = args[0].strip().lower()
            if self._runtime_staged.pop(name, None) is None:
                self._last_control_message = f"pas en attente: {name}"
            else:
                self._last_control_message = f"unstaged: {name}"
            return
        if cmd in ("discard", "reset"):
            self._runtime_staged.clear()
            self._last_control_message = "Modifications en attente effacees."
            return
        if cmd == "apply":
            await self._apply_staged_updates(args)
            return
        if cmd in ("set-strike", "strike"):
            if len(args) != 1:
                self._last_control_message = "usage: set-strike <valeur>"
                return
            await self._set_current_window_strike(float(args[0]))
            return
        if cmd in ("stop", "quit", "exit"):
            self._last_control_message = "Arret du collecteur..."
            self._stop_event.set()
            return

        self._last_control_message = f"commande inconnue '{cmd}'"

    async def _set_current_window_strike(self, value: float) -> None:
        if not self._current_window:
            self._last_control_message = "Aucune fenetre active."
            return
        ov = self._overrides.upsert(
            self._current_window.window_epoch, value, source="manual_runtime_control_cli"
        )
        self._apply_manual_override_for_current_window()
        self._last_control_message = (
            f"strike applique pour window={ov.window_epoch} value=${ov.value:,.2f}"
        )

    async def _apply_staged_updates(self, names: list[str]) -> None:
        if not self._runtime_staged:
            self._last_control_message = "Aucune modification en attente."
            return
        selected: dict[str, object] = {}
        if names:
            for raw_name in names:
                name = raw_name.strip().lower()
                if name not in self._runtime_staged:
                    self._last_control_message = f"'{name}' n'est pas en attente."
                    return
                selected[name] = self._runtime_staged[name]
        else:
            selected = dict(self._runtime_staged)

        previous: dict[str, object] = {}
        async with self._runtime_lock:
            changed_fast_mode = False
            changed_coinbase = False
            try:
                for name, value in selected.items():
                    current = getattr(self._settings, name)
                    previous[name] = current
                    if current != value:
                        setattr(self._settings, name, value)
                        if name == "rtds_fast_btcusdt":
                            changed_fast_mode = True
                        if name in (
                            "coinbase_source_mode",
                            "coinbase_poll_interval_seconds",
                            "coinbase_vol_window_candles",
                        ):
                            changed_coinbase = True
                if changed_fast_mode:
                    await self._sync_fast_client_state()
                if changed_coinbase:
                    await self._coinbase_vol.update_config(
                        source_mode=self._settings.coinbase_source_mode,
                        poll_interval_s=self._settings.coinbase_poll_interval_seconds,
                        window_candles=self._settings.coinbase_vol_window_candles,
                    )
            except Exception:
                for name, old_value in previous.items():
                    setattr(self._settings, name, old_value)
                raise

        for name in selected:
            self._runtime_staged.pop(name, None)
        applied = ", ".join(f"{k}={v!r}" for k, v in selected.items())
        self._last_control_message = f"applied: {applied}"
        logger.info("Runtime settings applied: {}", applied)

    def _runtime_values_summary(self) -> str:
        pairs = []
        for name in RUNTIME_MUTABLE_FIELDS:
            pairs.append(f"{name}={getattr(self._settings, name)!r}")
        return "runtime: " + ", ".join(pairs)

    def _staged_summary(self) -> str:
        if not self._runtime_staged:
            return "staged: (empty)"
        pairs = [f"{k}={v!r}" for k, v in sorted(self._runtime_staged.items())]
        return "staged: " + ", ".join(pairs)

    def _runtime_state_payload(self) -> dict[str, object]:
        window_epoch = self._current_window.window_epoch if self._current_window else None
        window_slug = self._current_window.slug if self._current_window else None
        return {
            "window_epoch": window_epoch,
            "window_slug": window_slug,
            "strike_status": self._strike.status,
            "strike_value": self._strike.value,
            "runtime": {name: getattr(self._settings, name) for name in RUNTIME_MUTABLE_FIELDS},
            "staged": dict(self._runtime_staged),
            "last_control_message": self._last_control_message,
        }

    def _install_signal_handlers(self) -> None:
        loop = asyncio.get_running_loop()
        for sig in (signal.SIGINT, signal.SIGTERM):
            try:
                loop.add_signal_handler(sig, self._stop_event.set)
            except NotImplementedError:
                pass


def _fmt_age(age_ms: int | None) -> str:
    if age_ms is None:
        return "n/a"
    if age_ms < 1000:
        return f"{age_ms}ms"
    return f"{age_ms / 1000:.2f}s"
