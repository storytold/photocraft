# SPDX-License-Identifier: MIT OR Apache-2.0
"""说画 / PhotoCraft Agent loop.

One HTTP request = one agent run.  The LLM sees only real tools fetched from
``tools/list``; it must call one tool at a time, and every mutating call is
followed automatically by ``doc_inspect`` + ``doc_render_preview`` before the
result is handed back to the model.

The MCP server is a single stdio process shared by all requests and guarded by
an asyncio lock, so documents survive across /chat calls.
"""

from __future__ import annotations

import asyncio
import base64
import difflib
import json
import os
import re
import shutil
import sys
import time
from contextlib import suppress
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Protocol

import openai
from dotenv import load_dotenv
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

from prompts import auto_check_message, build_system_prompt

BASE_DIR = Path(__file__).resolve().parent
REPO_ROOT = BASE_DIR.parent
IS_WINDOWS = os.name == "nt"
EXE = ".exe" if IS_WINDOWS else ""


# ---------------------------------------------------------------------------
# Settings
# ---------------------------------------------------------------------------

PROVIDER_DEFAULTS = {
    "deepseek": ("https://api.deepseek.com", "deepseek-chat"),
    "qwen": ("https://dashscope.aliyuncs.com/compatible-mode/v1", "qwen-plus"),
    "openai": ("https://api.openai.com/v1", "gpt-4o-mini"),
}


@dataclass
class Settings:
    provider: str
    api_key: str
    base_url: str
    model: str
    mcp_cli: Path
    workspace: Path
    output_dir: Path
    max_steps: int = 16
    mcp_timeout: float = 120.0
    mock_reason: str | None = None


def _find_cli(explicit: str | None) -> Path:
    candidates: list[Path] = []
    if explicit:
        p = Path(explicit).expanduser()
        candidates.append(p if p.is_absolute() else (BASE_DIR / p))
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
        "  cargo build --release -p photocraft-cli --no-default-features\n"
        "or set PHOTOCRAFT_CLI=/absolute/path/to/photocraft-cli"
    )


def load_settings() -> Settings:
    load_dotenv(BASE_DIR / ".env")
    load_dotenv()  # also accept .env in the current working directory

    def env(name: str) -> str | None:
        value = os.getenv(name)
        return value.strip() if value and value.strip() else None

    explicit = (env("LLM_PROVIDER") or "auto").lower()
    key_by_provider = {
        "deepseek": env("DEEPSEEK_API_KEY"),
        "qwen": env("QWEN_API_KEY") or env("DASHSCOPE_API_KEY"),
        "openai": env("OPENAI_API_KEY"),
    }

    mock_reason: str | None = None
    if explicit == "auto":
        provider = next((p for p, key in key_by_provider.items() if key), "mock")
        if provider == "mock":
            mock_reason = (
                "No API key found (DEEPSEEK_API_KEY / DASHSCOPE_API_KEY / "
                "OPENAI_API_KEY); running the offline deterministic planner."
            )
    else:
        provider = explicit

    if provider not in {*PROVIDER_DEFAULTS, "mock"}:
        raise RuntimeError(
            f"Unknown LLM_PROVIDER={provider!r}; expected auto, deepseek, qwen, openai or mock"
        )

    if provider == "mock":
        api_key, base_url, model = "", "", "mock"
    else:
        default_base, default_model = PROVIDER_DEFAULTS[provider]
        api_key = env("LLM_API_KEY") or key_by_provider[provider] or ""
        if not api_key:
            env_name = {
                "deepseek": "DEEPSEEK_API_KEY",
                "qwen": "DASHSCOPE_API_KEY (or QWEN_API_KEY)",
                "openai": "OPENAI_API_KEY",
            }[provider]
            raise RuntimeError(
                f"LLM_PROVIDER={provider} but no API key is set. "
                f"Put {env_name}=... in ai-agent/.env (or set LLM_API_KEY)."
            )
        base_url = env("LLM_BASE_URL") or default_base
        model = env("LLM_MODEL") or default_model

    workspace = Path(env("PHOTOCRAFT_WORKSPACE") or BASE_DIR / "workspace").expanduser()
    if not workspace.is_absolute():
        workspace = (BASE_DIR / workspace).resolve()
    workspace.mkdir(parents=True, exist_ok=True)

    output_dir = BASE_DIR / "outputs"
    output_dir.mkdir(parents=True, exist_ok=True)

    return Settings(
        provider=provider,
        api_key=api_key,
        base_url=base_url,
        model=model,
        mcp_cli=_find_cli(env("PHOTOCRAFT_CLI")),
        workspace=workspace,
        output_dir=output_dir,
        max_steps=int(env("MAX_STEPS") or 16),
        mcp_timeout=float(env("MCP_TIMEOUT") or 120),
        mock_reason=mock_reason,
    )


# ---------------------------------------------------------------------------
# LLM interface
# ---------------------------------------------------------------------------


@dataclass
class ToolCall:
    id: str
    name: str
    arguments: str


@dataclass
class LLMReply:
    content: str | None
    tool_calls: list[ToolCall] = field(default_factory=list)


class LLM(Protocol):
    async def complete(self, messages: list[dict[str, Any]], tools: list[dict[str, Any]]) -> LLMReply: ...


class OpenAILLM:
    """One adapter for DeepSeek, Qwen (DashScope compatible mode) and OpenAI."""

    def __init__(self, settings: Settings) -> None:
        self.model = settings.model
        self.client = openai.AsyncOpenAI(
            api_key=settings.api_key,
            base_url=settings.base_url,
            timeout=90.0,
            max_retries=2,
        )

    async def complete(self, messages: list[dict[str, Any]], tools: list[dict[str, Any]]) -> LLMReply:
        response = await self.client.chat.completions.create(
            model=self.model,
            messages=messages,
            tools=tools,
            tool_choice="auto",
            temperature=0.1,
        )
        message = response.choices[0].message
        calls = [
            ToolCall(
                id=call.id,
                name=call.function.name,
                arguments=call.function.arguments or "{}",
            )
            for call in (message.tool_calls or [])
        ]
        return LLMReply(content=message.content, tool_calls=calls)


class MockLLM:
    """Offline planner used only when no API key is configured.

    It is a deterministic policy, not a language model: every tool call below
    still goes through the real MCP server and every result is real.
    """

    def __init__(self) -> None:
        self._n = 0

    def _call(self, name: str, arguments: dict[str, Any]) -> LLMReply:
        self._n += 1
        return LLMReply(
            content=None,
            tool_calls=[ToolCall(id=f"mock-{self._n}", name=name, arguments=json.dumps(arguments))],
        )

    async def complete(self, messages: list[dict[str, Any]], tools: list[dict[str, Any]]) -> LLMReply:
        user_text = next(
            (str(m.get("content") or "") for m in messages if m.get("role") == "user"),
            "",
        )
        called: list[str] = []
        for message in messages:
            if message.get("role") != "assistant":
                continue
            for call in message.get("tool_calls") or []:
                called.append(call["function"]["name"])

        wants_fill = any(word in user_text for word in ("填充", "fill", "涂"))
        if "command_list" not in called:
            return self._call("command_list", {"filter": "fill" if wants_fill else "new"})

        if "doc_new" not in called:
            width, height = 1920, 1080
            match = re.search(r"(\d{2,5})\s*[x×X*]\s*(\d{2,5})", user_text)
            if match:
                width, height = int(match.group(1)), int(match.group(2))
            return self._call(
                "doc_new",
                {
                    "width": width,
                    "height": height,
                    "background": "#ffffff",
                    "name": "说画",
                },
            )

        if wants_fill and "command_run" not in called:
            color = "#0000ff"
            hex_match = re.search(r"#[0-9a-fA-F]{6}", user_text)
            if hex_match:
                color = hex_match.group(0).lower()
            elif "红" in user_text:
                color = "#ff0000"
            elif "绿" in user_text:
                color = "#00ff00"
            elif "黑" in user_text:
                color = "#000000"
            elif "白" in user_text:
                color = "#ffffff"
            return self._call(
                "command_run",
                {"id": "layer.newFillLayer.solidColor", "params": {"color": color}},
            )

        action = "新建了画布并创建蓝色填充图层" if wants_fill else "新建了画布"
        return LLMReply(content=f"已完成：{action}。请查看执行日志中的 doc_inspect。", tool_calls=[])



# ---------------------------------------------------------------------------
# MCP client
# ---------------------------------------------------------------------------


def _flatten_exceptions(exc: BaseException):
    if isinstance(exc, BaseExceptionGroup):
        for inner in exc.exceptions:
            yield from _flatten_exceptions(inner)
    else:
        yield exc


def _format_exception(exc: BaseException) -> str:
    return "; ".join(f"{type(e).__name__}: {e}" for e in _flatten_exceptions(exc))


class MCPClient:
    """A long-lived stdio MCP session running in its own asyncio task."""

    def __init__(self, settings: Settings) -> None:
        self.settings = settings
        self._session: ClientSession | None = None
        self._task: asyncio.Task[None] | None = None
        self._ready = asyncio.Event()
        self._stop = asyncio.Event()
        self._error: BaseException | None = None
        self._call_lock = asyncio.Lock()
        self._start_lock = asyncio.Lock()

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
            self._task = asyncio.create_task(self._run(), name="photocraft-mcp")
            await self._ready.wait()
            if self._error is not None:
                raise RuntimeError(f"MCP server failed to start: {_format_exception(self._error)}")
            if self._session is None:
                raise RuntimeError("MCP server exited before the handshake completed")

    async def _run(self) -> None:
        args = [
            "mcp",
            "--automation-read-root",
            str(self.settings.workspace),
            "--automation-write-root",
            str(self.settings.workspace),
        ]
        params = StdioServerParameters(
            command=str(self.settings.mcp_cli),
            args=args,
            env=None,
        )
        try:
            async with stdio_client(params, errlog=sys.stderr) as (read, write):
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
            await asyncio.wait_for(self._task, timeout=8)
        except (TimeoutError, asyncio.CancelledError):
            self._task.cancel()
            with suppress(BaseException):
                await self._task
        except BaseException:
            pass
        self._task = None

    async def _require_session(self) -> ClientSession:
        if not self.connected or self._session is None:
            await self.start()
        if self._session is None:
            raise RuntimeError("MCP session is not connected")
        return self._session

    async def call(self, name: str, arguments: dict[str, Any]) -> Any:
        session = await self._require_session()
        async with self._call_lock:
            return await asyncio.wait_for(
                session.call_tool(name, arguments),
                timeout=self.settings.mcp_timeout,
            )

    async def list_tools(self) -> list[Any]:
        session = await self._require_session()
        async with self._call_lock:
            result = await asyncio.wait_for(
                session.list_tools(),
                timeout=self.settings.mcp_timeout,
            )
        return list(result.tools)


# ---------------------------------------------------------------------------
# Agent runtime
# ---------------------------------------------------------------------------

# headless-safe tools only: no ui_*, no control_call, no render_preview alias
ALLOWED_TOOLS = {
    "session_list",
    "doc_open",
    "doc_new",
    "doc_place",
    "doc_replace_smart_object",
    "doc_export_layers",
    "doc_export_data_sets",
    "doc_import_data_sets",
    "doc_save",
    "doc_export",
    "doc_inspect",
    "doc_render_preview",
    "doc_select",
    "doc_close",
    "command_list",
    "command_run",
    "command_batch",
    "jobs_list",
    "jobs_cancel",
}

MUTATING_TOOLS = {
    "doc_new",
    "doc_open",
    "doc_place",
    "doc_replace_smart_object",
    "command_run",
    "command_batch",
}

MAX_TOOL_TEXT = 12_000


def _normalize_schema(value: Any) -> Any:
    """Turn MCP/Pydantic JSON Schema into the subset LLM vendors accept reliably.

    The server emits ``"type": ["string", "null"]``; several OpenAI-compatible
    APIs reject a type array, so nullable fields become their non-null type and
    ``default: null`` is dropped (the field stays optional via ``required``).
    """
    if isinstance(value, list):
        return [_normalize_schema(item) for item in value]
    if not isinstance(value, dict):
        return value

    out: dict[str, Any] = {}
    for key, item in value.items():
        if key == "default" and item is None:
            continue
        out[key] = _normalize_schema(item)

    kind = out.get("type")
    if isinstance(kind, list):
        non_null = [k for k in kind if k != "null"]
        if non_null:
            out["type"] = non_null[0] if len(non_null) == 1 else non_null
        else:
            out.pop("type", None)
    return out


def _to_openai_tool(tool: Any) -> dict[str, Any]:
    raw_schema = getattr(tool, "input_schema", None)
    if raw_schema is None:
        raw_schema = getattr(tool, "inputSchema", None)
    schema = _normalize_schema(dict(raw_schema or {}))
    schema.pop("$schema", None)
    if "type" not in schema:
        schema["type"] = "object"
    return {
        "type": "function",
        "function": {
            "name": tool.name,
            "description": (tool.description or "").strip(),
            "parameters": schema,
        },
    }


@dataclass
class RuntimeState:
    settings: Settings
    mcp: MCPClient
    tools: list[Any]
    tool_names: dict[str, Any]
    openai_tools: list[dict[str, Any]]
    command_ids: set[str]
    lock: asyncio.Lock = field(default_factory=asyncio.Lock)


class AgentRuntime:
    def __init__(self, settings: Settings) -> None:
        self.settings = settings
        self.mcp = MCPClient(settings)
        self.llm: LLM = MockLLM() if settings.provider == "mock" else OpenAILLM(settings)
        self.state: RuntimeState | None = None

    @property
    def provider_label(self) -> str:
        return "mock (offline planner)" if self.settings.provider == "mock" else self.settings.model

    async def start(self) -> None:
        await self.mcp.start()
        server_tools = await self.mcp.list_tools()
        tools = [t for t in server_tools if t.name in ALLOWED_TOOLS]
        missing = ALLOWED_TOOLS - {t.name for t in tools}
        if missing:
            print(f"warning: server is missing expected tools: {sorted(missing)}", file=sys.stderr)
        if not tools:
            raise RuntimeError("MCP server exposed none of the headless tools")
        command_ids = await self._load_command_ids()
        self.state = RuntimeState(
            settings=self.settings,
            mcp=self.mcp,
            tools=tools,
            tool_names={t.name: t for t in tools},
            openai_tools=[_to_openai_tool(t) for t in tools],
            command_ids=command_ids,
        )

    async def _load_command_ids(self) -> set[str]:
        try:
            result = await self.mcp.call("command_list", {})
            for block in result.content:
                text = getattr(block, "text", None)
                if text:
                    rows = json.loads(text)
                    return {str(row["id"]) for row in rows if isinstance(row, dict) and "id" in row}
        except Exception as exc:  # noqa: BLE001 - guard is optional
            print(f"warning: could not preload command ids: {_format_exception(exc)}", file=sys.stderr)
        return set()

    async def stop(self) -> None:
        await self.mcp.stop()

    async def chat(self, message: str, max_steps: int | None = None) -> dict[str, Any]:
        if self.state is None:
            raise RuntimeError("runtime is not started")
        async with self.state.lock:
            agent = Agent(
                state=self.state,
                llm=self.llm,
                message=message,
                max_steps=max_steps or self.settings.max_steps,
                provider=self.settings.provider,
                mock_reason=self.settings.mock_reason,
            )
            return await agent.run()


# ---------------------------------------------------------------------------
# Agent loop
# ---------------------------------------------------------------------------


class ToolGuardError(RuntimeError):
    pass


class Agent:
    def __init__(
        self,
        state: RuntimeState,
        llm: LLM,
        message: str,
        max_steps: int,
        provider: str,
        mock_reason: str | None,
    ) -> None:
        self.state = state
        self.llm = llm
        self.settings = state.settings
        self.max_steps = max_steps
        self.provider = provider
        self.mock_reason = mock_reason
        self.steps: list[dict[str, Any]] = []
        self.warnings: list[str] = []
        self.reply = ""
        self.discovered = False
        self.messages: list[dict[str, Any]] = [
            {"role": "system", "content": build_system_prompt(list(state.tool_names))},
            {"role": "user", "content": message},
        ]

    async def run(self) -> dict[str, Any]:
        for _ in range(self.max_steps):
            reply = await self.llm.complete(self.messages, self.state.openai_tools)
            self.messages.append(self._assistant_message(reply))

            if not reply.tool_calls:
                self.reply = (reply.content or "").strip() or "完成。"
                return self._result()

            if len(reply.tool_calls) > 1:
                self.warnings.append(
                    f"LLM returned {len(reply.tool_calls)} tool calls; only the first was executed. "
                    "The agent enforces one tool per step."
                )

            primary = reply.tool_calls[0]
            arguments = self._parse_arguments(primary)
            if arguments is None:
                self.messages.append(
                    self._tool_message(
                        primary.id,
                        {"error": "invalid JSON tool arguments; send a JSON object"},
                    )
                )
                for extra in reply.tool_calls[1:]:
                    self.messages.append(
                        self._tool_message(extra.id, {"error": "only one tool call per turn is allowed"})
                    )
                continue

            log, llm_text = await self._execute_tool(primary.name, arguments)
            self.messages.append(self._tool_message(primary.id, llm_text))
            for extra in reply.tool_calls[1:]:
                self.messages.append(
                    self._tool_message(extra.id, {"error": "only one tool call per turn is allowed"})
                )

            if log["is_error"] is False and primary.name in MUTATING_TOOLS:
                await self._auto_check(primary.name)

        self.reply = (
            f"已达到最大步数 {self.max_steps}，最后一步结果请查看执行日志。"
        )
        return self._result()

    # -- message helpers -------------------------------------------------

    @staticmethod
    def _assistant_message(reply: LLMReply) -> dict[str, Any]:
        message: dict[str, Any] = {"role": "assistant", "content": reply.content or ""}
        if reply.tool_calls:
            message["tool_calls"] = [
                {
                    "id": call.id,
                    "type": "function",
                    "function": {"name": call.name, "arguments": call.arguments},
                }
                for call in reply.tool_calls
            ]
        return message

    @staticmethod
    def _tool_message(call_id: str, content: Any) -> dict[str, Any]:
        if not isinstance(content, str):
            content = json.dumps(content, ensure_ascii=False)
        return {"role": "tool", "tool_call_id": call_id, "content": content}

    @staticmethod
    def _parse_arguments(call: ToolCall) -> dict[str, Any] | None:
        try:
            value = json.loads(call.arguments or "{}")
        except json.JSONDecodeError:
            return None
        return value if isinstance(value, dict) else None

    # -- tool execution --------------------------------------------------

    async def _execute_tool(
        self,
        name: str,
        arguments: dict[str, Any],
        *,
        auto: bool = False,
        after: str | None = None,
    ) -> tuple[dict[str, Any], str]:
        started = time.perf_counter()
        log: dict[str, Any] = {
            "step": len(self.steps) + 1,
            "tool": name,
            "arguments": arguments,
            "auto": auto,
            "is_error": False,
        }
        if after:
            log["after"] = after

        try:
            if name not in self.state.tool_names:
                raise ToolGuardError(
                    f"unknown tool {name!r}; use only the mounted tools: "
                    f"{', '.join(sorted(self.state.tool_names))}"
                )
            if name in {"command_run", "command_batch"} and not self.discovered:
                raise ToolGuardError(
                    "call command_list first to discover real command ids before running commands"
                )
            if name == "command_run":
                self._validate_command(arguments.get("id"))
            if name == "command_batch":
                steps = arguments.get("steps")
                if not isinstance(steps, list):
                    raise ToolGuardError("command_batch requires a steps array")
                for step in steps:
                    if not isinstance(step, dict):
                        raise ToolGuardError("each batch step must be an object")
                    self._validate_command(step.get("id"))

            raw = await self.state.mcp.call(name, arguments)
            is_error = bool(getattr(raw, "is_error", False))
            payload, llm_text = self._payload(name, raw)
        except Exception as exc:  # noqa: BLE001 - reported to the LLM
            is_error = True
            payload = {"error": _format_exception(exc)}
            llm_text = json.dumps(payload, ensure_ascii=False)

        log["is_error"] = is_error
        log["result"] = payload
        log["duration_ms"] = round((time.perf_counter() - started) * 1000, 1)
        self.steps.append(log)

        if name == "command_list" and not is_error:
            self.discovered = True
        return log, llm_text

    def _validate_command(self, command_id: Any) -> None:
        if not self.state.command_ids:
            return  # preload failed; let the engine report a bad id
        if not isinstance(command_id, str) or not command_id:
            raise ToolGuardError("command id must be a non-empty string")
        if command_id not in self.state.command_ids:
            close = difflib.get_close_matches(command_id, self.state.command_ids, n=3)
            hint = f" Did you mean: {', '.join(close)}?" if close else ""
            raise ToolGuardError(
                f"unknown command id {command_id!r}.{hint} Call command_list to discover real ids."
            )

    def _payload(self, name: str, raw: Any) -> tuple[Any, str]:
        texts: list[str] = []
        images: list[dict[str, Any]] = []
        for block in getattr(raw, "content", []) or []:
            kind = getattr(block, "type", None)
            if kind == "text":
                texts.append(getattr(block, "text", "") or "")
            elif kind == "image":
                images.append(self._save_image(block))

        if name == "doc_render_preview" and images:
            payload: Any = {
                "preview_png": images[0]["path"],
                "bytes": images[0]["bytes"],
                "text": " ".join(t for t in texts if t),
            }
        elif len(texts) == 1:
            try:
                payload = json.loads(texts[0])
            except json.JSONDecodeError:
                payload = texts[0]
        elif texts:
            payload = texts
        else:
            payload = {"images": images} if images else {}

        if name == "command_list" and isinstance(payload, list):
            payload = self._compact_commands(payload)

        llm_text = json.dumps(payload, ensure_ascii=False)
        if len(llm_text) > MAX_TOOL_TEXT:
            llm_text = llm_text[:MAX_TOOL_TEXT] + " …[truncated]"
        return payload, llm_text

    def _save_image(self, block: Any) -> dict[str, Any]:
        data = getattr(block, "data", None)
        mime = getattr(block, "mime_type", None) or getattr(block, "mimeType", "") or ""
        suffix = ".png" if "png" in str(mime).lower() or not mime else ".bin"
        target = self.settings.output_dir / f"preview-{int(time.time() * 1000)}-{len(self.steps)}{suffix}"
        try:
            raw = base64.b64decode(data or "")
            target.write_bytes(raw)
            return {"path": str(target), "bytes": len(raw), "mime": mime}
        except Exception as exc:  # noqa: BLE001
            return {"error": f"could not save preview: {exc}"}

    @staticmethod
    def _compact_commands(rows: list[Any]) -> dict[str, Any]:
        shown = []
        for row in rows[:80]:
            if not isinstance(row, dict):
                continue
            shown.append(
                {
                    "id": row.get("id"),
                    "label": row.get("label"),
                    "params": row.get("params"),
                    "enabled": row.get("enabled"),
                    "needsDocument": row.get("needsDocument"),
                }
            )
        return {
            "count": len(rows),
            "shown": len(shown),
            "commands": shown,
            "hint": "filter command_list by keyword to see more" if len(rows) > len(shown) else "",
        }

    # -- mandatory verification -----------------------------------------

    async def _auto_check(self, tool_name: str) -> None:
        inspect_log, _ = await self._execute_tool(
            "doc_inspect", {}, auto=True, after=tool_name
        )
        if inspect_log["is_error"]:
            return

        preview_log, _ = await self._execute_tool(
            "doc_render_preview", {"max_side": 768}, auto=True, after=tool_name
        )
        preview_path = None
        if isinstance(preview_log["result"], dict):
            preview_path = preview_log["result"].get("preview_png")

        summary = self._doc_summary(inspect_log["result"])
        self.messages.append(
            {
                "role": "user",
                "content": auto_check_message(tool_name, summary, preview_path),
            }
        )

    @staticmethod
    def _doc_summary(payload: Any) -> Any:
        if not isinstance(payload, dict):
            return payload
        layers = payload.get("layers") or []
        return {
            "name": payload.get("name"),
            "width": payload.get("width"),
            "height": payload.get("height"),
            "mode": payload.get("mode"),
            "depth": payload.get("depth"),
            "activeLayer": payload.get("activeLayer"),
            "layers": [
                {
                    "id": layer.get("id"),
                    "name": layer.get("name"),
                    "kind": layer.get("kind"),
                    "visible": layer.get("visible"),
                    "selected": layer.get("selected"),
                    "bounds": layer.get("bounds"),
                }
                for layer in layers
                if isinstance(layer, dict)
            ],
            "history": payload.get("history"),
        }

    # -- response --------------------------------------------------------

    def _result(self) -> dict[str, Any]:
        document = None
        for step in reversed(self.steps):
            if step["tool"] == "doc_inspect" and not step["is_error"] and isinstance(step["result"], dict):
                document = step["result"]
                break
        return {
            "reply": self.reply,
            "provider": self.provider,
            "model": "mock" if self.provider == "mock" else getattr(self.llm, "model", ""),
            "mock_warning": self.mock_reason,
            "all_steps_ok": all(not step["is_error"] for step in self.steps),
            "steps": self.steps,
            "document": document,
            "warnings": self.warnings,
        }
