//! Keep new GPU objects private until every error scope has completed. WebGPU resolves scopes
//! asynchronously, so an initially pending scope is neither success nor a permanent CPU fallback.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};
use std::task::{Context, Poll, Waker};

use eframe::wgpu;

trait ValidationFuture: Future<Output = bool> + wgpu::WasmNotSend {}
impl<F: Future<Output = bool> + wgpu::WasmNotSend> ValidationFuture for F {}

type Pending<T> = (T, Pin<Box<dyn ValidationFuture>>);

pub(super) struct Validated<T> {
    ready: OnceLock<Option<T>>,
    pending: Mutex<Option<Pending<T>>>,
}

impl<T> Validated<T> {
    pub(super) fn new(value: T, future: impl Future<Output = bool> + wgpu::WasmNotSend + 'static) -> Self {
        Self { ready: OnceLock::new(), pending: Mutex::new(Some((value, Box::pin(future)))) }
    }

    pub(super) fn get(&self) -> Option<&T> {
        if let Some(value) = self.ready.get() {
            return value.as_ref();
        }
        let mut pending = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, future)) = pending.as_mut() {
            let mut cx = Context::from_waker(Waker::noop());
            if let Poll::Ready(valid) = future.as_mut().poll(&mut cx)
                && let Some((value, _)) = pending.take()
            {
                let _ = self.ready.set(valid.then_some(value));
            }
        }
        self.ready.get().and_then(Option::as_ref)
    }

    pub(super) fn is_pending(&self) -> bool {
        self.ready.get().is_none()
    }
}

/// Match the canvas's local allocation handling: an unsupported preview must not fault the
/// shared device or spend its global OOM recovery retries. Guards also pop on early returns.
pub(super) struct ErrorScopes {
    // Rust drops fields in declaration order. Early returns must pop the top scope first.
    internal: wgpu::ErrorScopeGuard,
    memory: wgpu::ErrorScopeGuard,
    validation: wgpu::ErrorScopeGuard,
}

impl ErrorScopes {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        Self { internal, memory, validation }
    }

    pub(super) fn finish<T>(self, value: T) -> Validated<T> {
        // Pop immediately: the guards are thread-bound, while their futures may be polled later.
        let scopes = [self.internal.pop(), self.memory.pop(), self.validation.pop()];
        Validated::new(value, async move {
            let mut valid = true;
            for scope in scopes {
                if let Some(e) = scope.await {
                    log::warn!("channel preview uses CPU: {e}");
                    #[cfg(test)]
                    eprintln!("channel preview uses CPU: {e}");
                    valid = false;
                }
            }
            valid
        })
    }
}

pub(super) fn validated<T>(device: &wgpu::Device, build: impl FnOnce() -> T) -> Validated<T> {
    let scopes = ErrorScopes::new(device);
    scopes.finish(build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn an_early_return_pops_nested_scopes_in_reverse_order_without_faulting_the_device() {
        let _lock = photocraft_testkit::gpu_lock();
        let Ok(rs) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())
        })) else {
            eprintln!("no software GPU adapter for scope cleanup regression");
            return;
        };
        let gpu = crate::gpu_canvas::GpuCanvas::new(&rs);
        let abandon = || {
            let _scopes = ErrorScopes::new(&rs.device);
            let _ = rs.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("abandoned preview scope regression"),
                size: 16,
                usage: wgpu::BufferUsages::empty(),
                mapped_at_creation: false,
            });
            None::<()>
        };
        assert!(abandon().is_none());
        let next = super::super::track_resources(&rs, ErrorScopes::new(&rs.device));
        assert_eq!(super::super::test_wait(&rs, || next.get()), Some(&()));
        assert!(gpu.health().is_ok());
    }

    #[test]
    fn pending_validation_is_retried_and_late_failure_never_exposes_the_object() {
        for valid in [true, false] {
            let polls = Arc::new(AtomicUsize::new(0));
            let count = polls.clone();
            let future = std::future::poll_fn(move |_| if count.fetch_add(1, Ordering::SeqCst) < 2 { Poll::Pending } else { Poll::Ready(valid) });
            let object = Validated::new(42, future);
            for _ in 0..2 {
                assert!(object.get().is_none());
                assert!(object.is_pending());
            }
            assert_eq!(object.get(), valid.then_some(&42));
            assert!(!object.is_pending());
            assert_eq!(object.get(), valid.then_some(&42));
            assert_eq!(polls.load(Ordering::SeqCst), 3, "completed scopes are never polled twice");
        }
    }
}
