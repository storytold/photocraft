//! A panic during an attempt fails the run instead of unwinding into the caller.
// Native only: the tests time runs with `std::time`, which has no clock on wasm32-unknown-unknown.
#![cfg(not(target_arch = "wasm32"))]

use photocraft_script_js::{Host, HostError, RunOptions, Runner, Step, Unlimited};
use serde_json::Value;

/// A host with a bug: every command panics.
struct Broken;

impl Host for Broken {
    fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
        panic!("host bug")
    }

    fn write_line(&mut self, _: &str) {}
}

#[test]
fn a_panic_inside_an_attempt_fails_the_run() {
    let mut r = Runner::new("try { app.documents; } finally { $.writeln('x'); }", Box::new(Broken), Box::new(Unlimited), RunOptions::default()).unwrap();
    let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r.step())).expect("the panic stayed inside the run");
    let Step::Failed(m) = step else { panic!("{step:?}") };
    assert!(m.contains("internal error"), "{m}");
    assert_eq!(r.step(), Step::Failed(m), "the run stays failed");
}
