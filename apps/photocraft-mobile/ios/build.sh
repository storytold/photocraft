#!/bin/sh
# Build PhotoCraft.app for the iPad simulator (default) or a device: ios/build.sh [sim|device]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
kind=${1:-sim}
case $kind in
  sim) rust=aarch64-apple-ios-sim; sdk=iphonesimulator; triple=arm64-apple-ios17.0-simulator ;;
  device) rust=aarch64-apple-ios; sdk=iphoneos; triple=arm64-apple-ios17.0 ;;
  *) echo "usage: $0 [sim|device]" >&2; exit 2 ;;
esac
# C code in dependencies (ring) must target the same minimum iOS as the Swift shell.
export IPHONEOS_DEPLOYMENT_TARGET=17.0
target_dir=${CARGO_TARGET_DIR:-$here/../../../target}
cargo build --release -p photocraft-mobile --target "$rust"
app=$target_dir/$rust/release/PhotoCraft.app
rm -rf "$app" && mkdir -p "$app"
# The static archive by path: -l would pick the cdylib built beside it.
xcrun --sdk "$sdk" swiftc -O -target "$triple" -import-objc-header "$here/PhotoCraft.h" -parse-as-library "$here/PhotoCraftApp.swift" \
  "$target_dir/$rust/release/libphotocraft_mobile.a" -lc++ \
  -framework SwiftUI -framework UIKit -framework Metal -framework QuartzCore -framework CoreGraphics -framework CoreText \
  -framework Foundation -framework Security -framework SystemConfiguration \
  -o "$app/PhotoCraft"
cp "$here/Info.plist" "$app/"
codesign --force --sign - "$app"
echo "$app"
