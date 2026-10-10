//! Background and cached lookups for the LUT list: the header details shown in a row's tooltip, and
//! the scan that finds packs another pack already contains, and the check for files changed by hand
//! ([`stale`]). The first two are cached in egui memory, and the
//! scan reads every LUT the first time, so it runs on a thread and its answer arrives a moment later.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use egui::{Context, Id};
use photocraft_engine::lut_library::meta::{self, Header};

mod stale;

/// Headers read per frame.
const HEADERS_PER_FRAME: usize = 24;

#[derive(Default)]
struct Inner {
    headers: HashMap<String, Option<Header>>,
    /// Library revision the duplicate scan last started for, and its result.
    scan_rev: Option<u64>,
    redundant: HashMap<String, String>,
    /// The check for files added or removed by hand ([`stale`]).
    stale: stale::Check,
}

/// The header and duplicate-scan caches (shared with the worker thread).
#[derive(Clone, Default)]
pub(super) struct Scans(Arc<Mutex<Inner>>);

impl Scans {
    pub(super) fn get(ctx: &Context) -> Scans {
        let id = Id::new("lutlib-scans");
        if let Some(s) = ctx.data(|d| d.get_temp::<Scans>(id)) {
            return s;
        }
        let s = Scans::default();
        ctx.data_mut(|d| d.insert_temp(id, s.clone()));
        s
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The header of the LUT file at `path`, read on the first request.
    pub(super) fn header(&self, path: &str) -> Option<Header> {
        let mut inner = self.inner();
        if let Some(h) = inner.headers.get(path) {
            return h.clone();
        }
        if inner.headers.len() >= HEADERS_PER_FRAME * 400 {
            inner.headers.clear();
        }
        let h = meta::read_header(std::path::Path::new(path));
        inner.headers.insert(path.to_string(), h.clone());
        h
    }

    /// Packs that another pack already contains (`pack -> the pack that covers it`).
    pub(super) fn redundant(&self, ctx: &Context, root: &std::path::Path, rev: u64) -> HashMap<String, String> {
        let mut inner = self.inner();
        if inner.scan_rev != Some(rev) {
            inner.scan_rev = Some(rev);
            let (shared, ctx, root) = (self.clone(), ctx.clone(), root.to_path_buf());
            let work = move || {
                let found = photocraft_engine::lut_library::LutLibrary::new(root).redundant_packs();
                let mut inner = shared.inner();
                if inner.scan_rev == Some(rev) {
                    inner.redundant = found;
                }
                drop(inner);
                ctx.request_repaint();
            };
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::spawn(work);
            #[cfg(target_arch = "wasm32")]
            work();
        }
        inner.redundant.clone()
    }
}
