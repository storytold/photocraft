//! The gate's per-shard archives (`cargo xtask ci archive` with $NEXTEST_ARCHIVE_DIR): which test
//! binaries each shard runs, balanced by the recorded test times in .config/ci-test-times.tsv.
//!
//! Each shard downloads an archive that holds only its own binaries. A binary heavier than half a
//! shard's share is split by test filter along its module tree, and its executable goes into the
//! archive of every shard that runs part of it. With a GPU shard (the Linux gate), shard 1 takes
//! every binary that holds a test of nextest's `gpu` group and runs that group alone; the other
//! shards run every other test.
//!
//! The times only decide the balance. Which binaries and tests exist comes from the build's own
//! nextest listing, so a missing or stale time never drops or repeats a test.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where the recorded times live, relative to the workspace root (scripts/ci-test-times.py writes it).
pub const TIMES_FILE: &str = ".config/ci-test-times.tsv";
/// The plan, written into the target directory, which every shard's archive carries
/// (.config/nextest.toml `archive.include`).
pub const PLAN_FILE: &str = "gate-shards.json";

/// A binary's weight when the times file has no binaries at all.
const DEFAULT_BINARY_SECONDS: f64 = 10.0;
/// A test's weight when the times file knows nothing about its binary.
const DEFAULT_TEST_SECONDS: f64 = 0.3;
/// A split binary's tests are cut into parts of at most a shard's share divided by this, so the
/// parts fill the shards evenly while the filterset stays short.
const PARTS_PER_SHARD: f64 = 8.0;

/// Recorded test times of one OS.
#[derive(Debug, Default)]
pub struct Times {
    /// Binary id -> (test count, total seconds).
    binaries: BTreeMap<String, (u64, f64)>,
    /// Binary id -> test name -> seconds, for the tests the file lists one by one.
    tests: BTreeMap<String, BTreeMap<String, f64>>,
    /// Binary id -> the mean time of its tests the file does not list one by one.
    small: BTreeMap<String, f64>,
    /// The median binary total: the weight of a binary the file does not know.
    default_binary: f64,
}

impl Times {
    /// The rows of `os`, tab-separated, `#` lines are comments:
    /// `bin <os> <binary-id> <test count> <total seconds>` and
    /// `test <os> <binary-id> <seconds> <test name>`.
    pub fn parse(text: &str, os: &str) -> Result<Times, String> {
        let mut times = Times::default();
        for (n, line) in text.lines().enumerate() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = || format!("{TIMES_FILE}:{}: bad row `{line}`", n + 1);
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() != 5 {
                return Err(bad());
            }
            if cols[1] != os {
                continue;
            }
            match cols[0] {
                "bin" => {
                    let count = cols[3].parse().map_err(|_| bad())?;
                    let total = cols[4].parse().map_err(|_| bad())?;
                    times.binaries.insert(cols[2].to_string(), (count, total));
                }
                "test" => {
                    let seconds = cols[3].parse().map_err(|_| bad())?;
                    times.tests.entry(cols[2].to_string()).or_default().insert(cols[4].to_string(), seconds);
                }
                _ => return Err(bad()),
            }
        }
        for (id, &(count, total)) in &times.binaries {
            let listed = times.tests.get(id);
            let n = listed.map_or(0, BTreeMap::len) as u64;
            let sum: f64 = listed.map_or(0.0, |t| t.values().sum());
            if count > n {
                times.small.insert(id.clone(), ((total - sum) / (count - n) as f64).max(0.0));
            }
        }
        let mut totals: Vec<f64> = times.binaries.values().map(|b| b.1).collect();
        totals.sort_by(f64::total_cmp);
        times.default_binary = totals.get(totals.len() / 2).copied().unwrap_or(DEFAULT_BINARY_SECONDS);
        Ok(times)
    }

    fn binary(&self, id: &str) -> f64 {
        self.binaries.get(id).map_or(self.default_binary, |b| b.1)
    }

    fn test(&self, binary: &str, name: &str) -> f64 {
        if let Some(&t) = self.tests.get(binary).and_then(|t| t.get(name)) {
            return t;
        }
        self.small.get(binary).copied().unwrap_or(DEFAULT_TEST_SECONDS)
    }
}

/// A test binary of the build, from nextest's listing.
#[derive(Debug, Clone)]
pub struct Binary {
    pub id: String,
    /// `None` when the listing skipped the binary (only split candidates are listed without a GPU
    /// shard).
    pub tests: Option<Vec<Test>>,
}

#[derive(Debug, Clone)]
pub struct Test {
    pub name: String,
    /// In nextest's `gpu` test group (only known when the listing's filterset was that group).
    pub gpu: bool,
}

/// The binaries of `cargo nextest list --message-format json --run-ignored all`, and the target
/// directory. With `gpu_listing`, the listing's filterset was the `gpu` group, so a test that
/// matches it is a GPU test.
pub fn parse_listing(json: &serde_json::Value, gpu_listing: bool) -> Result<(Vec<Binary>, PathBuf), String> {
    let target =
        json.pointer("/rust-build-meta/target-directory").and_then(serde_json::Value::as_str).ok_or("nextest list: no rust-build-meta.target-directory")?;
    let suites = json.get("rust-suites").and_then(serde_json::Value::as_object).ok_or("nextest list: no rust-suites")?;
    let mut binaries = Vec::new();
    for (id, suite) in suites {
        let tests = match suite.get("status").and_then(serde_json::Value::as_str) {
            Some("listed") => {
                let cases = suite.get("testcases").and_then(serde_json::Value::as_object).ok_or_else(|| format!("nextest list: {id} has no testcases"))?;
                let tests = cases
                    .iter()
                    .map(|(name, case)| {
                        let matches = case.pointer("/filter-match/status").and_then(serde_json::Value::as_str) == Some("matches");
                        Test { name: name.clone(), gpu: gpu_listing && matches }
                    })
                    .collect();
                Some(tests)
            }
            Some("skipped") => None,
            other => return Err(format!("nextest list: {id} has status {other:?}")),
        };
        binaries.push(Binary { id: id.clone(), tests });
    }
    Ok((binaries, PathBuf::from(target)))
}

/// A share of a split binary's tests.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Part {
    /// Every test (the binary turned out light enough to stay whole).
    All,
    /// Every test under this module path.
    Module(String),
    /// One test.
    Test(String),
}

impl Part {
    fn filter(&self) -> String {
        match self {
            Part::All => "all()".to_string(),
            Part::Module(path) => format!("test(/^{}::/)", regex_escape(path)),
            Part::Test(name) => format!("test(={name})"),
        }
    }

    fn matches(&self, name: &str) -> bool {
        match self {
            Part::All => true,
            Part::Module(path) => name.strip_prefix(path.as_str()).is_some_and(|rest| rest.starts_with("::")),
            Part::Test(test) => name == test,
        }
    }
}

fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if !(c.is_alphanumeric() || c == '_' || c == ':') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// What one shard runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Shard {
    /// The binaries in the shard's archive.
    pub binaries: BTreeSet<String>,
    /// The split binaries among them, with the parts of their tests this shard runs.
    pub parts: BTreeMap<String, Vec<Part>>,
    /// Estimated test-seconds, for the log.
    pub seconds: f64,
}

impl Shard {
    /// The filterset that keeps the split binaries to this shard's parts; `None` when the shard runs
    /// whole binaries only.
    pub fn filter(&self) -> Option<String> {
        if self.parts.is_empty() {
            return None;
        }
        let split: Vec<String> = self.parts.keys().map(|b| format!("binary_id(={b})")).collect();
        let mut terms = vec![format!("not ({})", split.join(" | "))];
        for (binary, parts) in &self.parts {
            let parts: Vec<String> = parts.iter().map(Part::filter).collect();
            terms.push(format!("(binary_id(={binary}) & ({}))", parts.join(" | ")));
        }
        Some(terms.join(" | "))
    }

    /// The `cargo nextest archive -E` filterset that writes this shard's archive. A shard can be
    /// empty when a pull request's build leaves out most packages ($PHOTOCRAFT_CI_EXCLUDE).
    pub fn archive_filter(&self) -> String {
        if self.binaries.is_empty() {
            return "none()".to_string();
        }
        let ids: Vec<String> = self.binaries.iter().map(|b| format!("binary_id(={b})")).collect();
        ids.join(" | ")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// Shard 1 runs the `gpu` group alone.
    pub gpu_shard: bool,
    pub shards: Vec<Shard>,
}

impl Plan {
    /// The non-GPU shards.
    fn cpu_shards(&self) -> &[Shard] {
        &self.shards[usize::from(self.gpu_shard)..]
    }

    /// One line per shard, for the build log.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for (k, shard) in self.shards.iter().enumerate() {
            let what = if self.gpu_shard && k == 0 { " (the gpu group)".to_string() } else { format!(", about {:.0} test-s", shard.seconds) };
            if shard.binaries.is_empty() {
                out += &format!("shard {}: no tests\n", k + 1);
                continue;
            }
            let split: Vec<String> = shard.parts.iter().map(|(b, p)| format!("{b} ({} parts)", p.len())).collect();
            let split = if split.is_empty() { String::new() } else { format!("; part of {}", split.join(", ")) };
            out += &format!("shard {}: {} binaries{what}{split}\n", k + 1, shard.binaries.len());
        }
        out
    }
}

/// The binaries a shard's share is too small for, and that share. A split binary's tests are spread
/// over the shards; every other binary stays whole.
fn split_binaries(binaries: &[Binary], times: &Times, cpu_shards: usize) -> (BTreeSet<String>, f64) {
    let total: f64 = binaries.iter().map(|b| times.binary(&b.id)).sum();
    let share = total / cpu_shards as f64;
    let split = binaries.iter().filter(|b| times.binary(&b.id) > share / 2.0).map(|b| b.id.clone()).collect();
    (split, share)
}

/// The binaries nextest has to list for the plan when there is no GPU shard: the split ones.
pub fn binaries_to_list(ids: &[String], times: &Times, shards: usize) -> BTreeSet<String> {
    let binaries: Vec<Binary> = ids.iter().map(|id| Binary { id: id.clone(), tests: None }).collect();
    split_binaries(&binaries, times, shards).0
}

#[derive(Default)]
struct Node {
    weight: f64,
    modules: BTreeMap<String, Node>,
    tests: BTreeMap<String, f64>,
}

/// Cuts a binary's tests into parts along the module tree: a module at or under `limit` is one part,
/// a heavier one is cut into its submodules and its own tests. Every test is in exactly one part.
fn cut_into_parts(tests: &[(String, f64)], limit: f64) -> Vec<(Part, f64)> {
    let mut root = Node::default();
    for (name, weight) in tests {
        let mut node = &mut root;
        node.weight += weight;
        let mut segments: Vec<&str> = name.split("::").collect();
        let leaf = segments.pop().unwrap_or_default();
        for segment in segments {
            node = node.modules.entry(segment.to_string()).or_default();
            node.weight += weight;
        }
        *node.tests.entry(leaf.to_string()).or_default() += weight;
    }
    let mut parts = Vec::new();
    if root.weight <= limit {
        parts.push((Part::All, root.weight));
    } else {
        cut(&root, "", limit, &mut parts);
    }
    parts
}

fn cut(node: &Node, prefix: &str, limit: f64, parts: &mut Vec<(Part, f64)>) {
    let path = |segment: &str| if prefix.is_empty() { segment.to_string() } else { format!("{prefix}::{segment}") };
    for (segment, child) in &node.modules {
        if child.weight <= limit {
            parts.push((Part::Module(path(segment)), child.weight));
        } else {
            cut(child, &path(segment), limit, parts);
        }
    }
    for (segment, &weight) in &node.tests {
        parts.push((Part::Test(path(segment)), weight));
    }
}

/// Assigns the binaries (and the parts of split binaries) to `shards` shards, heaviest first onto
/// the lightest shard. With `gpu_shard`, shard 1 takes every binary with a GPU test.
pub fn plan(binaries: &[Binary], times: &Times, shards: usize, gpu_shard: bool) -> Result<Plan, String> {
    let first = usize::from(gpu_shard);
    if shards <= first {
        return Err(format!("{shards} shard(s) leave none for the tests outside the gpu group"));
    }
    let cpu_shards = shards - first;
    let gpu = |t: &Test| gpu_shard && t.gpu;
    // Binaries whose every test is a GPU test run on the GPU shard only.
    let cpu: Vec<Binary> = binaries.iter().filter(|b| b.tests.as_ref().is_none_or(|t| t.iter().any(|t| !gpu(t)))).cloned().collect();
    let (split, share) = split_binaries(&cpu, times, cpu_shards);
    let limit = share / PARTS_PER_SHARD;

    let mut items: Vec<(f64, &str, Option<Part>)> = Vec::new();
    for binary in &cpu {
        // A binary the listing skipped stays whole.
        if let Some(tests) = binary.tests.as_ref().filter(|_| split.contains(&binary.id)) {
            let weighted: Vec<(String, f64)> = tests.iter().filter(|t| !gpu(t)).map(|t| (t.name.clone(), times.test(&binary.id, &t.name))).collect();
            for (part, weight) in cut_into_parts(&weighted, limit) {
                items.push((weight, &binary.id, Some(part)));
            }
        } else {
            items.push((times.binary(&binary.id), &binary.id, None));
        }
    }
    items.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| (a.1, &a.2).cmp(&(b.1, &b.2))));

    let mut plan = Plan { gpu_shard, shards: vec![Shard::default(); shards] };
    for (weight, binary, part) in items {
        let lightest = (first..shards).min_by(|&a, &b| plan.shards[a].seconds.total_cmp(&plan.shards[b].seconds)).unwrap_or(first);
        let shard = &mut plan.shards[lightest];
        shard.seconds += weight;
        shard.binaries.insert(binary.to_string());
        if let Some(part) = part {
            shard.parts.entry(binary.to_string()).or_default().push(part);
        }
    }
    if gpu_shard {
        let gpu = binaries.iter().filter(|b| b.tests.as_ref().is_some_and(|t| t.iter().any(|t| t.gpu)));
        plan.shards[0].binaries = gpu.map(|b| b.id.clone()).collect();
    }
    for shard in &mut plan.shards {
        for parts in shard.parts.values_mut() {
            parts.sort();
        }
    }
    verify(&plan, binaries)?;
    Ok(plan)
}

/// The binaries in a nextest archive's `target/nextest/binaries-metadata.json`.
pub fn archived_binaries(json: &serde_json::Value) -> Result<BTreeSet<String>, String> {
    let binaries = json.get("rust-binaries").and_then(serde_json::Value::as_object).ok_or("binaries-metadata.json: no rust-binaries")?;
    binaries
        .values()
        .map(|b| {
            b.get("binary-id")
                .and_then(serde_json::Value::as_str)
                .map(String::from)
                .ok_or_else(|| "binaries-metadata.json: a binary without binary-id".to_string())
        })
        .collect()
}

/// Shard `k` (1-based) of `plan`, checked against how the shard runs and what its archive holds:
/// `gpu` is `Some(true)` for `--gpu`, `Some(false)` for `--no-gpu` and `None` for every test, and
/// `archived` the binaries of the archive. A plan from another build or another mode would lose or
/// repeat tests without an error.
pub fn shard_to_run<'a>(plan: &'a Plan, k: usize, gpu: Option<bool>, archived: &BTreeSet<String>) -> Result<&'a Shard, String> {
    let shard = plan.shards.get(k.wrapping_sub(1)).ok_or_else(|| format!("the shard plan has no shard {k}"))?;
    let expected = match (plan.gpu_shard, k) {
        (false, _) => None,
        (true, 1) => Some(true),
        (true, _) => Some(false),
    };
    if gpu != expected {
        let mode = |g: Option<bool>| match g {
            Some(true) => "the gpu group",
            Some(false) => "the tests outside the gpu group",
            None => "every test",
        };
        return Err(format!("shard {k} runs {}, but the shard plan has it run {}", mode(gpu), mode(expected)));
    }
    if &shard.binaries != archived {
        return Err(format!("shard {k}'s archive does not hold the binaries its plan names: the plan is from another build"));
    }
    Ok(shard)
}

/// Every test runs on exactly one shard: each binary with tests outside the gpu group on one shard
/// or, split, every such test in exactly one part across the shards that hold it, and with a GPU
/// shard, that shard holds exactly the binaries with GPU tests. Without a GPU shard every test
/// counts as outside the group.
pub fn verify(plan: &Plan, binaries: &[Binary]) -> Result<(), String> {
    let known: BTreeSet<&str> = binaries.iter().map(|b| b.id.as_str()).collect();
    for (k, shard) in plan.shards.iter().enumerate() {
        if let Some(unknown) = shard.binaries.iter().find(|b| !known.contains(b.as_str())) {
            return Err(format!("shard {} holds {unknown}, which the build has not", k + 1));
        }
    }
    if plan.gpu_shard {
        let gpu: BTreeSet<String> = binaries.iter().filter(|b| b.tests.as_ref().is_some_and(|t| t.iter().any(|t| t.gpu))).map(|b| b.id.clone()).collect();
        if plan.shards[0].binaries != gpu {
            return Err("the GPU shard does not hold exactly the binaries with GPU tests".into());
        }
    }
    let cpu = plan.cpu_shards();
    for binary in binaries {
        let tests: Option<Vec<&Test>> = binary.tests.as_ref().map(|t| t.iter().filter(|t| !(plan.gpu_shard && t.gpu)).collect());
        let holders: Vec<&Shard> = cpu.iter().filter(|s| s.binaries.contains(&binary.id)).collect();
        if tests.as_ref().is_some_and(Vec::is_empty) {
            if !holders.is_empty() {
                return Err(format!("{} has only GPU tests but is on a CPU shard", binary.id));
            }
            continue;
        }
        let split = holders.iter().filter(|s| s.parts.contains_key(&binary.id)).count();
        if split == 0 {
            if holders.len() != 1 {
                return Err(format!("{} is on {} shards", binary.id, holders.len()));
            }
            continue;
        }
        if split != holders.len() {
            return Err(format!("{} is split, but a shard holds it whole", binary.id));
        }
        let tests = tests.ok_or_else(|| format!("{} is split, but the listing skipped it", binary.id))?;
        for test in tests {
            let runs: usize = holders.iter().map(|s| s.parts[&binary.id].iter().filter(|p| p.matches(&test.name)).count()).sum();
            if runs != 1 {
                return Err(format!("{} {} would run {runs} times", binary.id, test.name));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary(id: &str, tests: &[&str]) -> Binary {
        Binary { id: id.into(), tests: Some(tests.iter().map(|t| Test { name: t.to_string(), gpu: t.contains("gpu_tests::") }).collect()) }
    }

    const TIMES: &str = "\
# kind\tos\tbinary\t...
bin\tlinux\tui\t6\t60
test\tlinux\tui\t20\tdialogs::slow
test\tlinux\tui\t16\tdialogs::other::slower
bin\tlinux\tengine\t2\t12
bin\tlinux\talgo\t1\t8
bin\twindows\tui\t6\t600
";

    fn build() -> Vec<Binary> {
        vec![
            binary("ui", &["dialogs::slow", "dialogs::other::slower", "dialogs::other::quick", "canvas::a", "canvas::b", "top", "canvas::gpu_tests::g"]),
            binary("engine", &["e1", "e2"]),
            binary("algo", &["a1"]),
            binary("gpu::it", &["gpu_tests::only"]),
            binary("new", &["n1"]),
        ]
    }

    #[test]
    fn times_fill_in_what_the_file_does_not_list() {
        let times = Times::parse(TIMES, "linux").unwrap();
        assert_eq!(times.test("ui", "dialogs::slow"), 20.0);
        // (60 - 36) s over the 4 tests the file does not list one by one.
        assert_eq!(times.test("ui", "canvas::a"), 6.0);
        assert_eq!(times.test("nowhere", "x"), DEFAULT_TEST_SECONDS);
        assert_eq!(times.binary("new"), 12.0, "the median binary");
        assert_eq!(Times::parse(TIMES, "windows").unwrap().binary("ui"), 600.0);
        assert_eq!(Times::parse("", "linux").unwrap().binary("x"), DEFAULT_BINARY_SECONDS);
        assert!(Times::parse("bin\tlinux\tui\tsix\t1", "linux").is_err());
    }

    #[test]
    fn every_test_runs_once_and_the_heavy_binary_is_spread() {
        let times = Times::parse(TIMES, "linux").unwrap();
        let plan = plan(&build(), &times, 3, true).unwrap();
        assert_eq!(plan.shards[0].binaries, BTreeSet::from(["gpu::it".to_string(), "ui".to_string()]));
        assert!(plan.cpu_shards().iter().all(|s| !s.binaries.contains("gpu::it")));
        let holders = plan.cpu_shards().iter().filter(|s| s.binaries.contains("ui")).count();
        assert_eq!(holders, 2, "{}", plan.summary());
        // Balanced to within the heaviest item (one 20 s test).
        let loads: Vec<f64> = plan.cpu_shards().iter().map(|s| s.seconds).collect();
        assert!((loads[0] - loads[1]).abs() <= 20.0, "{loads:?}");
        // Tampering is caught: a part dropped, or a whole binary on two shards.
        let mut broken = plan.clone();
        broken.shards[1].parts.values_mut().next().unwrap().pop();
        assert!(verify(&broken, &build()).is_err());
        let mut twice = plan.clone();
        let whole = twice.shards[1].binaries.iter().find(|b| !twice.shards[1].parts.contains_key(*b)).unwrap().clone();
        twice.shards[2].binaries.insert(whole);
        assert!(verify(&twice, &build()).is_err());
    }

    #[test]
    fn without_a_gpu_shard_unlisted_binaries_stay_whole() {
        let times = Times::parse(TIMES, "linux").unwrap();
        let ids: Vec<String> = build().iter().map(|b| b.id.clone()).collect();
        assert_eq!(binaries_to_list(&ids, &times, 2), BTreeSet::from(["ui".to_string()]));
        let listing: Vec<Binary> = build().into_iter().map(|b| if b.id == "ui" { b } else { Binary { tests: None, ..b } }).collect();
        let plan = plan(&listing, &times, 2, false).unwrap();
        let all: BTreeSet<String> = plan.shards.iter().flat_map(|s| s.binaries.iter().cloned()).collect();
        assert_eq!(all.len(), 5);
        assert!(plan.shards.iter().all(|s| s.parts.get("ui").is_none_or(|p| !p.is_empty())));
        // More shards than work: the extra shards are empty, and still every test runs once.
        let wide = super::plan(&build(), &times, 20, false).unwrap();
        assert!(wide.shards.iter().any(|s| s.binaries.is_empty()));
        assert_eq!(Shard::default().archive_filter(), "none()");
    }

    #[test]
    fn parts_follow_the_module_tree_and_render_as_filtersets() {
        let tests: Vec<(String, f64)> =
            [("a::b::x", 1.0), ("a::b::y", 1.0), ("a::c", 5.0), ("a::b", 0.5), ("top", 1.0)].iter().map(|(n, w)| (n.to_string(), *w)).collect();
        let parts = cut_into_parts(&tests, 3.0);
        let names: Vec<&Part> = parts.iter().map(|p| &p.0).collect();
        assert_eq!(names, [&Part::Module("a::b".into()), &Part::Test("a::b".into()), &Part::Test("a::c".into()), &Part::Test("top".into())]);
        for (name, _) in &tests {
            assert_eq!(parts.iter().filter(|p| p.0.matches(name)).count(), 1, "{name}");
        }
        assert_eq!(cut_into_parts(&tests, 100.0)[0].0, Part::All);
        let shard = Shard {
            binaries: ["ui".to_string(), "e".to_string()].into(),
            parts: [("ui".to_string(), vec![Part::Module("a::b".into()), Part::Test("top".into())])].into(),
            seconds: 0.0,
        };
        assert_eq!(shard.filter().unwrap(), "not (binary_id(=ui)) | (binary_id(=ui) & (test(/^a::b::/) | test(=top)))");
        assert_eq!(shard.archive_filter(), "binary_id(=e) | binary_id(=ui)");
        assert_eq!(Shard::default().filter(), None);
    }

    #[test]
    fn a_shard_runs_only_the_plan_its_archive_and_mode_match() {
        let times = Times::parse(TIMES, "linux").unwrap();
        let plan = plan(&build(), &times, 3, true).unwrap();
        let held = |k: usize| plan.shards[k - 1].binaries.clone();
        assert!(shard_to_run(&plan, 1, Some(true), &held(1)).is_ok());
        assert!(shard_to_run(&plan, 2, Some(false), &held(2)).is_ok());
        assert!(shard_to_run(&plan, 1, Some(false), &held(1)).is_err(), "the GPU shard run as a CPU shard");
        assert!(shard_to_run(&plan, 2, None, &held(2)).is_err(), "a GPU-shard plan run without --gpu-shard");
        assert!(shard_to_run(&plan, 2, Some(false), &held(3)).is_err(), "another shard's archive");
        assert!(shard_to_run(&plan, 4, Some(false), &held(2)).is_err());
        let json: serde_json::Value = serde_json::from_str(r#"{"rust-binaries": {"a": {"binary-id": "a"}, "b::it": {"binary-id": "b::it"}}}"#).unwrap();
        assert_eq!(archived_binaries(&json).unwrap(), BTreeSet::from(["a".to_string(), "b::it".to_string()]));
    }

    #[test]
    fn the_listing_marks_gpu_tests_and_skipped_binaries() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"rust-build-meta": {"target-directory": "/t"}, "rust-suites": {
                "ui": {"status": "listed", "testcases": {
                    "a::gpu_tests::g": {"filter-match": {"status": "matches"}},
                    "a::t": {"filter-match": {"status": "mismatch", "reason": "expression"}}}},
                "engine": {"status": "skipped", "testcases": {}}}}"#,
        )
        .unwrap();
        let (binaries, target) = parse_listing(&json, true).unwrap();
        assert_eq!(target, PathBuf::from("/t"));
        let ui = binaries.iter().find(|b| b.id == "ui").unwrap().tests.as_ref().unwrap();
        assert_eq!(ui.iter().filter(|t| t.gpu).map(|t| t.name.as_str()).collect::<Vec<_>>(), ["a::gpu_tests::g"]);
        assert!(binaries.iter().find(|b| b.id == "engine").unwrap().tests.is_none());
        assert!(parse_listing(&json, false).unwrap().0.iter().flat_map(|b| b.tests.iter().flatten()).all(|t| !t.gpu));
    }
}
