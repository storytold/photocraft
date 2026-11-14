//! Builds the compositor without blocking the UI's first document refresh.
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) enum BuildState {
    #[default]
    NotStarted,
    Building,
    Ready(Box<photocraft_gpu::Compositor>),
    Failed(photocraft_gpu::Unsupported),
    Adopted,
}

pub(super) type Slot = Arc<Mutex<BuildState>>;
pub(super) type Completion = Arc<Mutex<Option<Result<(), (String, bool)>>>>;

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn start(device: &eframe::wgpu::Device, format: eframe::wgpu::TextureFormat, ctx: &egui::Context, slot: &Slot, completion: &Completion) {
    let mut state = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !matches!(*state, BuildState::NotStarted) {
        return;
    }
    *state = BuildState::Building;
    if builds_on_the_drawing_thread(device) {
        // Build here: the shaders compile on this thread, where the app's own device calls
        // (the window's frames, the canvas uploads) are serialized behind them. On a worker
        // thread they would not be, and the GL backend would abort the process instead.
        let (result, outcome) = build_here_recovered(|| build_here(device, format));
        publish(&mut state, completion, result, outcome);
        ctx.request_repaint();
        return;
    }
    let (device, ctx, out) = (device.clone(), ctx.clone(), slot.clone());
    let worker_completion = completion.clone();
    let spawned = std::thread::Builder::new().name("gpu-compositor-build".into()).spawn(move || {
        let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            photocraft_gpu::Compositor::try_new_with_format_threads_classified(&device, format, threads)
        }));
        let (result, outcome) = match result {
            Ok(Ok(comp)) => (BuildState::Ready(Box::new(comp)), Ok(())),
            Ok(Err(error)) => {
                let deterministic = error.kind == photocraft_gpu::BuildFailureKind::DeterministicShaderCompilation;
                (BuildState::Failed(photocraft_gpu::Unsupported(error.message.clone())), Err((error.message, deterministic)))
            }
            Err(_) => panicked(),
        };
        publish(&mut out.lock().unwrap_or_else(std::sync::PoisonError::into_inner), &worker_completion, result, outcome);
        ctx.request_repaint();
    });
    if let Err(error) = spawned {
        record_spawn_failure(&mut state, completion, &error);
    }
}

/// Whether the compositor's pipelines must be built on the thread that draws.
///
/// wgpu-hal's GL device takes one context lock around every call and gives up after a second
/// ("Could not lock adapter context. This is most-likely a deadlock."), so a build on a worker
/// thread races the window's own frames and aborts the process. The other backends keep the
/// worker: the UI stays responsive while the shaders compile.
#[cfg(not(target_arch = "wasm32"))]
fn builds_on_the_drawing_thread(device: &eframe::wgpu::Device) -> bool {
    photocraft_gpu::device_access_is_exclusive(device.adapter_info().backend)
}

/// The compositor built on this thread, and why it could not be built.
#[cfg(not(target_arch = "wasm32"))]
fn build_here(device: &eframe::wgpu::Device, format: eframe::wgpu::TextureFormat) -> (BuildState, Result<(), (String, bool)>) {
    match photocraft_gpu::Compositor::try_new_with_format_classified(device, format) {
        Ok(comp) => (BuildState::Ready(Box::new(comp)), Ok(())),
        Err(error) => {
            let deterministic = error.kind == photocraft_gpu::BuildFailureKind::DeterministicShaderCompilation;
            (BuildState::Failed(photocraft_gpu::Unsupported(error.message.clone())), Err((error.message, deterministic)))
        }
    }
}

/// The failure a build that panicked publishes: the same one the worker path does, so a slot
/// never stays `Building` and the caller sees a retryable error.
#[cfg(not(target_arch = "wasm32"))]
fn panicked() -> (BuildState, Result<(), (String, bool)>) {
    (
        BuildState::Failed(photocraft_gpu::Unsupported("building the GPU compositor panicked".into())),
        Err(("building the GPU compositor panicked".into(), false)),
    )
}

/// The build with the worker's panic recovery: a panic must not escape the drawing thread and
/// leave the slot `Building`.
#[cfg(not(target_arch = "wasm32"))]
fn build_here_recovered(build: impl FnOnce() -> (BuildState, Result<(), (String, bool)>)) -> (BuildState, Result<(), (String, bool)>) {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(build)) {
        Ok(built) => built,
        Err(_) => panicked(),
    }
}

/// Publish the build's state and its outcome: the slot leaves `Building` either way.
#[cfg(not(target_arch = "wasm32"))]
fn publish(state: &mut BuildState, completion: &Completion, result: BuildState, outcome: Result<(), (String, bool)>) {
    *state = result;
    *completion.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
}

#[cfg(not(target_arch = "wasm32"))]
fn record_spawn_failure(state: &mut BuildState, completion: &Completion, error: &std::io::Error) {
    let message = format!("failed to spawn GPU compositor build thread: {error}");
    *state = BuildState::Failed(photocraft_gpu::Unsupported(message.clone()));
    *completion.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Err((message, false)));
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_panicking_build_leaves_the_slot_failed_and_reports_a_retryable_error() {
        let completion = super::Completion::default();
        let mut state = super::BuildState::Building;
        let (result, outcome) = super::build_here_recovered(|| panic!("the shader compiler gave up"));
        super::publish(&mut state, &completion, result, outcome);
        assert!(matches!(state, super::BuildState::Failed(_)), "the slot must not stay Building");
        let outcome = completion.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        assert_eq!(outcome, Some(Err(("building the GPU compositor panicked".into(), false))));
    }

    #[test]
    fn a_build_that_finishes_keeps_its_own_state_and_outcome() {
        let completion = super::Completion::default();
        let mut state = super::BuildState::Building;
        let (result, outcome) = super::build_here_recovered(|| (super::BuildState::Failed(photocraft_gpu::Unsupported("no".into())), Err(("no".into(), true))));
        super::publish(&mut state, &completion, result, outcome);
        assert!(matches!(state, super::BuildState::Failed(_)));
        let outcome = completion.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        assert_eq!(outcome, Some(Err(("no".into(), true))), "a deterministic failure keeps its own outcome");
    }

    #[test]
    fn spawn_failure_is_reported_as_retryable() {
        let completion = super::Completion::default();
        let mut state = super::BuildState::Building;
        super::record_spawn_failure(&mut state, &completion, &std::io::Error::other("resource exhausted"));
        assert!(matches!(state, super::BuildState::Failed(_)));
        let outcome = completion.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        assert_eq!(outcome, Some(Err(("failed to spawn GPU compositor build thread: resource exhausted".into(), false))));
    }

    #[test]
    fn compositor_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<photocraft_gpu::Compositor>();
    }
}
