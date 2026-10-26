#!/usr/bin/env bash
# Build artifacts first; only the reviewed server/site enter the runtime image.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
STAGE="$ROOT/dist/web-container"
test -s "$TARGET/release/photocraft-web-gateway"
test -s "$ROOT/dist/web/index.html"
rm -rf "$STAGE/site"
mkdir -p "$STAGE/site"
mkdir -p "$STAGE/licenses"
cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$STAGE/licenses/"
if [ -n "${CRAFT_FONTS_DIR:-}" ]; then
  for license in "$CRAFT_FONTS_DIR"/fonts/*/OFL.txt; do
    if [ -f "$license" ]; then
      family="$(basename "$(dirname "$license")")"
      cp "$license" "$STAGE/licenses/OFL-$family.txt"
    fi
  done
fi
cp "$TARGET/release/photocraft-web-gateway" "$STAGE/photocraft-web-gateway"
cp -a "$ROOT/dist/web/." "$STAGE/site/"
docker build -f "$ROOT/packaging/web/Dockerfile.gateway" -t "${PHOTOCRAFT_WEB_IMAGE:-photocraft-web:0.8.2-ai-20261013}" "$STAGE"
