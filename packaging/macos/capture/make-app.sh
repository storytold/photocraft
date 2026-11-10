#!/bin/bash
# Builds "PhotoCraft Capture.app" (a small launcher around photocraft-capture.sh) into the folder
# given (default: /Applications), signed ad hoc for local use.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${1:-/Applications}"
APP="$DEST/PhotoCraft Capture.app"
ICON="${ICON:-/Applications/PhotoCraft.app/Contents/Resources/PhotoCraft.icns}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
TMP="$WORK/PhotoCraft Capture.app"
mkdir -p "$TMP/Contents/MacOS" "$TMP/Contents/Resources"
cp "$HERE/photocraft-capture.sh" "$TMP/Contents/MacOS/PhotoCraft Capture"
chmod 755 "$TMP/Contents/MacOS/PhotoCraft Capture"
[ -f "$ICON" ] && cp "$ICON" "$TMP/Contents/Resources/PhotoCraft.icns"
cat > "$TMP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>PhotoCraft Capture</string>
  <key>CFBundleDisplayName</key><string>PhotoCraft Capture</string>
  <key>CFBundleIdentifier</key><string>ai.storyteller.photocraft.capture</string>
  <key>CFBundleExecutable</key><string>PhotoCraft Capture</string>
  <key>CFBundleIconFile</key><string>PhotoCraft</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <!-- No Dock icon or menu bar while it waits for the capture. -->
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
xattr -cr "$TMP"
codesign --force --sign - --timestamp=none "$TMP"
rm -rf "$APP"
mv "$TMP" "$APP"
codesign -v "$APP"
echo "Built $APP"
