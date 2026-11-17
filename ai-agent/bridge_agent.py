# SPDX-License-Identifier: MIT OR Apache-2.0
"""General LLM-driven PhotoCraft agent for arbitrary bridge operations.

The scripted VisualDirector is used for the demo flow (new canvas + fill).  Any
other request is handled here: the configured model receives the real bridge
tool schemas, calls ONE tool per turn, and gets an automatic ui.inspect after
every mutation.
"""

from __future__ import annotations

import asyncio
import json
from typing import Any, AsyncIterator

import openai

import settings_store
from bridge import BridgeError, BridgeMCP

ALLOWED = {
    "session_list", "doc_open", "doc_new", "doc_place", "doc_replace_smart_object",
    "doc_export_layers", "doc_save", "doc_export", "doc_inspect", "doc_select", "doc_close",
    "command_list", "command_run", "command_batch", "jobs_list", "jobs_cancel",
    "ui_inspect", "ui_set", "ui_pointer", "ui_menu_invoke",
}
MUTATING = {"doc_new", "doc_open", "doc_place", "command_run", "command_batch", "ui_pointer", "ui_set", "ui_menu_invoke"}
MAX_STEPS = 14


def _schema(tool: Any) -> dict[str, Any]:
    raw = getattr(tool, "input_schema", None) or getattr(tool, "inputSchema", None) or {}
    def norm(value: Any) -> Any:
        if isinstance(value, list):
            return [norm(v) for v in value]
        if not isinstance(value, dict):
            return value
        out = {}
        for key, item in value.items():
            if key == "default" and item is None:
                continue
            out[key] = norm(item)
        kind = out.get("type")
        if isinstance(kind, list):
            non_null = [k for k in kind if k != "null"]
            out["type"] = non_null[0] if len(non_null) == 1 else (non_null or "string")
        return out
    schema = norm(dict(raw))
    schema.pop("$schema", None)
    schema.setdefault("type", "object")
    return schema


def _openai_tool(tool: Any) -> dict[str, Any]:
    return {"type": "function", "function": {
        "name": tool.name,
        "description": (tool.description or "").strip(),
        "parameters": _schema(tool),
    }}


def _text(result: Any) -> str:
    parts = []
    for block in getattr(result, "content", []) or []:
        if getattr(block, "type", None) == "text":
            parts.append(getattr(block, "text", "") or "")
    return "\n".join(parts)


class GeneralAgent:
    def __init__(self, bridge: BridgeMCP) -> None:
        self.bridge = bridge

    async def run(self, message: str) -> AsyncIterator[dict[str, Any]]:
        config = settings_store.load()
        key = str(config.get("api_key") or "")
        if not key:
            yield {"type": "status", "text": "未配置 API Key，无法理解这句话…"}
            yield {"type": "done", "reply": "请先在 PhotoCraftAI 设置里填写 API Key，然后再试这句话。"}
            return

        state = await self.bridge.inspect()
        server_tools = await self.bridge.list_tools()
        tools = [t for t in server_tools if t.name in ALLOWED]
        openai_tools = [_openai_tool(t) for t in tools]
        if not tools:
            raise BridgeError("bridge exposed no usable tools")

        command_ids = set()
        try:
            listing = await self.bridge.json_call("command_list", {})
            if isinstance(listing, list):
                command_ids = {str(row.get("id")) for row in listing if isinstance(row, dict)}
        except Exception:
            pass

        doc = state.get("document") or {}
        context = {
            "tool": state.get("tool"),
            "window": state.get("window"),
            "document": {
                "name": doc.get("name"), "width": doc.get("width"), "height": doc.get("height"),
                "layers": [l.get("name") for l in (doc.get("layers") or [])],
                "activeLayer": doc.get("activeLayer"),
            } if doc else None,
        }
        system = """你是 PhotoCraftAI，一个真正在操作桌面版 PhotoCraft 的 Photoshop 操作员。
你只能调用提供的真实工具，不能编造工具名或 command id。
每轮只调用一个工具；修改后系统会自动 ui_inspect，你要读结果并检查。
编辑已有文档时，除非用户明确要求，不要新建文档。
常用：command_list 查命令；command_run 执行命令；ui_set 切换工具；ui_pointer 在画布上操作；ui_menu_invoke 打开菜单。
完成后不要再调用工具，用中文简短汇报。当前界面状态：""" + json.dumps(context, ensure_ascii=False)

        messages: list[dict[str, Any]] = [
            {"role": "system", "content": system},
            {"role": "user", "content": message},
        ]
        step_no = 0

        def step_event(tool: str, args: dict[str, Any], result: Any, is_error: bool) -> dict[str, Any]:
            nonlocal step_no
            step_no += 1
            text = result if isinstance(result, str) else json.dumps(result, ensure_ascii=False)
            if len(text) > 900:
                text = text[:900] + "…"
            return {
                "type": "step", "step": step_no, "tool": tool,
                "label": tool, "arguments": args, "result": text,
                "is_error": is_error, "cursor": None,
            }

        for _ in range(MAX_STEPS):
            reply = await self._complete(config, messages, openai_tools)
            if not reply.get("tool_calls"):
                yield {"type": "done", "reply": (reply.get("content") or "完成。").strip()}
                return
            calls = reply["tool_calls"]
            if len(calls) > 1:
                yield {"type": "status", "text": "模型一次给了多个操作，只执行第一个…"}
            call = calls[0]
            name = call["function"]["name"]
            raw_args = call["function"].get("arguments") or "{}"
            try:
                args = json.loads(raw_args)
                if not isinstance(args, dict):
                    raise ValueError("arguments must be an object")
            except Exception as error:  # noqa: BLE001
                yield {"type": "status", "text": f"参数不是合法 JSON：{error}"}
                messages.append({"role": "assistant", "content": None, "tool_calls": calls})
                messages.append({"role": "tool", "tool_call_id": call["id"], "content": json.dumps({"error": str(error)})})
                continue

            messages.append({"role": "assistant", "content": reply.get("content") or "", "tool_calls": calls})
            if name not in ALLOWED:
                result_text = json.dumps({"error": f"unknown tool {name}"})
                yield step_event(name, args, {"error": f"unknown tool {name}"}, True)
                messages.append({"role": "tool", "tool_call_id": call["id"], "content": result_text})
                continue
            if name == "command_run" and command_ids and args.get("id") not in command_ids:
                err = f"unknown command id {args.get('id')!r}; call command_list first"
                yield step_event(name, args, {"error": err}, True)
                messages.append({"role": "tool", "tool_call_id": call["id"], "content": json.dumps({"error": err})})
                continue

            yield {"type": "status", "text": f"正在执行：{name}…"}
            try:
                result = await self.bridge.call(name, args, timeout=90)
                is_error = bool(getattr(result, "is_error", False))
                text = _text(result)
                try:
                    payload = json.loads(text)
                except Exception:
                    payload = text[:2000]
            except Exception as error:  # noqa: BLE001
                is_error = True
                payload = {"error": f"{type(error).__name__}: {error}"}
            yield step_event(name, args, payload, is_error)
            messages.append({"role": "tool", "tool_call_id": call["id"], "content": json.dumps(payload, ensure_ascii=False)[:9000]})

            if not is_error and name in MUTATING:
                await asyncio.sleep(0.6)
                try:
                    checked = await self.bridge.inspect()
                    check = {
                        "tool": checked.get("tool"),
                        "document": {
                            "name": (checked.get("document") or {}).get("name"),
                            "width": (checked.get("document") or {}).get("width"),
                            "height": (checked.get("document") or {}).get("height"),
                            "layers": [l.get("name") for l in ((checked.get("document") or {}).get("layers") or [])],
                        } if checked.get("document") else None,
                        "dialogs": [d.get("title") for d in checked.get("dialogs") or []],
                    }
                except Exception as error:  # noqa: BLE001
                    check = {"error": str(error)}
                yield step_event("ui_inspect", {}, check, "error" in check)
                messages.append({"role": "user", "content": "[AUTO-CHECK] ui.inspect 结果：" + json.dumps(check, ensure_ascii=False) + "。检查是否正确，正确就结束，否则继续用一个工具纠正。"})

        yield {"type": "done", "reply": "已达到最大步数，请查看日志确认当前状态。"}

    async def _complete(self, config: dict[str, Any], messages: list[dict[str, Any]], tools: list[dict[str, Any]]) -> dict[str, Any]:
        client = openai.AsyncOpenAI(
            api_key=config["api_key"],
            base_url=config.get("base_url") or None,
            timeout=60.0,
            max_retries=1,
        )
        response = await client.chat.completions.create(
            model=config.get("model") or "deepseek-chat",
            messages=messages,
            tools=tools,
            tool_choice="auto",
            temperature=0.1,
        )
        msg = response.choices[0].message
        calls = [
            {"id": c.id, "type": "function", "function": {"name": c.function.name, "arguments": c.function.arguments or "{}"}}
            for c in (msg.tool_calls or [])
        ]
        return {"content": msg.content, "tool_calls": calls}
