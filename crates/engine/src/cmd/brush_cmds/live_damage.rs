//! Frame-relative gradient damage for one live Brush/Pencil/Eraser, without document snapshots.
use photocraft_doc::{Document, Effect, FxPaint, Gradient, GradientStyle, Layer, LayerId};
use photocraft_geom::Rect;

#[derive(PartialEq)]
enum Geometry {
    Linear { c: f32, s: f32, chord: f32, cx: Option<f32>, cy: Option<f32> },
    Radial { cx: f32, cy: f32, length: f32 },
    Diamond { cx: f32, cy: f32, c: f32, s: f32, length: f32 },
    Angle { cx: f32, cy: f32, angle: f32 },
}

#[derive(PartialEq)]
struct Mapping {
    style: GradientStyle,
    reverse: bool,
    geometry: Geometry,
}

impl Mapping {
    /// Capture the effective paint mapping so frame changes with identical mappings stay local.
    fn new(g: &Gradient, frame: Rect) -> Self {
        // Match paint_fx's actual layout, including snapping and offset. Overlay/stroke paint
        // does not use Gradient.align or the effects reference (the latter anchors patterns).
        let (angle, scale, offset) = photocraft_compose::fill_layout::gradient_layout(g.style, g.angle, g.scale, g.offset, frame);
        let (w, h) = (frame.width().max(1) as f32, frame.height().max(1) as f32);
        let (cx, cy) = (frame.x0 as f32 + w / 2.0 + offset.0 * w, frame.y0 as f32 + h / 2.0 + offset.1 * h);
        let (s, c) = angle.to_radians().sin_cos();
        let (chord, length) = photocraft_compose::effects::gradient_units(angle, w, h);
        let (chord, length) = (chord * scale.max(1e-3), length * scale.max(1e-3));
        let geometry = match g.style {
            GradientStyle::Linear | GradientStyle::Reflected => Geometry::Linear {
                c,
                s,
                chord,
                // Ignore only an actually zero coefficient, not an approximate right angle.
                cx: (c != 0.0).then_some(cx),
                cy: (s != 0.0).then_some(cy),
            },
            GradientStyle::Radial => Geometry::Radial { cx, cy, length },
            GradientStyle::Diamond => Geometry::Diamond { cx, cy, c, s, length },
            GradientStyle::Angle => Geometry::Angle { cx, cy, angle },
        };
        Self { style: g.style, reverse: g.reverse, geometry }
    }
}

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

fn mappings(layer: &Layer, frame: Rect) -> Vec<Mapping> {
    gradients(layer).map(|gradient| Mapping::new(gradient, frame)).collect()
}

struct Watched {
    path: Vec<usize>,
    id: LayerId,
    frame: Rect,
    mappings: Vec<Mapping>,
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
                if mappings != watched.mappings {
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
