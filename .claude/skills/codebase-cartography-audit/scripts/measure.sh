#!/usr/bin/env bash
# Run every measured step of the audit for the PhotoCraft workspace and save raw output
# to <report>/metrics/. Usage: measure.sh [repo-root] [report-dir]
# Steps can be skipped with SKIP_CLIPPY=1, SKIP_TESTS=1, SKIP_JSCPD=1 (say so in the report).
set -uo pipefail
repo="$(cd "${1:-.}" && pwd)"
report="${2:-$repo/codebase-audit-report}"
m="$report/metrics"
bin="${AUDIT_BIN:-$HOME/.cache/cartography/bin}"
scc="$(command -v scc || echo "$HOME/go/bin/scc")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo/target/agent-audit}"   # AGENTS.md section 6
# Metrics only need test *listings*, not debuggable binaries: no debuginfo, no incremental
# data. This keeps the build within a cloud session's disk allowance (a default debug
# build of this workspace is >10 GB and exhausted it). Delete the target dir afterwards.
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
mkdir -p "$m" "$report/images" "$report/diagrams"
cd "$repo"
rel_report="$(realpath --relative-to="$repo" "$report" 2>/dev/null || basename "$report")"
excl="target,.git,.claude,corpus,node_modules,dist,$rel_report"

log() { echo "+ $*" | tee -a "$m/commands.txt"; }
: > "$m/commands.txt"
{ echo "git: $(git rev-parse HEAD)"; "$scc" --version; rustc --version; cargo clippy --version; } > "$m/versions.txt"

# 1. LOC + complexity proxy, whole repo
log "scc --exclude-dir $excl ."
"$scc" --exclude-dir "$excl" . > "$m/scc-summary.txt"
log "scc --exclude-dir $excl --by-file -f csv ."
"$scc" --exclude-dir "$excl" --by-file -f csv . > "$m/scc-by-file.csv"

# 2. Rollup: categories, inline-test carve-out, McCabe, reconciliation
log "metrics metrics/scc-by-file.csv . metrics/"
"$bin/metrics" "$m/scc-by-file.csv" "$repo" "$m" > /dev/null

# 3. Workspace layering (the onion check PhotoCraft enforces itself)
log "cargo xtask layers"
cargo xtask layers > "$m/layers.txt" 2>&1; echo "exit=$?" >> "$m/layers.txt"
log "cargo metadata --format-version 1 --no-deps"
cargo metadata --format-version 1 --no-deps > "$m/cargo-metadata.json" 2>/dev/null

# 4. Lint: clippy with the never-crash lints + pedantic, grouped by lint id
if [ -z "${SKIP_CLIPPY:-}" ]; then
  lints="-W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::panic -W clippy::indexing_slicing -W clippy::todo -W clippy::unimplemented -W clippy::unreachable"
  log "cargo clippy --workspace --message-format=json -- $lints"
  cargo clippy --workspace --message-format=json -- $lints > "$m/clippy.jsonl" 2> "$m/clippy.stderr"
  grep -o '"code":{"code":"[^"]*"' "$m/clippy.jsonl" | cut -d'"' -f6 | sort | uniq -c | sort -rn > "$m/clippy-by-lint.txt"
fi

# 5. Duplication (clone blocks) over Rust sources
if [ -z "${SKIP_JSCPD:-}" ]; then
  log "npx jscpd@4.0.5 --min-lines 12 --format rust --ignore '**/target/**,**/corpus/**' --reporters json -o metrics/jscpd crates apps xtask"
  npx -y jscpd@4.0.5 --min-lines 12 --format rust --ignore "**/target/**,**/corpus/**" \
    --reporters json,console --silent -o "$m/jscpd" crates apps xtask > "$m/jscpd-console.txt" 2>&1
fi

# 6. Test counts from the runner, per package (default features: corpus tests are opt-in)
if [ -z "${SKIP_TESTS:-}" ]; then
  : > "$m/test-list.txt"
  for d in crates/* apps/photocraft apps/photocraft-cli xtask; do
    [ -f "$d/Cargo.toml" ] || continue
    name="$(grep -m1 '^name' "$d/Cargo.toml" | cut -d'"' -f2)"
    log "cargo test -p $name -- --list --format terse"
    { echo "### PACKAGE $name $d"; cargo test -p "$name" -- --list --format terse 2>&1;
      echo "### IGNORED $name $d"; cargo test -p "$name" -- --list --format terse --ignored 2>&1; } >> "$m/test-list.txt"
  done
  log "testlist metrics/test-list.txt ."
  "$bin/testlist" "$m/test-list.txt" "$repo" > "$m/test-suites.md"
fi

echo "done: $m"
