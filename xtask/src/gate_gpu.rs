//! The nextest `gpu` test group, for the CI gate.
//!
//! The gate runs that group in `scripts/ci-gpu-lavapipe.py`, on lavapipe with Vulkan forced, where
//! a skipped GPU comparison fails the run. `cargo xtask ci` therefore leaves the group out when the
//! gate says so, instead of running every GPU test a second time (#15445). Both sides read the
//! group's filters from `.config/nextest.toml`, so the two runs always split the same way.

use serde::Deserialize;

/// The nextest configuration, relative to the workspace root.
pub const CONFIG: &str = ".config/nextest.toml";

/// Set to [`SEPARATE`] by a caller that runs the `gpu` group itself after `cargo xtask ci`.
pub const GROUP_VAR: &str = "PHOTOCRAFT_GATE_GPU_GROUP";
/// The value of [`GROUP_VAR`] that leaves the group out of the gate's tests.
pub const SEPARATE: &str = "separate";

#[derive(Deserialize)]
struct Nextest {
    profile: Profiles,
}

#[derive(Deserialize)]
struct Profiles {
    default: Profile,
}

#[derive(Deserialize)]
struct Profile {
    #[serde(default)]
    overrides: Vec<Override>,
}

#[derive(Deserialize)]
struct Override {
    filter: Option<String>,
    #[serde(rename = "test-group")]
    test_group: Option<String>,
}

/// The filterset of the `gpu` group: the default profile's overrides that assign it, joined as
/// `scripts/ci-gpu-lavapipe.py` joins them. No such override is an error, not an empty filter:
/// excluding nothing here would look like success while the group ran twice, and selecting
/// nothing there is already refused. A `gpu` override without a filter is an error on both sides.
fn gpu_group_filter(nextest_toml: &str) -> Result<String, String> {
    let config: Nextest = toml::from_str(nextest_toml).map_err(|e| format!("{CONFIG}: {e}"))?;
    let mut filters = Vec::new();
    for row in config.profile.default.overrides.iter().filter(|row| row.test_group.as_deref() == Some("gpu")) {
        let filter = row.filter.as_deref().ok_or_else(|| format!("{CONFIG}: a gpu test group override has no filter"))?;
        filters.push(format!("({filter})"));
    }
    if filters.is_empty() {
        return Err(format!("{CONFIG}: the gpu test group has no filters"));
    }
    Ok(filters.join(" | "))
}

/// The filterset of every test outside the `gpu` group.
pub fn without_gpu_group(nextest_toml: &str) -> Result<String, String> {
    Ok(format!("not ({})", gpu_group_filter(nextest_toml)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_GPU_ROWS: &str = r#"
[test-groups]
gpu = { max-threads = 4 }

[[profile.default.overrides]]
filter = 'package(a)'
test-group = 'gpu'

[[profile.default.overrides]]
filter = 'package(b)'
test-group = 'cpu'

[[profile.default.overrides]]
filter = 'test(c)'
test-group = 'gpu'

[profile.ci]
fail-fast = false
"#;

    #[test]
    fn gpu_rows_are_joined_as_the_lavapipe_script_joins_them() {
        assert_eq!(gpu_group_filter(TWO_GPU_ROWS).unwrap(), "(package(a)) | (test(c))");
        assert_eq!(without_gpu_group(TWO_GPU_ROWS).unwrap(), "not ((package(a)) | (test(c)))");
    }

    #[test]
    fn a_config_without_a_gpu_row_is_refused() {
        let none = "[[profile.default.overrides]]\nfilter = 'package(b)'\ntest-group = 'cpu'\n";
        assert!(gpu_group_filter(none).unwrap_err().contains("no filters"));
        assert!(without_gpu_group("[profile.default]\n").is_err());
    }

    #[test]
    fn a_gpu_row_without_a_filter_is_refused() {
        let unfiltered = "[[profile.default.overrides]]\ntest-group = 'gpu'\n";
        assert!(gpu_group_filter(unfiltered).unwrap_err().contains("no filter"));
    }

    /// What is wrong with a workflow that leaves the `gpu` group out of `cargo xtask ci`, if
    /// anything. Leaving it out is only safe while the same command goes on to the checked run:
    /// the variable and `scripts/ci-gpu-lavapipe.py` are two lines of `ci.yml`, and deleting the
    /// second alone would stop every GPU test on the Linux gate with nothing turning red.
    fn handoff_problem(workflow: &str) -> Option<&'static str> {
        let lines = || workflow.lines().map(str::trim);
        if !lines().any(|line| line == format!("{GROUP_VAR}={SEPARATE}")) {
            return None;
        }
        let Some(command) = lines().filter_map(|line| line.strip_prefix("command: ")).find(|command| command.contains("cargo xtask ci")) else {
            return Some("the gpu group is left out, but no command runs `cargo xtask ci`");
        };
        if command.contains("cargo xtask ci && python3 scripts/ci-gpu-lavapipe.py") {
            None
        } else {
            Some("the gpu group is left out of `cargo xtask ci`, but the command does not go on to scripts/ci-gpu-lavapipe.py")
        }
    }

    const GATE: &str =
        "      env: |\n        PHOTOCRAFT_GATE_GPU_GROUP=separate\n      command: python3 a.py && cargo xtask ci && python3 scripts/ci-gpu-lavapipe.py\n";

    #[test]
    fn leaving_the_gpu_group_out_needs_the_checked_run_in_the_same_command() {
        assert_eq!(handoff_problem(GATE), None);
        // The checked run deleted, or no longer reached only when the gate passed.
        assert!(handoff_problem(&GATE.replace(" && python3 scripts/ci-gpu-lavapipe.py", "")).is_some());
        assert!(handoff_problem(&GATE.replace("ci && python3 scripts", "ci || python3 scripts")).is_some());
        assert!(handoff_problem(&GATE.replace("command: ", "warm-command: ")).is_some());
        // Not left out: `cargo xtask ci` runs the group itself, with or without the checked run.
        let all_here = GATE.replace("        PHOTOCRAFT_GATE_GPU_GROUP=separate\n", "");
        assert_eq!(handoff_problem(&all_here), None);
        assert_eq!(handoff_problem(&all_here.replace(" && python3 scripts/ci-gpu-lavapipe.py", "")), None);
    }

    #[test]
    fn the_ci_workflow_hands_the_gpu_group_to_the_checked_run() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/workflows/ci.yml");
        assert_eq!(handoff_problem(&std::fs::read_to_string(path).unwrap()), None);
    }

    #[test]
    fn the_workspace_config_has_a_gpu_group() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(CONFIG);
        let filter = gpu_group_filter(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(filter.contains("package(photocraft-gpu)"), "{filter}");
    }
}
