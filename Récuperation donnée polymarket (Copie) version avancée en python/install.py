from __future__ import annotations

import os
import shutil
import subprocess
import sys
import venv
from pathlib import Path

# --- BOOTSTRAPPER ---
# This part must only use standard library modules.

def _is_venv() -> bool:
    return sys.prefix != sys.base_prefix

def _run(cmd: list[str], cwd: Path | None = None) -> None:
    subprocess.run(cmd, check=True, cwd=str(cwd) if cwd else None)

def bootstrap():
    project_dir = Path.cwd()
    venv_dir = project_dir / ".venv"
    
    print("--- Polymarket BTC 5m Tracker Installation ---")
    
    # 1. Create VENV if it doesn't exist
    if not venv_dir.exists():
        print(f"Creating virtual environment in {venv_dir}...")
        venv.create(venv_dir, with_pip=True)
    
    python_bin = venv_dir / "bin" / "python"
    if sys.platform == "win32":
        python_bin = venv_dir / "Scripts" / "python.exe"
        
    # 2. Install/Update dependencies
    print("Installing dependencies (typer, rich, httpx, etc.)...")
    _run([str(python_bin), "-m", "pip", "install", "--upgrade", "pip"])
    _run([str(python_bin), "-m", "pip", "install", "-e", "."])
    
    # 3. Re-run this script inside the venv to use the advanced UI
    print("Dependencies installed. Launching interactive wizard...")
    os.execv(str(python_bin), [str(python_bin), __file__])

# --- WIZARD ---
# This part is only executed when running inside the venv.

if _is_venv():
    try:
        import httpx
        import typer
        import websockets.sync.client
        from rich.console import Console
        from rich.panel import Panel
        from rich.progress import Progress, SpinnerColumn, TextColumn
        from rich.prompt import Confirm, IntPrompt, Prompt
    except ImportError:
        # If imports fail even in venv, something went wrong with installation
        bootstrap()
        sys.exit(0)

    console = Console()
    app = typer.Typer()

    def _check_connectivity(gamma_url: str, poly_ws: str, rtds_ws: str) -> None:
        import json as _json

        with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), transient=True) as progress:
            progress.add_task(description="Testing Gamma HTTP (slug lookup)...", total=None)
            import time as _time

            ep = (int(_time.time()) // 300) * 300
            resp = httpx.get(
                f"{gamma_url.rstrip('/')}/events",
                params={"slug": f"btc-updown-5m-{ep}"},
                timeout=15,
            )
            resp.raise_for_status()
            progress.add_task(description="Testing Polymarket CLOB WS...", total=None)
            with websockets.sync.client.connect(poly_ws, open_timeout=10, close_timeout=3) as ws:
                ws.send('{"type":"market","assets_ids":[]}')
            progress.add_task(description="Testing Polymarket RTDS...", total=None)
            sub = {
                "action": "subscribe",
                "subscriptions": [
                    {
                        "topic": "crypto_prices_chainlink",
                        "type": "*",
                        "filters": _json.dumps({"symbol": "btc/usd"}),
                    }
                ],
            }
            with websockets.sync.client.connect(rtds_ws, open_timeout=10, close_timeout=3) as ws:
                ws.send(_json.dumps(sub))
                ws.recv(timeout=15)
            progress.add_task(description="Testing Coinbase candles endpoints...", total=None)
            now = int(_time.time())
            start = now - 3600
            ex = httpx.get(
                "https://api.exchange.coinbase.com/products/BTC-USD/candles",
                params={"start": start, "end": now, "granularity": 60},
                timeout=15,
            )
            ex.raise_for_status()
            adv = httpx.get(
                "https://api.coinbase.com/api/v3/brokerage/market/products/BTC-USD/candles",
                params={
                    "start": str(start),
                    "end": str(now),
                    "granularity": "ONE_MINUTE",
                    "limit": "120",
                },
                timeout=15,
            )
            adv.raise_for_status()

    @app.command()
    def wizard() -> None:
        project_dir = Path.cwd()
        console.print(Panel.fit("[bold cyan]Polymarket BTC 5m Interactive Wizard[/bold cyan]"))
        
        # Configuration prompts
        sample_ms = IntPrompt.ask("Sampling interval (ms)", default=500)
        vol_ticks = IntPrompt.ask("Legacy RTDS volatility window (ticks)", default=120)
        discovery = IntPrompt.ask("Discovery refresh interval (seconds)", default=5)
        coinbase_window = IntPrompt.ask("Coinbase volatility window (1m candles)", default=60)
        coinbase_poll = IntPrompt.ask("Coinbase polling interval (seconds)", default=10)
        coinbase_mode = Prompt.ask(
            "Coinbase source mode",
            choices=["auto", "exchange", "advanced"],
            default="auto",
        )

        # Generate .env
        env_path = project_dir / ".env"
        if env_path.exists() and not Confirm.ask(".env exists. Overwrite it?", default=False):
            console.print("[yellow]Keeping existing .env[/yellow]")
        else:
            env_text = (
                "GAMMA_BASE_URL=https://gamma-api.polymarket.com\n"
                "CLOB_BASE_URL=https://clob.polymarket.com\n"
                "POLYMARKET_WS_URL=wss://ws-subscriptions-clob.polymarket.com/ws/market\n"
                "RTDS_WS_URL=wss://ws-live-data.polymarket.com\n"
                f"SAMPLE_INTERVAL_MS={sample_ms}\n"
                f"VOL_WINDOW_TICKS={vol_ticks}\n"
                f"DISCOVERY_INTERVAL_SECONDS={discovery}\n"
                "LOG_LEVEL=INFO\n"
                "DATA_DIR=data_5m\n"
                "LOG_DIR=logs\n"
                "RUNTIME_CONTROL_SOCKET_PATH=/tmp/polymarket_btc5m_control.sock\n"
                f"COINBASE_VOL_WINDOW_CANDLES={coinbase_window}\n"
                f"COINBASE_POLL_INTERVAL_SECONDS={coinbase_poll}\n"
                f"COINBASE_SOURCE_MODE={coinbase_mode}\n"
                "COINBASE_PRODUCT_ID=BTC-USD\n"
                "COINBASE_WS_URL=wss://ws-feed.exchange.coinbase.com\n"
                "COINBASE_RT_ENABLED=true\n"
                "COINBASE_RT_BAR_MS=200\n"
                "COINBASE_RT_EWMA_HALFLIFE_S=12\n"
                "COINBASE_RT_WINDOW_FAST_S=30\n"
                "COINBASE_RT_WINDOW_SLOW_S=120\n"
                "COINBASE_RT_STALE_AFTER_MS=6000\n"
                "# Optional: first-window strike from Polymarket UI (overridden by --strike)\n"
                "# INITIAL_STRIKE_USD=\n"
                "# Runtime fallback: keep fast RTDS btcusdt enabled when chainlink feed is stale.\n"
                "RTDS_FAST_BTCUSDT=true\n"
                "# Optional Coinbase credentials (not required for public candles)\n"
                "# COINBASE_API_KEY=\n"
                "# COINBASE_API_SECRET=\n"
                "# COINBASE_API_PASSPHRASE=\n"
            )
            env_path.write_text(env_text, encoding="utf-8")
            console.print(f"[green]Wrote {env_path}[/green]")

        # Tests
        if Confirm.ask("Run connectivity tests now?", default=True):
            try:
                _check_connectivity(
                    "https://gamma-api.polymarket.com",
                    "wss://ws-subscriptions-clob.polymarket.com/ws/market",
                    "wss://ws-live-data.polymarket.com",
                )
                console.print("[green]Connectivity tests passed.[/green]")
            except Exception as e:
                console.print(f"[red]Connectivity test failed: {e}[/red]")

        # Final message
        python_bin = sys.executable
        run_cmd = f"{python_bin} -m collector.cli run"
        console.print(Panel.fit(f"[bold green]Setup complete[/bold green]\nRun collector:\n[bold]{run_cmd}[/bold]"))

        if Confirm.ask("Start collector now?", default=False):
            os.execv(str(python_bin), [str(python_bin), "-m", "collector.cli", "run"])

    @app.callback(invoke_without_command=True)
    def main(ctx: typer.Context) -> None:
        if ctx.invoked_subcommand is None:
            wizard()

    if __name__ == "__main__":
        app()

else:
    if __name__ == "__main__":
        bootstrap()
