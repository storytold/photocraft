#!/usr/bin/env bash
# Temporary (draft PR, never merged): the Windows real-app diagnostic for #15549, #15556 and
# #15870, run by the Windows gate's build job in place of its after-archive command. Its result
# never fails the job; the doctests then run as they do on every other branch.
set -uo pipefail
exe="${CARGO_TARGET_DIR:-target}/debug/photocraft.exe"
echo "DIAG: start, exe $exe"
timeout 900 python scripts/win-diag.py "$exe" || echo "DIAG: the script ended with status $?"
set -e
cargo xtask-gate doctest
