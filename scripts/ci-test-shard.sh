#!/usr/bin/env bash
# One shard of the gate's tests: the gate's `shard-command` (.github/workflows/ci.yml). The gate's
# build job wrote one nextest archive per shard with `cargo xtask ci archive` (`shard-archives`), each
# holding only the test binaries that shard runs, the xtask binary and the shard plan
# (.config/nextest.toml). The shard extracts its archive and runs the test steps with
# `xtask ci tests --shard <k>` without building anything. The gate sets NEXTEST_ARCHIVE_FILE,
# GATE_SHARD and GATE_SHARDS.
#
# The build job balances the shards by recorded test times (.config/ci-test-times.tsv,
# xtask/src/shards.rs); a binary too heavy for one shard is split across shards by test filter.
#
#   ci-test-shard.sh              the shard runs its binaries
#   ci-test-shard.sh --gpu-shard  shard 1 runs nextest's `gpu` group alone on lavapipe, checked by
#                                 scripts/ci-gpu-lavapipe.py (the build job ran
#                                 `ci archive --gpu-shard`); the other shards run the other tests
set -euo pipefail

archive=target/nextest-archive
mkdir -p "$archive"
cargo nextest run --archive-file "$NEXTEST_ARCHIVE_FILE" --workspace-remap . --extract-to "$archive" --extract-overwrite -E 'none()' --no-tests=pass
xtask=("$archive/target/debug/xtask" ci tests --from "$archive" --shard "$GATE_SHARD")
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) xtask=("$archive/target/debug/xtask.exe" ci tests --from "$archive" --shard "$GATE_SHARD") ;; esac

# The gate builds the real-file corpus tests in (PHOTOCRAFT_CORPUS_TESTS=1, xtask's CORPUS_FEATURES):
# they fail when the corpus is missing, so a shard fetches it first (pinned commits, sha256-verified).
if [ "${PHOTOCRAFT_CORPUS_TESTS:-}" = 1 ]; then
  "${xtask[0]}" corpus --all
fi

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
    exec "${xtask[@]}" --no-gpu
    ;;
  *)
    echo "usage: $0 [--gpu-shard]" >&2
    exit 2
    ;;
esac
