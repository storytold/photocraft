#!/bin/bash
# PhotoCraft Capture: a screenshot into the PhotoCraft screenshot library, without PhotoCraft open.
#
# Shows the macOS crosshair (drag a region, Space for a window, Esc to cancel), saves the shot as
# "YYYY-MM-DD HH-MM-SS.png" in the library (the folder chosen in PhotoCraft's Screenshot Library,
# else ~/Pictures/PhotoCraft Screenshots), then opens it in PhotoCraft, which makes the editable
# .pcraft copy beside it. Bind it to a key with the Shortcuts app ("Open App").
set -u

# The app to open captures in (the first one installed).
for candidate in "PhotoCraft Dev" "PhotoCraft"; do
  for root in /Applications "$HOME/Applications"; do
    if [ -d "$root/$candidate.app" ]; then
      APP="$candidate"
      APP_PATH="$root/$candidate.app"
      break 2
    fi
  done
done

# PhotoCraft's settings folder, by the app's own rules (apps/photocraft/src/app_dirs.rs):
# PHOTOCRAFT_CONFIG_DIR when set; else portable mode, a marker file beside the app's executable
# putting everything in <exe dir>/PhotoCraftData; else ~/Library/Application Support/Photocraft.
EXE_DIR="${APP_PATH:-}/Contents/MacOS"
if [ -n "${PHOTOCRAFT_CONFIG_DIR:-}" ]; then
  CONFIG="$PHOTOCRAFT_CONFIG_DIR"
elif [ -n "${APP_PATH:-}" ] && { [ -e "$EXE_DIR/portable.txt" ] || [ -e "$EXE_DIR/PhotoCraft.portable" ]; } && [ -d "$EXE_DIR/PhotoCraftData" ]; then
  CONFIG="$EXE_DIR/PhotoCraftData"
else
  CONFIG="$HOME/Library/Application Support/Photocraft"
fi
# The library folder PhotoCraft uses (preferences.json › dialogs › screenshot.library › folder).
PREFS="$CONFIG/preferences.json"
DIR=""
if [ -f "$PREFS" ]; then
  DIR=$(/usr/bin/osascript -l JavaScript -e '
    function run(argv) {
      try {
        const text = $.NSString.stringWithContentsOfFileEncodingError(argv[0], $.NSUTF8StringEncoding, null).js;
        const p = JSON.parse(text);
        const d = p.dialogs && p.dialogs["screenshot.library"];
        return (d && d.folder) || "";
      } catch (e) { return ""; }
    }' "$PREFS" 2>/dev/null)
fi
[ -n "$DIR" ] || DIR="$HOME/Pictures/PhotoCraft Screenshots"
mkdir -p "$DIR" || exit 1

# A name from the local date and time, unique if two shots land in the same second.
STAMP=$(/bin/date "+%Y-%m-%d %H-%M-%S")
NAME="$STAMP"
n=2
while [ -e "$DIR/$NAME.png" ] || [ -e "$DIR/$NAME.pcraft" ]; do
  NAME="$STAMP $n"
  n=$((n + 1))
done
FILE="$DIR/$NAME.png"

/usr/sbin/screencapture -i -x -t png "$FILE" || exit 0
# Esc cancels without writing a file.
[ -f "$FILE" ] || exit 0

if [ -n "${APP:-}" ]; then
  /usr/bin/open -a "$APP" "$FILE"
else
  /usr/bin/open "$FILE"
fi
