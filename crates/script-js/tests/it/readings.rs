//! Past the bound on clock and `Math.random()` readings, the run stops at the reading itself.
// Native only: the tests time runs with `std::time`, which has no clock on wasm32-unknown-unknown.
#![cfg(not(target_arch = "wasm32"))]

use photocraft_script_js::tape::MAX_READINGS;
use photocraft_script_js::{Host, HostError, RunOptions, Runner, Step, Unlimited};
use serde_json::{Value, json};

struct Quiet;

impl Host for Quiet {
    fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
        Ok(json!({"active": null, "documents": []}))
    }

    fn write_line(&mut self, _: &str) {}
}

fn stops_at_the_bound(src: &str) {
    // Without a host call in the loop, only the reading itself can stop it before the loop limit.
    let Step::Failed(m) = Runner::new(src, Box::new(Quiet), Box::new(Unlimited), RunOptions::default()).unwrap().step() else { panic!("{src}: not stopped") };
    assert!(m.contains(&format!("more than {MAX_READINGS} times")), "{src}: {m}");
}

#[test]
fn endless_random_reads_stop_at_the_bound() {
    stops_at_the_bound("var n = 0; for (;;) { try { Math.random(); } catch (e) {} n++; }");
}

#[test]
fn endless_date_now_reads_stop_at_the_bound() {
    stops_at_the_bound("for (;;) { try { Date.now(); } catch (e) {} }");
}
