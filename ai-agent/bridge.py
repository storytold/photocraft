# SPDX-License-Identifier: MIT OR Apache-2.0
"""Bridge-mode MCP client for driving the real PhotoCraft desktop window.

The desktop app owns the window and the control server:

    photocraft --control 7878 --control-token-file runtime/control.token \
      --in-window-menus ...

This module starts ``photocraft-cli mcp --bridge 127.0.0.1:7878`` and speaks
MCP over stdio.  Every call is a real bridge call, so the visible app moves.
"""

from __future__ import annotations

import asyncio
import base64
import json
import os
import secrets
import shutil
import sys
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

BASE_DIR = Path(__file__).resolve().parent
REPO_ROOT = BASE_DIR.parent
IS_WINDOWS = os.name == "nt"
EXE = ".exe" if IS_WINDOWS else ""


class BridgeError(RuntimeError):
    pass


def _flatten(exc: BaseException):
    if isinstance(exc, BaseExceptionGroup):
        for inner in exc.exceptions:
            yield from _flatten(inner)
    else:
        yield exc


def _error_text(exc: BaseException) -> str:
    return "; ".join(f"{type(e).__name__}: {e}" for e in _flatten(exc))


def _find_cli(explicit: str | None) -> Path:
    candidates: list[Path] = []
    if explicit:
        p = Path(explicit).expanduser()
        candidates.append(p if p.is_absolute() else BASE_DIR / p)
    candidates += [
        REPO_ROOT / "target" / "release" / f"photocraft-cli{EXE}",
        REPO_ROOT / "target" / "debug" / f"photocraft-cli{EXE}",
    ]
    for path in candidates:
        if path.is_file():
            return path.resolve()
    found = shutil.which("photocraft-cli")
    if found:
        return Path(found).resolve()
    raise RuntimeError(
        "photocraft-cli not found. Build it with:\n"
        "  cargo build --release -p photocraft-cli --no-default-features"
    )


def ensure_token(path: Path) -> str:
    """The app and the CLI share one 64-hex control token file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        token = path.read_text(encoding="utf-8").strip()
        if len(token) == 64 and all(c in "0123456789abcdef" for c in token.lower()):
            return token
    token = secrets.token_hex(32)
    path.write_text(token + "\n", encoding="utf-8")
    with suppress(OSError):
        path.chmod(0o600)
    return token


@dataclass
class BridgeSettings:
    control_port: int
    token_file: Path
    workspace: Path
    cli: Path
    screenshot_side: int = 1800
    step_delay: float = 0.6

    @classmethod
    def from_env(cls) -> "BridgeSettings":
        port = int(os.getenv("PHOTOCRAFT_CONTROL_PORT", "7878"))
        token = Path(os.getenv("PHOTOCRAFT_TOKEN_FILE", str(BASE_DIR / "runtime" / "control.token"))).expanduser()
        if not token.is_absolute():
            token = (BASE_DIR / token).resolve()
        workspace = Path(os.getenv("PHOTOCRAFT_WORKSPACE", str(BASE_DIR / "workspace"))).expanduser()
        if not workspace.is_absolute():
            workspace = (BASE_DIR / workspace).resolve()
        workspace.mkdir(parents=True, exist_ok=True)
        ensure_token(token)
        return cls(
            control_port=port,
            token_file=token,
            workspace=workspace,
            cli=_find_cli(os.getenv("PHOTOCRAFT_CLI")),
            screenshot_side=int(os.getenv("SCREENSHOT_MAX_SIDE", "1800")),
            step_delay=float(os.getenv("VISUAL_STEP_DELAY", "0.6")),
        )


class BridgeMCP:
    """Long-lived MCP session connected to ``photocraft-cli mcp --bridge``."""

    def __init__(self, settings: BridgeSettings) -> None:
        self.settings = settings
        self._session: ClientSession | None = None
        self._task: asyncio.Task[None] | None = None
        self._ready = asyncio.Event()
        self._stop = asyncio.Event()
        self._error: BaseException | None = None
        self._lock = asyncio.Lock()
        self._start_lock = asyncio.Lock()
        self.generation = 0

    @property
    def connected(self) -> bool:
        return self._session is not None and self._task is not None and not self._task.done()

    async def start(self) -> None:
        async with self._start_lock:
            if self.connected:
                return
            self._ready = asyncio.Event()
            self._stop = asyncio.Event()
            self._error = None
            self._task = asyncio.create_task(self._run(), name="photocraft-bridge")
            await self._ready.wait()
            if self._error is not None:
                raise BridgeError(_error_text(self._error))
            if self._session is None:
                raise BridgeError("bridge MCP exited before the handshake")
            self.generation += 1

    async def _run(self) -> None:
        args = [
            "mcp",
            "--bridge",
            f"127.0.0.1:{self.settings.control_port}",
            "--control-token-file",
            str(self.settings.token_file),
        ]
        try:
            async with stdio_client(
                StdioServerParameters(command=str(self.settings.cli), args=args, env=None),
                errlog=sys.stderr,
            ) as (read, write):
                async with ClientSession(read, write) as session:
                    await session.initialize()
                    self._session = session
                    self._ready.set()
                    await self._stop.wait()
        except asyncio.CancelledError:
            raise
        except BaseException as exc:  # noqa: BLE001 - surfaced by start()
            self._error = exc
            self._ready.set()
        finally:
            self._session = None
            self._ready.set()

    async def stop(self) -> None:
        if self._task is None:
            return
        self._stop.set()
        try:
            await asyncio.wait_for(self._task, 8)
        except (TimeoutError, asyncio.CancelledError):
            self._task.cancel()
            with suppress(BaseException):
                await self._task
        except BaseException:
            pass
        self._task = None

    async def _require(self) -> ClientSession:
        if not self.connected or self._session is None:
            await self.start()
        if self._session is None:
            raise BridgeError("bridge MCP is not connected")
        return self._session

    async def call(self, tool: str, arguments: dict[str, Any], timeout: float = 90) -> Any:
        session = await self._require()
        async with self._lock:
            return await asyncio.wait_for(session.call_tool(tool, arguments), timeout=timeout)

    @staticmethod
    def _text(result: Any) -> str:
        parts = []
        for block in getattr(result, "content", []) or []:
            if getattr(block, "type", None) == "text":
                parts.append(getattr(block, "text", "") or "")
        return "\n".join(parts)

    async def list_tools(self) -> list[Any]:
        session = await self._require()
        async with self._lock:
            result = await session.list_tools()
        return list(result.tools)

    async def json_call(self, tool: str, arguments: dict[str, Any], timeout: float = 90) -> Any:
        result = await self.call(tool, arguments, timeout)
        if bool(getattr(result, "is_error", False)):
            raise BridgeError(self._text(result) or f"{tool} failed")
        text = self._text(result).strip()
        if not text:
            return None
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return text

    async def control(self, method: str, params: dict[str, Any] | None = None, timeout: float = 30) -> Any:
        payload = await self.json_call("control_call", {"method": method, "params": params or {}}, timeout)
        if isinstance(payload, dict):
            if payload.get("ok") is False:
                raise BridgeError(str(payload.get("error") or f"{method} failed"))
            if "result" in payload and set(payload) <= {"ok", "result"}:
                payload = payload["result"]
        return payload

    # -- UI primitives -------------------------------------------------

    async def inspect(self) -> dict[str, Any]:
        data = await self.json_call("ui_inspect", {})
        if not isinstance(data, dict):
            raise BridgeError("ui_inspect returned no object")
        return data

    async def move(self, x: float, y: float) -> None:
        await self.control("ui.move", {"x": x, "y": y})

    async def click(self, x: float, y: float, count: int = 1) -> None:
        await self.control("ui.click", {"x": x, "y": y, "count": count})

    async def key(self, key: str) -> None:
        await self.control("ui.key", {"key": key})

    async def pointer(self, points: list[dict[str, Any]], button: str = "left") -> Any:
        return await self.json_call("ui_pointer", {"events": points, "button": button})

    async def set_tool(self, tool: str) -> Any:
        return await self.json_call("ui_set", {"fields": {"tool": tool}})

    async def menu(self, menu_id: str) -> Any:
        return await self.json_call("ui_menu_invoke", {"id": menu_id}, timeout=90)

    async def dialog_set(self, dialog: Any, field: str, value: Any) -> Any:
        return await self.control("ui.dialog.set", {"dialog": dialog, "field": field, "value": value})

    async def dialog_confirm(self, dialog: Any) -> Any:
        return await self.control("ui.dialog.confirm", {"dialog": dialog}, timeout=90)

    async def command(self, command_id: str, params: dict[str, Any] | None = None) -> Any:
        return await self.json_call("command_run", {"id": command_id, "params": params or {}}, timeout=90)

    async def focus(self) -> None:
        await self.control("ui.focus", {}, timeout=15)

    async def screenshot(self, focus: bool = False) -> tuple[bytes, int, int]:
        payload = await self.control("ui.screenshot", {"focus": focus}, timeout=30)
        if not isinstance(payload, dict):
            raise BridgeError("screenshot returned no data")
        data = payload.get("result", payload)
        encoded = data.get("base64")
        if not encoded:
            raise BridgeError("screenshot response did not contain base64")
        try:
            raw = base64.b64decode(encoded)
        except Exception as exc:  # noqa: BLE001
            raise BridgeError(f"screenshot decode failed: {exc}") from exc
        return raw, int(data.get("width") or 0), int(data.get("height") or 0)
