# SPDX-License-Identifier: MIT OR Apache-2.0
"""Visible, human-paced PhotoCraft automation over Bridge mode.

Every action below is a real bridge call against the running desktop app:
menus are clicked, the New Document dialog is filled, the Paint Bucket tool is
selected, a layer is created, the foreground colour is changed, and the canvas
is clicked.  Between actions we pause so the screenshot stream can show it.
"""

from __future__ import annotations

import asyncio
import re
from typing import Any, AsyncIterator

from bridge import BridgeError, BridgeMCP, BridgeSettings


COLOR_NAMES = {
    "蓝": "#0000ff",
    "红": "#ff0000",
    "绿": "#00ff00",
    "黑": "#000000",
    "白": "#ffffff",
    "黄": "#ffff00",
    "紫": "#800080",
    "橙": "#ffa500",
}


def parse_request(message: str) -> dict[str, Any]:
    text = message.strip()
    match = re.search(r"(\d{2,5})\s*[x×X*]\s*(\d{2,5})", text)
    if match:
        width, height = int(match.group(1)), int(match.group(2))
    elif "海报" in text or "竖" in text:
        width, height = 1080, 1920
    else:
        width, height = 1920, 1080

    color = None
    hex_match = re.search(r"#[0-9a-fA-F]{6}", text)
    if hex_match:
        color = hex_match.group(0).lower()
    else:
        for word, value in COLOR_NAMES.items():
            if word in text:
                color = value
                break
    wants_fill = color is not None or any(w in text for w in ("填充", "涂", "fill"))
    return {
        "width": width,
        "height": height,
        "height_input": height,
        "color": color or "#0000ff",
        "wants_fill": wants_fill,
        "poster": "海报" in text,
    }


class VisualDirector:
    def __init__(self, bridge: BridgeMCP, settings: BridgeSettings) -> None:
        self.bridge = bridge
        self.settings = settings

    # -- geometry ------------------------------------------------------

    @staticmethod
    def _pt(window: dict[str, Any], fx: float, fy: float) -> tuple[float, float]:
        width = float(window.get("width") or 1440)
        height = float(window.get("height") or 851)
        return round(width * fx, 1), round(height * fy, 1)

    async def _pause(self, factor: float = 1.0) -> None:
        await asyncio.sleep(max(0.25, self.settings.step_delay * factor))

    async def _inspect(self) -> dict[str, Any]:
        return await self.bridge.inspect()

    @staticmethod
    def _first_dialog(state: dict[str, Any], kind: str | None = None) -> dict[str, Any] | None:
        for dialog in state.get("dialogs") or []:
            if kind is None or dialog.get("kind") == kind:
                return dialog
        return None

    async def _wait_dialog(self, kind: str, timeout: float = 4.0) -> tuple[dict[str, Any], dict[str, Any]]:
        deadline = asyncio.get_running_loop().time() + timeout
        state = await self._inspect()
        while asyncio.get_running_loop().time() < deadline:
            dialog = self._first_dialog(state, kind)
            if dialog:
                return state, dialog
            await asyncio.sleep(0.2)
            state = await self._inspect()
        raise BridgeError(f"{kind} dialog did not open")

    # -- stream --------------------------------------------------------

    async def run(self, message: str, plan: dict[str, Any] | None = None) -> AsyncIterator[dict[str, Any]]:
        req = parse_request(message)
        if plan:
            req.update({k: v for k, v in plan.items() if k in req})
        step_no = 0

        def event(tool: str, label: str, arguments: dict[str, Any], result: Any = None,
                  cursor: tuple[float, float] | None = None, is_error: bool = False) -> dict[str, Any]:
            nonlocal step_no
            step_no += 1
            return {
                "type": "step",
                "step": step_no,
                "tool": tool,
                "label": label,
                "arguments": arguments,
                "result": result if result is not None else "ok",
                "is_error": is_error,
                "cursor": {"x": cursor[0], "y": cursor[1]} if cursor else None,
            }

        yield {"type": "status", "text": "正在连接桌面版 PhotoCraft…"}
        state = await self._inspect()
        window = state.get("window") or {"width": 1440, "height": 851}
        yield {"type": "hello", "mode": "bridge", "window": window}
        yield event("ui_inspect", "读取 PhotoCraft 界面", {}, {
            "window": window,
            "tool": state.get("tool"),
            "documents": len((state.get("session") or {}).get("documents") or []),
        })

        # A restored dialog or popup would swallow the next click.
        if state.get("dialogs"):
            for dialog in list(state["dialogs"]):
                await self.bridge.control("ui.dialog.cancel", {"dialog": dialog["id"]})
            await self._pause(0.5)
            state = await self._inspect()
        await self.bridge.key("Escape")

        file_pos = self._pt(window, 0.0806, 0.0211)
        new_pos = self._pt(window, 0.0868, 0.0541)
        layer_pos = self._pt(window, 0.1778, 0.0211)
        bucket_pos = self._pt(window, 0.0139, 0.5993)
        create_pos = self._pt(window, 0.7465, 0.8508)
        ok_pos = self._pt(window, 0.6194, 0.6416)

        # 1. File menu, visibly open.
        yield {"type": "status", "text": "正在把鼠标移到 File 菜单…"}
        await self.bridge.move(*file_pos)
        yield event("ui.move", "鼠标移到 File 菜单", {"x": file_pos[0], "y": file_pos[1]}, "moved", file_pos)
        await self._pause(0.5)
        await self.bridge.click(*file_pos)
        yield event("ui.click", "点击 File，展开菜单", {"x": file_pos[0], "y": file_pos[1]}, "menu open", file_pos)
        await self._pause(0.8)

        # 2. Choose New… by clicking the visible menu row; fall back to the
        #    menu-invoke bridge call if the popup geometry changed.
        yield {"type": "status", "text": "正在选择 File → New…"}
        await self.bridge.move(*new_pos)
        yield event("ui.move", "鼠标移到 New…", {"x": new_pos[0], "y": new_pos[1]}, "moved", new_pos)
        await self._pause(0.35)
        await self.bridge.click(*new_pos)
        yield event("ui.click", "点击 New…，打开新建画布弹窗", {"x": new_pos[0], "y": new_pos[1]}, "dialog", new_pos)
        await self._pause(0.6)

        state = await self._inspect()
        if not self._first_dialog(state, "NewDocument"):
            await self.bridge.menu("file.new")
            yield event("ui_menu_invoke", "菜单打开新建画布弹窗", {"id": "file.new"}, "dialog")
            await self._pause(0.6)

        state, dialog = await self._wait_dialog("NewDocument")
        dialog_id = dialog["id"]
        yield event("ui_inspect", "读取新建画布弹窗", {"dialog": dialog_id}, {
            "title": dialog.get("title"), "fields": dialog.get("fields"),
        })
        await self._pause(0.35)

        # 3. Fill the dialog fields; the values appear in the dialog itself.
        for field, value in (("width", req["width"]), ("height", req["height"]), ("name", "招新海报")):
            await self.bridge.dialog_set(dialog_id, field, value)
            yield event("ui.dialog.set", f"填写画布{field}", {"dialog": dialog_id, "field": field, "value": value}, "set")
            await self._pause(0.25)

        yield {"type": "status", "text": f"正在创建 {req['width']} × {req['height']} 画布…"}
        await self.bridge.move(*create_pos)
        await self.bridge.click(*create_pos)
        yield event("ui.click", "点击 Create 创建画布", {"x": create_pos[0], "y": create_pos[1]}, "created", create_pos)
        await self._pause(0.8)

        state = await self._inspect()
        if self._first_dialog(state, "NewDocument"):
            await self.bridge.dialog_confirm(dialog_id)
            yield event("ui.dialog.confirm", "确认创建画布（弹窗按钮）", {"dialog": dialog_id}, "created")
        await self._pause(1.0)

        state = await self._inspect()
        if not state.get("document"):
            raise BridgeError("画布没有创建成功，请检查 New Document 弹窗")
        yield event("ui_inspect", "确认画布已经创建", {}, {
            "name": state["document"].get("name"),
            "width": state["document"].get("width"),
            "height": state["document"].get("height"),
            "layers": [layer.get("name") for layer in state["document"].get("layers") or []],
        })

        # 4. The visible toolbar: move to the bucket icon, click, and make the
        #    selection explicit (same result if the click already selected it).
        yield {"type": "status", "text": "正在选中填充工具（Paint Bucket）…"}
        await self.bridge.move(*bucket_pos)
        yield event("ui.move", "鼠标移到工具栏填充工具", {"x": bucket_pos[0], "y": bucket_pos[1]}, "moved", bucket_pos)
        await self._pause(0.3)
        await self.bridge.click(*bucket_pos)
        await self.bridge.set_tool("PaintBucket")
        yield event("ui_set", "选中填充工具 PaintBucket", {"tool": "PaintBucket"}, "tool selected", bucket_pos)
        await self._pause(0.7)

        if not req["wants_fill"]:
            state = await self._inspect()
            yield {"type": "done", "reply": f"画布已创建：{req['width']} × {req['height']}。", "document": state.get("document")}
            return

        # 5. Layer → New Layer, so the Layers panel gains a row before filling.
        yield {"type": "status", "text": "正在打开 Layer → New Layer…"}
        await self.bridge.move(*layer_pos)
        await self.bridge.click(*layer_pos)
        yield event("ui.click", "点击 Layer，展开菜单", {"x": layer_pos[0], "y": layer_pos[1]}, "menu open", layer_pos)
        await self._pause(0.7)
        await self.bridge.menu("layer.new.layer")
        yield event("ui_menu_invoke", "选择 Layer → New → Layer…", {"id": "layer.new.layer"}, "dialog")
        await self._pause(0.7)

        state, dialog = await self._wait_dialog("Command")
        layer_dialog_id = dialog["id"]
        await self.bridge.dialog_set(layer_dialog_id, "name", "Color Fill 1")
        yield event("ui.dialog.set", "把新图层命名为 Color Fill 1", {
            "dialog": layer_dialog_id, "field": "name", "value": "Color Fill 1",
        }, "set")
        await self._pause(0.4)
        await self.bridge.move(*ok_pos)
        await self.bridge.click(*ok_pos)
        yield event("ui.click", "点击 OK 创建图层", {"x": ok_pos[0], "y": ok_pos[1]}, "layer created", ok_pos)
        await self._pause(0.7)
        state = await self._inspect()
        if self._first_dialog(state, "Command"):
            await self.bridge.dialog_confirm(layer_dialog_id)
            yield event("ui.dialog.confirm", "确认创建图层", {"dialog": layer_dialog_id}, "layer created")
        await self.bridge.key("Escape")
        await self._pause(0.6)

        # 6. Set the foreground colour; the Color panel visibly changes.
        yield {"type": "status", "text": f"正在把前景色设为 {req['color']}…"}
        await self.bridge.command("tools.setColors", {"foreground": req["color"]})
        yield event("ui.run_command", "设置前景色", {"id": "tools.setColors", "params": {"foreground": req["color"]}}, req["color"])
        await self._pause(0.6)

        # 7. Click the canvas with the bucket: the layer fills with colour.
        canvas = state.get("document") or {}
        cx = round(float(canvas.get("width") or req["width"]) / 2)
        cy = round(float(canvas.get("height") or req["height"]) / 2)
        yield {"type": "status", "text": "正在用填充工具点击画布…"}
        await self.bridge.pointer([
            {"kind": "move", "x": cx, "y": cy},
            {"kind": "down", "x": cx, "y": cy},
            {"kind": "up", "x": cx, "y": cy},
        ])
        yield event("ui_pointer", "用填充工具点击画布中心", {"x": cx, "y": cy}, "filled")
        await self._pause(1.0)

        state = await self._inspect()
        layers = [layer.get("name") for layer in (state.get("document") or {}).get("layers") or []]
        yield event("ui_inspect", "检查最终图层与画布", {}, {
            "width": (state.get("document") or {}).get("width"),
            "height": (state.get("document") or {}).get("height"),
            "layers": layers,
        })
        yield {
            "type": "done",
            "reply": f"完成：{req['width']} × {req['height']} 画布，已用填充工具填入 {req['color']}。",
            "document": state.get("document"),
            "layers": layers,
        }
