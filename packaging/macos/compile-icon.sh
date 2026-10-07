#!/usr/bin/env bash
# Compile the Icon Composer document into Liquid Glass Assets.car and legacy .icns.
# Requires Xcode 26+ (actool); iconutil alone cannot compile layered icons.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:?usage: compile-icon.sh <output-resource-directory>}"
SOURCE="$ROOT/assets/app-icon/PhotoCraft.icon"
SOURCE_HASH="$(cat "$SOURCE/icon.json" "$SOURCE/Assets/01-panel.svg" \
  "$SOURCE/Assets/02-perforations.svg" "$SOURCE/Assets/03-lettering.svg" | shasum -a 256 | cut -d ' ' -f 1)"

# A CI-compiled icon can be reused by a local release build without Xcode. The source digest
# prevents accidentally packaging artwork compiled from a different revision.
if [ -n "${PHOTOCRAFT_COMPILED_ICON_DIR:-}" ]; then
  INPUT="$PHOTOCRAFT_COMPILED_ICON_DIR"
  if [ "$(cat "$INPUT/source.sha256" 2>/dev/null || true)" != "$SOURCE_HASH" ]; then
    echo "error: precompiled icon does not match the current icon source" >&2
    exit 1
  fi
  for name in Assets.car PhotoCraft.icns; do
    if [ ! -s "$INPUT/$name" ]; then
      echo "error: missing precompiled icon resource $name" >&2
      exit 1
    fi
  done
  mkdir -p "$OUT"
  cp "$INPUT/Assets.car" "$INPUT/PhotoCraft.icns" "$OUT/"
  echo "layered macOS icon copied to $OUT"
  exit 0
fi

if ! xcrun --find actool >/dev/null 2>&1; then
  echo "error: Xcode 26+ actool is required to compile the layered macOS icon" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
# Use an absolute source path: actool's helper caches by document path.
xcrun actool "$SOURCE" --compile "$TMP" \
  --platform macosx --minimum-deployment-target 11.0 \
  --app-icon PhotoCraft --output-partial-info-plist "$TMP/icon-partial.plist" \
  --output-format human-readable-text --errors
for name in Assets.car PhotoCraft.icns; do
  if [ ! -s "$TMP/$name" ]; then
    echo "error: actool did not create $name" >&2
    exit 1
  fi
done
mkdir -p "$OUT"
cp "$TMP/Assets.car" "$TMP/PhotoCraft.icns" "$OUT/"
printf '%s\n' "$SOURCE_HASH" >"$OUT/source.sha256"
echo "layered macOS icon written to $OUT"
