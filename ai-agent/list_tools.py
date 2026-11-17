#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""List the real tools exposed by PhotoCraft's MCP server.

The script starts ``photocraft-cli mcp`` over stdio (the only transport the
upstream server implements), performs the MCP handshake, calls ``tools/list``
and prints whatever the server returns.  Nothing here is hard-coded: if the
server exposes a different set of tools, the output changes accordingly.

Usage
-----
    python ai-agent/list_tools.py
    python ai-agent/list_tools.py --cli ./target/release/photocraft-cli
    python ai-agent/list_tools.py --read-root ./photocraft-work \
                                 --write-root ./photocraft-work --smoke
    python ai-agent/list_tools.py --json > tools.json

The CLI is looked up in this order:
    1. ``--cli``
    2. ``$PHOTOCRAFT_CLI``
    3. ``<repo>/target/release/photocraft-cli[.exe]``
    4. ``<repo>/target/debug/photocraft-cli[.exe]``
    5. ``photocraft-cli`` on ``$PATH``
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import shutil
import sys
import textwrap
from pathlib import Path
from typing import Any

try:
    from mcp import ClientSession, StdioServerParameters
    from mcp.client.stdio import stdio_client
except ImportError as exc:  # pragma: no cover - dependency guard
    sys.exit(
        "error: the official MCP Python SDK is missing.\n"
        "  python -m venv ai-agent/.venv\n"
        "  ai-agent/.venv/bin/pip install -r ai-agent/requirements.txt\n"
        f"  (import error: {exc})"
    )

REPO_ROOT = Path(__file__).resolve().parent.parent
IS_WINDOWS = os.name == "nt"
EXE = ".exe" if IS_WINDOWS else ""


def find_cli(explicit: str | None) -> Path:
    """Return the photocraft-cli path, or exit with a helpful message."""
    candidates: list[Path] = []
    if explicit:
        candidates.append(Path(explicit).expanduser())
    if os.environ.get("PHOTOCRAFT_CLI"):
        candidates.append(Path(os.environ["PHOTOCRAFT_CLI"]).expanduser())
    candidates.append(REPO_ROOT / "target" / "release" / f"photocraft-cli{EXE}")
    candidates.append(REPO_ROOT / "target" / "debug" / f"photocraft-cli{EXE}")

    for path in candidates:
        if path.is_file():
            return path.resolve()

    on_path = shutil.which("photocraft-cli")
    if on_path:
        return Path(on_path).resolve()

    sys.exit(
        "error: photocraft-cli was not found.\n"
        "Build it first:\n"
        "  cargo build --release -p photocraft-cli\n"
        "or pass --cli /path/to/photocraft-cli"
    )


def describe_type(schema: dict[str, Any] | None) -> str:
    """Render a compact, human-readable JSON-Schema type."""
    if not isinstance(schema, dict) or not schema:
        return "any"
    if "$ref" in schema:
        return str(schema["$ref"]).rsplit("/", 1)[-1]
    if "anyOf" in schema:
        parts = [describe_type(s) for s in schema["anyOf"] if s != {"type": "null"}]
        if {"type": "null"} in schema["anyOf"]:
            parts.append("null")
        return " | ".join(dict.fromkeys(parts)) or "any"
    if "oneOf" in schema:
        return " | ".join(describe_type(s) for s in schema["oneOf"])
    if "const" in schema:
        return json.dumps(schema["const"])
    if "enum" in schema:
        return " | ".join(json.dumps(v) for v in schema["enum"])

    kind = schema.get("type")
    if isinstance(kind, list):  # JSON Schema 2020-12: ["boolean", "null"]
        return " | ".join(str(k) for k in kind)
    if kind == "array":
        return f"array<{describe_type(schema.get('items'))}>"
    if kind == "object":
        props = schema.get("properties")
        if props:
            return "object{" + ", ".join(props) + "}"
        extra = schema.get("additionalProperties")
        if isinstance(extra, dict):
            return f"object<string, {describe_type(extra)}>"
        return "object"
    return str(kind) if kind else "any"


def tool_schema(tool: Any) -> dict[str, Any]:
    """The SDK exposes input_schema (older releases used inputSchema)."""
    schema = getattr(tool, "inputSchema", None)
    if schema is None:
        schema = getattr(tool, "input_schema", None)
    return schema or {}


def print_tool(index: int, tool: Any) -> None:
    schema = tool_schema(tool)
    required = set(schema.get("required") or [])
    props: dict[str, Any] = schema.get("properties") or {}
    description = (tool.description or "").strip()

    print(f"\n[{index:02d}] {tool.name}")
    if description:
        for line in textwrap.wrap(description, width=96):
            print(f"     {line}")

    if not props:
        print("     params: (none)")
        return

    print("     params:")
    for name, prop in props.items():
        prop = prop if isinstance(prop, dict) else {}
        bits = [describe_type(prop)]
        if name in required:
            bits.append("required")
        else:
            bits.append("optional")
        default = prop.get("default", None)
        if "default" in prop and default is not None:
            bits.append(f"default={json.dumps(default)}")
        doc = (prop.get("description") or "").strip().replace("\n", " ")
        print(f"       - {name}: {', '.join(bits)}")
        if doc:
            for line in textwrap.wrap(doc, width=92):
                print(f"         {line}")


async def run(args: argparse.Namespace) -> int:
    cli = find_cli(args.cli)
    if not os.access(cli, os.X_OK):
        sys.exit(f"error: {cli} is not executable (chmod +x it, or rebuild)")

    server_args = ["mcp"]
    for root in args.read_root:
        server_args += ["--automation-read-root", str(Path(root).expanduser().resolve())]
    if args.write_root:
        server_args += ["--automation-write-root", str(Path(args.write_root).expanduser().resolve())]
    if args.gpu:
        server_args.append("--gpu")
    if args.no_gpu:
        server_args.append("--no-gpu")

    params = StdioServerParameters(command=str(cli), args=server_args, env=None)
    print(f"PhotoCraft CLI : {cli}", file=sys.stderr)
    print(f"MCP command    : {cli} {' '.join(server_args)}", file=sys.stderr)

    async with stdio_client(params) as (read, write):
        async with ClientSession(read, write) as session:
            async with asyncio.timeout(args.timeout):
                init = await session.initialize()
                result = await session.list_tools()

            # The SDK uses snake_case attributes; keep the camelCase fallback for
            # clients pinned to an older release.
            info = getattr(init, "server_info", None) or getattr(init, "serverInfo", None)
            protocol = getattr(init, "protocol_version", None) or getattr(
                init, "protocolVersion", "?"
            )
            if info is not None:
                print(
                    f"Connected to   : {info.name} {info.version} "
                    f"(MCP protocol {protocol})",
                    file=sys.stderr,
                )

            tools = list(result.tools)
            if args.json:
                payload = [
                    {
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": tool_schema(t),
                    }
                    for t in tools
                ]
                print(json.dumps(payload, indent=2, ensure_ascii=False))
            else:
                print(f"Tools exposed  : {len(tools)}")
                for i, tool in enumerate(tools, 1):
                    print_tool(i, tool)

            if args.smoke:
                smoke = await session.call_tool("session_list", {})
                print("\n--- smoke test: session_list ---")
                for block in smoke.content:
                    text = getattr(block, "text", None)
                    print(text if text is not None else block)

    return 0


def iter_exceptions(exc: BaseException):
    """Flatten nested ExceptionGroups (anyio wraps errors from the session body)."""
    if isinstance(exc, BaseExceptionGroup):
        for inner in exc.exceptions:
            yield from iter_exceptions(inner)
    else:
        yield exc


def main() -> int:
    parser = argparse.ArgumentParser(
        description="List the real tools of the PhotoCraft MCP server (stdio)."
    )
    parser.add_argument("--cli", help="path to photocraft-cli (default: auto-detect)")
    parser.add_argument(
        "--read-root",
        action="append",
        default=[],
        metavar="DIR",
        help="grant headless read access to DIR (repeatable)",
    )
    parser.add_argument(
        "--write-root",
        metavar="DIR",
        help="grant headless write access to DIR",
    )
    gpu = parser.add_mutually_exclusive_group()
    gpu.add_argument("--gpu", action="store_true", help="enable optional GPU rendering")
    gpu.add_argument("--no-gpu", action="store_true", help="force CPU rendering")
    parser.add_argument("--json", action="store_true", help="print the raw tool list as JSON")
    parser.add_argument(
        "--smoke",
        action="store_true",
        help="also call session_list to prove tool execution works",
    )
    parser.add_argument(
        "--timeout", type=float, default=60.0, help="handshake/list timeout in seconds"
    )
    args = parser.parse_args()

    try:
        return asyncio.run(run(args))
    except TimeoutError:
        print(
            "error: timed out waiting for the MCP server.\n"
            "  Is another process holding the binary? Does it start at all?\n"
            f"  Try: {find_cli(args.cli)} mcp --help",
            file=sys.stderr,
        )
        return 2
    except BaseExceptionGroup as group:  # anyio task groups wrap startup errors
        print("error: the MCP session failed (server startup or tool listing).", file=sys.stderr)
        for exc in iter_exceptions(group):
            print(f"  {type(exc).__name__}: {exc}", file=sys.stderr)
        print(
            "  Check that the binary was built and runs:\n"
            "    target/release/photocraft-cli --version\n"
            "    target/release/photocraft-cli mcp --help",
            file=sys.stderr,
        )
        return 2
    except Exception as exc:  # noqa: BLE001 - CLI, report cleanly
        print(f"error: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
