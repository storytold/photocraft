#!/usr/bin/env bash
# Build, sign and (optionally) notarize the macOS release artifacts:
#
#   $DIST/photocraft-<version>-macos-<arch>.dmg          PhotoCraft.app on a drag-to-Applications DMG
#   $DIST/photocraft-cli-<version>-macos-<arch>.zip      the headless CLI
#
# Usage: packaging/macos/package.sh [--arch aarch64|x86_64] [--skip-build]
#
# Signing (env):
#   MACOS_SIGN_IDENTITY   codesign identity (name or SHA-1). Default "-" = ad-hoc (local testing;
#                         Gatekeeper will reject the result on other Macs). In CI, import-cert.sh sets it.
#   MACOS_KEYCHAIN        keychain holding the identity (optional)
# Notarization (env; all three needed, and a real identity):
#   APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/macos"

ARCH="$(uname -m)"
[ "$ARCH" != arm64 ] || ARCH=aarch64
SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) [ $# -ge 2 ] || { echo "error: --arch needs a value" >&2; exit 2; }; ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-package-$ARCH"
# Private matching symbols stay outside the published artifact directory.
DIAGNOSTICS="$CARGO_TARGET_DIR/release-diagnostics/$VERSION/${PHOTOCRAFT_BUILD_SHA:-unknown}/macos-$ARCH"
APP="$WORK/PhotoCraft.app"
DMG="$DIST/photocraft-$VERSION-macos-$ARCH.dmg"
CLI_ZIP="$DIST/photocraft-cli-$VERSION-macos-$ARCH.zip"

NOTARIZE=0
if [ "$IDENTITY" = "-" ]; then
  warn "macOS: ad-hoc signing (no MACOS_SIGN_IDENTITY); artifacts are not notarized"
elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  NOTARIZE=1
else
  warn "macOS: APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID incomplete; signed but not notarized"
fi

echo "==> PhotoCraft $VERSION for macOS ($ARCH), identity: $IDENTITY, notarize: $NOTARIZE"

# ---- build -------------------------------------------------------------------------------------
if [ "$SKIP_BUILD" = 0 ]; then
  args=()
  for t in "${TARGETS[@]}"; do args+=(--target "$t"); done
  (cd "$ROOT" && cargo build --profile native-release --locked -p photocraft -p photocraft-cli --features heif "${args[@]}")
fi

rm -rf "$WORK"
mkdir -p "$WORK/bin"
mkdir -p "$DIAGNOSTICS"
WANT_ARCH=x86_64
[ "$ARCH" != aarch64 ] || WANT_ARCH=arm64
for bin in photocraft photocraft-cli; do
  input="$CARGO_TARGET_DIR/${TARGETS[0]}/native-release/$bin"
  archs="$(lipo -archs "$input")"
  if [ "$archs" != "$WANT_ARCH" ]; then
    echo "error: $input must contain only $WANT_ARCH (has: $archs)" >&2
    exit 1
  fi
  # Generate diagnostics before stripping while the build objects still exist.
  # Failure is fatal rather than silently losing release crash diagnostics.
  rm -rf "$DIAGNOSTICS/$bin.dSYM"
  xcrun dsymutil "$input" -o "$DIAGNOSTICS/$bin.dSYM"
  xcrun dwarfdump --uuid "$input" >"$DIAGNOSTICS/$bin.uuid.txt"
  cp "$input" "$WORK/bin/$bin"
  strip -S -x "$WORK/bin/$bin"
  lipo -info "$WORK/bin/$bin"
done

# ---- signing helpers ---------------------------------------------------------------------------
sign() {
  # Hardened runtime + secure timestamp for a real identity; ad-hoc can't be timestamped.
  local ts=(--timestamp)
  [ "$IDENTITY" = "-" ] && ts=(--timestamp=none)
  local kc=()
  [ -n "${MACOS_KEYCHAIN:-}" ] && kc=(--keychain "$MACOS_KEYCHAIN")
  codesign --force --sign "$IDENTITY" ${kc[@]+"${kc[@]}"} "${ts[@]}" "$@"
}

notarize() {
  local file="$1" out id status
  echo "==> notarizing $(basename "$file") (this can take a few minutes)"
  out="$(xcrun notarytool submit "$file" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" --wait --timeout 1h --output-format json)" || true
  echo "$out"
  id="$(printf '%s' "$out" | plutil -extract id raw -o - - 2>/dev/null || true)"
  status="$(printf '%s' "$out" | plutil -extract status raw -o - - 2>/dev/null || true)"
  if [ "$status" != "Accepted" ]; then
    if [ -n "$id" ]; then
      xcrun notarytool log "$id" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" || true
    fi
    echo "error: notarization of $(basename "$file") failed (status: ${status:-unknown})" >&2
    exit 1
  fi
}

# ---- PhotoCraft.app ----------------------------------------------------------------------------
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# Executable and icon carry the display name (CFBundleExecutable / CFBundleIconFile).
cp "$WORK/bin/photocraft" "$APP/Contents/MacOS/PhotoCraft"
cp "$ROOT/assets/app-icon/photocraft.icns" "$APP/Contents/Resources/PhotoCraft.icns"
# Licences of the embedded craft-fonts fonts (only when built with CRAFT_FONTS_DIR).
if [ -n "${CRAFT_FONTS_DIR:-}" ]; then
  mkdir -p "$APP/Contents/Resources/Licenses"
  copy_font_licences "$APP/Contents/Resources/Licenses"
fi
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${PHOTOCRAFT_BUILD_SHA:-unknown}/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

# Sign inside-out: nested code first, then the bundle itself (no --deep on the final signature).
# Today the only nested code is the main executable; frameworks/helpers would be signed here too.
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP/Contents/MacOS/PhotoCraft"
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

if [ "$NOTARIZE" = 1 ]; then
  ditto -c -k --keepParent "$APP" "$WORK/PhotoCraft-notarize.zip"
  notarize "$WORK/PhotoCraft-notarize.zip"
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
  spctl --assess --type execute -vvv "$APP"
fi

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> building $DMG"
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/PhotoCraft.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG" "$WORK/raw.dmg"
# Avoid create -srcfolder (flaky on CI). makehybrid synthesizes FinderInfo on
# resource files, so clear that metadata on a writable image before compression;
# otherwise the app's valid signature fails strict verification inside the DMG.
hdiutil makehybrid -hfs -hfs-volume-name "PhotoCraft $VERSION" -hfs-openfolder "$STAGE" -o "$WORK/raw.dmg" "$STAGE"
hdiutil convert "$WORK/raw.dmg" -format UDRW -o "$WORK/writable.dmg"
MOUNT="$WORK/dmg-clean"
MOUNTED=0
cleanup_mount() {
  if [ "$MOUNTED" = 1 ]; then hdiutil detach "$MOUNT" -quiet || true; fi
}
trap cleanup_mount EXIT
mkdir -p "$MOUNT"
hdiutil attach "$WORK/writable.dmg" -nobrowse -mountpoint "$MOUNT" -quiet
MOUNTED=1
# Preserve all other attributes, including notarization metadata. Do not follow
# the /Applications symlink or touch volume-level Finder presentation metadata.
find "$MOUNT/PhotoCraft.app" -xattrname com.apple.FinderInfo \
  -exec xattr -d com.apple.FinderInfo {} +
codesign --verify --strict --deep --verbose=2 "$MOUNT/PhotoCraft.app"
hdiutil detach "$MOUNT" -quiet
MOUNTED=0
trap - EXIT
hdiutil convert "$WORK/writable.dmg" -format UDZO -imagekey zlib-level=9 -o "$DMG"
rm -f "$WORK/raw.dmg" "$WORK/writable.dmg"
sign "$DMG"
codesign --verify --strict --verbose=2 "$DMG"
if [ "$NOTARIZE" = 1 ]; then
  notarize "$DMG"
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl --assess --type open --context context:primary-signature -vvv "$DMG"
fi

# ---- CLI ---------------------------------------------------------------------------------------
echo "==> building $CLI_ZIP"
CLI_DIR="$WORK/photocraft-cli-$VERSION-macos-$ARCH"
mkdir -p "$CLI_DIR"
cp "$WORK/bin/photocraft-cli" "$CLI_DIR/"
copy_docs "$CLI_DIR"
# Same Developer ID and hardened runtime as the app, with an explicit reverse-DNS identifier
# (codesign would otherwise use the bare file name).
sign --options runtime --identifier ai.storyteller.photocraft-cli "$CLI_DIR/photocraft-cli"
codesign --verify --strict --verbose=2 "$CLI_DIR/photocraft-cli"
rm -f "$CLI_ZIP"
ditto -c -k --keepParent "$CLI_DIR" "$CLI_ZIP"
# notarytool accepts a zip of the bare binary. A bare Mach-O can't carry a stapled ticket, so
# Gatekeeper looks the notarization up online when a quarantined copy first runs.
if [ "$NOTARIZE" = 1 ]; then notarize "$CLI_ZIP"; fi

# Run the packaged, signed CLI; cross-architecture execution needs OS support.
if [ "$ARCH" = x86_64 ] && [ "$(uname -m)" = arm64 ]; then
  if /usr/bin/arch -x86_64 /usr/bin/true 2>/dev/null; then
    /usr/bin/arch -x86_64 "$CLI_DIR/photocraft-cli" --version
  else
    warn "Intel CLI runtime smoke skipped: Rosetta is unavailable; architecture and signature verified"
  fi
elif [ "$ARCH" = aarch64 ] && [ "$(uname -m)" != arm64 ]; then
  warn "Apple silicon CLI runtime smoke skipped on Intel host; architecture and signature verified"
else
  "$CLI_DIR/photocraft-cli" --version
fi
echo "==> done (check the artifacts as shipped with packaging/macos/verify.sh --arch $ARCH)"
ls -lh "$DMG" "$CLI_ZIP"
