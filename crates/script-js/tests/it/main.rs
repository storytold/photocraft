//! The crate's integration tests, linked into one test binary.

mod allowlist;
mod isolate;
mod jobs;
mod panic;
mod run;
mod session_host;

/// A clock tests move by hand, for time limits that must not depend on how fast the machine is.
#[derive(Default)]
pub(crate) struct ManualClock(std::sync::Mutex<std::time::Duration>);

impl ManualClock {
    pub(crate) fn advance(&self, by: std::time::Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl photocraft_script_js::Now for ManualClock {
    fn now(&self) -> std::time::Duration {
        *self.0.lock().unwrap()
    }
}
