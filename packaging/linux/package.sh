#!/usr/bin/env bash
# Build and package PhotoCraft for Linux (<arch> is x86_64 or aarch64):
#
#   $DIST/photocraft-<version>-linux-<arch>.AppImage  any distro with glibc >= the build host's
#   $DIST/photocraft-<version>-linux-<arch>.AppImage.zsync  delta updates (needs zsyncmake)
#   $DIST/photocraft-<version>-linux-<arch>.deb       Debian, Ubuntu, Mint, Pop!_OS, ...
#   $DIST/photocraft-<version>-linux-<arch>.rpm       Fedora, openSUSE, RHEL, ...
#   $DIST/photocraft-<version>-linux-<arch>.tar.gz    GUI FHS-style tree (bin/, share/)
#   $DIST/photocraft-cli-<version>-linux-<arch>.tar.gz separate headless CLI
#
# Usage: packaging/linux/package.sh [--skip-build] [--formats "appimage deb rpm tar"]
#
# Needs: cargo; nfpm for deb/rpm (https://nfpm.goreleaser.com); appimagetool for the AppImage
# (downloaded into $CARGO_TARGET_DIR if missing). Build on an old distro (CI: Ubuntu 22.04,
# glibc 2.35) so the binaries run on newer ones. Optional: desktop-file-validate, appstreamcli,
# zsyncmake (the zsync package) for the AppImage's .zsync.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=ai.storyteller.photocraft

SKIP_BUILD=0
FORMATS="appimage deb rpm tar"
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    -h | --help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) DEB_ARCH=amd64; TARGET=x86_64-unknown-linux-gnu ;;
  aarch64 | arm64) ARCH=aarch64; DEB_ARCH=arm64; TARGET=aarch64-unknown-linux-gnu ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
export PHOTOCRAFT_MAINTAINER="${PHOTOCRAFT_MAINTAINER:-PhotoCraft maintainers <photocraft@storyteller.ai>}"
BASENAME="photocraft-$VERSION-linux-$ARCH"
CLI_BASENAME="photocraft-cli-$VERSION-linux-$ARCH"

echo "==> PhotoCraft $VERSION for Linux $ARCH ($FORMATS)"

if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --profile native-release --target "$TARGET" --locked -p photocraft -p photocraft-cli --features heif)
fi
BIN="$CARGO_TARGET_DIR/$TARGET/native-release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
CLI_STAGE="$WORK/cli/$CLI_BASENAME"
# Private debug data lives outside DIST and is archived separately by release CI.
SYMBOLS="$CARGO_TARGET_DIR/release-symbols/linux-$ARCH"
rm -rf "$WORK"

# ---- stage an FHS tree (shared by every format) -------------------------------------------------
install -Dm755 "$BIN/photocraft" "$STAGE/usr/bin/photocraft"
install -Dm755 "$BIN/photocraft-cli" "$CLI_STAGE/bin/photocraft-cli"
mkdir -p "$SYMBOLS"
for binary in photocraft photocraft-cli; do
  # Fail closed: missing objcopy, missing debug info extraction or strip failure must not
  # silently ship an unexpectedly large binary or lose diagnostics. Keep the ELF build ID.
  objcopy --only-keep-debug "$BIN/$binary" "$SYMBOLS/$binary.debug"
done
strip --strip-unneeded "$STAGE/usr/bin/photocraft" "$CLI_STAGE/bin/photocraft-cli"
mkdir -p "$CLI_STAGE/share/doc/photocraft-cli"
copy_docs "$CLI_STAGE/share/doc/photocraft-cli"
# The CLI is always a separate download, including when only installers are requested.
tar -C "$WORK/cli" -cf - "$CLI_BASENAME" | gzip -9 >"$DIST/$CLI_BASENAME.tar.gz"
echo "wrote $DIST/$CLI_BASENAME.tar.gz"
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$PHOTOCRAFT_BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/photocraft"
copy_docs "$STAGE/usr/share/doc/photocraft"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi

has() { case " $FORMATS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

# ---- .tar.gz ------------------------------------------------------------------------------------
if has tar; then
  mkdir -p "$WORK/tar"
  cp -R "$STAGE/usr" "$WORK/tar/$BASENAME"
  tar -C "$WORK/tar" -cf - "$BASENAME" | gzip -9 >"$DIST/$BASENAME.tar.gz"
  echo "wrote $DIST/$BASENAME.tar.gz"
fi

# ---- .deb / .rpm --------------------------------------------------------------------------------
if has deb || has rpm; then
  command -v nfpm >/dev/null || { echo "error: nfpm not found (https://nfpm.goreleaser.com/install/)" >&2; exit 1; }
  export VERSION
  export NFPM_ARCH="$DEB_ARCH"
  # nfpm expands env vars in fields like `version` and `arch`, but not in `contents[].src`.
  sed "s|\${STAGE}|$STAGE|g" "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
  for fmt in deb rpm; do
    if has "$fmt"; then (cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p "$fmt" -t "$DIST/$BASENAME.$fmt"); fi
  done
fi

# ---- AppImage -----------------------------------------------------------------------------------
if has appimage; then
  APPDIR="$WORK/PhotoCraft.AppDir"
  cp -R "$STAGE" "$APPDIR"
  ln -s usr/bin/photocraft "$APPDIR/AppRun"
  cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
  cp "$ROOT/assets/app-icon/hicolor/256x256/apps/$APP_ID.png" "$APPDIR/$APP_ID.png"
  ln -s "$APP_ID.png" "$APPDIR/.DirIcon"

  TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  if [ -z "$TOOL" ]; then
    TOOL="$CARGO_TARGET_DIR/appimagetool-$ARCH.AppImage"
    if [ ! -x "$TOOL" ]; then
      curl -fsSL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
      chmod +x "$TOOL"
    fi
  fi
  # Absolute, because appimagetool runs in $DIST below (CARGO_TARGET_DIR or APPIMAGETOOL may be
  # relative, e.g. target/agent-<name>).
  OUT="$(cd "$DIST" && pwd)/$BASENAME.AppImage"
  APPDIR="$(cd "$APPDIR" && pwd)"
  TOOL="$(cd "$(dirname "$TOOL")" && pwd)/$(basename "$TOOL")"
  # A .zsync left by an earlier run would hide a missing zsyncmake and describe another file.
  rm -f "$OUT.zsync"
  # Update information (#349): AppImageUpdate, AppImageLauncher and the like read it from the
  # file and fetch only the blocks that changed in a newer release, through the .zsync published
  # next to each AppImage on GitHub Releases. `latest` is the newest published release that is
  # not a pre-release. A fork's builds point at its own releases through GITHUB_REPOSITORY.
  REPO="${GITHUB_REPOSITORY:-storytold/photocraft}"
  UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|photocraft-*-linux-$ARCH.AppImage.zsync"
  # Extract-and-run: works without FUSE (containers, CI). The output embeds the static runtime,
  # so users don't need libfuse2 either. With zsyncmake on the host (CI installs the zsync
  # package) appimagetool also writes the .zsync, into its working directory, hence the cd.
  (cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream -u "$UPDATE_INFO" "$APPDIR" "$OUT")
  echo "wrote $OUT"
  if [ -s "$OUT.zsync" ]; then
    echo "wrote $OUT.zsync"
  else
    warn "zsyncmake not found, so $OUT.zsync was not written; AppImage delta updates need it"
  fi
fi

"$CLI_STAGE/bin/photocraft-cli" --version
"$STAGE/usr/bin/photocraft" --version
echo "==> done"
ls -lh "$DIST"
