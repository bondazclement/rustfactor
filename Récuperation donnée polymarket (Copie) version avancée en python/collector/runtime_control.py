from __future__ import annotations

import asyncio
import json
from pathlib import Path
from typing import Any, Awaitable, Callable


CommandHandler = Callable[[str], Awaitable[dict[str, Any]]]
StateProvider = Callable[[], dict[str, Any]]


class RuntimeControlServer:
    def __init__(
        self,
        socket_path: Path,
        handle_command: CommandHandler,
        get_state: StateProvider,
    ) -> None:
        self._socket_path = socket_path
        self._handle_command = handle_command
        self._get_state = get_state
        self._server: asyncio.AbstractServer | None = None

    async def start(self) -> None:
        self._socket_path.parent.mkdir(parents=True, exist_ok=True)
        if self._socket_path.exists():
            self._socket_path.unlink()
        self._server = await asyncio.start_unix_server(
            self._handle_client,
            path=str(self._socket_path),
        )

    async def stop(self) -> None:
        if self._server is not None:
            self._server.close()
            await self._server.wait_closed()
            self._server = None
        if self._socket_path.exists():
            self._socket_path.unlink()

    async def _handle_client(
        self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
        try:
            line = await reader.readline()
            if not line:
                self._write(writer, {"ok": False, "error": "empty_request"})
                return
            try:
                request = json.loads(line.decode("utf-8"))
            except json.JSONDecodeError:
                self._write(writer, {"ok": False, "error": "invalid_json"})
                return

            action = str(request.get("action", "")).strip().lower()
            if action == "ping":
                self._write(writer, {"ok": True, "message": "pong"})
                return
            if action == "state":
                self._write(writer, {"ok": True, "state": self._get_state()})
                return
            if action == "command":
                raw = str(request.get("command", "")).strip()
                result = await self._handle_command(raw)
                payload = {"ok": True}
                payload.update(result)
                self._write(writer, payload)
                return

            self._write(writer, {"ok": False, "error": f"unknown_action:{action}"})
        except Exception as exc:
            self._write(writer, {"ok": False, "error": f"server_error:{exc}"})
        finally:
            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass

    def _write(self, writer: asyncio.StreamWriter, data: dict[str, Any]) -> None:
        raw = (json.dumps(data, ensure_ascii=True) + "\n").encode("utf-8")
        writer.write(raw)
