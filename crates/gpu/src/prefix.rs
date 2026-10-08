//! One lower-stack composite, held in bounded chunk textures. Exact COW snapshots deliberately
//! invalidate on more than rendering needs: completeness matters more than a clever render key.
use std::collections::HashMap;

use photocraft_doc::{Document, Layer};
use photocraft_geom::Rect;

use crate::Tex;
use crate::plan::{Checkpoint, Plan};

pub(crate) const MAX_BYTES: u64 = 512 << 20;

pub(crate) struct Prefix {
    context: Option<Document>,
    layers: Vec<Layer>,
    checkpoint: Option<Checkpoint>,
    chunks: HashMap<Rect, Tex>,
    bytes: u64,
    budget: u64,
}

impl Default for Prefix {
    fn default() -> Self {
        Self { context: None, layers: Vec::new(), checkpoint: None, chunks: HashMap::new(), bytes: 0, budget: MAX_BYTES }
    }
}

impl Prefix {
    pub(crate) fn clear(&mut self) {
        self.context = None;
        self.layers.clear();
        self.checkpoint = None;
        self.chunks.clear();
        self.bytes = 0;
    }

    pub(crate) fn forget(&mut self, doc: photocraft_doc::DocId) {
        if self.context.as_ref().is_some_and(|d| d.id == doc) {
            self.clear();
        }
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(crate) fn budget(&self) -> u64 {
        self.budget
    }

    pub(crate) fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes.min(MAX_BYTES);
        if self.bytes > self.budget || self.budget == 0 {
            self.clear();
        }
    }

    pub(crate) fn prepare(&mut self, doc: &Document, plan: &Plan<'_>) -> Option<Checkpoint> {
        if self.budget == 0 {
            return None;
        }
        // Keeping every non-layer field makes new document inputs invalidate automatically.
        // Tile data is Arc-shared; shared Tile equality does not scan its pixel bytes.
        let mut context = doc.clone();
        let mut layers = std::mem::take(&mut context.layers);
        let same_context = self.context.as_ref() == Some(&context);
        let changed = self.layers.iter().zip(&layers).position(|(a, b)| a != b).or_else(|| (layers.len() < self.layers.len()).then_some(layers.len()));
        let old = self.checkpoint.filter(|c| same_context && changed.is_none() && plan.checkpoints.contains(c));
        let checkpoint = old.or_else(|| {
            if same_context && let Some(changed) = changed {
                plan.checkpoints.iter().rev().find(|c| c.layer_end <= changed && c.pass_end > 1).copied()
            } else {
                let mut useful = plan.checkpoints.iter().rev().filter(|c| c.pass_end > 1);
                let last = useful.next().copied();
                useful.next().copied().or(last)
            }
        });
        if old.is_none() {
            self.clear();
        }
        let Some(checkpoint) = checkpoint else {
            // Remember bottom-layer edits even when no useful prefix survives: otherwise
            // every other edit would populate and immediately discard the default checkpoint.
            self.context = Some(context);
            self.layers = layers;
            return None;
        };
        layers.truncate(checkpoint.layer_end);
        self.context = Some(context);
        self.layers = layers;
        self.checkpoint = Some(checkpoint);
        Some(checkpoint)
    }

    pub(crate) fn contains(&self, rect: Rect) -> bool {
        self.chunks.contains_key(&rect)
    }

    pub(crate) fn chunk(&self, rect: Rect) -> Option<&Tex> {
        self.chunks.get(&rect)
    }

    pub(crate) fn capture(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, rect: Rect, src: &wgpu::Texture) {
        if self.chunks.contains_key(&rect) {
            return;
        }
        let bpp = if src.format() == wgpu::TextureFormat::Rgba32Float { 16 } else { 8 };
        let bytes = u64::from(rect.width()).saturating_mul(u64::from(rect.height())).saturating_mul(bpp);
        if bytes == 0 || bytes > self.budget.saturating_sub(self.bytes) {
            return;
        }
        // Admit while space remains; replacing entries on every miss would make a full refresh
        // larger than the cache evict its own next hits.
        let tex =
            Tex::new(device, "pc_compose_prefix", rect.width(), rect.height(), src.format(), wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST);
        copy(encoder, src, &tex.texture, rect);
        self.bytes = self.bytes.saturating_add(bytes);
        self.chunks.insert(rect, tex);
    }
}

pub(crate) fn copy(encoder: &mut wgpu::CommandEncoder, src: &wgpu::Texture, dst: &wgpu::Texture, rect: Rect) {
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo { texture: src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyTextureInfo { texture: dst, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::Extent3d { width: rect.width(), height: rect.height(), depth_or_array_layers: 1 },
    );
}
