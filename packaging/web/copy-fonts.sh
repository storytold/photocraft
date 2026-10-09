#!/bin/sh
# Trunk post_build hook (apps/photocraft-web/Trunk.toml): copy the craft-fonts faces listed in
# crates/text/web-fonts.txt from $CRAFT_FONTS_DIR into the site, each at
# fonts/<first 16 hex digits of its SHA-256>/<file> with its licence beside it: the URLs
# crates/text/build.rs puts in WEB_FONTS. A no-op without CRAFT_FONTS_DIR.
set -eu
[ -n "${CRAFT_FONTS_DIR:-}" ] || exit 0
: "${TRUNK_STAGING_DIR:?run me as a Trunk post_build hook (TRUNK_STAGING_DIR unset)}"
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
LIST="$ROOT/crates/text/web-fonts.txt"
MANIFEST="$CRAFT_FONTS_DIR/fonts/manifest.txt"
[ -f "$MANIFEST" ] || { echo "copy-fonts: $MANIFEST not found" >&2; exit 1; }

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

grep -v '^[[:space:]]*#' "$LIST" | grep -v '^[[:space:]]*$' | while IFS= read -r line; do
  family=$(printf '%s\n' "$line" | awk -F' [|] ' '{print $2}')
  style=$(printf '%s\n' "$line" | awk -F' [|] ' '{print $3}')
  row=$(awk -F' [|] ' -v f="$family" -v s="$style" '$1 == f && $2 == s { print $3 "|" $6 "|" $7; exit }' "$MANIFEST")
  [ -n "$row" ] || { echo "copy-fonts: $family $style is not in $MANIFEST" >&2; exit 1; }
  file=${row%%|*}; rest=${row#*|}; licence=${rest%%|*}; sha=${rest#*|}
  got=$(sha256 "$CRAFT_FONTS_DIR/$file")
  [ "$got" = "$sha" ] || { echo "copy-fonts: $file: sha256 $got, manifest says $sha" >&2; exit 1; }
  dest="$TRUNK_STAGING_DIR/fonts/$(printf '%s' "$sha" | cut -c1-16)"
  mkdir -p "$dest"
  cp "$CRAFT_FONTS_DIR/$file" "$dest/"
  cp "$CRAFT_FONTS_DIR/$licence" "$dest/"
done
