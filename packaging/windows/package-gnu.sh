#!/usr/bin/env bash
# Unsigned x64 development installer, cross-compiled on Linux with MinGW + NSIS.
# Official releases use package.ps1 (MSVC, WiX and optional signing).
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
SKIP_BUILD=0
if [ "${1:-}" = --skip-build ]; then SKIP_BUILD=1; shift; fi
if [ "$#" != 0 ]; then echo 'Usage: package-gnu.sh [--skip-build]' >&2; exit 2; fi
TARGET=x86_64-pc-windows-gnu
NUMERIC_VERSION="${VERSION%%-*}"
if [[ ! "$NUMERIC_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo 'VERSION must be a semantic version' >&2; exit 2
fi
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
export PHOTOCRAFT_REQUIRE_WINRES=1
if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release --locked -p photocraft -p photocraft-cli --features heif --target "$TARGET")
fi
BIN="$CARGO_TARGET_DIR/$TARGET/release"
STAGE="$CARGO_TARGET_DIR/windows-gnu-package/$VERSION"
mkdir -p "$STAGE"
# NSIS changes to its script directory; resolve caller-relative build/output paths first.
STAGE="$(cd "$STAGE" && pwd)"
DIST="$(cd "$DIST" && pwd)"
install -m644 "$BIN/photocraft.exe" "$BIN/photocraft-cli.exe" "$STAGE/"
x86_64-w64-mingw32-strip "$STAGE/photocraft.exe" "$STAGE/photocraft-cli.exe"
python3 - "$STAGE" "$ROOT" "$VERSION" <<'PY'
import json, os, struct, sys
from pathlib import Path
stage, root = map(Path, sys.argv[1:3])
for name, subsystem in [('photocraft.exe', 2), ('photocraft-cli.exe', 3)]:
    data = (stage / name).read_bytes()
    if len(data) < 64 or data[:2] != b'MZ':
        raise SystemExit(f'{name}: invalid executable')
    offset = struct.unpack_from('<I', data, 60)[0]
    if offset + 94 > len(data) or data[offset:offset+4] != b'PE\0\0':
        raise SystemExit(f'{name}: invalid PE header')
    if struct.unpack_from('<H', data, offset+4)[0] != 0x8664 or struct.unpack_from('<H', data, offset+92)[0] != subsystem:
        raise SystemExit(f'{name}: wrong architecture or subsystem')
licenses = [root / 'LICENSE-MIT', root / 'LICENSE-APACHE']
nsis_license = Path('/usr/share/doc/nsis/copyright')
if nsis_license.is_file():
    licenses.append(nsis_license)
fonts = os.environ.get('CRAFT_FONTS_DIR')
if fonts:
    licenses += sorted(Path(fonts).glob('fonts/*/OFL.txt'))
(stage / 'LICENSES.txt').write_text('\n\n'.join(p.name + '\n' + p.read_text() for p in licenses))
info = {'product': 'PhotoCraft', 'packageVersion': sys.argv[3], 'target': 'x86_64-pc-windows-gnu',
        'buildSource': os.environ.get('PHOTOCRAFT_BUILD_SHA', ''),
        'buildDate': os.environ.get('PHOTOCRAFT_BUILD_DATE', ''), 'signature': 'unsigned'}
(stage / 'BUILD-INFO.json').write_text(json.dumps(info, indent=2) + '\n')
PY
# Rust and the MinGW support libraries must be linked statically: do not silently
# ship an executable requiring an unbundled compiler runtime DLL.
for name in photocraft.exe photocraft-cli.exe; do
  if x86_64-w64-mingw32-objdump -p "$STAGE/$name" | grep -Ei 'DLL Name:.*(libgcc|libstdc\+\+|libwinpthread)' >/dev/null; then
    echo "error: $name requires an unbundled MinGW runtime DLL" >&2; exit 1
  fi
done
BASENAME="photocraft-$VERSION-windows-x64"
makensis -WX -DVERSION="$NUMERIC_VERSION" -DDISPLAY_VERSION="$VERSION" \
  -DBIN_DIR="$STAGE" -DOUT_FILE="$DIST/$BASENAME-setup.exe" \
  -DICON_FILE="$ROOT/assets/app-icon/photocraft.ico" "$ROOT/packaging/windows/photocraft-dev.nsi"
# Portable mode is activated only in the ZIP; installed copies use normal settings.
printf 'PhotoCraft portable mode\n' >"$STAGE/portable.txt"
OUT_ZIP="$DIST/$BASENAME-portable.zip"
python3 - "$STAGE" "$OUT_ZIP" <<'PY'
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile
import sys
stage = Path(sys.argv[1])
with ZipFile(sys.argv[2], 'w', compression=ZIP_DEFLATED, compresslevel=9) as archive:
    for name in ['photocraft.exe', 'photocraft-cli.exe', 'LICENSES.txt', 'BUILD-INFO.json', 'portable.txt']:
        archive.write(stage / name, name)
PY
echo "wrote $DIST/$BASENAME-setup.exe"
echo "wrote $OUT_ZIP"
