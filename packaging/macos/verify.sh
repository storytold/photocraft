#!/usr/bin/env bash
# Check the macOS release artifacts exactly as users download them:
#
#   $DIST/photocraft-<version>-macos-<arch>.dmg       signed, notarized, ticket stapled
#   $DIST/photocraft-cli-<version>-macos-<arch>.zip   unpacked here; the binary inside must be
#                                                    Developer ID signed with the hardened runtime,
#                                                    timestamped and notarized
#
# Usage: packaging/macos/verify.sh [--arch aarch64|x86_64]
#
# With a real signing identity and notarization credentials in the environment (the same
# MACOS_SIGN_IDENTITY / APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID that package.sh uses), every
# check is strict and a failure exits non-zero. Without them (local ad-hoc builds, or a release run
# whose secrets are missing) only the signature's integrity is checked and the rest is a warning,
# matching package.sh. Nothing here reads or prints a secret's value; it only tests whether one is set.
#
# A bare Mach-O can't carry a stapled notarization ticket (only .app, .dmg and .pkg can), so for the
# CLI Gatekeeper looks the ticket up online the first time a quarantined copy runs. `spctl --assess
# --type install` does that same online lookup here, which is how this script proves the CLI was
# notarized.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

ARCH="$(uname -m)"
[ "$ARCH" != arm64 ] || ARCH=aarch64
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) [ $# -ge 2 ] || { echo "error: --arch needs a value" >&2; exit 2; }; ARCH="$2"; shift 2 ;;
    -h | --help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  aarch64) WANT_ARCHS="arm64" ;;
  x86_64) WANT_ARCHS="x86_64" ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

DMG="$DIST/photocraft-$VERSION-macos-$ARCH.dmg"
CLI_ZIP="$DIST/photocraft-cli-$VERSION-macos-$ARCH.zip"
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
STRICT=0
if [ "$IDENTITY" != "-" ] && [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  STRICT=1
fi

FAILED=0
fail() {
  if [ "$STRICT" = 1 ]; then
    echo "error: $*" >&2
    FAILED=1
  else
    warn "$* (expected without signing secrets)"
  fi
}

# spctl's online ticket lookup can lag a few seconds behind notarytool's "Accepted".
assess() {
  local out attempt tries=1
  [ "$STRICT" = 1 ] && tries=5
  for attempt in $(seq "$tries"); do
    if out="$(spctl "$@" 2>&1)"; then
      printf '%s\n' "$out"
      return 0
    fi
    [ "$attempt" = "$tries" ] || sleep 10
  done
  printf '%s\n' "$out"
  return 1
}

for f in "$DMG" "$CLI_ZIP"; do
  if [ ! -f "$f" ]; then
    echo "error: missing $f" >&2
    exit 1
  fi
done

WORK="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/photocraft-verify.XXXXXX")"
MOUNT="$WORK/mount"
MOUNTED=0
cleanup() {
  if [ "$MOUNTED" = 1 ]; then hdiutil detach "$MOUNT" -quiet || true; fi
  rm -rf "$WORK"
}
trap cleanup EXIT

check_arch() {
  local binary="$1" archs
  archs="$(lipo -archs "$binary")"
  if [ "$archs" != "$WANT_ARCHS" ]; then
    echo "error: $binary must contain only $WANT_ARCHS (has: $archs)" >&2
    exit 1
  fi
}
smoke_version() {
  local binary="$1"
  if [ "$ARCH" = x86_64 ] && [ "$(uname -m)" = arm64 ]; then
    if /usr/bin/arch -x86_64 /usr/bin/true 2>/dev/null; then
      /usr/bin/arch -x86_64 "$binary" --version
    else
      warn "Intel runtime smoke skipped: Rosetta is unavailable; architecture and signature verified"
    fi
  elif [ "$ARCH" = aarch64 ] && [ "$(uname -m)" != arm64 ]; then
    warn "Apple silicon runtime smoke skipped on Intel host; architecture and signature verified"
  else
    "$binary" --version
  fi
}

# ---- CLI zip -----------------------------------------------------------------------------------
echo "==> $(basename "$CLI_ZIP")"
ditto -x -k "$CLI_ZIP" "$WORK/cli"
CLI="$WORK/cli/photocraft-cli-$VERSION-macos-$ARCH/photocraft-cli"
if [ ! -x "$CLI" ]; then
  echo "error: $(basename "$CLI_ZIP") has no executable photocraft-cli-$VERSION-macos-$ARCH/photocraft-cli" >&2
  exit 1
fi

check_arch "$CLI"

# Integrity first: this must hold even for ad-hoc builds.
codesign --verify --strict --verbose=2 "$CLI"
details="$(codesign -d --verbose=2 "$CLI" 2>&1)"
printf '%s\n' "$details" | grep -E '^(Identifier|Format|CodeDirectory|Authority|Timestamp|TeamIdentifier)[= ]' || true

printf '%s\n' "$details" | grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' \
  || fail "photocraft-cli is not signed with the hardened runtime"
printf '%s\n' "$details" | grep -q '^Authority=Developer ID Application:' \
  || fail "photocraft-cli is not signed with a Developer ID Application identity"
printf '%s\n' "$details" | grep -q '^Timestamp=' \
  || fail "photocraft-cli signature has no secure timestamp"
if [ -n "${APPLE_TEAM_ID:-}" ]; then
  printf '%s\n' "$details" | grep -qx "TeamIdentifier=$APPLE_TEAM_ID" \
    || fail "photocraft-cli is signed by a different team"
fi
if gk="$(assess --assess --type install -vv "$CLI")"; then
  printf '%s\n' "$gk"
  printf '%s\n' "$gk" | grep -q 'source=Notarized Developer ID' \
    || fail "Gatekeeper accepts photocraft-cli but not as notarized"
else
  printf '%s\n' "$gk"
  fail "Gatekeeper rejects photocraft-cli (not notarized?)"
fi
smoke_version "$CLI"

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> $(basename "$DMG")"
codesign --verify --strict --verbose=2 "$DMG"
if [ "$STRICT" = 1 ]; then
  xcrun stapler validate "$DMG" || fail "no notarization ticket stapled to the DMG"
  assess --assess --type open --context context:primary-signature -vv "$DMG" \
    || fail "Gatekeeper rejects the DMG"
fi

# Verify the executable inside the shipped DMG, including exact single architecture.
mkdir -p "$MOUNT"
hdiutil attach "$DMG" -readonly -nobrowse -mountpoint "$MOUNT" -quiet
MOUNTED=1
APP="$MOUNT/PhotoCraft.app"
GUI="$APP/Contents/MacOS/PhotoCraft"
if [ ! -x "$GUI" ]; then
  echo "error: DMG has no PhotoCraft.app executable" >&2
  exit 1
fi
check_arch "$GUI"
codesign --verify --strict --deep --verbose=2 "$APP"
if [ "$STRICT" = 1 ]; then
  xcrun stapler validate "$APP" || fail "no notarization ticket stapled to PhotoCraft.app"
  assess --assess --type execute -vv "$APP" || fail "Gatekeeper rejects PhotoCraft.app"
fi
smoke_version "$GUI"
hdiutil detach "$MOUNT" -quiet
MOUNTED=0

if [ "$FAILED" != 0 ]; then
  echo "error: macOS artifacts failed verification" >&2
  exit 1
fi

if [ "$STRICT" = 1 ]; then
  result="signed (Developer ID, hardened runtime), notarized; DMG stapled"
else
  result="signature intact; Developer ID and notarization not required (no signing secrets)"
fi
echo "==> verified: $result"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "- macOS \`$(basename "$DMG")\`, \`$(basename "$CLI_ZIP")\`: $result"
  } >>"$GITHUB_STEP_SUMMARY"
fi
