//! Stages of smart object renders: the placed source and the result after each visible smart
//! filter, keyed by everything that produced them. A re-render starts from the deepest cached
//! stage, so editing filter `i` only reruns the filters from `i` up: hiding or tweaking the top
//! filter of a long stack doesn't rerun the expensive filters under it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use photocraft_color::PixelFormat;
use photocraft_doc::{SmartFilter, SmartObject};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// blake3 of a stage's inputs: the source contents, the placement and the filters so far.
pub(super) type StageKey = [u8; 32];

#[derive(Clone)]
pub(super) struct Stage {
    pub surface: Surface,
    /// Where filters repeat edge pixels: the canvas ∪ the placed content.
    pub extent: Rect,
}

pub(super) struct Entry {
    key: StageKey,
    stage: Stage,
    bytes: usize,
}

/// Most recently used last.
static STAGES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
const ENTRIES: usize = 64;
/// Default budget in accounted tile bytes; a stage larger than the budget on its own isn't kept.
#[cfg(not(target_arch = "wasm32"))]
pub const BUDGET: usize = 768 << 20;
#[cfg(target_arch = "wasm32")]
pub const BUDGET: usize = 256 << 20;
/// The budget in force (Preferences › Performance `smartStageCacheMb`; 0 keeps nothing between
/// commands).
static BUDGET_NOW: AtomicUsize = AtomicUsize::new(BUDGET);

/// Sets the budget, dropping the oldest stages over it.
pub fn set_budget(bytes: usize) {
    BUDGET_NOW.store(bytes, Ordering::Relaxed);
    trim(&mut STAGES.lock().unwrap_or_else(PoisonError::into_inner), bytes);
}

/// The budget in force, in bytes.
pub fn budget() -> usize {
    #[cfg(test)]
    if let Some(b) = OVERRIDE.with(std::cell::Cell::get) {
        return b;
    }
    BUDGET_NOW.load(Ordering::Relaxed)
}

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Tests: a budget for this thread only (other tests keep theirs) while alive.
#[cfg(test)]
pub(super) struct BudgetOverride(Option<usize>);

#[cfg(test)]
impl BudgetOverride {
    pub(super) fn new(bytes: usize) -> Self {
        Self(OVERRIDE.with(|o| o.replace(Some(bytes))))
    }
}

#[cfg(test)]
impl Drop for BudgetOverride {
    fn drop(&mut self) {
        OVERRIDE.with(|o| o.set(self.0));
    }
}

/// The first stage: what placing `bytes` (the source contents) through the smart object's
/// transform, warp or perspective gives at pixel format `fmt` on `canvas`.
pub(super) fn placed_key(bytes: &[u8], fmt: PixelFormat, sm: &SmartObject, canvas: Rect) -> StageKey {
    let mut h = blake3::Hasher::new();
    h.update(bytes);
    for v in sm.transform.m {
        h.update(&v.to_bits().to_le_bytes());
    }
    h.update(format!("{fmt:?}|{:?}|{:?}|{:?}|{canvas:?}", sm.warp, sm.perspective, sm.stack_mode).as_bytes());
    *h.finalize().as_bytes()
}

/// The stage after applying `f` to stage `below`.
pub(super) fn filter_key(below: &StageKey, f: &SmartFilter) -> StageKey {
    let mut h = blake3::Hasher::new();
    h.update(below);
    h.update(format!("{}\0{}\0{:?}\0", f.command, f.params, f.blend).as_bytes());
    h.update(&f.opacity.to_bits().to_le_bytes());
    *h.finalize().as_bytes()
}

thread_local! {
    /// The stages of the open [`reuse_stages`] scope on this thread (`None`: no scope).
    static SCOPE: std::cell::RefCell<Option<Vec<(StageKey, Stage)>>> = const { std::cell::RefCell::new(None) };
}

/// Open while alive: see [`reuse_stages`]. Bound to the thread that opened it.
pub(super) struct StageScope {
    owner: bool,
    _thread: std::marker::PhantomData<*const ()>,
}

impl Drop for StageScope {
    fn drop(&mut self) {
        if self.owner {
            SCOPE.with(|s| *s.borrow_mut() = None);
        }
    }
}

/// While the returned guard lives, stages put on this thread are also kept for it whatever the
/// budget, so one command reuses them (instances at one place, a filter mask mixing the placed
/// contents back in) even when nothing is kept between commands. Dropping it frees them. Scopes
/// nest; the outermost one owns them.
pub(super) fn reuse_stages() -> StageScope {
    SCOPE.with(|s| {
        let mut s = s.borrow_mut();
        let owner = s.is_none();
        if owner {
            *s = Some(Vec::new());
        }
        StageScope { owner, _thread: std::marker::PhantomData }
    })
}

pub(super) fn get(key: &StageKey) -> Option<Stage> {
    if let Some(hit) = SCOPE.with(|s| s.borrow().as_ref().and_then(|v| v.iter().find(|(k, _)| k == key).map(|(_, st)| st.clone()))) {
        return Some(hit);
    }
    let mut c = STAGES.lock().unwrap_or_else(PoisonError::into_inner);
    let i = c.iter().position(|e| e.key == *key)?;
    let e = c.remove(i);
    let stage = e.stage.clone();
    c.push(e);
    Some(stage)
}

pub(super) fn put(key: StageKey, stage: &Stage) {
    SCOPE.with(|s| {
        if let Some(v) = s.borrow_mut().as_mut() {
            v.retain(|(k, _)| *k != key);
            v.push((key, stage.clone()));
        }
    });
    insert(&mut STAGES.lock().unwrap_or_else(PoisonError::into_inner), key, stage, budget());
}

/// Adds `stage` as the most recent entry of `c` unless it alone is over `budget`, then trims.
pub(super) fn insert(c: &mut Vec<Entry>, key: StageKey, stage: &Stage, budget: usize) {
    let bytes = surface_bytes(&stage.surface);
    if bytes > budget {
        return;
    }
    c.retain(|e| e.key != key);
    c.push(Entry { key, stage: stage.clone(), bytes });
    trim(c, budget);
}

/// Drops the oldest entries of `c` until it is within [`ENTRIES`] and `budget`.
pub(super) fn trim(c: &mut Vec<Entry>, budget: usize) {
    while !c.is_empty() && (c.len() > ENTRIES || c.iter().map(|e| e.bytes).sum::<usize>() > budget) {
        c.remove(0);
    }
}

/// Bytes held by cached stages.
pub fn bytes() -> usize {
    STAGES.lock().unwrap_or_else(PoisonError::into_inner).iter().map(|e| e.bytes).sum()
}

/// Drop every cached stage (Edit › Purge › All). Returns the bytes released.
pub fn purge() -> usize {
    let mut c = STAGES.lock().unwrap_or_else(PoisonError::into_inner);
    let freed = c.iter().map(|e| e.bytes).sum();
    c.clear();
    freed
}

/// Bytes a surface holds: its default pixel and every tile.
pub(super) fn surface_bytes(s: &Surface) -> usize {
    // Tiles shared with the document (or another stage) are charged anyway: the cache is what
    // keeps them alive once the document moves on.
    s.tiles().fold(s.default_bytes().len(), |n, (_, t)| n.saturating_add(t.len()))
}
