#!/usr/bin/env bash
# Smoke test for packaging/linux/AppRun: the AppImage's desktop integration writes
# and refreshes the host .desktop entry and icons, is idempotent, honours the opt-out
# and an unwritable HOME, and always launches the app (#593).
#
# Runs in seconds with no build (CI: .github/workflows/packaging-lint.yml).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
APP_ID=ai.storyteller.photocraft

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Fake AppDir with the real AppRun, .desktop template, icons, and a stub binary.
APPDIR="$TMP/PhotoCraft.AppDir"
mkdir -p "$APPDIR/usr/bin"
install -Dm755 "$HERE/AppRun" "$APPDIR/AppRun"
install -Dm644 "$HERE/$APP_ID.desktop" "$APPDIR/usr/share/applications/$APP_ID.desktop"
mkdir -p "$APPDIR/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$APPDIR/usr/share/icons/"
printf '#!/bin/sh\necho "stub: $*"\n' >"$APPDIR/usr/bin/photocraft"
chmod +x "$APPDIR/usr/bin/photocraft"

APPIMAGE="$TMP/photocraft test.AppImage" # spaces on purpose: Exec must stay quoted
: >"$APPIMAGE"
export APPIMAGE
export HOME="$TMP/home"
export XDG_DATA_HOME="$HOME/.local/share"
mkdir -p "$HOME"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# 1. Launch works and passes arguments through.
out="$("$APPDIR/AppRun" open file.png)" || fail "AppRun exited non-zero"
[ "$out" = "stub: open file.png" ] || fail "arguments not forwarded: $out"

DESKTOP="$XDG_DATA_HOME/applications/$APP_ID.desktop"
[ -f "$DESKTOP" ] || fail "desktop entry not installed"

# 2. Exec rewritten to the AppImage (quoted, spaces intact), TryExec dropped,
#    everything else preserved from the packaged entry.
expected_exec="Exec=\"$(readlink -f "$APPIMAGE")\" %F"
grep -Fxq "$expected_exec" "$DESKTOP" || fail "Exec not rewritten: $(grep '^Exec=' "$DESKTOP")"
grep -q '^TryExec=' "$DESKTOP" && fail "TryExec kept, the entry would be hidden"
grep -Fxq "Icon=$APP_ID" "$DESKTOP" || fail "Icon line lost"
grep -Fxq 'Categories=Graphics;2DGraphics;RasterGraphics;Photography;' "$DESKTOP" || fail "metadata lines lost"
if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$DESKTOP" || fail "generated desktop entry is invalid"
fi

# 3. Icons landed under the name the .desktop entry references.
for f in "16x16/apps/$APP_ID.png" "256x256/apps/$APP_ID.png" "scalable/apps/$APP_ID.svg"; do
  [ -f "$XDG_DATA_HOME/icons/hicolor/$f" ] || fail "icon missing: $f"
done

# 4. Idempotent: a second launch must not rewrite anything (inode unchanged).
before="$(stat -c %i "$DESKTOP")"
out="$("$APPDIR/AppRun")" || fail "second launch exited non-zero"
[ "$out" = "stub: " ] || fail "second launch output: $out"
after="$(stat -c %i "$DESKTOP")"
[ "$before" = "$after" ] || fail "desktop entry rewritten although unchanged"

# 5. Moving the AppImage refreshes Exec.
mv "$APPIMAGE" "$TMP/moved.AppImage"
export APPIMAGE="$TMP/moved.AppImage"
"$APPDIR/AppRun" >/dev/null || fail "launch after move exited non-zero"
grep -Fxq "Exec=\"$(readlink -f "$APPIMAGE")\" %F" "$DESKTOP" || fail "Exec not refreshed after move"

# 6. Opt-out installs nothing.
# Customized launch options and metadata survive launches and relocation (#2905).
old_path="$(readlink -f "$APPIMAGE")"
sed -i "s|^Exec=.*|Exec=env WAYLAND_DISPLAY= \"$old_path\" --custom %F|; s|^Name=.*|Name=My PhotoCraft|" "$DESKTOP"
printf '\nX-User-Setting=keep me\n' >>"$DESKTOP"
cp "$DESKTOP" "$TMP/custom.desktop"
before="$(stat -c %i "$DESKTOP")"
"$APPDIR/AppRun" >/dev/null
cmp -s "$DESKTOP" "$TMP/custom.desktop" || fail "customized launcher overwritten"
[ "$before" = "$(stat -c %i "$DESKTOP")" ] || fail "custom launcher rewritten although unchanged"
mv "$APPIMAGE" "$TMP/new version.AppImage"
export APPIMAGE="$TMP/new version.AppImage"
"$APPDIR/AppRun" >/dev/null
grep -Fxq "Exec=env WAYLAND_DISPLAY= \"$APPIMAGE\" --custom %F" "$DESKTOP" || fail "custom Exec not relocated"
grep -Fxq 'Name=My PhotoCraft' "$DESKTOP" || fail "custom Name lost"
grep -Fxq 'X-User-Setting=keep me' "$DESKTOP" || fail "custom key lost"

# Packaged updates apply only to untouched keys; user deletions stay deleted.
sed -i '/^Comment=/d' "$DESKTOP"
sed -i 's/^Name=.*/Name=Packaged update/; s/^Categories=.*/Categories=Graphics;/; s/^Comment=.*/Comment=Updated description/' "$APPDIR/usr/share/applications/$APP_ID.desktop"
printf '\nX-New-Packaged-Key=added\n' >>"$APPDIR/usr/share/applications/$APP_ID.desktop"
"$APPDIR/AppRun" >/dev/null
grep -Fxq 'Name=My PhotoCraft' "$DESKTOP" || fail "packaged update erased custom Name"
grep -Fxq 'Categories=Graphics;' "$DESKTOP" || fail "untouched metadata not updated"
grep -q '^Comment=' "$DESKTOP" && fail "user deletion undone"
grep -Fxq 'X-New-Packaged-Key=added' "$DESKTOP" || fail "new packaged key missing"
if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$DESKTOP" || fail "customized entry is invalid"
fi

# A replaced executable may mention the old AppImage as an argument: leave it.
sed -i "s|^Exec=.*|Exec=\"/custom/wrapper\" \"$APPIMAGE\" %F|" "$DESKTOP"
expected_wrapper="$(grep '^Exec=' "$DESKTOP")"
mv "$APPIMAGE" "$TMP/another version.AppImage"
export APPIMAGE="$TMP/another version.AppImage"
"$APPDIR/AppRun" >/dev/null
grep -Fxq "$expected_wrapper" "$DESKTOP" || fail "wrapper argument mistaken for managed executable"

# Existing launchers from before snapshots: bootstrap without losing edits.
rm -f "$XDG_DATA_HOME/applications/.$APP_ID.generated"
cp "$DESKTOP" "$TMP/legacy.desktop"
"$APPDIR/AppRun" >/dev/null
cmp -s "$DESKTOP" "$TMP/legacy.desktop" || fail "legacy custom launcher overwritten"
rm -f "$XDG_DATA_HOME/applications/.$APP_ID.generated"
sed -i 's|^Exec=.*|Exec="/custom/wrapper" %F|' "$DESKTOP"
"$APPDIR/AppRun" >/dev/null
grep -Fxq 'Exec="/custom/wrapper" %F' "$DESKTOP" || fail "legacy executable customization overwritten"

rm -rf "$XDG_DATA_HOME"
PHOTOCRAFT_NO_DESKTOP_INTEGRATION=1 "$APPDIR/AppRun" >/dev/null || fail "opt-out launch exited non-zero"
[ ! -e "$XDG_DATA_HOME" ] || fail "opt-out still integrated"

# 7. An unwritable data dir never blocks the launch.
mkdir -p "$TMP/ro" && chmod a-w "$TMP/ro"
out="$(XDG_DATA_HOME="$TMP/ro/data" "$APPDIR/AppRun" out.txt)" || fail "launch blocked by unwritable data dir"
[ "$out" = "stub: out.txt" ] || fail "stub not run against unwritable data dir: $out"
chmod u+w "$TMP/ro"

# 8. No $APPIMAGE (extracted AppDir): launch only, nothing installed.
rm -rf "$XDG_DATA_HOME"
env -u APPIMAGE "$APPDIR/AppRun" >/dev/null || fail "extracted-AppDir launch exited non-zero"
[ ! -e "$XDG_DATA_HOME" ] || fail "integrated without $APPIMAGE"

# 9. A path that would need Exec escaping (here a `$`): launch only, nothing installed.
rm -rf "$XDG_DATA_HOME"
odd="$TMP/odd\$(id).AppImage"
: >"$odd"
out="$(APPIMAGE="$odd" "$APPDIR/AppRun" x)" || fail "launch with an odd path exited non-zero"
[ "$out" = "stub: x" ] || fail "odd path: $out"
[ ! -e "$XDG_DATA_HOME" ] || fail "integrated an AppImage path that needs escaping"

echo "AppRun desktop integration ok"
