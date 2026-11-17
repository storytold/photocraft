//! The promise job queue is bounded.

use photocraft_script_js::{Host, HostError, RunOptions, Runner, Step, Unlimited};
use serde_json::{Value, json};

struct Quiet;

impl Host for Quiet {
    fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
        Ok(json!({"active": null, "documents": []}))
    }

    fn write_line(&mut self, _: &str) {}
}

fn step(src: &str) -> Step {
    Runner::new(src, Box::new(Quiet), Box::new(Unlimited), RunOptions::default()).unwrap().step()
}

#[test]
fn queuing_too_many_promise_reactions_is_a_named_error() {
    // Queued without a host call in between, and with one.
    for src in [
        "for (var i = 0; i < 200000; i++) Promise.resolve().then(function () {});",
        "for (var i = 0; i < 200000; i++) { Promise.resolve().then(function () {}); } $.writeln('queued');",
    ] {
        let Step::Failed(m) = step(src) else { panic!("{src}: not stopped") };
        assert!(m.contains("promise reactions"), "{m}");
    }
}

#[test]
fn many_reactions_within_the_limit_still_run() {
    assert_eq!(step("var n = 0; for (var i = 0; i < 50000; i++) Promise.resolve().then(function () { n++; });"), Step::Done);
}
