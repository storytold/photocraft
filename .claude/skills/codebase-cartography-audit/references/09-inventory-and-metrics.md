# Module 9 — Inventory, Metrics & Linting (tool-driven)

This module produces the **objective, measured** half of the audit: lines of
code, cyclomatic complexity, and lint/analyzer findings — gathered by running
real command-line tools, not by eyeballing. Every number in the report must be
reproducible by re-running a command that is recorded in the report.

## 9.1 Principle

- **Every objective number comes from an executable tool, never from inference.**
  LOC, type/function counts, cyclomatic complexity, test counts, lint counts,
  duplication, function points, and cost are all produced by a command or a
  committed helper (`scripts/*.rs`) whose output is captured in `metrics/`. Do NOT eyeball a
  directory and assert "~600 tests" or "about 300 types" — run the tool and
  quote its number with the command. Only the *qualitative* defect findings are
  analyst-authored, and each still cites `file:line`. If a number cannot be
  produced by a tool, label it explicitly as an estimate and show the method.
- **Test counts come from the test runner, not from counting `#[test]` attributes
  by hand.** Attribute-grepping misses macro-generated tests, `rstest` cases and
  doc-tests, and cannot tell ignored tests apart. Use the runner's own enumeration:
  `cargo test -p <pkg> -- --list --format terse` (one `name: test` line per test,
  grouped under `Running ...` / `Doc-tests ...` headers), plus `--ignored` for the
  `#[ignore]` set. `measure.sh` does this per package and `testlist` turns it into
  the suite matrix. Count what the runner reports, per package/layer.
- **Helpers are single-file, std-only Rust.** Any custom analysis logic (rollups,
  McCabe counting, function-point math, parsing tool output) lives in
  `scripts/metrics.rs`, `scripts/sizing.rs`, `scripts/testlist.rs`, compiled with
  plain `rustc -O --edition 2021` (no Cargo project, so nothing joins the
  workspace, and no extra runtime). This matches PhotoCraft's Rust-only rule. Shell
  is fine for orchestration glue (`setup-tools.sh`, `measure.sh`), never for the
  metric/FP computation.
- **Right tool for the stack.** Detect the languages first (Module: architecture
  map), then pick tools that actually parse those languages. A polyglot repo
  needs a polyglot LOC tool plus per-language complexity/lint tools.
- **Component-level rollups.** Aggregate file metrics up to the architectural
  components (projects/modules/packages) so they can annotate the logical C4
  diagram (Module 10) and rank refactor risk.

## 9.2 Tool selection matrix

| Need | PhotoCraft (Rust) tool | Notes |
|------|------------------------|-------|
| LOC + file complexity proxy | **scc 3.5.0** | Lines/Code/Comments/Complexity per file and language |
| Cyclomatic complexity | **`metrics` helper** (§9.4) | scc's Complexity column is a keyword proxy; the helper strips comments/strings and counts real decisions. `clippy::cognitive_complexity` is a different metric; don't mix them |
| Lint | **`cargo clippy`** with `-W clippy::pedantic` and the never-crash restriction lints (`unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `todo`, `unimplemented`, `unreachable`) | `clippy.toml` already allows these in tests, so hits are non-test code; group by lint id |
| Layering | **`cargo xtask layers`** | PhotoCraft's own dependency-direction check |
| Duplication | **jscpd 4.0.5** via `npx` (`--format rust`) | clone blocks; scc `--dup` only drops whole-file duplicates |
| Tests | **`cargo test -p <pkg> -- --list`** | per package; `--ignored` for ignored tests |

Other languages in the repo (shell, PowerShell, Python, YAML, TOML in `scripts/`,
`packaging/`, `.github/`) are DevOps assets; scc's numbers are enough for them.

**scc** is the workhorse for LOC + a fast complexity proxy across every language
in one pass and needs no runtime. Use it first, always. Then add a per-language
complexity/lint tool for the dominant language(s).

### Acquiring tools (this container: `scripts/setup-tools.sh`, record every step)
- `scc`: `go install github.com/boyter/scc/v3@v3.5.0` (Go and proxy.golang.org are
  available here; GitHub release binaries also work).
- Graphviz: `apt-get install -y graphviz`.
- PlantUML: `plantuml-1.2025.4.jar` from the plantuml GitHub release into
  `~/.local/share/plantuml/`; the JRE is preinstalled.
- jscpd: `npx -y jscpd@4.0.5` (npm cache only; never `npm i` in the repo).
- Helpers: `rustc -O --edition 2021 scripts/<name>.rs -o ~/.cache/cartography/bin/<name>`.
- Never modify the target repo to install a tool. Exclude `target/`, `corpus/`,
  `.git/`, `.claude/` and the report directory from every metric run and say so.

## 9.3 Commands (record these verbatim in the report)

All of these are run by `scripts/measure.sh`, which also logs them to
`metrics/commands.txt`:

```bash
EX=target,.git,.claude,corpus,node_modules,dist,codebase-audit-report
# LOC, whole repo (not just crates/) so root build/deploy assets are counted (9.3b)
scc --exclude-dir $EX . > metrics/scc-summary.txt
scc --exclude-dir $EX --by-file -f csv . > metrics/scc-by-file.csv
# rollup: categories, inline-test carve-out, McCabe, reconciliation (9.4)
metrics metrics/scc-by-file.csv . metrics/
# layering
cargo xtask layers > metrics/layers.txt
# lint, grouped by lint id
cargo clippy --workspace --message-format=json -- -W clippy::pedantic -W clippy::unwrap_used \
  -W clippy::expect_used -W clippy::panic -W clippy::indexing_slicing -W clippy::todo \
  -W clippy::unimplemented -W clippy::unreachable > metrics/clippy.jsonl
# duplication
npx -y jscpd@4.0.5 --min-lines 12 --format rust --ignore "**/target/**,**/corpus/**" \
  --reporters json,console -o metrics/jscpd crates apps xtask
# tests, per package
cargo test -p <pkg> -- --list --format terse      # and again with --ignored
testlist metrics/test-list.txt . > metrics/test-suites.md
```

Always capture: tool name, version, the exact command, and the raw output path.

> `--exclude-dir` only removes whole directories; generated files and non-code
> languages (Markdown, SVG, JSON, plain text) are excluded **by `metrics` when it
> classifies files** — the raw scc total is NOT the code total.

## 9.3b Two LOC categories: System vs DevOps/Test

Classify **all** code — including tests and build/deploy assets — into exactly two
categories, and report a subtotal for each plus a grand total (for LOC, types,
CC, and downstream FP and cost):

- **System code** — the product runtime delivered to end users (the application
  projects/modules).
- **DevOps/Test code** — everything that supports building, verifying, migrating,
  and deploying the system: **all test suites**, **build/deploy scripts**
  (`*.ps1`, `*.yml`, `Dockerfile`, `docker-compose`, `*.cake`, CI YAML),
  database-migration/deploy projects, and one-off tooling utilities. These are
  DevOps assets, not shipped functionality.

**PhotoCraft mapping (implemented in `metrics.rs` `classify()`):** System =
`crates/<n>/src/**` (except `testkit` and `tests.rs`/`*_tests.rs` files) and
`apps/<n>/**` outside `tests/`; DevOps/Test = `crates/<n>/{tests,benches,examples,fuzz}`,
**inline `#[cfg(test)] mod` blocks carved out of System files** (credited to the
`crates/<n> (tests)` row), `crates/testkit`, `xtask/`, every `Cargo.toml`/`build.rs`,
`.github/`, `scripts/`, `packaging/`, `perf/`, `scorecard/`, `examples/` and root
config; Excluded = `docs/`, `book/`, Markdown/SVG/JSON/plain-text/licence files and
Rust files whose header marks them generated. The inline-test carve-out matters:
PhotoCraft keeps many unit tests in `src/`, so without it System LOC would be
overstated by tens of thousands of lines.

Run scc over the **whole repository** (not just `src/`) so root-level build/deploy
scripts are counted; then map each file to a category. The System/DevOps-Test LOC
split is itself a finding — a test+DevOps estate larger than the product is common
and worth calling out. Keep the classification list explicit and auditable in the
script, and note any judgment calls (e.g. whether a data-load utility is product
or tooling).

**Reconciliation rule (mandatory — this is where LLMs go wrong).** Every LOC in a
category subtotal MUST be attributable to a **named component line item**, and each
subtotal MUST equal the sum of its listed components. Compute subtotals *as* the
sum of the rows — never emit a subtotal larger than the visible rows. If a
whole-repo walk adds LOC that no component claims (a "root/misc" bucket), that
bucket must appear as its own explicit line item; a large unexplained gap (e.g.
tens of thousands of LOC not shown against any component) is a **bug to fix, not a
number to report** — it silently inflates FP and cost downstream. Print a
reconciliation check: `Σ(components) == subtotal` per category and overall.

**Exclude non-authored code from the LOC tally** (or list it in a clearly-separate
"excluded" bucket, never inside System or DevOps/Test): vendored/third-party
(`node_modules`, `packages`, `wwwroot/lib`, `Scripts` vendored libs), **minified**
(`*.min.js`, `*.min.css`), **generated** (Rust files headed `@generated`/
`Generated by`, `target/`, build-script output, measured lookup tables), and pure
data/config blobs (large `.json`/`.sql` seed dumps). Flag any single file or
directory contributing >~2,000 LOC and confirm it is authored source before
counting it — a lone generated/minified/seed file can add tens of thousands of
phantom LOC and wreck the totals.

## 9.4 Rolling up for the annotated diagram (`scripts/metrics.rs`)

`metrics <scc-by-file.csv> <repo> <out-dir>` already does all of this; know what it
does so you can explain and, if needed, adjust it:
1. Parses `scc-by-file.csv`.
2. Maps each file to its component (`crates/<n>`, `apps/<n>`, `xtask`, a DevOps
   bucket) and category (§9.3b), splitting inline `#[cfg(test)] mod` blocks out of
   System files.
3. Sums `Code` (LOC) per component and computes **McCabe cyclomatic complexity**
   for every `.rs` file itself rather than trusting scc's keyword proxy: it blanks
   comments (incl. nested block comments), string/raw-string/char literals (lifetimes
   are kept apart from char literals), then `CC = max(fn_count,1) + decisions` where a
   decision is `if | while | for` (loop `for` only - not `impl Trait for T` or
   `for<'a>`), each `match` arm beyond the first (`=>` count minus `match` count), and
   binary `&&`/`||` (not `&&x` references or `|| expr` closures). `else if` counts
   via its `if`. `?` early returns are counted in a separate column, not in CC. This
   is **extended (CC2-style) McCabe**; `=>` arms in `macro_rules!` are included (a
   small overstatement) - state both caveats. Non-Rust files use scc's proxy.
4. Counts **types** per component (`struct|enum|trait` declarations; Rust has no
   classes) and reports **average CC per type** alongside CC per fn. Note that
   behaviour lives in `impl` blocks and free functions, so CC/fn is usually the more
   telling ratio in Rust.
5. Computes **complexity density** = total CC ÷ code LOC (size-independent
   hotspot signal).
6. Emits `component-rollup.txt` + `.json` with
   `component, category(System|DevOps/Test|Excluded), files, LOC, types,
   fns, CC, CC-per-type, CC-per-fn, density, ?-count, hottestFile(maxCC)`, **per-category subtotal
   rows** (System, DevOps/Test, and an **Excluded (non-authored)** row listing what
   was removed and why), then the GRAND TOTAL, the System/DevOps-Test LOC split %,
   and a top-15 most-complex-files list. Print the reconciliation identity
   explicitly: **Σ(System) + Σ(DevOps/Test) + Σ(Excluded) == whole-repo scc code
   total** — this is the clean check that the whole-repo scan and the categories
   agree, with no phantom LOC.
7. Feeds LOC + CC into the logical diagram box labels (Module 10.4).

It is one std-only file; `setup-tools.sh` compiles it. The skill directory holds
the source, and `metrics/commands.txt` records the run, so the numbers are
reproducible. If you change the classification or counting, say so in the report.

## 9.5 Interpreting the numbers (don't just print them)

- **Complexity per function > 10** = review; **> 20** = high risk / likely a god
  function. The rollup is per file; a file with CC in the hundreds is a god module
  or a mega-`match` — open it and find the worst functions (cross-link Modules 1
  (SRP/OCP) and 4). In egui code a long immediate-mode `ui()` function is the usual
  shape of this.
- **High LOC + high density component** = the refactor-risk epicenter; it should
  top the backlog and needs characterization tests first (Module 8).
- **Lint volume** is a proxy for consistency debt; group analyzer hits by rule id
  and report the top offenders, not the raw wall of warnings.
- **Duplication blocks** map straight to the Duplicated Code smell (Module 4);
  quote the two largest clone sites.

**Inspection prompt**
> Run `scripts/setup-tools.sh` and `scripts/measure.sh` for {scope}; record how
> each tool was obtained and its version. Run them excluding build/
> vendor output, saving raw output to `metrics/`. Produce (every number from a
> tool, not inference): (a) a language breakdown table; (b) a per-component
> rollup with files, LOC, type count, fns, total McCabe CC, **average CC per
> type and per fn**, and density; (c) the top 10 most-complex files; (d) a lint summary
> grouped by rule; (e) the largest duplication clones; (f) **test counts per
> layer from the test runner** (`cargo test -- --list`), not from attribute
> grepping. Cite every command so the numbers are reproducible.
