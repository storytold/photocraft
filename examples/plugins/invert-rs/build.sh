#!/bin/sh
# Builds the Invert example plug-in and refreshes the test fixture built from it.
# Needs Rust 1.95.0 and its wasm32-unknown-unknown target (see README.md).
set -eu
cd "$(dirname "$0")"
cargo +1.95.0 build --release --target wasm32-unknown-unknown
out=target/wasm32-unknown-unknown/release/photocraft_plugin_invert.wasm
fixture=../../../crates/plugins/tests/fixtures/invert.wasm
case "${1:-}" in
    --check)
        cmp "$out" "$fixture"
        echo "fixture matches the rebuilt module byte-for-byte"
        exit 0
        ;;
    "") cp "$out" "$fixture" ;;
    *) echo "usage: $0 [--check]" >&2; exit 2 ;;
esac
echo "built $out ($(wc -c < "$out") bytes) -> crates/plugins/tests/fixtures/invert.wasm"
