#!/usr/bin/env bash
# Point the AUR PKGBUILDs at a published GitHub release and regenerate their .SRCINFO files.
#
#   packaging/arch/update.sh 0.2.0      # or v0.2.0
#
# Sets pkgver, resets pkgrel to 1, and fills in the checksums (photocraft-bin: the Linux tarballs
# from the release's SHA256SUMS.txt; photocraft: the tag's source archive) and the commit the tag
# points at. The release must already be published: the tag only exists after that.
#
# Needs: curl, git, sha256sum; makepkg for .SRCINFO (skipped with a warning when missing, e.g.
# on a non-Arch machine). The aur workflow (.github/workflows/aur.yml) runs this, then pushes
# both directories to the AUR.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="${PHOTOCRAFT_REPO:-storytold/photocraft}"

if [ $# -ne 1 ]; then
  sed -n '2,12p' "$0"
  exit 2
fi
VERSION="${1#v}"
TAG="v$VERSION"
# makepkg's pkgver can't hold '-': 0.2.0-rc.1 becomes 0.2.0rc.1, which vercmp sorts before 0.2.0.
PKGVER="${VERSION//-/}"
BASE="https://github.com/$REPO"

echo "==> AUR PKGBUILDs for $TAG (pkgver $PKGVER)"

SUMS="$(curl -fsSL "$BASE/releases/download/$TAG/SHA256SUMS.txt")"
sum_of() {
  local s
  s="$(awk -v f="$1" '$2 == f { print $1 }' <<<"$SUMS")"
  if [ -z "$s" ]; then echo "error: $1 is not in $TAG's SHA256SUMS.txt" >&2; exit 1; fi
  echo "$s"
}
SUM_X86_64="$(sum_of "photocraft-$VERSION-linux-x86_64.tar.gz")"
SUM_AARCH64="$(sum_of "photocraft-$VERSION-linux-aarch64.tar.gz")"
SUM_SRC="$(curl -fsSL "$BASE/archive/refs/tags/$TAG.tar.gz" | sha256sum | cut -d' ' -f1)"

# An annotated tag lists its commit as "<tag>^{}"; a lightweight one only as "<tag>".
COMMIT="$(git ls-remote "$BASE.git" "refs/tags/$TAG" "refs/tags/$TAG^{}" | sort -k2 | tail -n1 | cut -f1)"
if [ -z "$COMMIT" ]; then echo "error: tag $TAG not found in $REPO" >&2; exit 1; fi

# The release tarballs keep the upstream version in their names, so the -bin PKGBUILD needs it
# separately from pkgver.
sed -i \
  -e "s/^pkgver=.*/pkgver=$PKGVER/" \
  -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "s/^_version=.*/_version=$VERSION/" \
  -e "s/^sha256sums_x86_64=.*/sha256sums_x86_64=('$SUM_X86_64')/" \
  -e "s/^sha256sums_aarch64=.*/sha256sums_aarch64=('$SUM_AARCH64')/" \
  "$HERE/photocraft-bin/PKGBUILD"
sed -i \
  -e "s/^pkgver=.*/pkgver=$PKGVER/" \
  -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "s/^_version=.*/_version=$VERSION/" \
  -e "s/^_commit=.*/_commit=$COMMIT/" \
  -e "s/^sha256sums=.*/sha256sums=('$SUM_SRC')/" \
  "$HERE/photocraft/PKGBUILD"

for pkg in photocraft photocraft-bin; do
  if command -v makepkg >/dev/null; then
    (cd "$HERE/$pkg" && makepkg --printsrcinfo >.SRCINFO)
    echo "wrote packaging/arch/$pkg/.SRCINFO"
  else
    echo "warning: makepkg not found; $pkg/.SRCINFO not regenerated" >&2
  fi
done
echo "==> done"
