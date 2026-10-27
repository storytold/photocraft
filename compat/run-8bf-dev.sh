#!/usr/bin/env bash
# Run this feature checkout with its own preferences and Wine prefix.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
export PHOTOCRAFT_CONFIG_DIR=${PHOTOCRAFT_CONFIG_DIR:-"$root/target/8bf-profile"}
export PHOTOCRAFT_8BF_WINEPREFIX=${PHOTOCRAFT_8BF_WINEPREFIX:-"$root/target/8bf-wine"}
export PHOTOCRAFT_8BF_WORKER32=${PHOTOCRAFT_8BF_WORKER32:-"$root/compat/8bf-worker/dist/photocraft-8bf-worker32.exe"}
export PHOTOCRAFT_8BF_WORKER64=${PHOTOCRAFT_8BF_WORKER64:-"$root/compat/8bf-worker/dist/photocraft-8bf-worker64.exe"}
if [[ ! -x "$root/target/release/photocraft" || ! -f "$PHOTOCRAFT_8BF_WORKER32" || ! -f "$PHOTOCRAFT_8BF_WORKER64" ]]; then
  echo 'Build PhotoCraft with cargo build -p photocraft --release, then run compat/build-8bf-workers.sh.' >&2
  exit 1
fi
exec "$root/target/release/photocraft" "$@"
