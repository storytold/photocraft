#!/usr/bin/env bash
# Regenerate every app icon from assets/app-icon/photocraft.svg (the canonical master).
#
# Needs: resvg or rsvg-convert. On macOS, iconutil also writes the
# .icns. The outputs are committed, so packaging never needs these tools.
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
SVG="$DIR/photocraft.svg"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if command -v resvg >/dev/null; then
  RENDERER=resvg
elif command -v rsvg-convert >/dev/null; then
  RENDERER=rsvg-convert
else
  echo "error: install resvg or rsvg-convert to regenerate icons" >&2
  exit 1
fi

# The macOS icon has a rounded tile padded onto Apple's icon grid. Windows and Linux use a
# tighter, slightly squarer tile. Both silhouettes keep transparent corners.
grep -q 'viewBox="0 0 512 512"' "$SVG" || { echo "error: expected viewBox=\"0 0 512 512\" in $SVG" >&2; exit 1; }
MAC="$TMP/mac.svg"
sed 's/viewBox="0 0 512 512"/viewBox="-62 -62 636 636"/' "$SVG" >"$MAC"
WINDOWS="$TMP/windows.svg"
sed 's/rx="79" fill="url(#frame)"/rx="58" fill="url(#frame)"/' "$SVG" >"$WINDOWS"

render() {
  if [ "$RENDERER" = resvg ]; then
    resvg -w "$2" -h "$2" "$1" "$3" </dev/null
  else
    rsvg-convert -w "$2" -h "$2" "$1" -o "$3"
  fi
}

# The runtime Dock icon uses Apple's padded icon grid, matching the size of other macOS icons.
render "$MAC" 1024 "$DIR/photocraft-1024.png"

# Linux hicolor theme.
for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$DIR/hicolor/${s}x${s}/apps"
  render "$WINDOWS" "$s" "$DIR/hicolor/${s}x${s}/apps/ai.storyteller.photocraft.png"
done
mkdir -p "$DIR/hicolor/scalable/apps"
# The flat-colour variant keeps the scalable theme icon cheap to render.
cp "$DIR/photocraft-small.svg" "$DIR/hicolor/scalable/apps/ai.storyteller.photocraft.svg"

# Windows .ico.
ICO_PNGS=()
for s in 16 20 24 32 40 48 64 128 256; do
  render "$WINDOWS" "$s" "$TMP/ico-$s.png"
  ICO_PNGS+=("$TMP/ico-$s.png")
done
(cd "$ROOT" && cargo run -q -p xtask -- ico "$DIR/photocraft.ico" "${ICO_PNGS[@]}")

# macOS .icns.
if command -v iconutil >/dev/null; then
  SET="$TMP/photocraft.iconset"
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    render "$MAC" "$s" "$SET/icon_${s}x${s}.png"
    render "$MAC" $((s * 2)) "$SET/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns -o "$DIR/photocraft.icns" "$SET"
else
  echo "warning: iconutil not found (macOS only); photocraft.icns not regenerated" >&2
fi
python3 "$ROOT/packaging/macos/generate-icon-layers.py"
echo "icons written to $DIR"
