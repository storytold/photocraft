//! Full-canvas composition aids (#5193), deliberately independent of document state and Crop.
//! Geometry comes from the Crop overlay's normalized polylines; only placement and visibility
//! differ. Orientation is a clockwise quarter-turn (low two bits), then a horizontal reflection
//! (bit 2), giving eight distinct orientations without changing Crop's four flip orientations.

use egui::{Pos2, Stroke};
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Rect;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::{PhotocraftApp, canvas::ViewXform, crop_overlay::CropOverlay, state::Tool};

/// Remembered application view settings, never part of a document or its undo history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CompositionGuides {
    pub enabled: bool,
    pub kind: GuideKind,
    pub orientation: u8,
}

/// This first contribution intentionally implements only these three patterns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuideKind {
    #[default]
    Thirds,
    GoldenRatio,
    GoldenSpiral,
}

impl GuideKind {
    /// Accept only documented string IDs at preference/control boundaries, excluding Serde's object-form enum representation.
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "thirds" => Some(Self::Thirds),
            "goldenRatio" => Some(Self::GoldenRatio),
            "goldenSpiral" => Some(Self::GoldenSpiral),
            _ => None,
        }
    }

    /// Reuse Crop's normalized geometry without changing its independent tool settings.
    fn crop(self) -> CropOverlay {
        match self {
            Self::Thirds => CropOverlay::Thirds,
            Self::GoldenRatio => CropOverlay::GoldenRatio,
            Self::GoldenSpiral => CropOverlay::GoldenSpiral,
        }
    }
}

// Preferences/UI snapshots are tolerant; automation below is deliberately strict. Decode fields
// separately so a damaged new field never discards unrelated preferences or the entire UI state.
impl<'de> Deserialize<'de> for CompositionGuides {
    /// Accept older UI snapshots with field-wise defaults; interactive control patches use stricter validation.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::from_saved(&Value::deserialize(d)?))
    }
}

impl CompositionGuides {
    /// Retain each valid saved field independently so one malformed value cannot discard the other preferences.
    pub(crate) fn from_saved(v: &Value) -> Self {
        Self {
            enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(false),
            kind: v.get("kind").and_then(Value::as_str).and_then(GuideKind::from_id).unwrap_or_default(),
            orientation: v.get("orientation").and_then(Value::as_u64).filter(|o| *o < 8).unwrap_or(0) as u8,
        }
    }

    /// Return a fully validated merged copy; an error leaves the caller's view preferences untouched.
    pub(crate) fn patched(mut self, v: &Value) -> Result<Self, String> {
        let m = v.as_object().ok_or("compositionGuides must be an object {enabled?, kind?, orientation?}")?;
        for (k, v) in m {
            match k.as_str() {
                "enabled" => self.enabled = v.as_bool().ok_or("compositionGuides.enabled must be a boolean")?,
                "kind" => {
                    self.kind = v.as_str().and_then(GuideKind::from_id).ok_or("compositionGuides.kind must be a string: thirds, goldenRatio or goldenSpiral")?
                }
                "orientation" => self.orientation = v.as_u64().filter(|o| *o < 8).ok_or("compositionGuides.orientation must be an integer 0..7")? as u8,
                _ => return Err(format!("unknown compositionGuides field `{k}`")),
            }
        }
        Ok(self)
    }

    /// Suspend drawing for either selected Crop tool or a retained frame that still owns Enter/Escape.
    /// A dormant Perspective Crop quad does not own input after an explicit tool switch.
    fn visible(self, app: &PhotocraftApp) -> bool {
        // Ordinary Crop's keyboard ownership follows the frame, including temporary pen erasure
        // and tool changes during a held drag. Its Auto/Never setting does not release ownership.
        self.enabled && app.ui.view.extras && !matches!(app.ui.tool, Tool::Crop | Tool::PerspectiveCrop) && app.ui.crop_rect.is_none()
    }
}

const PREFIX: &str = "view.compositionGuides.";
const ACTIONS: [&str; 7] = ["show", "thirds", "goldenRatio", "goldenSpiral", "rotate", "flipHorizontal", "flipVertical"];

/// Claim only the registered composition command IDs, leaving other command families to their owners.
pub(crate) fn handles(id: &str) -> bool {
    id.strip_prefix(PREFIX).is_some_and(|s| ACTIONS.contains(&s))
}

/// Allow configuration without a document; orientation actions need a selected spiral, even when Show is off.
pub(crate) fn enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if !handles(id) {
        return None;
    }
    Some(!is_orientation(id) || app.ui.view.composition_guides.kind == GuideKind::GoldenSpiral)
}

/// Identify actions that change geometry alone and must not reveal unrelated Extras overlays.
fn is_orientation(id: &str) -> bool {
    matches!(id.strip_prefix(PREFIX), Some("rotate" | "flipHorizontal" | "flipVertical"))
}

/// Report remembered Show and pattern choices even while Crop or Extras temporarily suppresses drawing.
pub(crate) fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let g = app.ui.view.composition_guides;
    match id.strip_prefix(PREFIX)? {
        "show" => Some(g.enabled),
        "thirds" => Some(g.kind == GuideKind::Thirds),
        "goldenRatio" => Some(g.kind == GuideKind::GoldenRatio),
        "goldenSpiral" => Some(g.kind == GuideKind::GoldenSpiral),
        _ => None,
    }
}

/// Validate a complete shell command before changing view preferences. Pattern/Show-on actions reveal Extras;
/// orientation-only actions preserve visibility and never enter document history.
pub(crate) fn invoke(app: &mut PhotocraftApp, id: &str, p: &Value) -> Result<Value, String> {
    let mut g = app.ui.view.composition_guides;
    let empty = serde_json::Map::new();
    let m = if p.is_null() { &empty } else { p.as_object().ok_or("composition guide command params must be an object")? };
    if m.keys().any(|k| id != "view.compositionGuides.show" || k != "on") {
        return Err("only Show Composition Guides accepts a parameter: on (boolean)".into());
    }
    if is_orientation(id) && g.kind != GuideKind::GoldenSpiral {
        return Err("select Golden Spiral before changing its orientation".into());
    }
    match id.strip_prefix(PREFIX) {
        Some("show") => {
            g.enabled = match m.get("on") {
                Some(v) => v.as_bool().ok_or("on must be a boolean")?,
                None => !g.enabled,
            }
        }
        Some("thirds") => {
            g.kind = GuideKind::Thirds;
            g.enabled = true;
        }
        Some("goldenRatio") => {
            g.kind = GuideKind::GoldenRatio;
            g.enabled = true;
        }
        Some("goldenSpiral") => {
            g.kind = GuideKind::GoldenSpiral;
            g.enabled = true;
        }
        // A screen-space clockwise turn reverses the stored turn when already reflected.
        Some("rotate") => g.orientation = (g.orientation & 4) | (((g.orientation % 4) + if g.orientation & 4 == 0 { 1 } else { 3 }) % 4),
        Some("flipHorizontal") => g.orientation ^= 4,
        Some("flipVertical") => g.orientation = (g.orientation ^ 4) ^ 2,
        _ => return Err(format!("unknown composition guide command `{id}`")),
    }
    app.ui.view.composition_guides = g;
    // Changing orientation must not reveal unrelated Extras (grids, guides, selection edges).
    // Only choosing a pattern or explicitly showing composition guides reveals them.
    if g.enabled && !is_orientation(id) {
        app.ui.view.extras = true;
    }
    Ok(json!(g))
}

/// Turn normalized canvas coordinates clockwise, then reflect horizontally; the low bits bound the work.
fn orient([mut x, mut y]: [f32; 2], orientation: u8) -> [f32; 2] {
    for _ in 0..orientation % 4 {
        (x, y) = (1.0 - y, x);
    }
    if orientation & 4 != 0 {
        x = 1.0 - x;
    }
    [x, y]
}

/// Recomputed from bounds each frame (no document/revision cache to become stale). Work is
/// bounded by the number of artboards and guide vertices, independent of pixel count/zoom.
fn screen_guides(g: CompositionGuides, bounds: Rect, xf: &ViewXform) -> Vec<Vec<Pos2>> {
    if bounds.is_empty() {
        return Vec::new();
    }
    let (w, h) = (bounds.width() as f32, bounds.height() as f32);
    crate::crop_overlay::guides(g.kind.crop(), 0, w, h)
        .into_iter()
        .filter_map(|line| {
            let points: Vec<_> = line
                .into_iter()
                .map(|p| {
                    let [x, y] = if g.kind == GuideKind::GoldenSpiral { orient(p, g.orientation) } else { p };
                    xf.to_screen(bounds.x0 as f32 + x * w, bounds.y0 as f32 + y * h)
                })
                .collect();
            points.iter().all(|p| p.is_finite()).then_some(points)
        })
        .collect()
}

/// Capture the displayed document at image paint time, before Move input or after deferred live input.
/// Crop ownership can change during input. Check that at draw time, and skip this capture only
/// when the view settings hide guides or the document index is missing or stale.
pub(crate) fn snapshot(app: &mut PhotocraftApp, idx: usize) -> Option<std::sync::Arc<Document>> {
    if !app.ui.view.composition_guides.enabled || !app.ui.view.extras || app.session.documents().get(idx).is_none() {
        return None;
    }
    Some(crate::canvas::display_doc(app, idx).0)
}

/// Draw noninteractive aids for a displayed document snapshot, using the same camera as the pixels.
/// Missing snapshots and hidden guides draw nothing. The shapes affect neither pixels nor history.
pub(crate) fn paint(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: Option<&Document>) {
    let Some(doc) = doc else { return };
    let g = app.ui.view.composition_guides;
    if !g.visible(app) {
        return;
    }
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
    // Dual strokes stay legible on both light and dark pixels, using the established outline
    // tokens instead of theme text colours (which have no contrast guarantee on document art).
    let [dark, light] = crate::theme::Tokens::cursor_outline();
    let draw = |bounds| {
        for points in screen_guides(g, bounds, xf) {
            painter.add(egui::Shape::line(points.clone(), Stroke::new(2.5, dark)));
            painter.add(egui::Shape::line(points, Stroke::new(1.0, light)));
        }
    };
    if doc.has_artboards() {
        for layer in &doc.layers {
            if layer.visible
                && let LayerContent::Group(group) = &layer.content
                && let Some(board) = &group.artboard
            {
                draw(board.rect);
            }
        }
    } else {
        draw(doc.bounds());
    }
}

#[cfg(test)]
mod tests;
