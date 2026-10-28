#!/usr/bin/env bash
# Package isolated fixture trees: matching hashes, independent hashes, ambiguous glue.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/photocraft-web-package-test.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT
mkdir -p "$work_dir/packaging/web" "$work_dir/dist/web" "$work_dir/dist/web-boot"
cp "$HERE/package.sh" "$HERE/.htaccess" "$HERE/_headers" "$HERE/README.md" "$work_dir/packaging/web/"
cp "$HERE/../env.sh" "$work_dir/packaging/env.sh"
cat >"$work_dir/Cargo.toml" <<'TOML'
[workspace.package]
version = "0.0.0"
TOML
printf '<html>app fixture</html>\n' >"$work_dir/dist/web/index.html"
printf '<html>boot fixture</html>\n' >"$work_dir/dist/web-boot/index.html"
printf 'wasm fixture\n' >"$work_dir/dist/web/photocraft-web-wasmhash_bg.wasm"
mkdir -p "$work_dir/dist/web/snippets/example"
printf 'generated snippet fixture\n' >"$work_dir/dist/web/snippets/example/inline0.js"
export PHOTOCRAFT_BUILD_SHA=test PHOTOCRAFT_BUILD_DATE=2026-10-27
export CARGO_TARGET_DIR="$work_dir/target" DIST="$work_dir/dist/release"
for js_hash in wasmhash independenthash; do
  printf 'glue fixture %s\n' "$js_hash" >"$work_dir/dist/web/photocraft-web-$js_hash.js"
  bash "$work_dir/packaging/web/package.sh" --skip-build >"$work_dir/run.log" 2>&1
  unzip -p "$DIST/photocraft-web-0.0.0.zip" photocraft-web-0.0.0/app/photocraft-web.js >"$work_dir/actual"
  cmp "$work_dir/actual" "$work_dir/dist/web/photocraft-web-$js_hash.js"
  unzip -p "$DIST/photocraft-web-0.0.0.zip" photocraft-web-0.0.0/app/snippets/example/inline0.js >"$work_dir/snippet"
  cmp "$work_dir/snippet" "$work_dir/dist/web/snippets/example/inline0.js"
  mv "$work_dir/dist/web/photocraft-web-$js_hash.js" "$work_dir/previous-glue"
done
printf 'first glue fixture\n' >"$work_dir/dist/web/photocraft-web-one.js"
printf 'second glue fixture\n' >"$work_dir/dist/web/photocraft-web-two.js"
if bash "$work_dir/packaging/web/package.sh" --skip-build >"$work_dir/run.log" 2>&1; then
  echo 'error: ambiguous app glue was accepted' >&2
  exit 1
fi
case "$(cat "$work_dir/run.log")" in
  *"more than one app glue file"*) ;;
  *) cat "$work_dir/run.log" >&2; exit 1 ;;
esac
echo 'web packaging: matching/independent hashes pass; ambiguous glue rejected'
