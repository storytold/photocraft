//! Layer comps (Window › Layer Comps) and artboards: pure data plus capture/apply helpers that need
//! nothing beyond the document model.
//!
//! A layer comp records, per layer, its visibility, position and appearance (blend mode, opacity,
//! fill opacity, layer style). Three flags choose which of those [`apply`](LayerComp) restores,
//! like the checkboxes of Photoshop's Layer Comp Options. Positions are whole-pixel coordinates:
//! content bounds for pixel layers, board bounds for artboards, and the transform origin for Type.
//! Type's ink bounds can change with its content or style without changing its placement.

use photocraft_color::{BlendMode, Color};
use photocraft_geom::Rect;
use serde::{Deserialize, Serialize};

use crate::{Document, Effects, Layer, LayerContent, LayerId};

/// One layer's state as captured by a comp. `None` = not recorded (PSD comps record only what
/// their capture flags asked for).
#[derive(Clone, Debug, PartialEq)]
pub struct CompLayerState {
    pub layer: LayerId,
    pub visible: Option<bool>,
    /// Top-left of the content bounds (or artboard rect). None for layers without a position
    /// (adjustment and fill layers, empty layers, plain groups).
    pub position: Option<(i32, i32)>,
    /// Whole-pixel transform origin for editable Type, independent of its current ink bounds.
    /// `None` in older native files: their `position` retains its content-bounds meaning.
    pub text_position: Option<(i32, i32)>,
    pub appearance: Option<CompAppearance>,
}

impl CompLayerState {
    /// Move from the layer's current placement to its captured placement. PSD offsets use the
    /// same displacement. Checked subtraction also rejects unreachable positions from files.
    pub fn position_delta(&self, bounds: Option<(i32, i32)>, text: Option<(i32, i32)>) -> Option<(i32, i32)> {
        let (saved, current) = match (self.text_position, text) {
            (Some(saved), Some(current)) => (saved, current),
            _ => (self.position?, bounds?),
        };
        Some((saved.0.checked_sub(current.0)?, saved.1.checked_sub(current.1)?))
    }

    /// Compare the position using the same anchor as application, including the legacy fallback.
    pub fn matches_position(&self, live: &Self) -> bool {
        match (self.text_position, live.text_position) {
            (Some(saved), Some(current)) => saved == current,
            _ => self.position.is_none() || self.position == live.position,
        }
    }
}

/// Type's whole-pixel placement, without consulting its rendered cache. Flooring retains a
/// fractional origin through integer translations, including moves across zero. Comps and PSD
/// offsets have always moved by whole pixels; do not quantize the Type transform itself.
pub fn text_position(layer: &Layer) -> Option<(i32, i32)> {
    let LayerContent::Text(t) = &layer.content else { return None };
    let [_, _, _, _, x, y] = t.transform.m;
    let whole = |v: f64| {
        let v = v.floor();
        (v.is_finite() && v >= f64::from(i32::MIN) && v <= f64::from(i32::MAX)).then_some(v as i32)
    };
    Some((whole(x)?, whole(y)?))
}

/// Layer › Layer Style state captured by "Appearance".
#[derive(Clone, Debug, PartialEq)]
pub struct CompAppearance {
    pub blend: BlendMode,
    pub opacity: f32,
    pub fill_opacity: f32,
    pub effects: Effects,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerComp {
    /// Stable id (the PSD `compID`). Never 0: Photoshop uses 0 for the Last Document State.
    pub id: u32,
    pub name: String,
    pub comment: String,
    pub apply_visibility: bool,
    pub apply_position: bool,
    pub apply_appearance: bool,
    /// Per-layer state, in document walk order.
    pub states: Vec<CompLayerState>,
}

impl LayerComp {
    /// PSD `capturedInfo` bit flags: 1 visibility, 2 position, 4 appearance.
    pub fn captured_info(&self) -> i32 {
        i32::from(self.apply_visibility) | i32::from(self.apply_position) << 1 | i32::from(self.apply_appearance) << 2
    }
    pub fn set_captured_info(&mut self, v: i32) {
        self.apply_visibility = v & 1 != 0;
        self.apply_position = v & 2 != 0;
        self.apply_appearance = v & 4 != 0;
    }
    pub fn state(&self, id: LayerId) -> Option<&CompLayerState> {
        self.states.iter().find(|s| s.layer == id)
    }
    /// Ids of recorded layers that no longer exist in `doc` (Photoshop's warning triangle).
    pub fn missing_layers(&self, doc: &Document) -> Vec<LayerId> {
        let live: std::collections::HashSet<LayerId> = doc.walk().into_iter().map(|(_, _, l)| l.id).collect();
        self.states.iter().map(|s| s.layer).filter(|id| !live.contains(id)).collect()
    }
}

/// Content bounds used for comp positions: pixels (or the rendered cache) of a layer, the board of
/// an artboard. Empty for adjustment/fill layers and plain groups.
pub fn position_bounds(l: &Layer) -> Rect {
    match &l.content {
        LayerContent::Group(g) => g.artboard.as_ref().map_or(Rect::EMPTY, |a| a.rect),
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => Rect::EMPTY,
        _ => l.surface().map_or(Rect::EMPTY, |s| s.content_bounds()),
    }
}

/// Comp position of a layer (see [`position_bounds`]).
pub fn layer_position(l: &Layer) -> Option<(i32, i32)> {
    let r = position_bounds(l);
    (!r.is_empty()).then_some((r.x0, r.y0))
}

/// Capture the full state of every layer.
pub fn capture_states(doc: &Document) -> Vec<CompLayerState> {
    doc.walk()
        .into_iter()
        .map(|(_, _, l)| CompLayerState {
            layer: l.id,
            visible: Some(l.visible),
            position: layer_position(l),
            text_position: text_position(l),
            appearance: Some(CompAppearance { blend: l.blend, opacity: l.opacity, fill_opacity: l.fill_opacity, effects: l.effects.clone() }),
        })
        .collect()
}

/// A fresh comp id, unique among `doc`'s comps (never 0).
pub fn next_comp_id(doc: &Document) -> u32 {
    // Photoshop uses large pseudo-random ids; any unique non-zero value round-trips.
    let mut id = doc.layer_comps.iter().map(|c| c.id).max().unwrap_or(0).wrapping_add(1).max(1);
    while doc.layer_comps.iter().any(|c| c.id == id) {
        id = id.wrapping_add(1).max(1);
    }
    id
}

/// What an artboard paints behind its contents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtboardBackground {
    #[default]
    White,
    Black,
    Transparent,
    Custom(Color),
}

impl ArtboardBackground {
    /// sRGB colour painted under the contents (None = transparent).
    pub fn rgba(&self) -> Option<[f32; 4]> {
        match self {
            ArtboardBackground::White => Some([1.0, 1.0, 1.0, 1.0]),
            ArtboardBackground::Black => Some([0.0, 0.0, 0.0, 1.0]),
            ArtboardBackground::Transparent => None,
            ArtboardBackground::Custom(c) => {
                let [r, g, b] = c.to_rgb();
                (c.alpha > 0.0).then_some([r, g, b, c.alpha])
            }
        }
    }
    /// PSD `artboardBackgroundType`: 1 white, 2 black, 3 transparent, 4 other.
    pub fn psd_type(&self) -> i32 {
        match self {
            ArtboardBackground::White => 1,
            ArtboardBackground::Black => 2,
            ArtboardBackground::Transparent => 3,
            ArtboardBackground::Custom(_) => 4,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            ArtboardBackground::White => "white",
            ArtboardBackground::Black => "black",
            ArtboardBackground::Transparent => "transparent",
            ArtboardBackground::Custom(_) => "custom",
        }
    }
}

/// An artboard: a top-level group that clips its children to `rect` and paints `background`
/// behind them (Layer › New › Artboard).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artboard {
    /// The board in document pixels.
    pub rect: Rect,
    pub background: ArtboardBackground,
    /// Size preset the board was made from ("" = custom).
    #[serde(default)]
    pub preset: String,
}

impl Artboard {
    pub fn new(rect: Rect) -> Self {
        Self { rect, background: ArtboardBackground::White, preset: String::new() }
    }
}

impl Document {
    /// Top-level artboards, bottom to top: (layer id, name, artboard).
    pub fn artboards(&self) -> Vec<(LayerId, &str, &Artboard)> {
        self.layers
            .iter()
            .filter_map(|l| match &l.content {
                LayerContent::Group(g) => g.artboard.as_ref().map(|a| (l.id, l.name.as_str(), a)),
                _ => None,
            })
            .collect()
    }
    pub fn has_artboards(&self) -> bool {
        self.layers.iter().any(|l| l.artboard().is_some())
    }
    pub fn comp(&self, id: u32) -> Option<&LayerComp> {
        self.layer_comps.iter().find(|c| c.id == id)
    }
    /// The artboard containing layer `id` (itself, or its top-level ancestor).
    pub fn artboard_of(&self, id: LayerId) -> Option<LayerId> {
        let path = self.path_of(id)?;
        let top = self.layers.get(*path.first()?)?;
        top.artboard().map(|_| top.id)
    }
}

impl Layer {
    pub fn artboard(&self) -> Option<&Artboard> {
        match &self.content {
            LayerContent::Group(g) => g.artboard.as_ref(),
            _ => None,
        }
    }
    pub fn artboard_mut(&mut self) -> Option<&mut Artboard> {
        match &mut self.content {
            LayerContent::Group(g) => g.artboard.as_mut(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorMode, PixelFormat, SampleType, Size};

    #[test]
    fn type_position_uses_placement_even_without_ink_and_retains_legacy_fallback() {
        let mut layer =
            Layer::new("Type", LayerContent::Text(crate::TextLayer { transform: photocraft_geom::Affine::translate(-0.5, 12.75), ..Default::default() }));
        let mut doc = doc();
        doc.layers.push(layer.clone());
        let saved = capture_states(&doc).pop().unwrap();
        assert_eq!(saved.position, None, "empty Type has no ink bounds");
        assert_eq!(saved.text_position, Some((-1, 12)));
        assert_eq!(saved.position_delta(layer_position(&layer), text_position(&layer)), Some((0, 0)));
        let LayerContent::Text(t) = &mut layer.content else { panic!("type") };
        t.transform = photocraft_geom::Affine::translate(39.5, -7.25);
        assert_eq!(saved.position_delta(layer_position(&layer), text_position(&layer)), Some((-40, 20)));
        let mut cache = photocraft_raster::Surface::new(PixelFormat::RGBA8);
        cache.fill_rect(Rect::new(4, 6, 8, 10), &[1.0, 0.0, 0.0, 1.0]);
        let LayerContent::Text(t) = &mut layer.content else { panic!("type") };
        t.cache = Some(cache);
        let legacy = CompLayerState { position: Some((10, 20)), text_position: None, ..saved.clone() };
        assert_eq!(legacy.position_delta(layer_position(&layer), text_position(&layer)), Some((6, 14)), "older native comps still refer to bounds");
        let unreachable = CompLayerState { text_position: Some((i32::MIN, 0)), ..saved };
        assert_eq!(unreachable.position_delta(layer_position(&layer), text_position(&layer)), None);
    }

    #[test]
    fn malformed_type_transform_does_not_create_a_saturated_position() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::from(i32::MAX) + 1.0] {
            for transform in [photocraft_geom::Affine::translate(value, 0.0), photocraft_geom::Affine::translate(0.0, value)] {
                let layer = Layer::new("Type", LayerContent::Text(crate::TextLayer { transform, ..Default::default() }));
                assert_eq!(text_position(&layer), None);
            }
        }
    }

    fn doc() -> Document {
        let mut d = Document::with_background("t", Size::new(64, 64), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut l = Layer::raster("A", PixelFormat::RGBA8);
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 12, 20, 22), &[1.0, 0.0, 0.0, 1.0]);
        d.layers.push(l);
        d
    }

    #[test]
    fn capture_records_every_layer() {
        let d = doc();
        let s = capture_states(&d);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].position, Some((10, 12)));
        assert_eq!(s[1].visible, Some(true));
        assert!(s[1].appearance.is_some());
    }

    #[test]
    fn captured_info_bits() {
        let mut c = LayerComp {
            id: 1,
            name: "c".into(),
            comment: String::new(),
            apply_visibility: false,
            apply_position: false,
            apply_appearance: false,
            states: vec![],
        };
        c.set_captured_info(5);
        assert!(c.apply_visibility && !c.apply_position && c.apply_appearance);
        assert_eq!(c.captured_info(), 5);
    }

    #[test]
    fn missing_layers_and_ids() {
        let mut d = doc();
        let states = capture_states(&d);
        d.layer_comps.push(LayerComp {
            id: next_comp_id(&d),
            name: "c".into(),
            comment: String::new(),
            apply_visibility: true,
            apply_position: true,
            apply_appearance: true,
            states,
        });
        assert_eq!(d.layer_comps[0].id, 1);
        assert_eq!(next_comp_id(&d), 2);
        let gone = d.layers[1].id;
        d.layers.pop();
        assert_eq!(d.layer_comps[0].missing_layers(&d), vec![gone]);
    }

    #[test]
    fn artboards_listed_and_found() {
        let mut d = doc();
        let child = Layer::raster("in", PixelFormat::RGBA8);
        let cid = child.id;
        let mut g = Layer::group("Artboard 1", vec![child]);
        if let LayerContent::Group(gr) = &mut g.content {
            gr.artboard = Some(Artboard::new(Rect::new(0, 0, 32, 32)));
        }
        let gid = g.id;
        d.layers.push(g);
        assert!(d.has_artboards());
        assert_eq!(d.artboards().len(), 1);
        assert_eq!(d.artboard_of(cid), Some(gid));
        assert_eq!(layer_position(d.layer(gid).unwrap()), Some((0, 0)));
        assert_eq!(ArtboardBackground::Black.rgba(), Some([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(ArtboardBackground::Transparent.rgba(), None);
    }
}
