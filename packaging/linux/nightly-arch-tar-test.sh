#!/usr/bin/env bash
# Validate the staged Arch launcher without building or opening PhotoCraft.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="$work/source"
dst="$work/dist"
base=photocraft-0.0.1-nightly.1-linux-x86_64
mkdir -p "$stage/bin" "$dst" "$work/extract"

# A local argv recorder stands in for the compiled binaries. It never opens a UI.
cat >"$stage/bin/photocraft" <<'RECORDER'
#!/bin/sh
printf '<%s>\n' "$@"
RECORDER
chmod 755 "$stage/bin/photocraft"
cp "$stage/bin/photocraft" "$stage/bin/photocraft-cli"

bash "$HERE/nightly-arch-tar.sh" "$stage" "$work/stage" "$dst" "$base"
tar -xzf "$dst/$base.tar.gz" -C "$work/extract"
bin="$work/extract/$base/bin"
# Simulate the existing Arch bin-tree copy and public launcher rename.
mkdir -p "$work/installed"
cp -R "$bin" "$work/installed/bin"
bin="$work/installed/bin"
mv "$bin/photocraft" "$bin/photocraft-nightly"
mv "$bin/photocraft-cli" "$bin/photocraft-nightly-cli"

test ! -e "$stage/bin/.photocraft-nightly"
cmp "$stage/bin/photocraft" "$bin/.photocraft-nightly/photocraft"
cmp "$stage/bin/photocraft-cli" "$bin/photocraft-nightly-cli"
export HOME="$work/home with space"
unset XDG_DATA_HOME
expect() {
  expected="$1"
  shift
  actual=$(PATH="$bin:$PATH" photocraft-nightly "$@")
  if [[ "$actual" != "$expected" ]]; then
    printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  fi
}
expect "$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$HOME/.local/share/photocraft-nightly" --)"
export XDG_DATA_HOME="$work/xdg with space"
expect "$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$XDG_DATA_HOME/photocraft-nightly" -- 'image with spaces.psd' '--other-name.psd')" \
  'image with spaces.psd' '--other-name.psd'
expect '<--version>' --version
expect '<-V>' -V
expect "$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$work/explicit with space" -- sample.psd)" \
  --launch-profile nightly-v1 --profile-root "$work/explicit with space" -- sample.psd

echo "PASS: Arch Nightly handoff isolates the profile and preserves launch arguments"
