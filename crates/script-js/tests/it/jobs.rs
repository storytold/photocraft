//! The promise job queue is bounded.
// Native only: the tests time runs with `std::time`, which has no clock on wasm32-unknown-unknown.
#![cfg(not(target_arch = "wasm32"))]

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

/// Records the lines a run writes.
struct Lines(std::rc::Rc<std::cell::RefCell<Vec<String>>>);

impl Host for Lines {
    fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
        Ok(json!({"active": null, "documents": []}))
    }

    fn write_line(&mut self, line: &str) {
        self.0.borrow_mut().push(line.to_owned());
    }
}

#[test]
fn many_reactions_within_the_limit_still_run() {
    let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let src = "var n = 0; for (var i = 0; i < 50000; i++) Promise.resolve().then(function () { if (++n == 50000) $.writeln(n); });";
    let step = Runner::new(src, Box::new(Lines(lines.clone())), Box::new(Unlimited), RunOptions::default()).unwrap().step();
    assert_eq!(step, Step::Done);
    assert_eq!(*lines.borrow(), ["50000"], "every reaction ran");
}
