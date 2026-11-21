//! Serializing the device's calls that must not overlap.
//!
//! wgpu-hal's GL backend takes one context lock for every call and gives up after a second
//! ("Could not lock adapter context. This is most-likely a deadlock"): a call that waits longer
//! panics, and that panic while another thread drops a pipeline aborts the process. A pipeline
//! build holds the lock while the driver compiles the shader (seconds on some drivers), so on
//! such a backend a build may not run beside another thread's call - not beside the app's
//! uploads and draws (which share the drawing thread with the build) and not beside the Camera
//! Raw develop (which runs on a worker).
//!
//! Elsewhere the device takes its own calls concurrently, so nothing is serialized there: a
//! build runs beside the app's uploads and draws, which is what keeps the parallel pipeline
//! build worth its threads on DX12 (FXC compiles one shader at a time, #6419).

use std::sync::{Mutex, MutexGuard};

/// The device's calls, one at a time, where they must not overlap.
static CALLS: Mutex<()> = Mutex::new(());

/// The device's calls, held for as long as the guard lives.
pub(crate) struct DeviceCalls(#[allow(dead_code)] MutexGuard<'static, ()>);

/// Takes the device's calls, waiting for the ones already in flight.
///
/// Only the backends that need them take them ([`device_access_is_exclusive`]); elsewhere the
/// guard is absent and the calls run as they come.
pub(crate) fn device_calls(backend: wgpu::Backend) -> Option<DeviceCalls> {
    crate::device_access_is_exclusive(backend).then(|| DeviceCalls(CALLS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)))
}
