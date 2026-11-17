# SPDX-License-Identifier: MIT OR Apache-2.0
"""Persisted PhotoCraftAI provider settings (API key never leaves the machine)."""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

BASE_DIR = Path(__file__).resolve().parent
SETTINGS_PATH = BASE_DIR / "runtime" / "ai_settings.json"

PROVIDERS: dict[str, dict[str, str]] = {
    "deepseek": {
        "label": "DeepSeek",
        "base_url": "https://api.deepseek.com",
        "model": "deepseek-chat",
        "env": "DEEPSEEK_API_KEY",
    },
    "qwen": {
        "label": "Qwen / 通义千问",
        "base_url": "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "model": "qwen-plus",
        "env": "DASHSCOPE_API_KEY",
    },
    "openai": {
        "label": "OpenAI",
        "base_url": "https://api.openai.com/v1",
        "model": "gpt-4o-mini",
        "env": "OPENAI_API_KEY",
    },
}


def _env_key(provider: str) -> str:
    env = PROVIDERS.get(provider, {}).get("env", "")
    return os.getenv(env, "").strip() if env else ""


def load() -> dict[str, Any]:
    provider = (os.getenv("LLM_PROVIDER") or "deepseek").strip().lower()
    if provider not in PROVIDERS:
        provider = "deepseek"
    data: dict[str, Any] = {
        "provider": provider,
        "model": (os.getenv("LLM_MODEL") or PROVIDERS[provider]["model"]).strip(),
        "base_url": (os.getenv("LLM_BASE_URL") or PROVIDERS[provider]["base_url"]).strip(),
        "api_key": _env_key(provider),
    }
    try:
        saved = json.loads(SETTINGS_PATH.read_text(encoding="utf-8"))
        if isinstance(saved, dict):
            data.update({k: v for k, v in saved.items() if k in data})
    except (OSError, json.JSONDecodeError):
        pass
    return data


def save(data: dict[str, Any]) -> dict[str, Any]:
    provider = str(data.get("provider") or "deepseek").strip().lower()
    if provider not in PROVIDERS:
        raise ValueError(f"不支持的 provider：{provider}")
    current = load()
    key = str(data.get("api_key") or "").strip() or current.get("api_key", "")
    model = str(data.get("model") or PROVIDERS[provider]["model"]).strip()
    base_url = str(data.get("base_url") or PROVIDERS[provider]["base_url"]).strip()
    stored = {"provider": provider, "api_key": key, "model": model, "base_url": base_url}
    SETTINGS_PATH.parent.mkdir(parents=True, exist_ok=True)
    SETTINGS_PATH.write_text(json.dumps(stored, ensure_ascii=False, indent=2), encoding="utf-8")
    try:
        SETTINGS_PATH.chmod(0o600)
    except OSError:
        pass
    return public(stored)


def public(data: dict[str, Any]) -> dict[str, Any]:
    key = str(data.get("api_key") or "")
    masked = ""
    if key:
        masked = f"{key[:4]}…{key[-4:]}" if len(key) > 8 else "已保存"
    return {
        "provider": data.get("provider", "deepseek"),
        "model": data.get("model", ""),
        "base_url": data.get("base_url", ""),
        "has_key": bool(key),
        "masked_key": masked,
        "providers": [
            {"id": pid, "label": meta["label"], "model": meta["model"], "base_url": meta["base_url"]}
            for pid, meta in PROVIDERS.items()
        ],
    }
