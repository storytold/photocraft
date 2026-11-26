use craft::behavior::{IconId, RulerUnits, Ticks};
use craft::draw::ui::transform::{BarSlot, TransformBar, TransformGroups, TransformRequest};
use craft::imaging::ui::warp::{CylinderValues, WarpFieldValues, WarpFields, WarpFieldsEdit, WarpFieldsOutput, WarpGridChoice, WarpStyleEntry};
use craft::theme::Srgba;
use craft::ui::buttons::{IconButton, PopupMark};
use craft::ui::canvas::RulerText;
use craft::ui::choice::{Radio, RadioGroup, Swatch};
use craft::ui::contract::{Bind, Entry, PopupKind, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::display::{Divider, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::popover::Popover;
use craft::ui::runtime::env::{Env, Label, UnitKind};
use craft::ui::text::RangeAlert;
use photocraft_engine::prefs::Unit;

use super::begin::{enter_warp, warps_type};
use super::commit::{cancel, commit};
use super::warp::{edit_session_warp, set_warp};
use super::{BezierMesh, PhotocraftApp, Warp, WarpStyle, json};
use crate::option_cells::number;
use crate::state::{SessionWarp, WarpGuideOptions, WarpGuideShow};

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
    // The guide-line options as a copy, with how many thin lines the grid draws on its own: the popup
    // edits the copy while the bar holds the app, and the bar writes it back after its show (#12501).
    let mut guides = app.ui.tool_options.warp_guides;
    let own_guides = t.session.warp().map_or(0, |w| w.mesh.own_guide_count());
    let mut ask_colour: Option<[f32; 3]> = None;
    let mut guide_alert: Option<RangeAlert> = None;
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
                Divider::vertical().show(ui);
                guide_gear(&mut guides, own_guides, &mut ask_colour, &mut guide_alert, ui);
            }));
        }
        bar.show(ui)
    };
    if settings != app.ui.tool_options.transform {
        app.ui.tool_options.transform = settings;
    }
    if guides != app.ui.tool_options.warp_guides {
        app.ui.tool_options.warp_guides = guides;
    }
    // The Color well's picker, opened once the bar has let go of the app (#12501).
    if let Some(rgb) = ask_colour
        && let Err(message) = crate::color_picker_ui::open_for_target(app, "warpGuideColor", &crate::color_picker_ui::title_for(tl!("Color")), rgb)
    {
        crate::notices::error(app, message);
    }
    if let Some(alert) = guide_alert.as_ref() {
        crate::option_cells::range_alert(app, ui, alert);
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

/// Whether the Warp guide-line popup is open, kept by the framework per viewer (#12501).
#[derive(Clone, Copy, Default)]
struct GuideGearOpen(bool);

/// The colour the warp guide lines show now, and the one the Color Picker opens on: the person's
/// choice, or the theme's own overlay line while none is set (#12501).
fn guide_colour(guides: &WarpGuideOptions, ui: &egui::Ui) -> [f32; 3] {
    guides.color.unwrap_or_else(|| Srgba::from(Env::of(ui).tokens.tokens().color.overlay.line).to_rgb_array())
}

/// The gear right of the Warp style dropdown and its popup of guide-line options (#12501): Color, a
/// well that opens the Color Picker; Opacity, the share of the line's alpha in percent; Density, the
/// thin lines inside each cell; and the three Show Guides rows. It edits the `guides` copy the bar
/// holds and hands back the colour the well was clicked on and any range alert, because the bar, not
/// the slot, holds the app. `own` is how many lines the grid draws on its own, which the Density field
/// shows until the person picks a number.
fn guide_gear(guides: &mut WarpGuideOptions, own: usize, ask_colour: &mut Option<[f32; 3]>, alert: &mut Option<RangeAlert>, ui: &mut egui::Ui) {
    let title = tl!("Set warp guide line display options").to_string();
    Row::new().show(ui, |ui, cx| {
        let open = &mut cx.memory.shown::<GuideGearOpen>(cx.id).0;
        let button = IconButton::new(IconId::new("settings"))
            .label(title.clone())
            .tooltip(tl!("Set warp guide line display options"))
            .popup(PopupKind::Dialog)
            .popup_mark(PopupMark::Menu)
            .expanded(*open)
            .show(ui)
            .response;
        if clicked(&button) {
            *open = !*open;
        }
        Popover::new(title.clone(), Bind::bound(open)).opened_by(&button).show(ui, |ui, _| {
            Text::new(Label::from(tl!("Warp guide lines").to_string())).show(ui);
            let rgb = guide_colour(guides, ui);
            let well = Field::new(Label::from(tl!("Color").to_string()))
                .show(ui, |ui, _| Swatch::new(Label::from(tl!("Color").to_string()), Srgba::from_rgb_array(rgb)).show(ui).clicked);
            if well.inner == Some(true) {
                *ask_colour = Some(rgb);
            }
            let opacity = number(Label::from(tl!("Opacity").to_string()), &mut guides.opacity, 0.0..=100.0).whole().popup_slider().show(ui);
            *alert = alert.take().or(opacity.out_of_range);
            // Density is an Option: the field shows the grid's own count until the person picks a
            // number, and the framework writes a typed commit to the field's binding without naming
            // it in the pass's events, so the copy of the binding tells the slot when to remember one.
            let own_lines = own as f32;
            let base = guides.density.unwrap_or(own_lines);
            let mut density = base;
            let lines = number(Label::from(tl!("Density").to_string()), &mut density, 1.0..=10.0).whole().popup_slider().show(ui);
            if density != base {
                guides.density = Some(density);
            }
            *alert = alert.take().or(lines.out_of_range);
            let picked = RadioGroup::new(tl!("Show guides"), Bind::given(guides.show))
                .show(ui, |ui, _| {
                    for (show, name) in [
                        (WarpGuideShow::Auto, tl!("Auto Show Guides")),
                        (WarpGuideShow::Always, tl!("Always Show Guides")),
                        (WarpGuideShow::Never, tl!("Never Show Guides")),
                    ] {
                        Radio::new(show).label(name.to_string()).show(ui);
                    }
                })
                .output
                .value()
                .cloned();
            if let Some(picked) = picked {
                guides.show = picked;
            }
        });
    });
}

/// A key of what the bar shows of the open session: the box, its skew, the phase, the interpolation, the reference
/// point, the Warp group's values and the armed split. A drag on the canvas changes them without a command, so the
/// options bar's content version holds this key.
pub(crate) fn bar_key(app: &PhotocraftApp) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let t = app.ui.transform.as_ref()?;
    let s = &t.session;
    let state = s.state();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for p in state.quad.iter().chain([&state.pivot]) {
        (p.x.to_bits(), p.y.to_bits()).hash(&mut hasher);
    }
    (s.skew_v().to_bits(), s.phase(), s.interpolation(), s.reference_point(), s.allows()).hash(&mut hasher);
    if let Some(w) = s.warp() {
        let v = warp_values(w);
        (v.style, v.vertical, v.bend.to_bits(), v.horizontal.to_bits(), v.vertical_distortion.to_bits(), v.grid, w.on_type).hash(&mut hasher);
    }
    app.transform_preview.as_ref().and_then(|pv| pv.split).hash(&mut hasher);
    Some(hasher.finish())
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
