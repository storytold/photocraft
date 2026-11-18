//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). External tools (`cargo`, `curl`, `tar`) are
//! invoked through `std::process::Command`.

mod cargo_env;
mod corpus;
mod corpus_pins;
mod i18n_report;
mod ico;
mod layers;
mod panel_commands;
mod perf;
mod pinned;
mod scorecard;
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
  ci [lint | build | tests [--gpu | --no-gpu] | <step>]
                  the CI gate: fmt --check, egui-fork, clippy -D warnings, layers, panel-commands, scorecard --check,
                  tests (nextest --profile ci), panic_hunt, doctests, wasm and the i18n-report, with --locked
                  --features heif (stops at first failure). The gate splits it over its jobs: `lint` runs the
                  steps that need no test build; `tests` runs the tests and panic_hunt on the partition in
                  $NEXTEST_PARTITION (`--gpu`: all of nextest's gpu test group, whatever the partition, each
                  test's output shown;
                  `--no-gpu`: every other test); `doctest` runs the doctests; `build` only builds what `tests`
                  builds (the cache warm-up). $PHOTOCRAFT_CI_EXCLUDE (packages separated by spaces) leaves
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
        Some("i18n-report") => i18n_report::run(&root()),
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

/// Two egui revisions in one build give two incompatible `Ui` types, so the workspace's egui
/// `[patch]` must name the revision the craft framework is built with.
fn cmd_egui_fork() -> Result<(), String> {
    let path = root().join("Cargo.lock");
    let lock = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    craft::egui_fork::check_lockfile(&lock)?;
    println!("OK: every egui package comes from {} at {}.", craft::egui_fork::GIT, craft::egui_fork::REV);
    Ok(())
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
    /// The doctests, which the gate runs in one shard (`ci doctest`).
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
    selection: Selection,
    /// Workspace packages left out of the test build and the doctests ($PHOTOCRAFT_CI_EXCLUDE).
    exclude: Vec<String>,
}

type StepFn = fn(&CiOptions) -> Result<(), String>;

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
    let what = shown(&c);
    run(c, &what)
}

fn step_i18n(_: &CiOptions) -> Result<(), String> {
    // Lists missing translations for the log; a missing string shows in English, so it never fails CI.
    if let Err(e) = i18n_report::run(&root()) {
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
fn test_build_args(exclude: &[String]) -> Vec<String> {
    let mut args = gate_args(exclude);
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
/// the group's filter, read only when the selection needs it. nextest's `ci` profile terminates
/// hung tests and repeats every failure at the end of the log (.config/nextest.toml).
fn nextest_args(o: &CiOptions, panic_hunt: bool, gpu: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = ["nextest", "run", "--profile", "ci"].map(String::from).to_vec();
    let mut filters = Vec::new();
    if panic_hunt {
        args.extend(["--run-ignored", "only"].map(String::from));
        filters.push("binary_id(photocraft-engine::panic_hunt)".to_string());
    }
    args.extend(test_build_args(&o.exclude));
    if let Some(partition) = &o.partition {
        args.extend(["--partition".to_string(), partition.clone()]);
    }
    // A partition or a selection may hold no panic hunt; the shard that does runs it.
    if panic_hunt && (o.partition.is_some() || o.selection != Selection::All) {
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
            c.arg("test").args(test_build_args(&o.exclude));
        }
    } else {
        let gpu = if o.selection == Selection::All { None } else { Some(read_gpu_filter()?) };
        c.args(nextest_args(o, panic_hunt, gpu.as_deref()));
    }
    let what = shown(&c);
    run(c, &what)
}

/// Builds the test steps' binaries without running them: what every shard builds, so the cache
/// warm-up's entry covers each shard's build.
fn build_tests(o: &CiOptions) -> Result<(), String> {
    let mut c = cargo();
    c.args(["test", "--no-run"]).args(test_build_args(&o.exclude));
    let what = shown(&c);
    run(c, &what)
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
fn ci_from_args(args: &[&str], exclude: Vec<String>) -> Result<(Vec<(&'static str, StepFn)>, CiOptions), String> {
    let mut options = CiOptions { build_only: false, partition: None, selection: Selection::All, exclude };
    let mut what = None;
    for &arg in args {
        match arg {
            "--gpu" => options.selection = Selection::Gpu,
            "--no-gpu" => options.selection = Selection::NoGpu,
            _ if what.is_none() && !arg.starts_with('-') => what = Some(arg),
            _ => return Err(format!("ci: unexpected argument `{arg}`")),
        }
    }
    if options.selection != Selection::All && what != Some("tests") {
        return Err("ci: --gpu and --no-gpu go with `ci tests`".into());
    }
    let parts: &[Part] = match what {
        None => &[Part::Lint, Part::Test, Part::Doc],
        Some("lint") => &[Part::Lint],
        Some("build") => {
            options.build_only = true;
            &[Part::Test]
        }
        Some("tests") => {
            options.partition = tests_partition(options.selection, std::env::var("NEXTEST_PARTITION").ok());
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

/// The partition `ci tests` runs. The gate sets $NEXTEST_PARTITION on every shard leg, and the shard
/// that runs the gpu group must still run all of it.
fn tests_partition(selection: Selection, env: Option<String>) -> Option<String> {
    env.filter(|p| !p.is_empty() && selection != Selection::Gpu)
}

/// The steps of `parts`, in CI_STEPS order. When the tests are only built, the test steps become
/// that one build.
fn ci_plan(parts: &[Part], build_only: bool) -> Vec<(&'static str, StepFn)> {
    let mut plan: Vec<(&'static str, StepFn)> = Vec::new();
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
        let options = CiOptions { build_only: false, partition: Some("hash:2/3".into()), selection: Selection::NoGpu, exclude: vec!["photocraft-web".into()] };
        let build = "--workspace --locked --features heif --exclude photocraft-web --lib --bins --tests";
        let args = nextest_args(&options, false, Some("package(g)")).join(" ");
        assert_eq!(args, format!("nextest run --profile ci {build} --partition hash:2/3 -E not (package(g))"));
        let gpu = CiOptions { selection: Selection::Gpu, partition: None, ..options.clone() };
        let hunt = nextest_args(&gpu, true, Some("package(g)")).join(" ");
        assert!(hunt.contains("--run-ignored only") && hunt.contains("--no-tests warn") && hunt.contains("--success-output immediate"), "{hunt}");
        assert!(hunt.ends_with("-E binary_id(photocraft-engine::panic_hunt) & (package(g))"), "{hunt}");
        // The whole run keeps nextest's default: a panic hunt that matches nothing fails.
        let here = CiOptions { partition: None, selection: Selection::All, ..options };
        assert!(!nextest_args(&here, true, None).join(" ").contains("--no-tests"));
        assert_eq!(test_build_args(&here.exclude).join(" "), build);
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
