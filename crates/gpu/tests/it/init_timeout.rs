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
    // `NativeContext::request` makes and drops an instance of its own; the kept one keeps the
    // Vulkan loader from being torn down when it goes.
    let _kept = crate::instance();
    let opts = NativeContextOptions { timeout: Some(Duration::ZERO), ..Default::default() };
    let res = NativeContext::request(&opts);
    let Err(GpuContextError::TimedOut { pending, .. }) = &res else {
        // The driver answered before the caller even waited: nothing was abandoned.
        eprintln!("SKIPPED: the GPU request did not time out ({res:?})");
        return;
    };
    assert!(pending.wait(Duration::from_secs(120)), "the abandoned initialization thread never finished");
    assert!(pending.is_finished());
}
