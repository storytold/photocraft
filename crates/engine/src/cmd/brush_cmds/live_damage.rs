//! Frame-relative gradient damage for one live Brush/Pencil/Eraser, without document snapshots.
use photocraft_compose::effects::{GradientMapping, GradientStrokeLayout};
use photocraft_doc::{Document, Effect, FxPaint, Gradient, Layer, LayerId};
use photocraft_geom::Rect;

/// Select visible, enabled gradients; Stroke geometry comes from the compositor itself.
fn gradients(layer: &Layer) -> impl Iterator<Item = (&Gradient, bool)> {
    layer.effects.items.iter().filter(move |_| layer.visible && layer.effects.enabled).filter_map(|effect| match effect {
        Effect::GradientOverlay { common, gradient, .. } if common.enabled && common.opacity > 0.0 => Some((gradient, false)),
        Effect::Stroke(stroke) if stroke.common.enabled && stroke.common.opacity > 0.0 => match &stroke.paint {
            FxPaint::Gradient(gradient) => Some((gradient, true)),
            _ => None,
        },
        _ => None,
    })
}

fn mappings(layer: &Layer, frame: Rect, layout: Option<&GradientStrokeLayout>) -> Vec<GradientMapping> {
    let mut frames = layout.map(|layout| layout.frames.iter());
    gradients(layer)
        .map(|(gradient, stroke)| {
            let frame = if stroke { frames.as_mut().and_then(|frames| frames.next()).copied().unwrap_or(frame) } else { frame };
            GradientMapping::for_effect(gradient, frame)
        })
        .collect()
}

struct Watched {
    path: Vec<usize>,
    id: LayerId,
    frame: Rect,
    mappings: Vec<GradientMapping>,
    bounds: Rect,
    layout: Option<GradientStrokeLayout>,
}

pub(in crate::cmd) struct Tracker {
    layers: Vec<Watched>,
    shapes: bool,
}

impl Tracker {
    /// Retain only renderer-derived geometry/coverage summaries, never a document or map image.
    /// Gradient Stroke lookups share the compositor's existing full-region map cache.
    pub(in crate::cmd) fn new(doc: &Document, layer: Option<LayerId>) -> Option<Self> {
        let path = doc.path_of(layer?)?;
        let mut layers = Vec::new();
        let mut relevant = false;
        for depth in 1..=path.len() {
            let path = &path[..depth];
            let layer = doc.layer_at(path)?;
            if !layer.visible {
                break;
            }
            relevant |= gradients(layer).next().is_some();
            let frame = photocraft_compose::effects_frame(layer, doc.bounds());
            let layout = photocraft_compose::gradient_stroke_layout(doc, layer);
            let mappings = mappings(layer, frame, layout.as_ref());
            layers.push(Watched {
                path: path.to_vec(),
                id: layer.id,
                frame,
                mappings,
                layout,
                bounds: photocraft_compose::change_bounds(layer, doc.bounds()).unwrap_or_else(|| doc.bounds()),
            });
        }
        let shapes = layers.iter().any(|layer| layer.layout.is_some());
        relevant.then_some(Self { layers, shapes })
    }

    /// Called once after a push batch. Map construction, including masks and child Fill/effects,
    /// is the same renderer work the next CPU/GPU frame requests; only its compact result is kept.
    pub(in crate::cmd) fn damage(&mut self, doc: &Document, raw: Rect) -> Rect {
        let mut relayout = Rect::EMPTY;
        let mut influence = raw;
        // A parent's shape includes its children's effects, so propagate their reach bottom-up.
        for watched in self.layers.iter_mut().rev() {
            if !self.shapes && watched.mappings.is_empty() {
                continue;
            }
            let Some(layer) = doc.layer_at(&watched.path).filter(|layer| layer.id == watched.id).or_else(|| doc.layer(watched.id)) else { continue };
            let frame = photocraft_compose::effects_frame(layer, doc.bounds());
            let bounds = photocraft_compose::change_bounds(layer, doc.bounds()).unwrap_or_else(|| doc.bounds());
            let layout = photocraft_compose::gradient_stroke_layout(doc, layer);
            match (&watched.layout, &layout) {
                (Some(before), Some(after)) => influence = influence.union(&before.shape.changed_bounds(&after.shape)),
                (None, None) => {}
                _ => influence = influence.union(&watched.bounds).union(&bounds),
            }
            let frames_changed = watched.layout.as_ref().map(|layout| &layout.frames) != layout.as_ref().map(|layout| &layout.frames);
            if frame != watched.frame || frames_changed {
                let mappings = mappings(layer, frame, layout.as_ref());
                if mappings.len() != watched.mappings.len() || mappings.iter().zip(&watched.mappings).any(|(a, b)| !a.same_mapping(b)) {
                    let changed = watched.bounds.union(&bounds);
                    relayout = relayout.union(&changed);
                    influence = influence.union(&changed);
                }
                watched.frame = frame;
                watched.mappings = mappings;
            }
            watched.layout = layout;
            watched.bounds = bounds;
            if self.shapes && photocraft_compose::effects::has_effects(layer) {
                influence = influence.inflate(photocraft_compose::effects::margin(layer));
            }
        }
        // Overlay-only strokes retain the original metadata-only path and exact damage policy.
        if self.shapes { influence } else { relayout }
    }
}
