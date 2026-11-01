//! Device health (#243): notices a lost wgpu device or an uncaptured wgpu error so callers stop
//! issuing GPU work and fall back to the CPU compositor instead of panicking.
//!
//! wgpu's default uncaptured-error handler panics ("Buffer 'pc_compose_uniforms' is invalid"
//! after the OS killed a huge submission), and `Device::poll` panics on a lost device whatever
//! the handler. [`DeviceHealth::watch`] replaces the handler with one that records the fault,
//! registers the device-lost callback, and [`DeviceHealth::wait`] polls only while the device is
//! healthy (catching the poll's panic as a last resort). Every GPU entry point checks
//! [`DeviceHealth::fault`] first.
//!
//! Builds for the web too: the callbacks only touch atomics and a mutex.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// Why the GPU can't be used any more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The device was lost (driver reset, the OS killed a submission, `Device::destroy`).
    Lost(String),
    /// An uncaptured wgpu error (out of memory, validation, internal): the GPU output can't be
    /// trusted, and on most drivers the device is about to be lost anyway.
    Error(String),
}

impl Fault {
    /// Whether the device itself was lost (rather than an error reported).
    pub fn is_lost(&self) -> bool {
        matches!(self, Fault::Lost(_))
    }

    /// An out-of-memory error on a device that still works: freeing memory and trying again can
    /// succeed, unlike after a loss or a validation error (#6564). Only wgpu's own out-of-memory
    /// error is recorded with exactly [`OUT_OF_MEMORY`]; a validation error that merely mentions
    /// memory is not one.
    pub fn is_out_of_memory(&self) -> bool {
        matches!(self, Fault::Error(s) if s == OUT_OF_MEMORY)
    }

    /// Why a lost device was lost, as far as its message says (`None` for an error on a device
    /// that still works). wgpu-hal reports a Vulkan result it doesn't map as `Unexpected`
    /// ("driver implementation is at fault") and drops the `vk::Result`, so for that cause the
    /// message is all there is to log (#10464).
    pub fn loss_cause(&self) -> Option<LossCause> {
        match self {
            Fault::Lost(s) => Some(LossCause::of(s)),
            Fault::Error(_) => None,
        }
    }

    /// What the canvas does about this fault: `auto_retries` is how many out-of-memory rebuilds
    /// this session already did, `max_auto_retries` how many it may do. Pure, so the table of
    /// faults and actions is testable without a GPU.
    pub fn recovery(&self, auto_retries: u32, max_auto_retries: u32) -> Recovery {
        if self.is_lost() {
            Recovery::CpuForSession
        } else if self.is_out_of_memory() && auto_retries < max_auto_retries {
            Recovery::RebuildCanvas
        } else {
            Recovery::CpuUntilRetry
        }
    }

    /// The driver's or wgpu's message.
    pub fn detail(&self) -> &str {
        match self {
            Fault::Lost(s) | Fault::Error(s) => s,
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fault::Lost(s) if s.is_empty() => f.write_str("GPU device lost"),
            Fault::Lost(s) => write!(f, "GPU device lost: {s}"),
            Fault::Error(s) => write!(f, "GPU error: {s}"),
        }
    }
}

/// The message an out-of-memory [`Fault::Error`] carries (wgpu's `Error::OutOfMemory`).
pub const OUT_OF_MEMORY: &str = "Out of Memory";

/// What to do about a [`Fault`] (see [`Fault::recovery`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// Out of memory on a device that still works: the dropped canvas freed its memory, so build
    /// a new one at once (#6564).
    RebuildCanvas,
    /// An error on a device that still works (validation, internal, or out of memory again):
    /// draw on the CPU and let the user choose Retry GPU (#6577).
    CpuUntilRetry,
    /// The device is lost and can't be used again in this process: draw on the CPU for the rest
    /// of the session. When the window is drawn with that same device, it can't be drawn any
    /// more either (see [`DeviceHealth::device_lost`]).
    CpuForSession,
}

/// Why a device was lost, from the message wgpu passes to the device-lost callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LossCause {
    /// `Device::destroy` (a simulated loss, or the app tearing the device down).
    Destroyed,
    /// wgpu-hal's `DeviceError::Unexpected`: a driver result it has no mapping for. wgpu-core
    /// loses the device for it and the `vk::Result` is not kept.
    DriverFault,
    /// wgpu-hal's `DeviceError::OutOfMemory`, which wgpu-core also answers by losing the device
    /// when the failing call can't recover.
    OutOfMemory,
    /// The driver reset or removed the device ("Device is lost", a poll that found it gone).
    Reset,
    /// A message none of the above recognises.
    Other,
}

impl LossCause {
    /// Classify a device-lost message, case-insensitively. The strings are wgpu-hal's
    /// `DeviceError` display texts and this crate's own ("device destroyed", "lost by the
    /// driver", "the device stopped responding").
    pub fn of(message: &str) -> Self {
        let m = message.to_ascii_lowercase();
        if m.contains("destroyed") {
            Self::Destroyed
        } else if m.contains("unexpected error variant") || m.contains("driver implementation is at fault") {
            Self::DriverFault
        } else if m.contains("out of memory") {
            Self::OutOfMemory
        } else if m.contains("lost") || m.contains("stopped responding") {
            Self::Reset
        } else {
            Self::Other
        }
    }

    /// Short English wording for logs and notices.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Destroyed => "the device was destroyed",
            Self::DriverFault => "the graphics driver reported an unexpected error",
            Self::OutOfMemory => "the graphics device ran out of memory",
            Self::Reset => "the graphics driver reset or removed the device",
            Self::Other => "the graphics device stopped working",
        }
    }
}

#[derive(Default)]
struct Inner {
    faulted: AtomicBool,
    fault: Mutex<Option<Fault>>,
    errors: AtomicU64,
    /// wgpu itself reported the device lost (see [`DeviceHealth::device_lost`]).
    device_lost: AtomicBool,
}

/// Shared health flag of one wgpu device. Cheap to clone; every clone sees the same state.
#[derive(Clone, Default)]
pub struct DeviceHealth(Arc<Inner>);

impl fmt::Debug for DeviceHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceHealth").field("fault", &self.fault()).field("errors", &self.errors()).finish()
    }
}

impl DeviceHealth {
    /// A healthy flag not attached to a device (tests, CPU-only callers).
    pub fn new() -> Self {
        Self::default()
    }

    /// Watch `device`: its device-lost callback and uncaptured errors mark the returned flag
    /// instead of panicking. Replaces any handler set before (wgpu keeps one of each).
    pub fn watch(device: &wgpu::Device) -> Self {
        let health = Self::new();
        let lost = health.clone();
        // Called by wgpu with its own locks held: only record, never call back into wgpu.
        device.set_device_lost_callback(move |reason, message| {
            let detail = match (reason, message.is_empty()) {
                (wgpu::DeviceLostReason::Destroyed, true) => "device destroyed".to_string(),
                (_, true) => "lost by the driver".to_string(),
                (_, false) => message,
            };
            log::error!("wgpu device lost ({reason:?}): {detail}");
            lost.0.device_lost.store(true, Ordering::Release);
            lost.mark(Fault::Lost(detail));
        });
        let errors = health.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| errors.uncaptured(&e)));
        health
    }

    /// Record an uncaptured wgpu error.
    fn uncaptured(&self, e: &wgpu::Error) {
        self.0.errors.fetch_add(1, Ordering::Relaxed);
        let text = e.to_string();
        log::error!("uncaptured wgpu error: {text}");
        self.mark(fault_of(e));
    }

    /// Record an error an error scope caught.
    ///
    /// [`DeviceHealth::watch`]'s uncaptured-error handler never sees a scoped error, so without
    /// this a watched device would keep being handed to a GPU that has already failed. Polls
    /// first: a lost device is a [`Fault::Lost`], and its callback may not have run yet (it fires
    /// on a poll, and `Device::poll` panics on a lost device).
    ///
    /// The poll is a single non-blocking check, never a fence wait. This runs on the UI thread
    /// immediately after a failed frame, so `PollType::Wait` with no timeout would stall the app
    /// until every in-flight submission drained (or forever, if the device is wedged). A
    /// non-blocking poll loses nothing here: wgpu still reads the fence and fires the
    /// device-lost callback for a lost device, it just never blocks waiting for one.
    ///
    /// A device that only recorded an error is still polled and marked, so a loss or an
    /// out-of-memory error can upgrade that fault as [`DeviceHealth::mark`] allows (#6564).
    pub fn caught(&self, device: &wgpu::Device, error: &wgpu::Error) {
        if self.lost() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.poll(device);
        #[cfg(target_arch = "wasm32")]
        let _ = device;
        if self.lost() {
            return;
        }
        self.mark(fault_of(error));
    }

    fn lost(&self) -> bool {
        self.fault().is_some_and(|f| f.is_lost())
    }

    /// Mark the device unusable. The first fault is kept; a later device loss upgrades an error,
    /// and a later out-of-memory error upgrades another one, so the canvas can still recover by
    /// itself when memory ran out after a validation error in the same frame (#6564).
    pub fn mark(&self, fault: Fault) {
        let mut slot = self.0.fault.lock().unwrap_or_else(PoisonError::into_inner);
        let replace = match slot.as_ref() {
            None => true,
            Some(old) => !old.is_lost() && (fault.is_lost() || (fault.is_out_of_memory() && !old.is_out_of_memory())),
        };
        if replace {
            *slot = Some(fault);
        }
        self.0.faulted.store(true, Ordering::Release);
    }

    /// Whether the device may still be used.
    pub fn is_ok(&self) -> bool {
        !self.0.faulted.load(Ordering::Acquire)
    }

    /// Why the device can't be used, if it can't.
    pub fn fault(&self) -> Option<Fault> {
        if self.is_ok() {
            return None;
        }
        self.0.fault.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Uncaptured wgpu errors seen so far.
    pub fn errors(&self) -> u64 {
        self.0.errors.load(Ordering::Relaxed)
    }

    /// Whether wgpu itself reported the device lost: its device-lost callback ran, or polling it
    /// panicked. Unlike a fault recorded with [`mark`](Self::mark) (a simulated loss, or an
    /// uncaptured error that names one), nothing drawn with the device reaches the screen any
    /// more, the window's own frames included (#3438).
    pub fn device_lost(&self) -> bool {
        self.0.device_lost.load(Ordering::Acquire)
    }

    /// Non-blocking poll that runs even after a fault was recorded, so a device that is lost
    /// later (a driver reset while the app sat idle or the PC slept) is noticed before the next
    /// frame is drawn with it. Skipped once [`device_lost`](Self::device_lost).
    pub fn probe(&self, device: &wgpu::Device) {
        if self.device_lost() {
            return;
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.poll(wgpu::PollType::Poll))).is_err() {
            self.lost_by_poll();
        }
    }

    /// `Device::poll` panicked: wgpu found the device lost.
    fn lost_by_poll(&self) {
        self.0.device_lost.store(true, Ordering::Release);
        self.mark(Fault::Lost("the device stopped responding".into()));
    }

    /// Block until `index` (or all submitted work) finished, unless the device is unusable.
    /// Returns whether the device is still healthy afterwards. `Device::poll` panics on a lost
    /// device, so a panic is caught and recorded as a loss (native only; the web has no blocking
    /// wait).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait(&self, device: &wgpu::Device, index: Option<wgpu::SubmissionIndex>) -> bool {
        self.wait_idle(device, index);
        self.is_ok()
    }

    /// Like [`Self::wait`], but returns whether the wait completed: `false` also when the poll
    /// itself failed, so the work may still be running.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_idle(&self, device: &wgpu::Device, index: Option<wgpu::SubmissionIndex>) -> bool {
        if !self.is_ok() {
            return false;
        }
        let poll = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.poll(wgpu::PollType::Wait { submission_index: index, timeout: None })));
        match poll {
            Ok(Ok(_)) => self.is_ok(),
            Ok(Err(e)) => {
                log::warn!("wgpu poll: {e}");
                false
            }
            Err(_) => {
                self.lost_by_poll();
                false
            }
        }
    }

    /// Non-blocking poll (reclaims staging memory), skipped once the device is unusable.
    pub fn poll(&self, device: &wgpu::Device) {
        if !self.is_ok() {
            return;
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.poll(wgpu::PollType::Poll))).is_err() {
            self.lost_by_poll();
        }
    }
}

/// The fault a wgpu error stands for. Errors about invalid or lost objects follow a device loss
/// whose callback may not have run yet (it fires on the next poll), so they count as a loss
/// rather than as an error. Only wgpu's own out-of-memory error is [`OUT_OF_MEMORY`].
fn fault_of(e: &wgpu::Error) -> Fault {
    let text = e.to_string();
    let lower = text.to_ascii_lowercase();
    if matches!(e, wgpu::Error::OutOfMemory { .. }) {
        Fault::Error(OUT_OF_MEMORY.into())
    } else if lower.contains("lost") || lower.contains("destroyed") || LossCause::of(&text) == LossCause::DriverFault {
        // wgpu-core loses the device for a driver fault ("Unexpected error variant"), whether the
        // error reaches us before or after the device-lost callback (#10464).
        Fault::Lost(first_line(&text))
    } else {
        Fault::Error(first_line(&text))
    }
}

fn first_line(s: &str) -> String {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut out: String = line.chars().take(200).collect();
    if line.chars().count() > 200 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_fault_wins_but_loss_upgrades_an_error() {
        let h = DeviceHealth::new();
        assert!(h.is_ok());
        assert_eq!(h.fault(), None);
        h.mark(Fault::Error("out of memory".into()));
        assert!(!h.is_ok());
        let c = h.clone();
        c.mark(Fault::Error("second".into()));
        assert_eq!(h.fault(), Some(Fault::Error("out of memory".into())));
        c.mark(Fault::Lost("reset".into()));
        assert_eq!(h.fault(), Some(Fault::Lost("reset".into())));
        c.mark(Fault::Lost("later".into()));
        assert_eq!(h.fault().map(|f| f.to_string()).as_deref(), Some("GPU device lost: reset"));
        // A recorded loss is not wgpu reporting the device lost (#3438).
        assert!(!h.device_lost());
    }

    fn internal(description: &str) -> wgpu::Error {
        wgpu::Error::Internal { source: description.into(), description: description.into() }
    }

    fn validation(description: &str) -> wgpu::Error {
        wgpu::Error::Validation { source: description.into(), description: description.into() }
    }

    /// #10464: wgpu-hal's `DeviceError::Unexpected` text reaches the app as the device-lost message
    /// (wgpu-core loses the device for it). Every message wgpu-hal can produce maps to a cause.
    #[test]
    fn loss_messages_name_their_cause() {
        let unexpected = "Unexpected error variant (driver implementation is at fault)";
        assert_eq!(LossCause::of(unexpected), LossCause::DriverFault);
        assert_eq!(LossCause::of(&unexpected.to_uppercase()), LossCause::DriverFault);
        assert_eq!(LossCause::of("Device is lost"), LossCause::Reset);
        assert_eq!(LossCause::of("lost by the driver"), LossCause::Reset);
        assert_eq!(LossCause::of("the device stopped responding"), LossCause::Reset);
        assert_eq!(LossCause::of("Out of memory"), LossCause::OutOfMemory);
        assert_eq!(LossCause::of("Out of Memory"), LossCause::OutOfMemory);
        assert_eq!(LossCause::of("device destroyed"), LossCause::Destroyed);
        assert_eq!(LossCause::of(""), LossCause::Other);
        assert_eq!(LossCause::of("something nobody wrote yet"), LossCause::Other);
        // Never panics on odd input, including multi-byte text.
        assert_eq!(LossCause::of("設備遺失 ✓"), LossCause::Other);
        // Every cause has wording for the log and the notice.
        for cause in [LossCause::Destroyed, LossCause::DriverFault, LossCause::OutOfMemory, LossCause::Reset, LossCause::Other] {
            assert!(!cause.describe().is_empty());
        }
        assert_eq!(Fault::Lost(unexpected.into()).loss_cause(), Some(LossCause::DriverFault));
        assert_eq!(Fault::Error("Validation Error".into()).loss_cause(), None);
    }

    /// An error that wgpu raises for a driver fault, or that names a lost device, is a loss: the
    /// device is gone (or about to be) whichever of the error and the lost callback comes first.
    #[test]
    fn wgpu_errors_classify_into_faults() {
        let oom = wgpu::Error::OutOfMemory { source: "x".into() };
        assert_eq!(fault_of(&oom), Fault::Error(OUT_OF_MEMORY.into()));
        assert!(fault_of(&oom).is_out_of_memory());
        assert_eq!(
            fault_of(&internal("Unexpected error variant (driver implementation is at fault)")),
            Fault::Lost("Unexpected error variant (driver implementation is at fault)".into())
        );
        assert!(fault_of(&internal("Parent device is lost")).is_lost());
        assert!(fault_of(&validation("Buffer 'pc_compose_uniforms' is invalid\nBuffer is destroyed")).is_lost());
        assert_eq!(fault_of(&validation("Buffer size 12 is not a multiple of 4")), Fault::Error("Buffer size 12 is not a multiple of 4".into()));
        // A validation error that merely talks about memory is neither a loss nor an OOM.
        let memory = fault_of(&validation("allocation would run out of memory"));
        assert!(!memory.is_lost() && !memory.is_out_of_memory());
        // The text shown to the user is one line, capped.
        assert!(fault_of(&internal(&"x".repeat(1000))).detail().chars().count() <= 201);
    }

    /// The table of faults and what the canvas does about each (#6564, #6577, #243).
    #[test]
    fn faults_map_to_a_recovery() {
        let oom = Fault::Error(OUT_OF_MEMORY.into());
        let validation = Fault::Error("Validation Error".into());
        let lost = Fault::Lost("Unexpected error variant (driver implementation is at fault)".into());
        let max = 3;
        // Out of memory on a live device is retried by itself while the budget lasts, then waits
        // for the user.
        for used in 0..max {
            assert_eq!(oom.recovery(used, max), Recovery::RebuildCanvas, "retry {used}");
        }
        assert_eq!(oom.recovery(max, max), Recovery::CpuUntilRetry);
        assert_eq!(oom.recovery(u32::MAX, max), Recovery::CpuUntilRetry);
        assert_eq!(oom.recovery(0, 0), Recovery::CpuUntilRetry, "a budget of none never retries");
        // Other errors are never retried by themselves.
        assert_eq!(validation.recovery(0, max), Recovery::CpuUntilRetry);
        // A loss is final for the process, whatever the retry budget.
        assert_eq!(lost.recovery(0, max), Recovery::CpuForSession);
        assert_eq!(Fault::Lost(String::new()).recovery(0, 0), Recovery::CpuForSession);
        assert_eq!(Fault::Lost(OUT_OF_MEMORY.into()).recovery(0, max), Recovery::CpuForSession, "an out-of-memory loss is not an out-of-memory error");
    }

    /// A device-lost callback and an uncaptured error from the driver-fault path mark the same
    /// flag: later calls see the loss, and a poll does nothing.
    #[test]
    fn a_driver_fault_error_marks_the_device_lost() {
        let h = DeviceHealth::new();
        h.uncaptured(&internal("Unexpected error variant (driver implementation is at fault)"));
        assert!(!h.is_ok());
        assert!(h.fault().is_some_and(|f| f.is_lost() && f.loss_cause() == Some(LossCause::DriverFault)));
        assert_eq!(h.errors(), 1);
    }

    #[test]
    fn fault_text() {
        assert_eq!(Fault::Lost(String::new()).to_string(), "GPU device lost");
        assert_eq!(Fault::Error("x".into()).to_string(), "GPU error: x");
        assert_eq!(first_line("\n  Buffer 'pc' is invalid\nmore"), "Buffer 'pc' is invalid");
        assert_eq!(first_line(&"a".repeat(300)).chars().count(), 201);
        assert!(Fault::Error("Out of Memory".into()).is_out_of_memory());
        assert!(!Fault::Error("Validation Error".into()).is_out_of_memory());
        assert!(!Fault::Error("Validation Error: buffer allocation would run out of memory".into()).is_out_of_memory());
        // A later out-of-memory error replaces another error, but never a loss.
        let h = DeviceHealth::new();
        h.mark(Fault::Error("Validation Error".into()));
        h.mark(Fault::Error(OUT_OF_MEMORY.into()));
        assert!(h.fault().is_some_and(|f| f.is_out_of_memory()));
        h.mark(Fault::Lost("reset".into()));
        h.mark(Fault::Error(OUT_OF_MEMORY.into()));
        assert!(h.fault().is_some_and(|f| f.is_lost()));
        assert!(!Fault::Lost("out of memory".into()).is_out_of_memory());
    }
}
