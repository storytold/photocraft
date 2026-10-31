//! Trusted host accelerators. WASM keeps its no-import sandbox and remains the fallback.

use std::cell::RefCell;
use std::sync::Arc;

use photocraft_raster::Interrupt;
use serde_json::Value;

use crate::Result;

pub struct Band<'a> {
    pub pixels: &'a mut [f32],
    pub size: [u32; 2],
    pub channels: usize,
    pub format: u32,
    pub params: &'a Value,
}

/// A host implementation of a manifest's explicitly requested algorithm.
pub trait Accelerator: Send + Sync {
    fn algorithm(&self) -> &str;
    /// `false` requests WASM fallback. The host restores the original band first.
    fn filter(&self, band: Band<'_>, ctl: &Interrupt<'_>) -> Result<bool>;
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<dyn Accelerator>>> = const { RefCell::new(None) };
}

/// Scoped to one session operation; workers explicitly install their session's accelerator.
pub struct Scope {
    previous: Option<Arc<dyn Accelerator>>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

pub fn scope(accelerator: Option<Arc<dyn Accelerator>>) -> Scope {
    Scope { previous: CURRENT.with(|a| a.replace(accelerator)), _thread: Default::default() }
}

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|a| a.replace(self.previous.take()));
    }
}

pub fn current() -> Option<Arc<dyn Accelerator>> {
    CURRENT.with(|a| a.borrow().clone())
}
