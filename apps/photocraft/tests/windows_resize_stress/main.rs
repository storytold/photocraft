//! Resize stress test for the Windows desktop app (#7252): launches an isolated PhotoCraft, resizes its window
//! hard, and checks that it stays alive, keeps answering, ends up drawing at the window's size, and still
//! takes commands.
//!
//! Most scenarios drag the window's border with the real mouse (through `SendInput`), so the OS runs its modal
//! sizing loop exactly as it does for a person. That takes over the mouse and opens a window, so every one is
//! `#[ignore]`d and also needs `PHOTOCRAFT_RESIZE_STRESS=1`; CI, and a plain `cargo test -- --ignored` over the
//! workspace, never run them. How to run them, on which GPUs, and what they have found:
//! `docs/development.md` › Windows resize stress test.
//!
//! The pieces that decide what to do and what counts as a failure (`plan`) are plain data and run in every
//! `cargo test`, on every platform.

#![cfg_attr(not(windows), allow(dead_code))]

mod plan;

#[cfg(windows)]
mod control;
#[cfg(windows)]
mod run;
#[cfg(windows)]
mod win;

/// The opt-in guard itself, end to end: the tests that take the real mouse refuse to run unless
/// `PHOTOCRAFT_RESIZE_STRESS` is exactly `1`.
#[cfg(windows)]
mod opt_in {
    use std::process::Command;

    /// Run one of the ignored scenarios in a child of this test binary, with `value` as the opt-in variable (or none
    /// at all), and return whether it failed and everything it printed. The scenario is the one that does not use the
    /// mouse, so a guard that let it through would open a window and nothing worse.
    fn run_scenario_with(value: Option<&str>) -> (bool, String) {
        let mut child = Command::new(std::env::current_exe().expect("this test binary"));
        child.args(["--ignored", "--exact", "resize::resize_storm_through_the_window_api", "--test-threads=1", "--nocapture"]);
        child.env_remove("PHOTOCRAFT_RESIZE_STRESS");
        if let Some(value) = value {
            child.env("PHOTOCRAFT_RESIZE_STRESS", value);
        }
        let output = child.output().expect("run the child test binary");
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        (!output.status.success(), text)
    }

    #[test]
    fn the_stress_tests_refuse_to_run_unless_the_variable_is_exactly_1() {
        for value in [None, Some(""), Some("0"), Some("false"), Some("true"), Some("yes")] {
            let (failed, output) = run_scenario_with(value);
            assert!(failed, "{value:?} must not turn the stress tests on: {output}");
            assert!(output.contains("takes over the real mouse"), "{value:?} was not refused with the opt-in message: {output}");
            assert!(!output.contains("[resize-stress]"), "{value:?} started a run: {output}");
        }
    }
}

#[cfg(windows)]
mod resize {
    use crate::run::{Scenario, run};

    /// An empty window and its right border, as in the report.
    #[test]
    #[ignore = "moves the real mouse; see docs/development.md › Windows resize stress test"]
    fn right_border_sweep_on_an_empty_window() {
        run(Scenario::RightBorder);
    }

    #[test]
    #[ignore = "moves the real mouse; see docs/development.md › Windows resize stress test"]
    fn left_border_sweep() {
        run(Scenario::LeftBorder);
    }

    /// Both axes at once, with a document big enough that the canvas has real work to do.
    #[test]
    #[ignore = "moves the real mouse; see docs/development.md › Windows resize stress test"]
    fn corner_sweep_with_a_large_document() {
        run(Scenario::Corner);
    }

    /// The drag keeps going past the window's minimum size and back.
    #[test]
    #[ignore = "moves the real mouse; see docs/development.md › Windows resize stress test"]
    fn border_sweep_through_the_minimum_size() {
        run(Scenario::BelowMinimum);
    }

    #[test]
    #[ignore = "opens and maximizes a window; see docs/development.md › Windows resize stress test"]
    fn maximize_and_restore_cycles() {
        run(Scenario::MaximizeRestore);
    }

    /// Resizes from the program with `SetWindowPos`: the same `WM_SIZE` handling as a drag, without the OS's
    /// modal loop and without touching the mouse.
    #[test]
    #[ignore = "opens a window; see docs/development.md › Windows resize stress test"]
    fn resize_storm_through_the_window_api() {
        run(Scenario::ApiStorm);
    }

    /// A seeded mix of all of the above: speeds, borders, corners, quick re-grabs, maximizing.
    #[test]
    #[ignore = "moves the real mouse; see docs/development.md › Windows resize stress test"]
    fn randomized_soak() {
        run(Scenario::Soak);
    }
}
