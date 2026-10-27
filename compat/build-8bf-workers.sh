#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
output=${1:-"$root/compat/8bf-worker/dist"}
mkdir -p "$output"
for pair in i686:32 x86_64:64; do
  arch=${pair%:*}
  bits=${pair#*:}
  cargo build --manifest-path "$root/compat/8bf-worker/Cargo.toml" --target-dir "$root/compat/8bf-worker/target" --release --locked --target "$arch-pc-windows-gnu"
  install -m 755 "$root/compat/8bf-worker/target/$arch-pc-windows-gnu/release/photocraft-8bf-worker.exe" "$output/photocraft-8bf-worker$bits.exe"
done
