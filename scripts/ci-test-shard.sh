#!/usr/bin/env bash
# One shard of the gate's tests: the gate's `shard-command` (.github/workflows/ci.yml). Each shard
# restores the dependency cache, builds the tests (`cargo xtask ci build` builds the same on main's
# cache warm-up leg) and runs its share with `cargo xtask ci tests`. The gate sets GATE_SHARD (1..N),
# GATE_SHARDS (N) and NEXTEST_PARTITION (hash:GATE_SHARD/GATE_SHARDS).
#
#   ci-test-shard.sh              each shard runs its hash partition of every test; shard 1 also
#                                 runs the doctests
#   ci-test-shard.sh --gpu-shard  shard 1 runs nextest's `gpu` group alone on lavapipe, checked by
#                                 scripts/ci-gpu-lavapipe.py; the other shards split the other tests,
#                                 and shard 2 also runs the doctests
set -euo pipefail

case "${1:-}" in
  "")
    cargo xtask ci tests
    doctests=1
    ;;
  --gpu-shard)
    if [ "$GATE_SHARDS" -lt 2 ]; then
      echo "::error::--gpu-shard needs at least two shards, got $GATE_SHARDS"
      exit 1
    fi
    if [ "$GATE_SHARD" = 1 ]; then
      # Build first, so only the test run goes through the lavapipe check.
      cargo xtask ci build
      python3 scripts/ci-gpu-lavapipe.py cargo xtask ci tests --gpu
    else
      NEXTEST_PARTITION="hash:$((GATE_SHARD - 1))/$((GATE_SHARDS - 1))" cargo xtask ci tests --no-gpu
    fi
    doctests=2
    ;;
  *)
    echo "usage: $0 [--gpu-shard]" >&2
    exit 2
    ;;
esac

if [ "$GATE_SHARD" = "$doctests" ]; then
  cargo xtask ci doctest
fi
