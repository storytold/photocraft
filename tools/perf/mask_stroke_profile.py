#!/usr/bin/env python3
"""Profile real Brush-tool layer-mask strokes through PhotoCraft's local control channel.

Runs an isolated PhotoCraft process with a temporary config directory, creates a 6000x4000
document, and compares strokes into a pixel layer mask with strokes into a plain raster layer. No
documents or preferences are read from or written to the user's normal profile.

Example:
  python tools/perf/mask_stroke_profile.py "C:/Program Files/PhotoCraft/photocraft.exe"
  python tools/perf/mask_stroke_profile.py target/release/photocraft.exe --runs 30 --moves 32
"""
from __future__ import annotations

import argparse
import json
import os
import re
import socket
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any


SIDE = (6000, 4000)  # 24 MP


class Control:
    def __init__(self, port: int, token: str) -> None:
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=8)
        self.sock.settimeout(65)
        self.reader = self.sock.makefile("r", encoding="utf-8", newline="\n")
        self.writer = self.sock.makefile("w", encoding="utf-8", newline="\n")
        self.request("auth", {"token": token}, auth=True)
        self.next_id = 0

    def request(self, method: str, params: dict[str, Any], *, auth: bool = False) -> Any:
        self.next_id = getattr(self, "next_id", 0) + 1
        request_id = "auth" if auth else self.next_id
        self.writer.write(json.dumps({"id": request_id, "method": method, "params": params}) + "\n")
        self.writer.flush()
        line = self.reader.readline()
        if not line:
            raise RuntimeError(f"PhotoCraft closed control connection during {method}")
        reply = json.loads(line)
        if not reply.get("ok"):
            raise RuntimeError(f"{method} failed: {reply.get('error', reply)}")
        return reply.get("result")

    def command(self, command: str, params: dict[str, Any] | None = None) -> Any:
        return self.request("engine.execute", {"command": command, "params": params or {}})

    def inspect(self) -> dict[str, Any]:
        return self.request("ui.inspect", {})

    def close(self) -> None:
        try:
            self.reader.close()
            self.writer.close()
            self.sock.close()
        except OSError:
            pass


def wait_for_control(log_path: Path, process: subprocess.Popen[str], timeout: float = 40) -> tuple[int, str]:
    deadline = time.monotonic() + timeout
    pattern = re.compile(r"control server listening on 127\.0\.0\.1:(\d+)")
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"PhotoCraft exited with {process.returncode}; log: {log_path.read_text(errors='replace')}")
        try:
            log = log_path.read_text(errors="replace")
        except OSError:
            log = ""
        match = pattern.search(log)
        if match:
            return int(match.group(1)), log
        time.sleep(0.1)
    raise TimeoutError(f"Timed out waiting for control server; log: {log_path.read_text(errors='replace')}")


def percentile(values: list[float], p: float) -> float:
    ordered = sorted(values)
    if not ordered:
        return 0.0
    return ordered[round((len(ordered) - 1) * p)]


def make_stroke(control: Control, x: int, y: int, moves: int, previous: Any) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    refreshes = []
    control.request("ui.pointer", {"events": [{"kind": "down", "x": x, "y": y, "pressure": 0.85}]})
    for step in range(1, moves + 1):
        control.request("ui.pointer", {"events": [{"kind": "move", "x": x + step * 3, "y": y + (step % 4), "pressure": 0.85}]})
        # Match a display frame cadence. The control channel's pointer events still pass through
        # the real live-stroke, canvas refresh, and release/commit path.
        time.sleep(1 / 60)
        timings = control.inspect().get("perf", {}).get("timings", {})
        if timings.get("lastRefreshPx", 0) > 0:
            refreshes.append({key: timings.get(key) for key in ("lastRefresh", "lastRefreshPx", "gpuUploadBytes", "gpuPasses", "gpuFallback")})
    control.request("ui.pointer", {"events": [{"kind": "up", "x": x + moves * 3, "y": y + (moves % 4), "pressure": 0.85}]})
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        snapshot = control.inspect()
        summary = snapshot.get("perf", {}).get("strokeTrace")
        if summary and summary.get("frames", 0) > 0 and summary != previous:
            return snapshot, refreshes
        time.sleep(0.04)
    raise TimeoutError(f"no new strokeTrace after the stroke at ({x}, {y}); the trace channel may be disabled")


def summarize(kind: str, snapshots: list[dict[str, Any]], refreshes: list[list[dict[str, Any]]], warmup: int) -> dict[str, Any]:
    rows = snapshots[warmup:]
    def samples_for(stage: str, quantile: int) -> list[float]:
        return [float(s["perf"]["strokeTrace"][stage][quantile]) for s in rows if s.get("perf", {}).get("strokeTrace")]

    samples = {stage: (samples_for(stage, 0), samples_for(stage, 1)) for stage in ("stroke", "canvas", "ui", "cpu", "wait")}
    refresh_rows = [r for stroke_rows in refreshes[warmup:] for r in stroke_rows]
    refresh_metrics = {}
    for key in ("lastRefreshPx", "gpuUploadBytes", "gpuPasses"):
        values = [float(r[key]) for r in refresh_rows if isinstance(r.get(key), (int, float))]
        refresh_metrics[key] = {
            "p50": round(statistics.median(values), 2) if values else None,
            "p95": round(percentile(values, 0.95), 2) if values else None,
        }
    last = rows[-1] if rows else snapshots[-1]
    timings = last.get("perf", {}).get("timings", {})
    return {
        "target": kind,
        "measured_strokes": len(rows),
        "p50_ms": {stage: round(statistics.median(values[0]), 3) if values[0] else None for stage, values in samples.items()},
        "p95_ms": {stage: round(percentile(values[1], 0.95), 3) if values[1] else None for stage, values in samples.items()},
        "refresh_samples": len(refresh_rows),
        "refresh_metrics": refresh_metrics,
        "refresh_kinds": sorted({str(r["lastRefresh"]) for r in refresh_rows if r.get("lastRefresh")}),
        "refresh_fallbacks": sorted({str(r["gpuFallback"]) for r in refresh_rows if r.get("gpuFallback")}),
        "last_refresh": timings.get("lastRefresh"),
        "last_refresh_pixels": timings.get("lastRefreshPx"),
        "gpu": timings.get("gpu"),
        "gpu_fallback": timings.get("gpuFallback"),
        "gpu_upload_bytes": timings.get("gpuUploadBytes"),
        "gpu_passes": timings.get("gpuPasses"),
        "adapter": timings.get("gpuInfo", {}).get("adapter"),
        "backend": timings.get("gpuInfo", {}).get("backend"),
    }


def run_case(control: Control, layer: int, kind: str, runs: int, moves: int, warmup: int) -> dict[str, Any]:
    control.command("layer.select", {"layer": layer, "mode": "replace"})
    mask = kind == "mask"
    control.request("ui.set", {"tool": "Brush", "maskTarget": mask, "brushSize": 36})
    control.command("tools.setBrush", {"brush": {"size": 36, "hardness": 0.8, "opacity": 1.0, "flow": 1.0}})
    snapshots = []
    refreshes = []
    for index in range(runs + warmup):
        # Keep each gesture in a new 48 px band; the 24 MP document stays otherwise sparse.
        y = 300 + (index * 48) % 3000
        x = 500 + (index % 10) * 350
        prior = control.inspect().get("perf", {}).get("strokeTrace")
        snapshot, current_refreshes = make_stroke(control, x, y, moves, prior)
        snapshots.append(snapshot)
        refreshes.append(current_refreshes)
    return summarize(kind, snapshots, refreshes, warmup)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path, help="installed or source PhotoCraft executable")
    parser.add_argument("--runs", type=int, default=20, help="measured strokes per target (default: 20)")
    parser.add_argument("--moves", type=int, default=32, help="moves per stroke (default: 32)")
    parser.add_argument("--warmup", type=int, default=4, help="warm-up strokes per target (default: 4)")
    args = parser.parse_args()
    executable = args.executable.resolve()
    if not executable.is_file():
        parser.error(f"executable does not exist: {executable}")
    if args.runs < 1 or args.moves < 2 or args.warmup < 0:
        parser.error("runs must be positive, moves at least 2, warmup nonnegative")

    with tempfile.TemporaryDirectory(prefix="photocraft-mask-perf-") as temp:
        root = Path(temp)
        config = root / "config"
        config.mkdir()
        token_file = root / "control.token"
        log_path = root / "photocraft.log"
        trace_path = root / "stroke-trace.csv"
        env = os.environ.copy()
        env["PHOTOCRAFT_CONFIG_DIR"] = str(config)
        env["PHOTOCRAFT_NEW_INSTANCE"] = "1"
        with log_path.open("w", encoding="utf-8") as log:
            process = subprocess.Popen(
                [str(executable), "--new-instance", "--control", "0", "--control-token-file", str(token_file),
                 "--automation-write-root", str(root), "--stroke-trace", str(trace_path)],
                cwd=str(root), env=env, stdout=log, stderr=subprocess.STDOUT, text=True,
            )
            control: Control | None = None
            try:
                port, _ = wait_for_control(log_path, process)
                token_deadline = time.monotonic() + 5
                while not token_file.exists() and time.monotonic() < token_deadline:
                    time.sleep(0.05)
                token = token_file.read_text(encoding="utf-8").strip()
                control = Control(port, token)
                control.command("file.new", {"width": SIDE[0], "height": SIDE[1], "background": "transparent"})
                mask_layer = control.command("layer.new.layer", {"name": "Mask Paint target"})["layer"]
                control.command("layer.layerMask.hideAll", {})
                pixel_layer = control.command("layer.new.layer", {"name": "Pixel Paint target"})["layer"]
                # Set trace through the control channel as well for versions that ignore the flag.
                control.request("ui.set", {"strokeTrace": True})
                initial = control.inspect().get("perf", {}).get("timings", {})
                results = [
                    run_case(control, mask_layer, "mask", args.runs, args.moves, args.warmup),
                    run_case(control, pixel_layer, "pixels", args.runs, args.moves, args.warmup),
                ]
                print(json.dumps({
                    "executable": str(executable), "version": initial.get("version"), "document": list(SIDE),
                    "moves_per_stroke": args.moves, "warmup": args.warmup, "results": results,
                }, indent=2))
            finally:
                if control:
                    control.close()
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, TimeoutError, json.JSONDecodeError) as error:
        print(f"mask stroke profile failed: {error}", file=sys.stderr)
        raise SystemExit(1)
