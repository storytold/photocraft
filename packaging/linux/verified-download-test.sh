#!/usr/bin/env bash
# Fast, network-free checks of the verified AppImage tooling cache and override.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=verified-download.sh
. "$HERE/verified-download.sh"

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
fail() { echo "FAIL: $*" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
source_asset="$TMP/source.AppImage"
cache="$TMP/cached.AppImage"
printf '#!/bin/sh\nexit 0\n' >"$source_asset"
expected="$(sha256 "$source_asset")"

# Offline preseed: a valid 0644 cache must not try the unavailable URL and must become runnable.
cp "$source_asset" "$cache"
chmod 644 "$cache"
download_verified "file://$TMP/unavailable" "$cache" "$expected" || fail "offline cache was not reused"
[ -x "$cache" ] || fail "valid preseed did not become executable"

# A corrupt cache is replaced only by bytes with the pinned digest.
printf 'corrupt' >"$cache"
download_verified "file://$source_asset" "$cache" "$expected" || fail "corrupt cache was not replaced"
cmp -s "$source_asset" "$cache" || fail "replacement differs from verified asset"
[ -x "$cache" ] || fail "replacement is not executable"

wrong_digest="$(printf '%064d' 0)"
bad="$TMP/bad.AppImage"
if download_verified "file://$source_asset" "$bad" "$wrong_digest" >/dev/null 2>&1; then fail "wrong digest accepted"; fi
[ ! -e "$bad" ] || fail "wrong-digest download left a cache entry"

verify_appimagetool_override "$cache" "$expected" 1.9.1 x86_64 || fail "valid override rejected"
chmod 644 "$cache"
if verify_appimagetool_override "$cache" "$expected" 1.9.1 x86_64 >/dev/null 2>&1; then fail "non-executable override accepted"; fi
chmod 755 "$cache"
if verify_appimagetool_override "$cache" "$wrong_digest" 1.9.1 x86_64 >/dev/null 2>&1; then fail "wrong-digest override accepted"; fi

echo "verified AppImage tooling ok"
