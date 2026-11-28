//! A panic during a run fails the run instead of unwinding into the caller.
// Native only: the tests time runs with `std::time`, which has no clock on wasm32-unknown-unknown.
#![cfg(not(target_arch = "wasm32"))]

use photocraft_script_js::{Host, HostError, ModalKind, RunOptions, Runner, Unlimited};
use serde_json::Value;

/// A host with a bug: every command panics.
struct Broken;

impl Host for Broken {
    fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
        panic!("host bug")
    }

    fn write_line(&mut self, _: &str) {}

    fn modal(&mut self, _: ModalKind, _: &Value) -> Result<Value, HostError> {
        panic!("host bug")
    }
}

#[test]
fn a_panic_inside_a_run_fails_the_run() {
    for src in ["try { app.documents; } finally { $.writeln('x'); }", "alert(1);"] {
        let r = Runner::new(src, Box::new(Broken), Box::new(Unlimited), RunOptions::default()).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r.run())).expect("the panic stayed inside the run");
        assert!(result.as_ref().is_err_and(|m| m.contains("internal error")), "{src}: {result:?}");
    }
}
