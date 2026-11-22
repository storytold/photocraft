//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). External tools (`cargo`, `curl`, `tar`) are
//! invoked through `std::process::Command`.

mod cache_units;
mod cargo_env;
mod corpus;
mod corpus_pins;
#[cfg(feature = "framework-checks")]
mod i18n_report;
mod ico;
mod layers;
mod panel_commands;
mod perf;
mod pinned;
mod scorecard;
mod shards;
mod stats;
mod test_binaries;
mod version;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use xtask::{psd_write_oracle, sha256};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  layers          enforce the crate dependency layering (docs/architecture.md §3)
  egui-fork       fail unless Cargo.lock takes every egui package from the craft framework's egui fork
  panel-commands  fail on a shell click handler that changes the document or session directly instead of
                  dispatching a command id (allowlist: xtask/src/panel_commands.rs)
  test-binaries   fail on a single-file test binary (crates/*/tests/*.rs): integration tests are modules of tests/it/main.rs
                  (allowlist: xtask/src/test_binaries.rs)
  wasm            cargo check --target wasm32-unknown-unknown for the wasm-safe crates (one call, plus
                  one per optional web feature)
  test            workspace tests (cargo nextest when installed, else cargo test), then doctests
  ci [lint | archive [--gpu-shard] | build | tests [--from <dir> [--shard <k>]] [--gpu | --no-gpu] | <step>]
                  the CI gate: fmt --check, egui-fork, clippy -D warnings, layers, panel-commands, scorecard --check,
                  tests (nextest --profile ci), panic_hunt, doctests, wasm and the i18n-report, with --locked
                  --features heif (stops at first failure). The gate splits it over its jobs: `lint` runs the
                  steps that need no test build; `archive` builds the tests into the nextest archive at
                  $NEXTEST_ARCHIVE_FILE (the gate's build job, which then runs `doctest`), or with $NEXTEST_ARCHIVE_DIR one
                  archive per gate shard ($GATE_SHARDS of them, at $NEXTEST_ARCHIVE_DIR/<k>/$NEXTEST_ARCHIVE_NAME), each
                  with only the test binaries that shard runs, balanced by .config/ci-test-times.tsv (`--gpu-shard`: shard 1
                  takes the binaries with gpu-group tests); `tests --from <dir>` runs the tests from that archive
                  extracted to <dir> (without --from it builds them; `--shard <k>`: shard k's part of the split binaries,
                  from the plan the archive carries); `tests` runs the tests and panic_hunt on the partition in
                  $NEXTEST_PARTITION (`--gpu`: all of nextest's gpu test group, whatever the partition, each
                  test's output shown;
                  `--no-gpu`: every other test); `doctest` runs the doctests; `build` only builds what `tests`
                  builds. $PHOTOCRAFT_CI_EXCLUDE (packages separated by spaces) leaves
                  packages out of the test build and the doctests
  corpus [--all | --pngsuite | --psd | --psd-tools | --pixls | --atn | --photoshop | --gimp] [--local] [--update-manifest]
                  show where test corpora live and their pins (xtask/src/corpus_pins.rs), or fetch
                  them into corpus/ (pinned commits, sha256-verified; --all = every corpus but pixls,
                  --photoshop/--gimp --local copy from ../photocraft-corpus or $PHOTOCRAFT_CORPUS_REPO)
  test-corpus [-p <crate>]... [--changed] [--pixls] [--local] [-- <test args>]
                  fetch every corpus, then cargo test --release --features corpus on the corpus
                  crates; --changed runs only if psd/io/codecs/compose/gpu/text/format/raw changed;
                  --pixls (or -p raw) also fetches corpus/pixls and runs the raw corpus tests;
                  --local takes corpus/photoshop and corpus/gimp from the photocraft-corpus authoring clone
  stats [--exact] count tests and lines per crate (--exact: ask the test harness via `-- --list`)
  parity          Photoshop menu parity; rewrites docs/parity-checklist.md
  i18n-report     report UI translation coverage, the missing strings and the rows kept in English per
                  registered language, from the craft-i18n checkout in CRAFT_I18N_DIR (the last `ci`
                  step; it never fails, and says so when CRAFT_I18N_DIR is unset)
  perf [--quick] [--update-baseline] [--threshold PCT] [--bench NAME]... [--skip-build] [--reuse]
                  run the release benches, merge them by scenario id into target/perf/results.json,
                  check perf/budgets.toml and perf/baseline.json (non-zero on a broken budget or regression)
  scorecard [--check]
                  regenerate docs/scorecard.md (--check: fail if it is stale)
  version [set X.Y.Z[-pre]]
                  print the workspace version, or set it (Cargo.toml + Cargo.lock)
  ico <out.ico> <in.png>...
                  pack square PNGs (<= 256 px) into a Windows .ico (see packaging/icons.sh)
  psd-write-oracle <recipes.json> <corpus-dir> <photocraft-cli> <new-output-dir>
                  run PSD export recipes and write a JSON report and manifest
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("layers") => cmd_layers(),
        Some("egui-fork") => cmd_egui_fork(),
        Some("panel-commands") => panel_commands::run(&root()),
        Some("test-binaries") => test_binaries::run(&root()),
        Some("wasm") => cmd_wasm(),
        Some("test") => cmd_test().and_then(|()| cmd_doctest()),
        Some("ci") => ci_from_args(&rest, ci_exclude()).and_then(|(plan, options)| cmd_ci(&plan, &options)),
        Some("corpus") => corpus::cmd(&rest),
        Some("test-corpus") => corpus::test_cmd(&rest),
        Some("stats") => stats::run(&root(), rest.contains(&"--exact")),
        Some("parity") => cmd_parity(),
        Some("i18n-report") => cmd_i18n_report(),
        Some("perf") => perf::run(&root(), &rest),
        Some("scorecard") => scorecard::run(&root(), &rest),
        Some("version") => version::run(&root(), &rest),
        Some("ico") => ico::run(&rest),
        Some("psd-write-oracle") => psd_write_oracle::run(&root(), &rest),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (parent of the xtask crate).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask has a parent dir").to_path_buf()
}

/// A cargo command in the workspace root, without the variables `cargo run` set for xtask.
pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    cargo_env::without_cargo_run_vars(&mut c);
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

/// The workspace's cargo target directory.
pub fn target_dir() -> Result<PathBuf, String> {
    let m = metadata()?;
    m.get("target_directory").and_then(serde_json::Value::as_str).map(PathBuf::from).ok_or_else(|| "cargo metadata: no target_directory".into())
}

/// Two egui revisions in one build give two incompatible `Ui` types, so the workspace's egui
/// `[patch]` must name the revision the craft framework is built with.
#[cfg(feature = "framework-checks")]
fn cmd_egui_fork() -> Result<(), String> {
    let path = root().join("Cargo.lock");
    let lock = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    craft::egui_fork::check_lockfile(&lock)?;
    println!("OK: every egui package comes from {} at {}.", craft::egui_fork::GIT, craft::egui_fork::REV);
    Ok(())
}

#[cfg(feature = "framework-checks")]
fn cmd_i18n_report() -> Result<(), String> {
    i18n_report::run(&root())
}

// An explicitly minimal xtask can still run either check when asked for it.
#[cfg(not(feature = "framework-checks"))]
fn run_framework_check(name: &str) -> Result<(), String> {
    let mut c = cargo();
    c.args(["run", "--locked", "-p", "xtask", "--features", "framework-checks", "--", name]);
    run(c, name)
}

#[cfg(not(feature = "framework-checks"))]
fn cmd_egui_fork() -> Result<(), String> {
    run_framework_check("egui-fork")
}

#[cfg(not(feature = "framework-checks"))]
fn cmd_i18n_report() -> Result<(), String> {
    run_framework_check("i18n-report")
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("Dependency layering (docs/architecture.md §3)\n");
    println!("{:<28} {:<14} workspace deps", "crate", "layer");
    for c in &crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", layers::short_name(&d.name))
            })
            .collect();
        println!("{:<28} {:<14} {}", c.name, layers::describe(layers::classify(&c.name)), ws.join(", "));
    }
    let violations = layers::check(&crates);
    println!();
    if violations.is_empty() {
        println!("OK: {} crates, no layering violations.", crates.len());
        Ok(())
    } else {
        println!("{} violation(s):", violations.len());
        for v in &violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

/// Workspace packages that must build for wasm32: all L0–L5 crates plus
/// the egui shell.
fn wasm_set() -> Result<Vec<String>, String> {
    let crates = layers::from_metadata(&metadata()?)?;
    Ok(crates
        .into_iter()
        .filter(|c| match layers::classify(&c.name) {
            Some(layers::Class::Layer(l)) => l <= 5 || layers::short_name(&c.name) == "ui-egui",
            Some(layers::Class::Standalone) => true,
            _ => false,
        })
        .map(|c| c.name)
        .collect())
}

/// Optional features that official builds enable and the web app ships, also checked for wasm32:
/// (package, feature).
const WASM_FEATURES: &[(&str, &str)] = &[("photocraft-io", "heif")];

/// One `cargo check` for the whole set: each crate is checked once, and `--keep-going` still
/// reports every failing crate. Features unify across the set, so each crate is checked with the
/// union of the features the set enables. The optional features get a call each, so the crates
/// are also checked without them.
fn cmd_wasm() -> Result<(), String> {
    let set = wasm_set()?;
    if set.is_empty() {
        return Err("no wasm-safe crates found in the workspace metadata".into());
    }
    let mut results = Vec::new();
    let mut c = cargo();
    c.args(["check", "--target", "wasm32-unknown-unknown", "--keep-going"]);
    for pkg in &set {
        c.args(["-p", pkg]);
    }
    let ok = run(c, &format!("cargo check --target wasm32-unknown-unknown --keep-going -p {}", set.join(" -p "))).is_ok();
    results.push((format!("{} crates", set.len()), ok));
    for (pkg, feature) in WASM_FEATURES {
        let mut c = cargo();
        c.args(["check", "--target", "wasm32-unknown-unknown", "-p", pkg, "--features", feature]);
        let ok = run(c, &format!("cargo check --target wasm32-unknown-unknown -p {pkg} --features {feature}")).is_ok();
        results.push((format!("{pkg} --features {feature}"), ok));
    }
    println!("\nwasm32-unknown-unknown check:");
    for (p, ok) in &results {
        println!("  {:<6} {p}", if *ok { "ok" } else { "FAIL" });
    }
    let failed = results.iter().filter(|r| !r.1).count();
    if failed == 0 { Ok(()) } else { Err(format!("{failed} wasm check(s) failed")) }
}

/// Test targets for the test build. Examples are left out so they aren't linked; clippy
/// `--all-targets` still compiles them.
const TEST_TARGETS: [&str; 4] = ["--workspace", "--lib", "--bins", "--tests"];

fn nextest_available() -> bool {
    cargo().args(["nextest", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn cmd_test() -> Result<(), String> {
    let mut c = cargo();
    let tool = if nextest_available() {
        c.args(["nextest", "run"]);
        "nextest run"
    } else {
        eprintln!("note: cargo-nextest is not installed, running cargo test (cargo install cargo-nextest --locked)");
        c.arg("test");
        "test"
    };
    c.args(TEST_TARGETS);
    run(c, &format!("cargo {tool} {}", TEST_TARGETS.join(" ")))
}

/// What the CI gate adds to every build: `--locked`, and the optional craft-heif decoder that
/// official builds open HEIC photos with.
const GATE_ARGS: [&str; 4] = ["--workspace", "--locked", "--features", "heif"];

/// Where a `ci` step runs when the gate splits the work over its jobs. `cargo xtask ci` alone runs
/// every step, in the order of [`CI_STEPS`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    /// Checks that need no test build: the gate's lint leg (`ci lint`).
    Lint,
    /// The tests, which the gate splits over its shards (`ci tests`).
    Test,
    /// The doctests, which the gate's build job runs once the archive is uploaded (`ci doctest`).
    Doc,
}

/// Which tests a test step takes, by nextest's `gpu` test group.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Selection {
    All,
    /// Only the group, with each test's output shown: the Linux gate runs it on lavapipe, and
    /// scripts/ci-gpu-lavapipe.py checks that output for skipped GPU comparisons.
    Gpu,
    /// Every test outside the group.
    NoGpu,
}

#[derive(Clone, Debug, PartialEq)]
struct CiOptions {
    /// Build the tests without running them (`ci build`, the cache warm-up).
    build_only: bool,
    /// The nextest partition the test steps run ($NEXTEST_PARTITION with `ci tests`).
    partition: Option<String>,
    /// `ci archive`: build the tests into nextest archives instead of running them.
    archive: Option<Archive>,
    /// `ci tests --from <dir>`: run the tests from a nextest archive extracted to this directory.
    from: Option<PathBuf>,
    /// `ci tests --from <dir> --shard <k>`: the archive is gate shard k's (1-based), and the plan it
    /// carries says which part of the split binaries it runs.
    shard: Option<usize>,
    selection: Selection,
    /// Workspace packages left out of the test build and the doctests ($PHOTOCRAFT_CI_EXCLUDE).
    exclude: Vec<String>,
    /// Build the real-file corpus tests in ($PHOTOCRAFT_CORPUS_TESTS=1, the gate): see CORPUS_FEATURES.
    corpus: bool,
}

/// The archives `ci archive` writes.
#[derive(Clone, Debug, PartialEq)]
enum Archive {
    /// One archive of every test binary ($NEXTEST_ARCHIVE_FILE).
    File(PathBuf),
    /// One archive per gate shard, at `<dir>/<k>/<name>`, each with only the binaries that shard
    /// runs (xtask/src/shards.rs).
    Shards { dir: PathBuf, name: String, shards: usize, gpu_shard: bool },
}

type StepFn = fn(&CiOptions) -> Result<(), String>;
/// The steps a `cargo xtask ci` call runs, in order, by name.
type CiPlan = Vec<(&'static str, StepFn)>;

/// The CI gate's steps, in the order `cargo xtask ci` runs them.
const CI_STEPS: [(&str, Part, StepFn); 12] = [
    ("fmt", Part::Lint, step_fmt),
    ("egui-fork", Part::Lint, |_| cmd_egui_fork()),
    ("clippy", Part::Lint, step_clippy),
    ("layers", Part::Lint, |_| cmd_layers()),
    ("panel-commands", Part::Lint, |_| panel_commands::run(&root())),
    ("test-binaries", Part::Lint, |_| test_binaries::run(&root())),
    ("scorecard", Part::Lint, |_| scorecard::run(&root(), &["--check"])),
    ("test", Part::Test, |o| step_tests(o, false)),
    ("panic_hunt", Part::Test, |o| step_tests(o, true)),
    ("doctest", Part::Doc, step_doctest),
    ("wasm", Part::Lint, |_| cmd_wasm()),
    ("i18n", Part::Lint, step_i18n),
];

fn step_fmt(_: &CiOptions) -> Result<(), String> {
    let mut c = cargo();
    c.args(["fmt", "--all", "--", "--check"]);
    run(c, "cargo fmt --all -- --check")
}

fn step_clippy(_: &CiOptions) -> Result<(), String> {
    let mut c = cargo();
    c.arg("clippy").args(GATE_ARGS).args(["--all-targets", "--keep-going", "--", "-D", "warnings"]);
    run(c, "cargo clippy --workspace --locked --features heif --all-targets --keep-going -- -D warnings")
}

fn step_doctest(o: &CiOptions) -> Result<(), String> {
    let mut c = cargo();
    c.arg("test").args(gate_args(&o.exclude)).arg("--doc");
    if cache_units::file().is_some() {
        c.args(["--message-format", "json-render-diagnostics"]);
    }
    let what = shown(&c);
    cache_units::run(c, &what)
}

fn step_i18n(_: &CiOptions) -> Result<(), String> {
    // Lists missing translations for the log; a missing string shows in English, so it never fails CI.
    if let Err(e) = cmd_i18n_report() {
        eprintln!("warning: i18n report failed: {e}");
    }
    Ok(())
}

/// GATE_ARGS without the excluded packages.
fn gate_args(exclude: &[String]) -> Vec<String> {
    let mut args: Vec<String> = GATE_ARGS.iter().map(|a| a.to_string()).collect();
    for package in exclude {
        args.extend(["--exclude".to_string(), package.clone()]);
    }
    args
}

/// The test build every test step shares: the gate's packages and features, TEST_TARGETS.
/// Packages whose real-file corpus tests (`feature = "corpus"`) the gate builds in with
/// PHOTOCRAFT_CORPUS_TESTS=1. Those tests fail when the corpus is missing, so the gate's shards
/// fetch it first (scripts/ci-test-shard.sh); a local `ci tests` leaves them out.
const CORPUS_FEATURES: [&str; 2] = ["photocraft-io/corpus", "photocraft-engine/corpus"];

fn test_build_args(exclude: &[String], corpus: bool) -> Vec<String> {
    let mut args = gate_args(exclude);
    if corpus {
        // `--features pkg/feature` names a package `--exclude` removed from the build: leave it out.
        let extra: Vec<&str> = CORPUS_FEATURES.iter().copied().filter(|f| !exclude.iter().any(|e| f.split('/').next() == Some(e.as_str()))).collect();
        if let Some(i) = args.iter().position(|a| a == "--features")
            && !extra.is_empty()
        {
            args[i + 1] = format!("{},{}", args[i + 1], extra.join(","));
        }
    }
    args.extend(TEST_TARGETS[1..].iter().map(|a| a.to_string()));
    args
}

/// The filter of nextest's `gpu` test group: the default profile's overrides that put tests in it.
fn gpu_filter(config: &toml::Table) -> Result<String, String> {
    let filters: Vec<String> = config
        .get("profile")
        .and_then(|p| p.get("default"))
        .and_then(|d| d.get("overrides"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("test-group").and_then(toml::Value::as_str) == Some("gpu"))
        .filter_map(|row| row.get("filter").and_then(toml::Value::as_str))
        .map(|filter| format!("({})", filter.trim()))
        .collect();
    if filters.is_empty() {
        return Err("the nextest `gpu` test group has no filters (.config/nextest.toml)".into());
    }
    Ok(filters.join(" | "))
}

fn read_gpu_filter() -> Result<String, String> {
    let path = root().join(".config/nextest.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    gpu_filter(&toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?)
}

/// The nextest command line of a test step. `panic_hunt` runs only the engine's `#[ignore]`d panic
/// hunt (AGENTS.md Rule 9), with the same build arguments as the tests, so nothing rebuilds. `gpu` is
/// the group's filter, read only when the selection needs it. `shard` is the filterset of the shard's
/// part of the split binaries (`ci tests --shard`). nextest's `ci` profile terminates hung tests and
/// repeats every failure at the end of the log (.config/nextest.toml).
fn nextest_args(o: &CiOptions, panic_hunt: bool, gpu: Option<&str>, shard: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = ["nextest", "run", "--profile", "ci"].map(String::from).to_vec();
    let mut filters = Vec::new();
    if panic_hunt {
        args.extend(["--run-ignored", "only"].map(String::from));
        filters.push("binary_id(photocraft-engine::panic_hunt)".to_string());
    }
    if let Some(dir) = &o.from {
        let target = dir.join("target");
        let path = |p: PathBuf| p.to_string_lossy().into_owned();
        args.extend([
            "--binaries-metadata".to_string(),
            path(target.join("nextest").join("binaries-metadata.json")),
            "--cargo-metadata".to_string(),
            path(target.join("nextest").join("cargo-metadata.json")),
            "--target-dir-remap".to_string(),
            path(target.clone()),
            "--workspace-remap".to_string(),
            path(root()),
        ]);
    } else {
        args.extend(test_build_args(&o.exclude, o.corpus));
    }
    if let Some(partition) = &o.partition {
        args.extend(["--partition".to_string(), partition.clone()]);
    }
    // A partition or a selection may hold no panic hunt; the shard that does runs it. A shard of the
    // build job's plan may hold no runnable test at all; the build job checked that every test runs
    // on exactly one shard.
    if (panic_hunt && (o.partition.is_some() || o.selection != Selection::All)) || o.shard.is_some() {
        args.extend(["--no-tests", "warn"].map(String::from));
    }
    match (o.selection, gpu) {
        (Selection::Gpu, Some(gpu)) => {
            filters.push(format!("({gpu})"));
            args.extend(["--success-output", "immediate"].map(String::from));
        }
        (Selection::NoGpu, Some(gpu)) => filters.push(format!("not ({gpu})")),
        _ => {}
    }
    if let Some(filter) = shard {
        filters.push(format!("({filter})"));
    }
    if !filters.is_empty() {
        args.extend(["-E".to_string(), filters.join(" & ")]);
    }
    args
}

fn shown(c: &Command) -> String {
    format!("cargo {}", c.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "))
}

/// The `test` and `panic_hunt` steps.
fn step_tests(o: &CiOptions, panic_hunt: bool) -> Result<(), String> {
    let mut c = cargo();
    if o.partition.is_none() && o.selection == Selection::All && !nextest_available() {
        if panic_hunt {
            // `-- --ignored` across the workspace would run every ignored test, not only the hunt. The
            // engine has no `heif` feature of its own; it reaches the decoder through photocraft-io.
            c.args(["test", "-p", "photocraft-engine", "--locked", "--features", "photocraft-io/heif", "--test", "panic_hunt", "--", "--ignored"]);
        } else {
            eprintln!("note: cargo-nextest is not installed, running cargo test (cargo install cargo-nextest --locked)");
            c.arg("test").args(test_build_args(&o.exclude, o.corpus));
        }
    } else {
        let gpu = if o.selection == Selection::All { None } else { Some(read_gpu_filter()?) };
        let shard = match (&o.from, o.shard) {
            (Some(from), Some(k)) => read_shard_filter(from, k, o.selection)?,
            _ => None,
        };
        c.args(nextest_args(o, panic_hunt, gpu.as_deref(), shard.as_deref()));
    }
    let what = shown(&c);
    run(c, &what)
}

/// Builds the test steps' binaries without running them: what every shard builds, so the cache
/// warm-up's entry covers each shard's build.
fn build_tests(o: &CiOptions) -> Result<(), String> {
    cache_units::record_xtask()?;
    let recording = cache_units::file().is_some();
    let mut c = cargo();
    match &o.archive {
        Some(Archive::File(file)) => {
            // A shard plan left in the target directory by a per-shard build would go into this
            // archive (.config/nextest.toml) and mislead its shards.
            let plan = target_dir()?.join(shards::PLAN_FILE);
            match std::fs::remove_file(&plan) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("{}: {e}", plan.display())),
                _ => {}
            }
            c.args(["nextest", "archive", "--profile", "ci"]).args(test_build_args(&o.exclude, o.corpus)).arg("--archive-file").arg(file);
            if recording {
                c.args(["--cargo-message-format", "json-render-diagnostics"]);
            }
        }
        Some(Archive::Shards { dir, name, shards, gpu_shard }) => return build_shard_archives(o, dir, name, *shards, *gpu_shard),
        None => {
            c.args(["test", "--no-run"]).args(test_build_args(&o.exclude, o.corpus));
            if recording {
                c.args(["--message-format", "json-render-diagnostics"]);
            }
        }
    }
    let what = shown(&c);
    cache_units::run(c, &what)
}

/// `cargo nextest list --profile ci --message-format json` of the test build, with `args`.
fn nextest_list(o: &CiOptions, args: &[&str], record: bool) -> Result<serde_json::Value, String> {
    let mut c = cargo();
    c.args(["nextest", "list", "--profile", "ci", "--message-format", "json"]).args(test_build_args(&o.exclude, o.corpus)).args(args);
    let record = record.then(cache_units::file).flatten();
    if record.is_some() {
        c.args(["--cargo-message-format", "json-render-diagnostics"]);
    }
    let what = shown(&c);
    let out = if let Some(file) = record {
        cache_units::capture_recorded(c, &what, &file)?
    } else {
        eprintln!("$ {what}");
        let out = c.stderr(Stdio::inherit()).output().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
        if !out.status.success() {
            return Err(format!("{what}: exited with {}", out.status));
        }
        out.stdout
    };
    serde_json::from_slice(&out).map_err(|e| format!("{what}: bad JSON: {e}"))
}

/// The gate's build job with per-shard archives: builds the tests, lists them, plans which binaries
/// (and which parts of the split ones) each shard runs, writes the plan into the target directory
/// (every archive carries it, .config/nextest.toml) and writes each shard's archive.
fn build_shard_archives(o: &CiOptions, dir: &Path, name: &str, shards: usize, gpu_shard: bool) -> Result<(), String> {
    let os = if cfg!(windows) { "windows" } else { "linux" };
    let times_path = root().join(shards::TIMES_FILE);
    let text = std::fs::read_to_string(&times_path).unwrap_or_else(|e| {
        eprintln!("warning: {}: {e}; every binary and test gets the default weight", times_path.display());
        String::new()
    });
    let times = shards::Times::parse(&text, os)?;
    // The build itself, and every binary it made.
    let built = nextest_list(o, &["--list-type", "binaries-only"], true)?;
    let ids: Vec<String> = built.get("rust-binaries").and_then(serde_json::Value::as_object).ok_or("nextest list: no rust-binaries")?.keys().cloned().collect();
    // With a GPU shard every binary is listed against the gpu group; otherwise only the split ones.
    let filter = if gpu_shard {
        read_gpu_filter()?
    } else {
        let split: Vec<String> = shards::binaries_to_list(&ids, &times, shards).iter().map(|b| format!("binary_id(={b})")).collect();
        if split.is_empty() { "none()".to_string() } else { split.join(" | ") }
    };
    let listing = nextest_list(o, &["--run-ignored", "all", "-E", &filter], false)?;
    let (binaries, target) = shards::parse_listing(&listing, gpu_shard)?;
    let plan = shards::plan(&binaries, &times, shards, gpu_shard)?;
    eprint!("{}", plan.summary());
    let plan_path = target.join(shards::PLAN_FILE);
    let json = serde_json::to_string_pretty(&plan).map_err(|e| format!("shard plan: {e}"))?;
    std::fs::write(&plan_path, json).map_err(|e| format!("{}: {e}", plan_path.display()))?;
    for (k, shard) in plan.shards.iter().enumerate() {
        let file = dir.join((k + 1).to_string()).join(name);
        std::fs::create_dir_all(dir.join((k + 1).to_string())).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut c = cargo();
        c.args(["nextest", "archive", "--profile", "ci"])
            .args(test_build_args(&o.exclude, o.corpus))
            .arg("-E")
            .arg(shard.archive_filter())
            .arg("--archive-file")
            .arg(&file);
        run(c, &format!("cargo nextest archive (shard {}: {} binaries) --archive-file {}", k + 1, shard.binaries.len(), file.display()))?;
    }
    Ok(())
}

/// Shard `k`'s filterset from the plan its archive carries, extracted to `from`, once the plan is
/// checked against the archive and the selection.
fn read_shard_filter(from: &Path, k: usize, selection: Selection) -> Result<Option<String>, String> {
    let read = |path: PathBuf| -> Result<String, String> { std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display())) };
    let plan_path = from.join("target").join(shards::PLAN_FILE);
    let plan: shards::Plan = serde_json::from_str(&read(plan_path.clone())?).map_err(|e| format!("{}: {e}", plan_path.display()))?;
    let metadata_path = from.join("target").join("nextest").join("binaries-metadata.json");
    let metadata: serde_json::Value = serde_json::from_str(&read(metadata_path.clone())?).map_err(|e| format!("{}: {e}", metadata_path.display()))?;
    let gpu = match selection {
        Selection::All => None,
        Selection::Gpu => Some(true),
        Selection::NoGpu => Some(false),
    };
    let shard = shards::shard_to_run(&plan, k, gpu, &shards::archived_binaries(&metadata)?)?;
    eprintln!(
        "shard {k}: {} test binaries{}",
        shard.binaries.len(),
        if shard.parts.is_empty() { String::new() } else { format!(", part of {}", shard.parts.keys().cloned().collect::<Vec<_>>().join(", ")) }
    );
    Ok(shard.filter())
}

/// nextest does not run doctests.
fn cmd_doctest() -> Result<(), String> {
    let mut c = cargo();
    c.args(["test", "--workspace", "--doc"]);
    run(c, "cargo test --workspace --doc")
}

fn cmd_parity() -> Result<(), String> {
    let mut c = cargo();
    c.args(["run", "-q", "-p", "photocraft-ui-egui", "--example", "parity", "--", "--write", "docs/parity-checklist.md"]);
    run(c, "cargo run -p photocraft-ui-egui --example parity")
}

/// The packages the gate leaves out of a test build: the Windows legs skip the browser apps, and
/// on pull requests the packages nothing changed reaches (.github/workflows/ci.yml).
fn ci_exclude() -> Vec<String> {
    std::env::var("PHOTOCRAFT_CI_EXCLUDE").unwrap_or_default().split_whitespace().map(String::from).collect()
}

/// The steps `cargo xtask ci <args>` runs, and how: no argument runs every step; a part (`lint`,
/// `build`, `tests`) runs the steps of one gate job; a step's name runs that step.
fn ci_from_args(args: &[&str], exclude: Vec<String>) -> Result<(CiPlan, CiOptions), String> {
    let mut options = CiOptions {
        build_only: false,
        partition: None,
        archive: None,
        from: None,
        shard: None,
        selection: Selection::All,
        exclude,
        corpus: std::env::var_os("PHOTOCRAFT_CORPUS_TESTS").is_some_and(|v| v == "1"),
    };
    let mut gpu_shard = false;
    let mut what = None;
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--from" => {
                let dir = it.next().ok_or("--from needs the directory the archive was extracted to")?;
                options.from = Some(std::path::absolute(dir).map_err(|e| format!("--from {dir}: {e}"))?);
            }
            "--shard" => {
                let k = it.next().ok_or("--shard needs the shard's number")?;
                options.shard = Some(k.parse().ok().filter(|&k| k > 0).ok_or_else(|| format!("--shard {k}: not a shard number"))?);
            }
            "--gpu-shard" => gpu_shard = true,
            "--gpu" => options.selection = Selection::Gpu,
            "--no-gpu" => options.selection = Selection::NoGpu,
            _ if what.is_none() && !arg.starts_with('-') => what = Some(arg),
            _ => return Err(format!("ci: unexpected argument `{arg}`")),
        }
    }
    if (options.selection != Selection::All || options.from.is_some() || options.shard.is_some()) && what != Some("tests") {
        return Err("ci: --from, --shard, --gpu and --no-gpu go with `ci tests`".into());
    }
    if options.shard.is_some() && options.from.is_none() {
        return Err("ci: --shard runs a shard's archive: give its directory with --from".into());
    }
    if gpu_shard && what != Some("archive") {
        return Err("ci: --gpu-shard goes with `ci archive`".into());
    }
    let parts: &[Part] = match what {
        None => &[Part::Lint, Part::Test, Part::Doc],
        Some("lint") => &[Part::Lint],
        Some("build") => {
            options.build_only = true;
            &[Part::Test]
        }
        // The gate's build job: the tests into the archive at $NEXTEST_ARCHIVE_FILE, which also
        // carries this xtask (.config/nextest.toml).
        Some("archive") => {
            options.archive = Some(archive_from_env(gpu_shard, |k| std::env::var(k).ok())?);
            options.build_only = true;
            &[Part::Test]
        }
        Some("tests") => {
            // A shard's archive holds only its binaries, and its plan says the rest.
            if options.shard.is_none() {
                options.partition = tests_partition(options.selection, std::env::var("NEXTEST_PARTITION").ok());
            }
            &[Part::Test]
        }
        Some(step) => {
            let plan: Vec<_> = CI_STEPS.iter().filter(|s| s.0 == step).map(|&(name, _, f)| (name, f)).collect();
            if plan.is_empty() {
                return Err(format!("ci: no part or step `{step}`"));
            }
            return Ok((plan, options));
        }
    };
    Ok((ci_plan(parts, options.build_only), options))
}

/// The archives `ci archive` writes: one per gate shard where the gate's build job sets
/// $NEXTEST_ARCHIVE_DIR (with $NEXTEST_ARCHIVE_NAME and $GATE_SHARDS), else one at
/// $NEXTEST_ARCHIVE_FILE.
fn archive_from_env(gpu_shard: bool, var: impl Fn(&str) -> Option<String>) -> Result<Archive, String> {
    let Some(dir) = var("NEXTEST_ARCHIVE_DIR").filter(|d| !d.is_empty()) else {
        if gpu_shard {
            return Err("ci archive --gpu-shard writes per-shard archives: set NEXTEST_ARCHIVE_DIR".into());
        }
        let file = var("NEXTEST_ARCHIVE_FILE").ok_or("ci archive: set NEXTEST_ARCHIVE_FILE to the archive to write")?;
        return Ok(Archive::File(file.into()));
    };
    let name = var("NEXTEST_ARCHIVE_NAME").ok_or("ci archive: NEXTEST_ARCHIVE_DIR needs NEXTEST_ARCHIVE_NAME")?;
    let shards = var("GATE_SHARDS").and_then(|n| n.parse().ok()).filter(|&n| n > 0).ok_or("ci archive: NEXTEST_ARCHIVE_DIR needs GATE_SHARDS")?;
    Ok(Archive::Shards { dir: dir.into(), name, shards, gpu_shard })
}

/// The partition `ci tests` runs. The gate sets $NEXTEST_PARTITION on every shard leg, and the shard
/// that runs the gpu group must still run all of it.
fn tests_partition(selection: Selection, env: Option<String>) -> Option<String> {
    env.filter(|p| !p.is_empty() && selection != Selection::Gpu)
}

/// The steps of `parts`, in CI_STEPS order. When the tests are only built, the test steps become
/// that one build.
fn ci_plan(parts: &[Part], build_only: bool) -> CiPlan {
    let mut plan: CiPlan = Vec::new();
    for &(name, part, f) in &CI_STEPS {
        if !parts.contains(&part) {
            continue;
        }
        if part == Part::Test && build_only {
            if !plan.iter().any(|s| s.0 == "build") {
                plan.push(("build", build_tests));
            }
        } else {
            plan.push((name, f));
        }
    }
    plan
}

fn cmd_ci(steps: &[(&'static str, StepFn)], options: &CiOptions) -> Result<(), String> {
    let mut done = Vec::new();
    for (name, f) in steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = f(options) {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}

#[cfg(test)]
mod ci_tests {
    use super::*;

    fn names(args: &[&str]) -> Vec<&'static str> {
        ci_from_args(args, Vec::new()).unwrap().0.into_iter().map(|s| s.0).collect()
    }

    #[test]
    fn the_gate_jobs_together_run_every_step_once() {
        let all = names(&[]);
        assert_eq!(all, CI_STEPS.map(|s| s.0));
        let lint = names(&["lint"]);
        let tests = names(&["tests", "--no-gpu"]);
        assert_eq!(tests, ["test", "panic_hunt"]);
        let mut split: Vec<_> = lint.iter().chain(&tests).chain(&names(&["doctest"])).copied().collect();
        split.sort_unstable();
        let mut every = all.clone();
        every.sort_unstable();
        assert_eq!(split, every);
        assert_eq!(names(&["build"]), ["build"]);
        assert_eq!(names(&["clippy"]), ["clippy"]);
        assert!(ci_from_args(&["nope"], Vec::new()).is_err());
        assert!(ci_from_args(&["lint", "--gpu"], Vec::new()).is_err());
    }

    #[test]
    fn a_shard_builds_what_the_warm_up_builds_and_runs_its_partition() {
        let options = CiOptions {
            build_only: false,
            archive: None,
            from: None,
            shard: None,
            partition: Some("hash:2/3".into()),
            selection: Selection::NoGpu,
            exclude: vec!["photocraft-web".into()],
            corpus: false,
        };
        let build = "--workspace --locked --features heif --exclude photocraft-web --lib --bins --tests";
        let args = nextest_args(&options, false, Some("package(g)"), None).join(" ");
        assert_eq!(args, format!("nextest run --profile ci {build} --partition hash:2/3 -E not (package(g))"));
        let gpu = CiOptions { selection: Selection::Gpu, partition: None, ..options.clone() };
        let hunt = nextest_args(&gpu, true, Some("package(g)"), None).join(" ");
        assert!(hunt.contains("--run-ignored only") && hunt.contains("--no-tests warn") && hunt.contains("--success-output immediate"), "{hunt}");
        assert!(hunt.ends_with("-E binary_id(photocraft-engine::panic_hunt) & (package(g))"), "{hunt}");
        // The whole run keeps nextest's default: a panic hunt that matches nothing fails.
        let here = CiOptions { partition: None, selection: Selection::All, ..options };
        assert!(!nextest_args(&here, true, None, None).join(" ").contains("--no-tests"));
        assert_eq!(test_build_args(&here.exclude, false).join(" "), build);
        assert_eq!(
            test_build_args(&here.exclude, true).join(" "),
            "--workspace --locked --features heif,photocraft-io/corpus,photocraft-engine/corpus --exclude photocraft-web --lib --bins --tests"
        );
        // A package the build leaves out cannot get a feature.
        let args = test_build_args(&["photocraft-io".to_string()], true).join(" ");
        assert!(args.contains("--features heif,photocraft-engine/corpus ") && !args.contains("photocraft-io/corpus"), "{args}");
    }

    #[test]
    fn a_shard_archive_runs_its_part_and_no_partition() {
        let (_, options) = ci_from_args(&["tests", "--from", "x", "--shard", "2", "--no-gpu"], Vec::new()).unwrap();
        assert_eq!((options.shard, options.partition.as_deref()), (Some(2), None));
        let args = nextest_args(&options, true, Some("package(g)"), Some("not (binary_id(=ui))")).join(" ");
        assert!(args.contains("--no-tests warn") && !args.contains("--partition"), "{args}");
        assert!(args.ends_with("-E binary_id(photocraft-engine::panic_hunt) & not (package(g)) & (not (binary_id(=ui)))"), "{args}");
        // A shard may hold no runnable test; the build job's plan check guarantees the coverage.
        assert!(nextest_args(&options, false, None, None).join(" ").contains("--no-tests warn"));
        assert!(ci_from_args(&["tests", "--shard", "2"], Vec::new()).is_err(), "--shard without --from");
        assert!(ci_from_args(&["tests", "--from", "x", "--shard", "0"], Vec::new()).is_err());
        assert!(ci_from_args(&["lint", "--gpu-shard"], Vec::new()).is_err());

        fn env(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
            move |k| vars.iter().find(|v| v.0 == k).map(|v| v.1.to_string())
        }
        let shards = &[("NEXTEST_ARCHIVE_DIR", "/a"), ("NEXTEST_ARCHIVE_NAME", "n.tar.zst"), ("GATE_SHARDS", "4"), ("NEXTEST_ARCHIVE_FILE", "/f")];
        assert_eq!(archive_from_env(true, env(shards)).unwrap(), Archive::Shards { dir: "/a".into(), name: "n.tar.zst".into(), shards: 4, gpu_shard: true });
        assert_eq!(archive_from_env(false, env(&[("NEXTEST_ARCHIVE_FILE", "/f")])).unwrap(), Archive::File("/f".into()));
        assert!(archive_from_env(true, env(&[("NEXTEST_ARCHIVE_FILE", "/f")])).is_err());
        assert!(archive_from_env(false, env(&[("NEXTEST_ARCHIVE_DIR", "/a"), ("NEXTEST_ARCHIVE_NAME", "n")])).is_err());
    }

    #[test]
    fn the_gpu_shard_runs_the_whole_gpu_group_whatever_the_partition() {
        let env = || Some("hash:1/4".to_string());
        assert_eq!(tests_partition(Selection::Gpu, env()), None);
        assert_eq!(tests_partition(Selection::NoGpu, env()).as_deref(), Some("hash:1/4"));
        assert_eq!(tests_partition(Selection::All, Some(String::new())), None);
    }

    #[test]
    fn the_gpu_group_takes_every_filter_that_selects_it() {
        let config: toml::Table = toml::from_str(
            r#"
            [[profile.default.overrides]]
            filter = 'package(a)'
            test-group = 'gpu'
            [[profile.default.overrides]]
            filter = 'package(b)'
            test-group = 'cpu'
            [[profile.default.overrides]]
            filter = '''
              test(c)
            '''
            test-group = 'gpu'
        "#,
        )
        .unwrap();
        assert_eq!(gpu_filter(&config).unwrap(), "(package(a)) | (test(c))");
        assert!(gpu_filter(&toml::from_str("[profile.default]").unwrap()).is_err());
        // The workspace's own group exists and parses.
        assert!(read_gpu_filter().unwrap().contains("package(photocraft-gpu)"));
    }
}
