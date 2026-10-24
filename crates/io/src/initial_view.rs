//! Native view hints applied once after a successful import, never persisted as editing rules.

use std::collections::{HashMap, HashSet};

use photocraft_doc::{Document, LayerId, MAX_GROUP_DEPTH};

use crate::IoError;

pub const MAX_INITIAL_SAMPLERS: usize = 10;
const MAX_INITIAL_LAYERS: usize = 8_192;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeImportTarget {
    #[default]
    Composite,
    LayerMask,
    AlphaChannel(usize),
}

/// References ordinary native items. A host may ignore unavailable view-only affordances,
/// reporting the difference; it must not restore these hints from conversion metadata later.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeInitialView {
    pub selected_layers: Vec<LayerId>,
    pub active_layer: Option<LayerId>,
    pub target: NativeImportTarget,
    pub selected_path: Option<usize>,
    /// Native alpha-channel overlay eyes, independent of the channel editing target.
    pub visible_alpha_channels: Vec<usize>,
    /// Show the active layer mask through the native mask view, without changing content.
    pub show_layer_mask: bool,
    /// Native sampler positions: session-only, without history or native save persistence.
    pub samplers: Vec<[f64; 2]>,
}

impl NativeInitialView {
    /// Validate against the pending native document before changing any host view state.
    pub fn validate(&self, doc: &Document) -> Result<(), IoError> {
        let invalid = |message: &str| IoError::Unsupported(format!("invalid native initial view: {message}"));
        if self.selected_layers.len() > MAX_INITIAL_LAYERS || doc.layers.len() > MAX_INITIAL_LAYERS || self.samplers.len() > MAX_INITIAL_SAMPLERS {
            return Err(invalid("too many selected layers or sampler points"));
        }
        let selected: HashSet<_> = self.selected_layers.iter().copied().collect();
        if selected.len() != self.selected_layers.len() {
            return Err(invalid("duplicate selected layer"));
        }
        // Avoid the recursive document lookup for untrusted hints, including malformed native
        // data supplied by an API caller rather than a bounded format reader.
        let mut layers = HashMap::new();
        let mut pending: Vec<_> = doc.layers.iter().map(|layer| (layer, 0usize)).collect();
        while let Some((layer, depth)) = pending.pop() {
            if depth > MAX_GROUP_DEPTH || layers.len() >= MAX_INITIAL_LAYERS {
                return Err(invalid("layer hierarchy exceeds the initial-view bounds"));
            }
            if layers.insert(layer.id, layer).is_some() {
                return Err(invalid("document contains ambiguous duplicate layer identities"));
            }
            if let Some(children) = layer.children() {
                if children.len() > MAX_INITIAL_LAYERS.saturating_sub(layers.len().saturating_add(pending.len())) {
                    return Err(invalid("layer hierarchy exceeds the initial-view bounds"));
                }
                pending.extend(children.iter().map(|child| (child, depth + 1)));
            }
        }
        if selected.iter().any(|id| !layers.contains_key(id)) {
            return Err(invalid("selected layer is absent"));
        }
        if self.active_layer.is_some_and(|id| !selected.contains(&id)) {
            return Err(invalid("active layer is not selected"));
        }
        match self.target {
            NativeImportTarget::LayerMask => {
                if self.active_layer.and_then(|id| layers.get(&id)).is_none_or(|layer| layer.mask.is_none()) {
                    return Err(invalid("the active layer has no layer mask"));
                }
            }
            NativeImportTarget::AlphaChannel(index) if doc.channels.get(index).is_none() => {
                return Err(invalid("target alpha channel is absent"));
            }
            _ => {}
        }
        if self.selected_path.is_some_and(|index| doc.paths.get(index).is_none()) {
            return Err(invalid("selected path is absent"));
        }
        if self.visible_alpha_channels.len() > doc.channels.len() {
            return Err(invalid("too many visible alpha channels"));
        }
        let visible: HashSet<_> = self.visible_alpha_channels.iter().copied().collect();
        if visible.len() != self.visible_alpha_channels.len() || visible.iter().any(|&index| doc.channels.get(index).is_none()) {
            return Err(invalid("visible alpha channels contain duplicate or absent targets"));
        }
        if self.show_layer_mask && self.active_layer.and_then(|id| layers.get(&id)).is_none_or(|layer| layer.mask.is_none()) {
            return Err(invalid("shown mask is absent from the active layer"));
        }
        for &[x, y] in &self.samplers {
            if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x >= f64::from(doc.size.width) || y >= f64::from(doc.size.height) {
                return Err(invalid("sampler is not a finite point inside the canvas"));
            }
        }
        Ok(())
    }
}
