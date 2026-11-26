//! What the mask thumbnails of a layer row do: their clicks, the selection they load, and the
//! vector mask's picture.

use egui::Color32;
use photocraft_doc::{Document, Layer, VectorMask};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Which of a layer's masks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskKind {
    Pixel,
    Vector,
}

impl MaskKind {
    /// The engine command prefix for this mask.
    fn command(self) -> &'static str {
        match self {
            MaskKind::Pixel => "layer.layerMask",
            MaskKind::Vector => "layer.vectorMask",
        }
    }
}

/// `(kind, enabled, linked)` for each of `l`'s masks, in row order.
pub fn masks(l: &Layer) -> Vec<(MaskKind, bool, bool)> {
    if l.frame.is_some() {
        return Vec::new();
    }
    let mut v = Vec::with_capacity(2);
    if let Some(m) = &l.mask {
        v.push((MaskKind::Pixel, m.enabled, m.linked));
    }
    if let Some(m) = &l.vector_mask {
        v.push((MaskKind::Vector, m.enabled, m.linked));
    }
    v
}

/// The command a click on mask thumbnail `kind` issues with modifiers `m`, if it is one that
/// replaces the normal click: ⇧ toggles the mask on/off; ⌥ views a layer mask alone in
/// grayscale and ⇧⌥ as a rubylith overlay (again: back to the composite), like Photoshop.
pub fn click_command(l: &Layer, kind: MaskKind, m: egui::Modifiers) -> Option<(String, Value)> {
    if m.command {
        return None;
    }
    if m.alt {
        let mode = if m.shift { "toggleOverlay" } else { "toggleGray" };
        return (kind == MaskKind::Pixel && l.mask.is_some()).then(|| (photocraft_engine::mask_view_cmds::ID.into(), json!({"layer": l.id.0, "mode": mode})));
    }
    if !m.shift {
        return None;
    }
    let enabled = masks(l).into_iter().find(|(k, _, _)| *k == kind)?.1;
    Some((format!("{}.enabled", kind.command()), json!({"layer": l.id.0, "enabled": !enabled})))
}

/// The `select.loadSelection` params for a ⌘-click on mask thumbnail `kind` (⌘⇧ add, ⌘⌥
/// subtract, ⌘⇧⌥ intersect); `mods` are the click's framework modifiers.
pub fn load_params(l: &Layer, kind: MaskKind, mods: craft::keymap::Mods) -> Value {
    let channel = match kind {
        MaskKind::Pixel => "mask",
        MaskKind::Vector => "vectorMask",
    };
    json!({"channel": channel, "layer": l.id.0, "operation": craft::behavior::geometry::SelectionOperation::from_mods(mods).name()})
}

/// What double-clicking a mask thumbnail does: pixel mask launches Select and Mask if preference is enabled.
pub fn double_click_command(app: &PhotocraftApp, kind: MaskKind) -> Option<(&'static str, Value)> {
    if kind == MaskKind::Pixel
        && app.session.prefs().tools.double_click_layer_mask_launches_select_and_mask
        && crate::menus::is_enabled(app, "select.selectAndMask")
    {
        Some(("select.selectAndMask", Value::Null))
    } else {
        None
    }
}

/// The active layer of `st` has a vector mask (shape layers' paths are content, not masks).
pub fn has_vector_mask(st: &photocraft_engine::DocState) -> bool {
    st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.vector_mask.is_some() && !matches!(l.content, photocraft_doc::LayerContent::Shape(_)))
}

/// Cheap identity of a vector mask's geometry and state.
pub fn vector_fingerprint(m: &VectorMask) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x100_0000_01b3);
    mix(u64::from(m.path.inverted) | u64::from(m.path.fill_rule == photocraft_doc::FillRule::EvenOdd) << 1);
    mix(u64::from(m.density.to_bits()));
    for s in &m.path.subpaths {
        mix(u64::from(s.closed) | (s.op as u64) << 1 | (s.knots.len() as u64) << 8);
        for k in &s.knots {
            for p in [k.anchor, k.in_ctrl, k.out_ctrl] {
                mix(p.x.to_bits());
                mix(p.y.to_bits());
            }
        }
    }
    h
}

/// A `side`-pixel square thumbnail of vector mask `m`: white where it reveals, grey where it
/// hides, letterboxed to the document's aspect like the other row thumbnails. Disabled masks
/// still show their shape (the row crosses them out).
pub fn vector_thumb_image(doc: &Document, m: &VectorMask, side: usize) -> egui::ColorImage {
    let (w, h) = (f64::from(doc.size.width.max(1)), f64::from(doc.size.height.max(1)));
    let s = side as f64 / w.max(h);
    let (ox, oy) = ((side as f64 - w * s) / 2.0, (side as f64 - h * s) / 2.0);
    let shown = VectorMask { path: m.path.transform(&photocraft_geom::Affine { m: [s, 0.0, 0.0, s, ox, oy] }), enabled: true, ..m.clone() };
    let n = side as i32;
    let values = photocraft_vector::vector_mask_values(&shown, photocraft_geom::Rect::new(0, 0, n, n));
    let (x0, y0, x1, y1) = (ox.floor() as usize, oy.floor() as usize, (ox + w * s).ceil() as usize, (oy + h * s).ceil() as usize);
    let mut px = vec![Color32::TRANSPARENT; side * side];
    for (i, p) in px.iter_mut().enumerate() {
        let (tx, ty) = (i % side, i / side);
        if tx < x0 || ty < y0 || tx >= x1 || ty >= y1 {
            continue;
        }
        let v = values.get(i).copied().unwrap_or(1.0).clamp(0.0, 1.0);
        *p = Color32::from_gray((128.0 + 127.0 * v + 0.5) as u8);
    }
    egui::ColorImage::new([side, side], px)
}

#[cfg(test)]
mod tests;
