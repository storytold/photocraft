#!/usr/bin/env bash
# Exercise the package recipe's real signing helper/call without signing code or using a keychain.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RECIPE="${1:-$HERE/../package.sh}"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
args_file="$scratch/args"
APP="$scratch/PhotoCraft with spaces.app"

codesign() {
  printf '%s\n' "$@" > "$args_file"
}
# Load only the signing helper, not the package/download/build/install steps.
eval "$(sed -n '/^sign() {/,/^}/p' "$RECIPE")"
framework_sign="$(sed -n '/^sign .*Sparkle.framework"$/p; /^codesign .*Sparkle.framework"$/p' "$RECIPE")"
[ -n "$framework_sign" ] || { echo "missing framework signing call" >&2; exit 1; }

assert_arg() { grep -Fxq -- "$1" "$args_file"; }
assert_no_arg() { ! grep -Fxq -- "$1" "$args_file"; }

IDENTITY="Developer ID Application: Test"
MACOS_KEYCHAIN="$scratch/release keychain"
eval "$framework_sign"
assert_arg "$IDENTITY"
assert_arg --timestamp
assert_arg --keychain
assert_arg "$MACOS_KEYCHAIN"
assert_arg --deep
assert_arg --options
assert_arg runtime
assert_arg "$APP/Contents/Frameworks/Sparkle.framework"

unset MACOS_KEYCHAIN
eval "$framework_sign"
assert_arg --timestamp
assert_no_arg --keychain

IDENTITY=-
eval "$framework_sign"
assert_arg --timestamp=none
assert_no_arg --timestamp
assert_no_arg --keychain
echo "Sparkle framework signing: release/keychain, release/default, and ad-hoc cases passed"
