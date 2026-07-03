from __future__ import annotations

import asyncio
import json
import re
import socket
import sys
from typing import Optional

import typer
from rich.console import Console
from rich.panel import Panel
from rich.prompt import Prompt
from rich.table import Table

from collector.config import load_settings
from collector.logging_setup import setup_logging
from collector.market_discovery import normalize_slug
from collector.runner import CollectorRunner
from collector.strike_overrides import StrikeOverrideStore

app = typer.Typer(help="Polymarket BTC Up/Down 5m collector")
console = Console()
SLUG_RE = re.compile(r"^btc-updown-5m-(\d+)$")


def run_logic(
    slug: Optional[str] = None,
    initial_strike: Optional[float] = None,
) -> None:
    settings = load_settings()
    settings.data_dir.mkdir(parents=True, exist_ok=True)
    settings.log_dir.mkdir(parents=True, exist_ok=True)
    setup_logging(settings.log_dir, settings.log_level)

    if slug:
        console.print(f"[bold cyan]Starting collector (initial slug/URL): {slug}[/bold cyan]")
    else:
        console.print("[bold green]Starting collector (auto-discovery by 5m UTC slot)[/bold green]")
    if initial_strike is not None:
        console.print(
            f"[bold yellow]Initial strike override (first window only): ${initial_strike:,.2f}[/bold yellow]"
        )
    console.print(
        "[dim]Controle runtime: utilisez un second terminal avec 'python -m collector.cli control'.[/dim]"
    )

    runner = CollectorRunner(
        settings,
        override_slug=slug,
        cli_initial_strike_usd=initial_strike,
    )
    try:
        asyncio.run(runner.run())
    except KeyboardInterrupt:
        console.print("\n[yellow]Shutdown by user.[/yellow]")


@app.command(name="run")
def run_cmd(
    slug: Optional[str] = typer.Argument(
        None,
        help='Slug or full URL (quote URLs: "https://polymarket.com/fr/event/btc-updown-5m-...")',
    ),
    slug_option: Optional[str] = typer.Option(
        None,
        "--slug",
        "-s",
        help="Same as positional slug; use if the shell mangles the URL.",
    ),
    strike: Optional[float] = typer.Option(
        None,
        "--strike",
        help="Price to beat for the first window only (match Polymarket UI). Env: INITIAL_STRIKE_USD.",
    ),
) -> None:
    """Start the data collector."""
    run_logic(slug_option or slug, initial_strike=strike)


@app.command(name="strike-set")
def strike_set_cmd(
    target: str = typer.Argument(
        ...,
        help='Window epoch (ex: 1774731900), slug, or URL "https://polymarket.com/.../btc-updown-5m-..."',
    ),
    value: float = typer.Argument(..., help="Exact strike value to apply a posteriori."),
) -> None:
    """Set or update a manual strike override for a specific window."""
    settings = load_settings()
    store = StrikeOverrideStore(settings.strike_overrides_path)
    window_epoch = _parse_window_epoch(target)
    ov = store.upsert(window_epoch, value, source="manual_cli_a_posteriori")
    console.print(
        "[bold green]Strike override saved[/bold green] "
        f"window={ov.window_epoch} value=${ov.value:,.2f} file={settings.strike_overrides_path}"
    )


@app.command(name="control")
def control_cmd(
    command: Optional[str] = typer.Option(
        None,
        "--command",
        "-c",
        help="Commande unique a envoyer (ex: \"params\", \"stage sample_interval_ms 300\").",
    )
) -> None:
    """Ouvre le menu de controle runtime dans un terminal separe."""
    settings = load_settings()
    if command:
        out = _send_control_request(settings.runtime_control_socket_path, {"action": "command", "command": command})
        if not out.get("ok"):
            raise typer.Exit(code=1)
        console.print(out.get("message", "ok"))
        return

    console.print(
        Panel.fit(
            "[bold cyan]Menu de controle runtime[/bold cyan]\n"
            "Ce menu n'impacte pas la cadence du collecteur. Lancez-le dans un second terminal."
        )
    )
    while True:
        state = _send_control_request(settings.runtime_control_socket_path, {"action": "state"})
        if not state.get("ok"):
            console.print(f"[red]{state.get('error', 'Erreur control') }[/red]")
            raise typer.Exit(code=1)
        _render_control_state(state.get("state", {}))
        choice = Prompt.ask(
            "Choix",
            choices=["1", "2", "3", "4", "5", "6", "7", "8", "9"],
            default="1",
        )
        if choice == "1":
            out = _send_control_request(
                settings.runtime_control_socket_path,
                {"action": "command", "command": "params"},
            )
            console.print(f"[green]{out.get('message', '')}[/green]")
        elif choice == "2":
            name = Prompt.ask("Parametre", default="sample_interval_ms").strip().lower()
            value = Prompt.ask("Nouvelle valeur")
            out = _send_control_request(
                settings.runtime_control_socket_path,
                {"action": "command", "command": f"stage {name} {value}"},
            )
            console.print(f"[green]{out.get('message', '')}[/green]")
        elif choice == "3":
            out = _send_control_request(
                settings.runtime_control_socket_path,
                {"action": "command", "command": "staged"},
            )
            console.print(f"[green]{out.get('message', '')}[/green]")
        elif choice == "4":
            raw = Prompt.ask(
                "Parametres a appliquer (separes par espace, vide=tout)",
                default="",
                show_default=False,
            ).strip()
            cmd = "apply" if not raw else f"apply {raw}"
            out = _send_control_request(
                settings.runtime_control_socket_path,
                {"action": "command", "command": cmd},
            )
            console.print(f"[green]{out.get('message', '')}[/green]")
        elif choice == "5":
            name = Prompt.ask("Parametre a retirer du staging").strip().lower()
            out = _send_control_request(
                settings.runtime_control_socket_path,
                {"action": "command", "command": f"unstage {name}"},
            )
            console.print(f"[green]{out.get('message', '')}[/green]")
        elif choice == "6":
            out = _send_control_request(
                settings.runtime_control_socket_path,
                {"action": "command", "command": "discard"},
            )
            console.print(f"[green]{out.get('message', '')}[/green]")
        elif choice == "7":
            strike = Prompt.ask("Strike exact a appliquer (fenetre courante)")
            out = _send_control_request(
                settings.runtime_control_socket_path,
                {"action": "command", "command": f"set-strike {strike}"},
            )
            console.print(f"[green]{out.get('message', '')}[/green]")
        elif choice == "8":
            _print_verification_links(state.get("state", {}))
        elif choice == "9":
            console.print("[yellow]Fermeture du menu control.[/yellow]")
            return


@app.callback(invoke_without_command=True)
def main(ctx: typer.Context) -> None:
    """Default entry point."""
    if ctx.invoked_subcommand is None:
        slug: Optional[str] = None
        if len(sys.argv) > 1 and not sys.argv[1].startswith("-"):
            slug = sys.argv[1]
        run_logic(slug)


def _parse_window_epoch(target: str) -> int:
    s = target.strip()
    if s.isdigit():
        return int(s)
    slug = normalize_slug(s)
    m = SLUG_RE.match(slug)
    if not m:
        raise typer.BadParameter(f"Cannot parse window epoch from target: {target}")
    return int(m.group(1))


def _send_control_request(socket_path, payload: dict[str, object]) -> dict[str, object]:
    req = (json.dumps(payload, ensure_ascii=True) + "\n").encode("utf-8")
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.settimeout(2.5)
            sock.connect(str(socket_path))
            sock.sendall(req)
            buf = b""
            while not buf.endswith(b"\n"):
                chunk = sock.recv(4096)
                if not chunk:
                    break
                buf += chunk
    except FileNotFoundError:
        console.print(
            f"[red]Canal control introuvable: {socket_path}. Lancez d'abord le bot.[/red]"
        )
        return {"ok": False, "error": "control_socket_missing"}
    except OSError as exc:
        console.print(f"[red]Impossible de joindre le bot: {exc}[/red]")
        return {"ok": False, "error": f"control_socket_error:{exc}"}

    try:
        return json.loads(buf.decode("utf-8")) if buf else {"ok": False, "error": "empty_reply"}
    except json.JSONDecodeError:
        return {"ok": False, "error": "invalid_reply"}


def _render_control_state(state: dict[str, object]) -> None:
    runtime = state.get("runtime", {})
    staged = state.get("staged", {})
    table = Table(title="Etat runtime")
    table.add_column("Cle")
    table.add_column("Valeur")
    table.add_row("Fenetre", str(state.get("window_slug") or "n/a"))
    table.add_row("Strike", str(state.get("strike_value")))
    table.add_row("Statut strike", str(state.get("strike_status")))
    if isinstance(runtime, dict):
        for k in sorted(runtime.keys()):
            table.add_row(str(k), repr(runtime[k]))
    if isinstance(staged, dict):
        for k in sorted(staged.keys()):
            table.add_row(f"(staged) {k}", repr(staged[k]))
    console.print(table)
    console.print(
        "[dim]1=params 2=stage 3=staged 4=apply 5=unstage 6=discard 7=set-strike 8=verifier en ligne 9=quitter[/dim]"
    )


def _print_verification_links(state: dict[str, object]) -> None:
    slug = state.get("window_slug")
    if slug:
        console.print(f"[cyan]Polymarket:[/cyan] https://polymarket.com/event/{slug}")
        console.print(
            f"[cyan]Gamma:[/cyan] https://gamma-api.polymarket.com/events?slug={slug}"
        )
    console.print(
        "[cyan]Coinbase candles Exchange:[/cyan] "
        "https://api.exchange.coinbase.com/products/BTC-USD/candles?granularity=60"
    )
    console.print(
        "[cyan]Coinbase candles Advanced Trade:[/cyan] "
        "https://api.coinbase.com/api/v3/brokerage/market/products/BTC-USD/candles?granularity=ONE_MINUTE"
    )


if __name__ == "__main__":
    app()
