#!/usr/bin/env nix-shell
#!nix-shell -i python3 -p python3
"""Check the eframe feature boundary without compiling or needing an Android device."""
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
cases = [
    ("photocraft-android", "aarch64-linux-android", False),
    ("photocraft", "x86_64-unknown-linux-gnu", True),
    ("photocraft-web", "wasm32-unknown-unknown", True),
]
for package, target, expected in cases:
    result = subprocess.run(
        [
            "nix", "shell", "--inputs-from", ".", "nixpkgs#cargo", "nixpkgs#rustc",
            "-c", "cargo", "tree", "--locked", "-p", package,
            "--target", target, "-e", "features",
        ],
        cwd=root, check=True, text=True, stdout=subprocess.PIPE,
    )
    present = 'eframe feature "accesskit"' in result.stdout
    if present != expected:
        raise SystemExit(f"FAIL: {target}: eframe/accesskit={present}, expected {expected}")
    print(f"PASS: {target}: eframe/accesskit={present}", flush=True)
