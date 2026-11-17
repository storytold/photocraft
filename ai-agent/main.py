# SPDX-License-Identifier: MIT OR Apache-2.0
"""FastAPI backend for PhotoCraftAI in Bridge mode.

The backend never creates a headless document.  It connects to the running
desktop app through:

    photocraft-cli mcp --bridge 127.0.0.1:7878 --control-token-file ...

and streams every visible UI step to the browser over SSE.  The browser mirrors
the real app window through /screen.
"""

from __future__ import annotations

import asyncio
import json
from pathlib import Path
import os
import socket
import subprocess
import sys
import time
from contextlib import asynccontextmanager
from typing import Any

import uvicorn
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import Response, StreamingResponse
from pydantic import BaseModel, Field

from bridge import BridgeError, BridgeMCP, BridgeSettings
from visual import VisualDirector
from bridge_agent import GeneralAgent

import re

import planner
import settings_store

APP_BIN = Path(__file__).resolve().parent.parent / "target" / "release" / ("photocraft.exe" if os.name == "nt" else "photocraft")


class AISettingsRequest(BaseModel):
    provider: str = Field("deepseek")
    api_key: str | None = None
    model: str | None = None
    base_url: str | None = None


class ChatRequest(BaseModel):
    message: str = Field(..., min_length=1, description="用户的一句话编辑需求")
    max_steps: int | None = Field(None, ge=1, le=60, description="可选的最大步骤数")


class Runtime:
    def __init__(self, settings: BridgeSettings) -> None:
        self.settings = settings
        self.bridge = BridgeMCP(settings)
        self.director = VisualDirector(self.bridge, settings)
        self.chat_lock = asyncio.Lock()
        self.screen_lock = asyncio.Lock()
        self.last_png: bytes | None = None
        self.last_at = 0.0
        self.last_size = (0, 0)
        self.active = False
        self.seen_generation = 0

    async def start(self) -> None:
        await self.bridge.start()
        state = await self.bridge.inspect()
        print(
            f"PhotoCraftAI bridge ready: 127.0.0.1:{self.settings.control_port} "
            f"window={state.get('window')} tool={state.get('tool')}",
            file=sys.stderr,
        )
        try:
            await self.screen(force=True)
        except Exception as exc:  # noqa: BLE001
            print(f"warning: first screenshot failed: {exc}", file=sys.stderr)

    async def stop(self) -> None:
        await self.bridge.stop()

    async def screen(self, force: bool = False) -> tuple[bytes, int, int]:
        async with self.screen_lock:
            now = time.monotonic()
            stale_app = self.bridge.generation != self.seen_generation
            if not self.active and not force and self.last_png is not None and not stale_app:
                return self.last_png, *self.last_size
            if not force and self.last_png is not None and now - self.last_at < 0.35:
                return self.last_png, *self.last_size
            try:
                png, width, height = await self.bridge.screenshot(focus=False)
            except Exception:
                # macOS stops rendering fully occluded windows; raising the app
                # for one frame is the only reliable fallback.
                png, width, height = await self.bridge.screenshot(focus=True)
            self.last_png = png
            self.last_at = time.monotonic()
            self.last_size = (width, height)
            self.seen_generation = self.bridge.generation
            return png, width, height

    async def stream_chat(self, message: str):
        async with self.chat_lock:
            self.active = True
            try:
                await self.bridge.focus()
                if not _is_new_canvas_request(message):
                    yield {"type": "status", "text": "正在让模型规划操作…"}
                    async for event in GeneralAgent(self.bridge).run(message):
                        yield event
                    return
                plan, source = await planner.plan(message)
                if source == "llm":
                    yield {"type": "status", "text": "正在用模型理解需求…"}
                elif source == "local-fallback":
                    yield {"type": "status", "text": "API Key 调用失败，已回退本地解析…"}
                else:
                    yield {"type": "status", "text": "未配置 API Key，正在用本地规则解析…"}
                async for event in self.director.run(message, plan):
                    yield event
            finally:
                self.active = False
                try:
                    png, width, height = await self.bridge.screenshot(focus=False)
                    self.last_png = png
                    self.last_at = time.monotonic()
                    self.last_size = (width, height)
                except Exception:
                    pass



def _port_open(port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.4)
        return sock.connect_ex(("127.0.0.1", port)) == 0


async def launch_desktop(settings: BridgeSettings) -> None:
    """Start the native app if its control port is not listening."""
    if _port_open(settings.control_port):
        return
    if not APP_BIN.is_file():
        raise BridgeError(f"desktop app not built: {APP_BIN}  (cargo build --release -p photocraft)")
    log_path = Path("/tmp/photocraft-desktop.log")
    log = open(log_path, "ab", buffering=0)  # noqa: SIM115 - owns the child log handle
    subprocess.Popen(
        [
            str(APP_BIN),
            "--control",
            str(settings.control_port),
            "--control-token-file",
            str(settings.token_file),
            "--automation-read-root",
            str(settings.workspace),
            "--automation-write-root",
            str(settings.workspace),
            "--in-window-menus",
        ],
        cwd=str(APP_BIN.parent.parent.parent),
        stdout=log,
        stderr=subprocess.STDOUT,
        env={**os.environ, "PHOTOCRAFT_CONFIG_DIR": str(settings.workspace / "config")},
        start_new_session=True,
    )
    for _ in range(40):
        if _port_open(settings.control_port):
            return
        await asyncio.sleep(0.25)
    raise BridgeError(f"desktop app did not open port {settings.control_port} (log: {log_path})")


def _is_new_canvas_request(message: str) -> bool:
    text = message.strip()
    if re.search(r"\d{2,5}\s*[x×X*]\s*\d{2,5}", text):
        return not any(w in text for w in ("改", "调整", "替换"))
    return any(w in text for w in ("新建", "新画布", "新文件", "海报", "做一张", "做张")) and not any(
        w in text for w in ("改", "调整", "替换")
    )


@asynccontextmanager
async def lifespan(app: FastAPI):
    settings = BridgeSettings.from_env()
    runtime = Runtime(settings)
    if os.getenv("PHOTOCRAFT_AUTO_START_APP", "1") != "0":
        try:
            await launch_desktop(settings)
        except Exception as exc:  # noqa: BLE001
            print(f"warning: could not auto-start the desktop app: {exc}", file=sys.stderr)
    try:
        await runtime.start()
    except Exception as exc:  # noqa: BLE001
        print(f"warning: PhotoCraft Bridge not ready yet: {exc}", file=sys.stderr)
        print("POST /app/start (or the web button) will launch the desktop app.", file=sys.stderr)
    app.state.runtime = runtime
    app.state.settings = settings
    try:
        yield
    finally:
        await runtime.stop()


app = FastAPI(
    title="PhotoCraftAI",
    version="0.3.0",
    description="内嵌在 PhotoCraft 里的可视化 AI Agent。",
    lifespan=lifespan,
)


def _runtime(request: Request) -> Runtime:
    runtime = getattr(request.app.state, "runtime", None)
    if runtime is None:
        raise HTTPException(status_code=503, detail="bridge runtime is not ready")
    return runtime


def _sse(event: dict[str, Any]) -> str:
    return f"data: {json.dumps(event, ensure_ascii=False)}\n\n"


@app.get("/settings")
async def get_settings() -> dict[str, Any]:
    return settings_store.public(settings_store.load())


@app.post("/settings")
async def save_settings(body: AISettingsRequest) -> dict[str, Any]:
    try:
        return settings_store.save(body.model_dump())
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=400, detail=str(exc)) from exc


@app.post("/settings/test")
async def test_settings() -> dict[str, Any]:
    config = settings_store.load()
    if not config.get("api_key"):
        raise HTTPException(status_code=400, detail="还没有填写 API Key")
    from openai import AsyncOpenAI

    try:
        client = AsyncOpenAI(
            api_key=config["api_key"],
            base_url=config.get("base_url") or None,
            timeout=20.0,
            max_retries=1,
        )
        response = await client.chat.completions.create(
            model=config.get("model") or "deepseek-chat",
            messages=[{"role": "user", "content": "只回复 OK"}],
            temperature=0.0,
        )
        return {
            "ok": True,
            "provider": config.get("provider"),
            "model": config.get("model"),
            "reply": (response.choices[0].message.content or "").strip()[:40],
        }
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=400, detail=f"连接失败：{exc}") from exc


@app.post("/app/start")
async def app_start(request: Request) -> dict[str, Any]:
    runtime = _runtime(request)
    try:
        await launch_desktop(runtime.settings)
        await runtime.start()
        await runtime.screen(force=True)
        return {"status": "started", "app_connected": runtime.bridge.connected}
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=503, detail=f"启动 PhotoCraft 失败：{exc}") from exc


@app.get("/health")
async def health(request: Request) -> dict[str, Any]:
    runtime = _runtime(request)
    return {
        "status": "ok",
        "mode": "bridge",
        "app_connected": runtime.bridge.connected,
        "control_port": runtime.settings.control_port,
        "token_file": str(runtime.settings.token_file),
        "window": {"width": runtime.last_size[0] / 2 if runtime.last_size[0] else 1440,
                   "height": runtime.last_size[1] / 2 if runtime.last_size[1] else 851},
    }


@app.get("/screen")
async def screen(request: Request, refresh: int = 0) -> Response:
    runtime = _runtime(request)
    try:
        png, width, height = await runtime.screen(force=bool(refresh))
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=503, detail=f"PhotoCraft 窗口不可用：{exc}") from exc
    return Response(
        content=png,
        media_type="image/png",
        headers={
            "Cache-Control": "no-store, max-age=0",
            "X-Window-Width": str(width or ""),
            "X-Window-Height": str(height or ""),
        },
    )


@app.post("/chat/stream")
async def chat_stream(request: Request, body: ChatRequest) -> StreamingResponse:
    runtime = _runtime(request)

    async def generate():
        try:
            async for event in runtime.stream_chat(body.message):
                yield _sse(event)
        except asyncio.CancelledError:
            raise
        except Exception as exc:  # noqa: BLE001
            yield _sse({"type": "error", "error": f"{type(exc).__name__}: {exc}"})
            yield _sse({"type": "done", "reply": "执行中断，请查看错误日志。", "steps": []})

    return StreamingResponse(
        generate(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache, no-transform",
            "Connection": "keep-alive",
            "X-Accel-Buffering": "no",
        },
    )


@app.post("/chat")
async def chat(request: Request, body: ChatRequest) -> dict[str, Any]:
    runtime = _runtime(request)
    steps: list[dict[str, Any]] = []
    reply = ""
    document = None
    error = None
    try:
        async for event in runtime.stream_chat(body.message):
            if event.get("type") == "step":
                steps.append(event)
            elif event.get("type") == "done":
                reply = event.get("reply") or ""
                document = event.get("document")
            elif event.get("type") == "error":
                error = event.get("error")
    except Exception as exc:  # noqa: BLE001
        error = f"{type(exc).__name__}: {exc}"
    if error and not steps:
        raise HTTPException(status_code=502, detail=error)
    return {
        "reply": reply,
        "provider": "bridge",
        "mode": "bridge",
        "all_steps_ok": all(not step.get("is_error") for step in steps),
        "steps": steps,
        "document": document,
        "error": error,
    }


if __name__ == "__main__":
    uvicorn.run(
        "main:app",
        host=os.getenv("HOST", "127.0.0.1"),
        port=int(os.getenv("PORT", "8000")),
        reload=False,
    )
