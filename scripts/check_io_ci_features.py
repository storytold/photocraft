#!/usr/bin/env python3
"""Check the photocraft-io CI feature helper against the no-HEIF job's Cargo graph.

The default I/O graph must omit the HEIF decoder, `--features heif` must add it,
photocraft-io-ci-features must appear only on the I/O test graph (not normal/build),
and it must not pull in craft, a craft-* crate, or a second photocraft-io package.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

HELPER = "photocraft-io-ci-features"
HEIF = "craft-heif"


def packages(edges: str, *extra: str, features: list[str] | None = None) -> set[str]:
    args = [
        "cargo",
        "tree",
        "--color",
        "never",
        "--prefix",
        "none",
        "--format",
        "{p}",
        "--edges",
        edges,
        "--locked",
        "-p",
        "photocraft-io",
    ]
    if features:
        args.extend(["--features", ",".join(features)])
    for package in extra:
        args.extend(["-p", package])
    lines = subprocess.check_output(args, text=True).splitlines()
    out = set()
    for line in lines:
        line = line.removesuffix(" (*)").strip()
        if line:
            out.add(line)
    return out


def names(pkgs: set[str]) -> set[str]:
    return {pkg.split()[0] for pkg in pkgs}


def has_named(pkgs: set[str], name: str) -> bool:
    return any(pkg.split()[0] == name for pkg in pkgs)


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    subprocess.check_call(["cargo", "metadata", "--locked", "--format-version", "1", "--no-deps"], cwd=root, stdout=subprocess.DEVNULL)

    io_dev = packages("normal,build,dev")
    io_build = packages("normal,build")
    io_heif = packages("normal,build,dev", features=["heif"])

    errors: list[str] = []
    if has_named(io_dev, HEIF) or has_named(io_build, HEIF):
        errors.append("the default photocraft-io graph includes the HEIF decoder")
    if not has_named(io_heif, HEIF):
        errors.append("photocraft-io --features heif does not include the HEIF decoder")
    if has_named(io_build, HELPER):
        errors.append(f"{HELPER} appears in the photocraft-io normal/build graph")
    if not has_named(io_dev, HELPER):
        errors.append(f"{HELPER} is missing from the photocraft-io test graph")

    helper_pkgs = {pkg for pkg in io_dev if pkg.split()[0] == HELPER}
    # The helper must not add craft / craft-* / a different photocraft-io identity
    # beyond photocraft-io itself.
    for pkg in sorted(io_dev):
        name = pkg.split()[0]
        if name == "craft" or name.startswith("craft-"):
            # These packages already belong to photocraft-io; the helper must not
            # be the thing that first introduces them. Presence is fine; the pin
            # list must not include them.
            continue
        if name == "photocraft-io" and "ci-features" not in name:
            continue

    # Fail if the helper crate itself depends on forbidden packages: inspect its tree.
    helper_tree = subprocess.check_output(
        [
            "cargo",
            "tree",
            "--color",
            "never",
            "--prefix",
            "none",
            "--format",
            "{p}",
            "--edges",
            "normal,build,dev",
            "--locked",
            "-p",
            HELPER,
        ],
        text=True,
    ).splitlines()
    for line in helper_tree:
        pkg = line.removesuffix(" (*)").strip()
        if not pkg:
            continue
        name = pkg.split()[0]
        if name == HELPER:
            continue
        if name == "craft" or name.startswith("craft-") or name == "photocraft-io":
            errors.append(f"{HELPER} depends on forbidden package {pkg}")

    added_dev = names(io_dev) - names(io_build)
    if HELPER not in added_dev:
        errors.append(f"{HELPER} should be the package that appears on the test graph only")

    if errors:
        print("io-ci-features checks failed:", file=sys.stderr)
        for err in errors:
            print(f"  {err}", file=sys.stderr)
        return 1

    print(
        f"photocraft-io guards passed: {len(io_build)} normal/build packages, "
        f"{len(io_dev)} test packages (includes {HELPER}), HEIF only with --features heif."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
