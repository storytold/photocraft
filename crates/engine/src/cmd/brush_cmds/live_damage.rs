//! Frame-relative gradient damage for one live Brush/Pencil/Eraser, without document snapshots.
use photocraft_compose::effects::GradientMapping;
use photocraft_doc::{Document, Effect, FxPaint, Gradient, Layer, LayerId};
use photocraft_geom::Rect;

/// Select visible, enabled gradient overlays and gradient strokes with positive opacity.
fn gradients(layer: &Layer) -> impl Iterator<Item = &Gradient> {
    layer.effects.items.iter().filter(move |_| layer.visible && layer.effects.enabled).filter_map(|effect| match effect {
        Effect::GradientOverlay { common, gradient, .. } if common.enabled && common.opacity > 0.0 => Some(gradient),
        Effect::Stroke(stroke) if stroke.common.enabled && stroke.common.opacity > 0.0 => match &stroke.paint {
            FxPaint::Gradient(gradient) => Some(gradient),
            _ => None,
        },
        _ => None,
    })
}

fn mappings(layer: &Layer, frame: Rect) -> Vec<GradientMapping> {
    gradients(layer).map(|gradient| GradientMapping::for_effect(gradient, frame)).collect()
}

struct Watched {
    path: Vec<usize>,
    id: LayerId,
    frame: Rect,
    mappings: Vec<GradientMapping>,
    bounds: Rect,
}

pub(super) struct Tracker {
    layers: Vec<Watched>,
}

impl Tracker {
    /// Watch the target and visible ancestors with active gradients, keeping only layout metadata.
    /// Return `None` when the target is unavailable or no relevant gradient needs tracking.
    pub(super) fn new(doc: &Document, layer: Option<LayerId>) -> Option<Self> {
        let path = doc.path_of(layer?)?;
        let mut layers = Vec::new();
        for depth in 1..=path.len() {
            let path = &path[..depth];
            let layer = doc.layer_at(path)?;
            if !layer.visible {
                break;
            }
            if gradients(layer).next().is_none() {
                continue;
            }
            let frame = photocraft_compose::effects_frame(layer, doc.bounds());
            let mappings = mappings(layer, frame);
            if !mappings.is_empty() {
                layers.push(Watched {
                    path: path.to_vec(),
                    id: layer.id,
                    frame,
                    mappings,
                    bounds: photocraft_compose::change_bounds(layer, doc.bounds()).unwrap_or_else(|| doc.bounds()),
                });
            }
        }
        (!layers.is_empty()).then_some(Self { layers })
    }

    /// Called once after a push batch, never by the dab renderer. Effects/tree are unchanged
    /// during a live stroke; target/ancestor paths and only their previous metadata are kept.
    pub(super) fn damage(&mut self, doc: &Document) -> Rect {
        let mut damage = Rect::EMPTY;
        for watched in &mut self.layers {
            let Some(layer) = doc.layer_at(&watched.path).filter(|layer| layer.id == watched.id).or_else(|| doc.layer(watched.id)) else { continue };
            let frame = photocraft_compose::effects_frame(layer, doc.bounds());
            let bounds = photocraft_compose::change_bounds(layer, doc.bounds()).unwrap_or_else(|| doc.bounds());
            if frame != watched.frame {
                let mappings = mappings(layer, frame);
                if mappings.len() != watched.mappings.len() || mappings.iter().zip(&watched.mappings).any(|(a, b)| !a.same_mapping(b)) {
                    damage = damage.union(&watched.bounds).union(&bounds);
                }
                watched.frame = frame;
                watched.mappings = mappings;
            }
            // A group's child effects can change its composite extent while the group's
            // gradient frame stays put. Keep the old extent current for a later relayout.
            watched.bounds = bounds;
        }
        damage
    }
}
