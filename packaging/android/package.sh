#!/usr/bin/env bash
# Build the Android APK package: $DIST/photocraft-android-<version>.apk
#
# Usage: packaging/android/package.sh [--release | --debug]
#
# Requirements:
#   - Rust Android target: rustup target add aarch64-linux-android
#   - Android NDK (r25c or newer) with ANDROID_NDK_HOME set
#   - cargo-apk (cargo install cargo-apk --locked) OR cargo-ndk
set -euo pipefail

# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/android"

MODE="release"
BUILD_FLAG="--release"
if [ "${1:-}" = "--debug" ]; then
  MODE="debug"
  BUILD_FLAG=""
fi

TARGET="${2:-${TARGET:-aarch64-linux-android}}"

echo "==> Packaging PhotoCraft for Android ($MODE, $TARGET)..."

# Ensure target is installed
if ! rustup target list --installed | grep -q "$TARGET"; then
  echo "warning: $TARGET target not installed. Run: rustup target add $TARGET" >&2
fi


# Build using cargo-apk if available
if command -v cargo-apk >/dev/null 2>&1; then
  echo "==> Building APK using cargo-apk..."
  (cd "$ROOT" && cargo apk build -p photocraft-android --target "$TARGET" $BUILD_FLAG)
  APK_SRC="$ROOT/target/$TARGET/$MODE/apk/photocraft_android.apk"
elif command -v cargo-ndk >/dev/null 2>&1; then
  echo "==> Building shared library using cargo-ndk..."
  (cd "$ROOT" && cargo ndk -t arm64-v8a build -p photocraft-android $BUILD_FLAG)
  echo "==> Shared library built at target/aarch64-linux-android/$MODE/libphotocraft_android.so"
  APK_SRC=""
else
  echo "error: Neither cargo-apk nor cargo-ndk found in PATH." >&2
  echo "Install cargo-apk: cargo install cargo-apk --locked" >&2
  exit 1
fi

if [ -n "$APK_SRC" ] && [ -f "$APK_SRC" ]; then
  NAME="photocraft-android-$VERSION.apk"
  cp "$APK_SRC" "$DIST/$NAME"
  echo "==> Successfully packaged: $DIST/$NAME"
  ls -lh "$DIST/$NAME"
fi

