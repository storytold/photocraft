use craft::behavior::{RulerUnits, Ticks};
use craft::draw::ui::transform::{BarSlot, TransformBar, TransformGroups, TransformRequest};
use craft::imaging::ui::warp::{CylinderValues, WarpFieldValues, WarpFields, WarpFieldsEdit, WarpFieldsOutput, WarpGridChoice, WarpStyleEntry};
use craft::ui::canvas::RulerText;
use craft::ui::contract::{Bind, Entry};
use craft::ui::runtime::env::{Label, UnitKind};
use photocraft_engine::prefs::Unit;

use super::begin::{enter_warp, warps_type};
use super::commit::{cancel, commit};
use super::warp::{edit_session_warp, set_warp};
use super::{BezierMesh, PhotocraftApp, Warp, WarpStyle, json};
use crate::state::SessionWarp;

/// Free Transform's options bar: the framework's [`TransformBar`] over the open session. The bar
/// reports edits and the app applies them in the order they came; the persisted choices (unit,
/// aspect link, interpolation, relative position, reference point) are written by the bar to
/// `tool_options.transform`.
///
/// Content-Aware Scale shows the reference point, position and size groups and its own Amount and
/// Protect group. The Warp phase shows the warp group in place of the fields. Each pass applies
/// only the edits its own bar reported: a second pass of a frame gets no input events, so it
/// reports none.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // The bar reads a copy of the session; its edits are applied to the open one after `show`.
    let Some(t) = app.ui.transform.clone() else { return };
    let ctx = ui.ctx().clone();
    let (x_text, y_text) = ruler_texts(app);
    let units = size_unit_entries();
    // A copy too, so the Content-Aware Scale group can borrow the app while the bar holds it.
    let mut settings = app.ui.tool_options.transform.clone();
    let styles = style_entries();
    let split = app.transform_preview.as_ref().and_then(|pv| pv.split);
    let mut warp_out = WarpFieldsOutput::default();
    // Held outside the bar's block: the warp slot borrows them for as long as the bar lives.
    let warp = t.session.warp().map(|w| (warp_values(w), w.on_type));
    let out = {
        let mut bar =
            TransformBar::new(Label::from(tl!("Free Transform").to_string()), &t.session, Bind::bound(&mut settings)).units(x_text, y_text).size_units(&units);
        if t.content_aware.is_some() {
            // `TransformGroups` is non-exhaustive: start from none and switch the shown groups on.
            let mut groups = TransformGroups::NONE;
            groups.reference = true;
            groups.position = true;
            groups.size = true;
            bar = bar.groups(groups).extra(BarSlot::new(|ui| crate::cas_transform::fields(app, ui, &t)));
        }
        if let Some((values, on_type)) = &warp {
            bar = bar.warp_fields(BarSlot::new(|ui| {
                let mut fields = WarpFields::new(&styles, values.clone());
                if *on_type {
                    fields = fields.on_type();
                }
                if let Some(armed) = split {
                    fields = fields.split(armed);
                }
                warp_out = fields.show(ui);
            }));
        }
        bar.show(ui)
    };
    if settings != app.ui.tool_options.transform {
        app.ui.tool_options.transform = settings;
    }
    let now = ui.input(|i| i.time);
    for edit in out.edits {
        let request = app.ui.transform.as_mut().and_then(|open| open.session.apply(edit, now));
        if let Some(TransformRequest::NeedWarp { .. }) = request {
            enter_warp(app, &ctx);
        }
    }
    for edit in warp_out.edits {
        warp_edit(app, edit);
    }
    if out.reset_warp {
        reset_warp(app);
    }
    if out.apply {
        commit(app);
    } else if out.discard {
        cancel(app);
    }
}

/// The units of the W and H menu, in Photoshop's order.
fn size_unit_entries() -> Vec<Entry<UnitKind>> {
    let name = |name: &str| Label::from(name.to_string());
    vec![
        Entry::new(UnitKind::Pixels, name(tl!("Pixels"))),
        Entry::new(UnitKind::Inches, name(tl!("Inches"))),
        Entry::new(UnitKind::Centimetres, name(tl!("Centimeters"))),
        Entry::new(UnitKind::Millimetres, name(tl!("Millimeters"))),
        Entry::new(UnitKind::Points, name(tl!("Points"))),
        Entry::new(UnitKind::Picas, name(tl!("Picas"))),
        Entry::new(UnitKind::Percent, name(tl!("Percent"))),
    ]
}

// PLAN: the `RulerText` construction below repeats `panels/layer_props_ui.rs` `ruler_text` (private there); panels-w36 owns the file and should expose one helper for both callers.
/// The rulers' text for X and Y: the Units & Rulers unit at the document's resolution, with
/// Percent measured against the document's width and height.
fn ruler_texts(app: &PhotocraftApp) -> (RulerText, RulerText) {
    let ur = &app.session.prefs().units_and_rulers;
    let dpi = app.session.active().map_or(72.0, |s| f64::from(s.doc.resolution_dpi));
    let ppi = ur.point_size.per_inch();
    let (dw, dh) = app.session.active().map_or((1.0, 1.0), |s| (f64::from(s.doc.size.width), f64::from(s.doc.size.height)));
    let text = |extent: f64| {
        let unit = ur.rulers;
        RulerText::new(RulerUnits::new(unit.from_px(1.0, dpi, extent, ppi), Ticks::decimal(unit == Unit::Pixels)), crate::prefs::unit_kind(unit))
    };
    (text(dw), text(dh))
}

/// The Warp menu's styles in Photoshop's order: None, Custom and the presets, then Cylinder
/// (#4112). Custom and Cylinder are not offered over type (#3004: Warp Text has neither), and a
/// cylinder has no splits or grid (its handles are its own).
pub(super) fn style_entries() -> Vec<WarpStyleEntry> {
    let entry = |s: WarpStyle| WarpStyleEntry::new(s.id(), s.label()).preset(s.is_preset());
    WarpStyle::all()
        .map(|s| if s == WarpStyle::Custom { entry(s).over_type(false) } else { entry(s) })
        .chain(std::iter::once(entry(WarpStyle::Cylinder).over_type(false).splits(false)))
        .collect()
}

/// What the warp group shows for the session's warp.
pub(super) fn warp_values(w: &SessionWarp) -> WarpFieldValues {
    let mut values = WarpFieldValues::new(w.warp.style.id());
    values.vertical = w.warp.vertical;
    values.bend = w.warp.bend;
    values.horizontal = w.warp.h_distort;
    values.vertical_distortion = w.warp.v_distort;
    values.grid = WarpGridChoice::of(&w.mesh);
    values
}

/// The cylinder the Warp menu starts with over the box `b`; `None` for a box too thin for one.
fn cylinder_default(b: [f64; 4]) -> Option<Warp> {
    (b[2] > b[0] && b[3] > b[1]).then(|| Warp::cylinder(&CylinderValues::for_box(b).to_array(), b)).flatten()
}

/// Applies one edit of the warp group to the session's warp. Each edit reads the warp as the
/// edits before it left it.
fn warp_edit(app: &mut PhotocraftApp, edit: WarpFieldsEdit) {
    let Some(w) = app.ui.transform.as_ref().and_then(|t| t.session.warp()).map(|w| w.warp.clone()) else { return };
    let b = w.bounds;
    let preset = w.style.is_preset();
    let next = match edit {
        WarpFieldsEdit::Split(s) => {
            if let Some(pv) = app.transform_preview.as_mut() {
                pv.split = s;
            }
            return;
        }
        WarpFieldsEdit::Grid(n) => {
            let _ = edit_session_warp(app, "edit.transform.warpGrid", &json!({ "size": n }));
            return;
        }
        WarpFieldsEdit::Style(key) => {
            let Some(style) = WarpStyle::parse(&key) else { return };
            let on_type = warps_type(app);
            match style {
                WarpStyle::Custom => Warp::custom(w.to_mesh(1, 1), b),
                WarpStyle::None if on_type => Warp::none(b),
                WarpStyle::None => Warp::custom(BezierMesh::identity(b, 1, 1), b),
                // A box too thin for a cylinder keeps its shape.
                WarpStyle::Cylinder => cylinder_default(b).unwrap_or_else(|| Warp::custom(w.to_mesh(1, 1), b)),
                s => Warp { style: s, bend: if preset { w.bend } else { 50.0 }, ..Warp::none(b) },
            }
        }
        // The orientation and the fields act on a preset style only.
        _ if !preset => return,
        WarpFieldsEdit::Orientation { vertical, .. } => Warp { vertical, ..w },
        WarpFieldsEdit::Bend(event) => match event.landed() {
            Some(v) => Warp { bend: *v, ..w },
            None => return,
        },
        WarpFieldsEdit::Horizontal(event) => match event.landed() {
            Some(v) => Warp { h_distort: *v, ..w },
            None => return,
        },
        WarpFieldsEdit::Vertical(event) => match event.landed() {
            Some(v) => Warp { v_distort: *v, ..w },
            None => return,
        },
        _ => return,
    };
    set_warp(app, next);
}

/// Back to the untouched box: the default one-patch grid over the box, no bend (over type, the
/// None style). One undo step through the session, and no Split tool armed.
pub fn reset_warp(app: &mut PhotocraftApp) {
    let Some((b, on_type)) = app.ui.transform.as_ref().and_then(|t| t.session.warp()).map(|w| (w.warp.bounds, w.on_type)) else { return };
    set_warp(app, if on_type { Warp::none(b) } else { Warp::custom(BezierMesh::identity(b, 1, 1), b) });
    if let Some(pv) = app.transform_preview.as_mut() {
        pv.split = None;
    }
}
