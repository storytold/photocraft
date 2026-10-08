#!/usr/bin/env bash
# Point the Homebrew cask at a published GitHub release.
#
#   packaging/homebrew/update.sh 0.3.0      # or v0.3.0
#
# Sets `version` and `sha256` in packaging/homebrew/photocraft.rb from the release's
# SHA256SUMS.txt (the universal DMG). Pre-releases are refused: the tap only carries stable
# versions. Needs curl. The homebrew workflow (.github/workflows/homebrew.yml) runs this, checks
# the cask with brew, then copies it to the tap.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CASK="$HERE/photocraft.rb"
REPO="${PHOTOCRAFT_REPO:-storytold/photocraft}"

if [ $# -ne 1 ]; then
  sed -n '2,9p' "$0"
  exit 2
fi
VERSION="${1#v}"
if ! printf '%s' "$VERSION" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
  echo "error: '$VERSION' is not a stable version like 1.2.3 (pre-releases aren't published to Homebrew)" >&2
  exit 1
fi
DMG="photocraft-$VERSION-macos-universal.dmg"

echo "==> Homebrew cask for v$VERSION"
SUMS="$(curl -fsSL "https://github.com/$REPO/releases/download/v$VERSION/SHA256SUMS.txt")"
SUM="$(awk -v f="$DMG" '$2 == f { print $1 }' <<<"$SUMS")"
if ! printf '%s' "$SUM" | grep -Eq '^[0-9a-f]{64}$'; then
  echo "error: $DMG is not in v$VERSION's SHA256SUMS.txt" >&2
  exit 1
fi

# Through a temp file: `sed -i` differs between GNU (Linux) and BSD (macOS, where CI runs this).
sed -e "s/^  version \".*\"$/  version \"$VERSION\"/" \
  -e "s/^  sha256 \".*\"$/  sha256 \"$SUM\"/" \
  "$CASK" >"$CASK.tmp"
mv "$CASK.tmp" "$CASK"
if ! grep -qx "  version \"$VERSION\"" "$CASK" || ! grep -qx "  sha256 \"$SUM\"" "$CASK"; then
  echo "error: couldn't set version/sha256 in $CASK" >&2
  exit 1
fi
echo "wrote packaging/homebrew/photocraft.rb ($DMG, sha256 $SUM)"
