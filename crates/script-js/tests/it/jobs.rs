//! The promise job queue is bounded.
// Native only: the tests time runs with `std::time`, which has no clock on wasm32-unknown-unknown.
#![cfg(not(target_arch = "wasm32"))]

use photocraft_script_js::{Host, HostError, MAX_QUEUED_REACTIONS, ModalKind, RunOptions, Runner, Unlimited};
use serde_json::{Value, json};

struct Quiet;

impl Host for Quiet {
    fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
        Ok(json!({"active": null, "documents": []}))
    }

    fn write_line(&mut self, _: &str) {}

    fn modal(&mut self, _: ModalKind, _: &Value) -> Result<Value, HostError> {
        Ok(Value::Null)
    }
}

fn run(src: &str) -> Result<(), String> {
    Runner::new(src, Box::new(Quiet), Box::new(Unlimited), RunOptions::default()).unwrap().run()
}

#[test]
fn queuing_too_many_promise_reactions_is_a_named_error() {
    // One past the limit, queued without a host call after it, and with one.
    let over = MAX_QUEUED_REACTIONS + 1;
    for src in [
        format!("for (var i = 0; i < {over}; i++) Promise.resolve().then(function () {{}});"),
        format!("for (var i = 0; i < {over}; i++) {{ Promise.resolve().then(function () {{}}); }} $.writeln('queued');"),
    ] {
        let src = src.as_str();
        let Err(m) = run(src) else { panic!("{src}: not stopped") };
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

    fn modal(&mut self, _: ModalKind, _: &Value) -> Result<Value, HostError> {
        Ok(Value::Null)
    }
}

#[test]
fn many_reactions_within_the_limit_still_run() {
    let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    // Exactly the limit, all waiting at once.
    let n = MAX_QUEUED_REACTIONS;
    let src = format!("var n = 0; for (var i = 0; i < {n}; i++) Promise.resolve().then(function () {{ if (++n == {n}) $.writeln(n); }});");
    let result = Runner::new(&src, Box::new(Lines(lines.clone())), Box::new(Unlimited), RunOptions::default()).unwrap().run();
    assert_eq!(result, Ok(()));
    assert_eq!(*lines.borrow(), [n.to_string()], "every reaction ran");
}
