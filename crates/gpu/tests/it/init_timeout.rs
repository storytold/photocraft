//! A request that times out leaves its initialization thread inside the driver. The process must
//! be able to wait for that thread before it exits: exiting under it crashes Lavapipe, which is
//! how two passing headless GPU tests ended in a SIGSEGV on CI (#9824).

use std::sync::PoisonError;
use std::time::Duration;

use photocraft_gpu::{GpuContextError, NativeContext, NativeContextOptions};

use crate::GPU_LOCK;

#[test]
fn a_timed_out_request_can_be_waited_for_before_exit() {
    let _gpu = GPU_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let opts = NativeContextOptions { timeout: Some(Duration::ZERO), ..Default::default() };
    let res = NativeContext::request(&opts);
    assert!(
        matches!(&res, Err(GpuContextError::NoAdapter(msg)) if msg.contains("timed out")),
        "a zero timeout must give up before the adapter is ready: {res:?}"
    );
    assert!(NativeContext::wait_for_abandoned_requests(Duration::from_secs(120)), "the abandoned initialization thread never finished");
    assert!(NativeContext::wait_for_abandoned_requests(Duration::ZERO), "a finished thread must not count as running");
}
