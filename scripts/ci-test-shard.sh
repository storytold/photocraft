#!/usr/bin/env bash
# One shard of the gate's tests: the gate's `shard-command` (.github/workflows/ci.yml). The gate's
# build job wrote a nextest archive of the tests with `cargo xtask ci archive`, and the archive
# carries that xtask binary (.config/nextest.toml), so the shard extracts it and runs the test steps
# with `xtask ci tests` without building anything. The gate sets NEXTEST_ARCHIVE_FILE, GATE_SHARD,
# GATE_SHARDS and NEXTEST_PARTITION.
#
#   ci-test-shard.sh              the shard runs its hash partition of every test
#   ci-test-shard.sh --gpu-shard  shard 1 runs nextest's `gpu` group alone on lavapipe, checked by
#                                 scripts/ci-gpu-lavapipe.py; the other shards split the other tests
set -euo pipefail

archive=target/nextest-archive
mkdir -p "$archive"
cargo nextest run --archive-file "$NEXTEST_ARCHIVE_FILE" --workspace-remap . --extract-to "$archive" --extract-overwrite -E 'none()' --no-tests=pass
xtask=("$archive/target/debug/xtask" ci tests --from "$archive")
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) xtask=("$archive/target/debug/xtask.exe" ci tests --from "$archive") ;; esac

case "${1:-}" in
  "") exec "${xtask[@]}" ;;
  --gpu-shard)
    if [ "$GATE_SHARDS" -lt 2 ]; then
      echo "::error::--gpu-shard needs at least two shards, got $GATE_SHARDS"
      exit 1
    fi
    if [ "$GATE_SHARD" = 1 ]; then
      exec python3 scripts/ci-gpu-lavapipe.py "${xtask[@]}" --gpu
    fi
    NEXTEST_PARTITION="hash:$((GATE_SHARD - 1))/$((GATE_SHARDS - 1))" exec "${xtask[@]}" --no-gpu
    ;;
  *)
    echo "usage: $0 [--gpu-shard]" >&2
    exit 2
    ;;
esac
