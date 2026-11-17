# SPDX-License-Identifier: MIT OR Apache-2.0
"""Optional LLM intent parsing for PhotoCraftAI.

The visible automation itself is deterministic; the configured model turns the
user's sentence into dimensions/colour so arbitrary wording works.
"""

from __future__ import annotations

import json
import re
from typing import Any

from openai import AsyncOpenAI

import settings_store

PROMPT = """你是 PhotoCraft 的意图解析器。只输出一个 JSON 对象，不要 Markdown：
{"width": 整数, "height": 整数, "color": "#rrggbb", "wants_fill": true/false, "poster": true/false}
规则：未给尺寸时，海报用 1080x1920，其他用 1920x1080；未给颜色用 #0000ff；
提到填充/涂色/变成某色时 wants_fill=true。"""


def fallback(message: str) -> dict[str, Any]:
    text = message.strip()
    match = re.search(r"(\d{2,5})\s*[x×X*]\s*(\d{2,5})", text)
    if match:
        width, height = int(match.group(1)), int(match.group(2))
    elif "海报" in text or "竖" in text:
        width, height = 1080, 1920
    else:
        width, height = 1920, 1080
    colors = {"蓝": "#0000ff", "红": "#ff0000", "绿": "#00ff00", "黑": "#000000", "白": "#ffffff"}
    color = next((v for k, v in colors.items() if k in text), None)
    hex_match = re.search(r"#[0-9a-fA-F]{6}", text)
    if hex_match:
        color = hex_match.group(0).lower()
    wants_fill = color is not None or any(w in text for w in ("填充", "涂", "fill"))
    return {
        "width": width,
        "height": height,
        "color": color or "#0000ff",
        "wants_fill": wants_fill,
        "poster": "海报" in text,
    }


async def plan(message: str) -> tuple[dict[str, Any], str]:
    """Return (plan, source). Invalid/missing keys fall back to local parsing."""
    config = settings_store.load()
    key = str(config.get("api_key") or "")
    if not key:
        return fallback(message), "local"
    client = AsyncOpenAI(
        api_key=key,
        base_url=config.get("base_url") or None,
        timeout=20.0,
        max_retries=1,
    )
    try:
        response = await client.chat.completions.create(
            model=config.get("model") or "deepseek-chat",
            messages=[
                {"role": "system", "content": PROMPT},
                {"role": "user", "content": message},
            ],
            temperature=0.0,
        )
        content = (response.choices[0].message.content or "").strip()
        match = re.search(r"\{.*\}", content, re.S)
        data = json.loads(match.group(0) if match else content)
        base = fallback(message)
        for field in ("width", "height", "color", "wants_fill", "poster"):
            if field in data:
                base[field] = data[field]
        base["width"] = max(1, min(300000, int(base["width"])))
        base["height"] = max(1, min(300000, int(base["height"])))
        if not re.fullmatch(r"#[0-9a-fA-F]{6}", str(base["color"])):
            base["color"] = "#0000ff"
        return base, "llm"
    except Exception:  # noqa: BLE001 - invalid key must not break the UI
        return fallback(message), "local-fallback"
