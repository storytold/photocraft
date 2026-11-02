//! Keep new GPU objects private until both error scopes have completed. WebGPU resolves scopes
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

pub(super) fn validated<T>(device: &wgpu::Device, build: impl FnOnce() -> T) -> Validated<T> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let result = build();
    // Pop immediately: the guards are thread-bound, while their futures may be polled later.
    let scopes = [internal.pop(), validation.pop()];
    Validated::new(result, async move {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
