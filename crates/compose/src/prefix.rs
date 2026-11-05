//! The composite of a document's lowest root layers, kept tile by tile, so that a redraw after an
//! edit high in the stack composites only the layers from the edit up (#6533). Hiding the top
//! adjustment layer of 150 full-canvas layers, or changing its settings, then costs a copy of the
//! cached tiles and that one layer, not the whole stack.
//!
//! It works as the wgpu compositor's prefix cache does (`photocraft-gpu` `prefix.rs`): the cache
//! remembers the root layers of the last render, and each render finds the lowest root layer that
//! changed since (layer equality is cheap: tiles are `Arc`-shared). Tiles holding the composite
//! of layers below it stay valid; the others are dropped. A render resumes each tile from the
//! highest valid composite it has, and captures it again just below the edit, so the cache
//! follows an edit up the stack. Anything else about the document (its size, mode, depth,
//! patterns, ...) changing clears it.
//!
//! Entries are keyed by the exact tile rectangle, so they hit for renders on the same tile grid:
//! a whole-document refresh, or the background redraw's 1024 px blocks. A small damage rect of a
//! brush stroke misses, and only adds entries while the budget has room. The result is bit for
//! bit the one [`crate::render`] gives: a tile resumed from a cached composite runs the same
//! per-pixel steps as one drawn from its backdrop.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use photocraft_doc::{DocId, Document, Layer};
use photocraft_geom::Rect;

use super::Buffer;

/// Default memory a [`PrefixCache`] may hold (f32 RGBA: 16 bytes a pixel, 32 MP).
pub const PREFIX_CACHE_BYTES: u64 = 512 << 20;

/// Composites of a document's lowest root layers, tile by tile (see the module docs). Shared by
/// the threads of a render: each tile takes the lock only to look up or store its entry.
pub struct PrefixCache {
    inner: Mutex<Inner>,
}

struct Inner {
    /// The document of the last render without its layers: any other change clears the cache.
    context: Option<Document>,
    /// The root layers of the last render.
    layers: Vec<Layer>,
    /// Where renders capture: a root layer index, the tiles then holding layers `..checkpoint`.
    checkpoint: Option<usize>,
    tiles: HashMap<Rect, Entry>,
    bytes: u64,
    budget: u64,
    /// Bumped whenever the layers change: entries carry the generation of the render that stored
    /// them, so a render still running on an older document (a background redraw an edit
    /// restarted) can't hand its tiles to a newer one.
    generation: u64,
    /// Tiles served from a cached composite since the cache was made.
    hits: u64,
}

/// What [`PrefixCache::prepare`] readies a render with: where it captures and its generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Prepared {
    pub at: usize,
    generation: u64,
}

struct Entry {
    generation: u64,
    /// The tile holds the composite of root layers `..at`.
    at: usize,
    buf: Arc<Buffer>,
}

impl Default for PrefixCache {
    fn default() -> Self {
        Self::with_budget(PREFIX_CACHE_BYTES)
    }
}

/// Bytes of a buffer of `rect`'s pixels.
fn bytes_of(rect: Rect) -> u64 {
    u64::from(rect.width()).saturating_mul(u64::from(rect.height())).saturating_mul(16)
}

/// Whether the root stack may be split before layer `k`: not inside a clipping group, whose
/// layers composite with their base.
fn boundary(layers: &[Layer], k: usize) -> bool {
    k > 0 && (k == layers.len() || layers.get(k).is_some_and(|l| !l.clipped))
}

impl PrefixCache {
    /// A cache holding at most `bytes` of composites (0 turns it off).
    pub fn with_budget(bytes: u64) -> Self {
        let inner = Inner { context: None, layers: Vec::new(), checkpoint: None, tiles: HashMap::new(), bytes: 0, budget: bytes, generation: 0, hits: 0 };
        Self { inner: Mutex::new(inner) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Drops every cached composite.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Drops the cache when it holds document `doc` (closed, say).
    pub fn forget(&self, doc: DocId) {
        let mut inner = self.lock();
        if inner.context.as_ref().is_some_and(|d| d.id == doc) {
            inner.clear();
        }
    }

    /// Drops the cache unless it holds a document `keep` accepts (the open ones).
    pub fn retain(&self, keep: impl FnOnce(DocId) -> bool) {
        let mut inner = self.lock();
        if inner.context.as_ref().is_some_and(|d| !keep(d.id)) {
            inner.clear();
        }
    }

    /// Bytes of composites held.
    pub fn bytes(&self) -> u64 {
        self.lock().bytes
    }

    /// Tiles a render resumed from a cached composite, since the cache was made.
    pub fn hits(&self) -> u64 {
        self.lock().hits
    }

    /// Caps the memory held at `bytes`, dropping everything when more is held already.
    pub fn set_budget(&self, bytes: u64) {
        let mut inner = self.lock();
        inner.budget = bytes;
        if inner.bytes > bytes {
            inner.clear();
        }
    }

    /// Readies the cache for a render of `doc`: drops the tiles an edit since the last render
    /// invalidated and returns the root layer index this render captures at, if any.
    pub(crate) fn prepare(&self, doc: &Document) -> Option<Prepared> {
        let mut inner = self.lock();
        if inner.budget == 0 {
            inner.clear();
            return None;
        }
        // Keeping every non-layer field makes any other change to the document clear the cache.
        let mut context = doc.clone();
        let layers = std::mem::take(&mut context.layers);
        let same = inner.context.as_ref() == Some(&context);
        // The lowest root layer changed since the last render (a layer added or removed changes
        // the stack from where the counts part).
        let changed = inner
            .layers
            .iter()
            .zip(&layers)
            .position(|(a, b)| a != b)
            .or_else(|| (layers.len() != inner.layers.len()).then_some(layers.len().min(inner.layers.len())));
        if same && changed.is_none() {
            // The same document again: every entry of this generation stays valid.
        } else if same {
            // Entries of an older generation were stored by a render of another document than
            // the last one prepared: what they hold below the edit is not known to match.
            let current = inner.generation;
            let next = current.wrapping_add(1);
            let mut freed = 0u64;
            inner.tiles.retain(|_, e| {
                let keep = e.generation == current && changed.is_none_or(|ch| e.at <= ch) && boundary(&layers, e.at);
                e.generation = next;
                if !keep {
                    freed = freed.saturating_add(bytes_of(e.buf.rect));
                }
                keep
            });
            inner.bytes = inner.bytes.saturating_sub(freed);
            inner.generation = next;
        } else {
            inner.clear();
            inner.generation = inner.generation.wrapping_add(1);
        }
        let kept = inner.checkpoint.filter(|&k| same && changed.is_none() && boundary(&layers, k));
        // Just below the edit; on a first render, below the top layer (and its clipping group),
        // where the next edit most likely is.
        let below = |top: usize| (1..=top.min(layers.len())).rev().find(|&k| boundary(&layers, k) && k < layers.len());
        let checkpoint = kept.or_else(|| match changed {
            Some(ch) if same => below(ch),
            _ => below(layers.len()),
        });
        inner.context = Some(context);
        inner.layers = layers;
        inner.checkpoint = checkpoint;
        checkpoint.map(|at| Prepared { at, generation: inner.generation })
    }

    /// The cached composite of tile `t` for render `p` and the root layer index it holds the
    /// stack up to.
    pub(crate) fn lookup(&self, t: Rect, p: Prepared) -> Option<(usize, Arc<Buffer>)> {
        let mut inner = self.lock();
        let hit = inner.tiles.get(&t).filter(|e| e.generation == p.generation).map(|e| (e.at, Arc::clone(&e.buf)));
        if hit.is_some() {
            inner.hits = inner.hits.saturating_add(1);
        }
        hit
    }

    /// Whether every tile of `tiles` is cached for render `p` at its capture point or above.
    pub(crate) fn covers(&self, tiles: impl IntoIterator<Item = Rect>, p: Prepared) -> bool {
        let inner = self.lock();
        tiles.into_iter().all(|t| inner.tiles.get(&t).is_some_and(|e| e.generation == p.generation && e.at >= p.at))
    }

    /// Keeps `buf`, tile `t`'s composite of root layers `..at`: replacing the tile's entry, or as
    /// a new one while the budget has room (evicting others for it would let a refresh larger
    /// than the budget evict its own next hits).
    pub(crate) fn store(&self, t: Rect, p: Prepared, buf: &Buffer) {
        let mut inner = self.lock();
        if p.generation != inner.generation {
            return;
        }
        let size = bytes_of(t);
        let old = inner.tiles.get(&t).map(|e| bytes_of(e.buf.rect));
        let after = inner.bytes.saturating_sub(old.unwrap_or(0)).saturating_add(size);
        if after > inner.budget {
            return;
        }
        inner.tiles.insert(t, Entry { generation: p.generation, at: p.at, buf: Arc::new(buf.clone()) });
        inner.bytes = after;
    }
}

impl Inner {
    fn clear(&mut self) {
        self.context = None;
        self.layers.clear();
        self.checkpoint = None;
        self.tiles.clear();
        self.bytes = 0;
    }
}
