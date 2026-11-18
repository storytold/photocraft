//! Properties panel › Document: what Photoshop shows when nothing (or the Background layer) is
//! selected. Canvas size and orientation, resolution, colour mode and bit depth, and the
//! Rulers & Grids and Guides controls. Every edit dispatches an engine or view command, so the panel stays thin.

use craft::behavior::Mixed;
use craft::keymap::CommandRef;
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::contract::{Bind, Entry, Props as _, Show, ShowContainer};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use craft::ui::sections::Section;
use craft::ui::text::{LinkedFields, NumberField};
use photocraft_doc::{ColorMode, Document, Layer, LayerId, SampleType};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The most a canvas side can be, in pixels.
const MAX_SIDE: f64 = 300_000.0;

/// Photoshop's Background layer: the locked, opaque raster layer at the bottom of the stack,
/// whatever its (localized) name ([`Document::background`]).
pub fn is_background(doc: &Document, l: &Layer) -> bool {
    doc.is_background(l.id)
}

/// Does the Properties panel show the Document view? (No layer selected, or the Background layer.)
pub fn shows_document(doc: &Document, active: Option<LayerId>) -> bool {
    match active.and_then(|id| doc.layer(id)) {
        None => true,
        Some(l) => is_background(doc, l),
    }
}

/// Image › Mode command id and label per colour mode, in Photoshop's dropdown order.
pub const MODES: &[(ColorMode, &str, &str)] = &[
    (ColorMode::Bitmap, "Bitmap", "image.mode.bitmap"),
    (ColorMode::Grayscale, "Grayscale", "image.mode.grayscale"),
    (ColorMode::Duotone, "Duotone", "image.mode.duotone"),
    (ColorMode::Indexed, "Indexed Color", "image.mode.indexedColor"),
    (ColorMode::Rgb, "RGB Color", "image.mode.rgb"),
    (ColorMode::Cmyk, "CMYK Color", "image.mode.cmyk"),
    (ColorMode::Lab, "Lab Color", "image.mode.lab"),
    (ColorMode::Multichannel, "Multichannel", "image.mode.multichannel"),
];

/// Image › Mode › bits per channel: sample type, label and command id.
pub const DEPTHS: &[(SampleType, &str, &str)] = &[
    (SampleType::U8, "8 Bits/Channel", "image.mode.bits8"),
    (SampleType::U16, "16 Bits/Channel", "image.mode.bits16"),
    (SampleType::F32, "32 Bits/Channel", "image.mode.bits32"),
];

/// Ruler units offered in the Rulers & Grids dropdown (`unitsAndRulers.rulers` values).
pub const UNITS: &[(&str, &str)] = &[
    ("pixels", "Pixels"),
    ("inches", "Inches"),
    ("cm", "Centimeters"),
    ("mm", "Millimeters"),
    ("points", "Points"),
    ("picas", "Picas"),
    ("percent", "Percent"),
];

/// The unit kind of a `unitsAndRulers.rulers` value.
fn ruler_kind(name: &str) -> UnitKind {
    match name {
        "inches" => UnitKind::Inches,
        "cm" => UnitKind::Centimetres,
        "mm" => UnitKind::Millimetres,
        "points" => UnitKind::Points,
        "picas" => UnitKind::Picas,
        "percent" => UnitKind::Percent,
        _ => UnitKind::Pixels,
    }
}

fn canvas_size(width: f64, height: f64) -> (String, Value) {
    ("image.canvasSize".to_string(), json!({"width": width, "height": height, "anchor": "center"}))
}

/// What the Canvas section shows of the document.
struct Facts {
    width: u32,
    height: u32,
    mode: ColorMode,
    depth: SampleType,
    ppi: f64,
    dpi_text: String,
    has_background: bool,
}

/// The Canvas section's body: size in the ruler unit, orientation, resolution, mode, depth and fill.
fn canvas_section(ui: &mut egui::Ui, run: &mut Vec<(String, Value)>, f: &Facts, kind: UnitKind) {
    let extent = [f64::from(f.width), f64::from(f.height)];
    // The fields keep pixels and show the ruler unit, so showing commits nothing (#5661).
    let lengths = LengthUnits::new(UnitKind::Pixels, kind, f.ppi);
    let shown = extent.map(|px| Mixed::Same(Some(px)));
    let out =
        LinkedFields::new(tl!("Canvas Size"), Bind::given(shown), Bind::kept()).captions([tl!("W"), tl!("H")]).lengths(lengths).range(1.0..=MAX_SIDE).show(ui);
    if let Some(parts) = out.edit.events.committed() {
        let pixels = |i: usize| match &parts[i] {
            Mixed::Same(Some(v)) => Some(v.round().clamp(1.0, MAX_SIDE)),
            _ => None,
        };
        if let (Some(w), Some(h)) = (pixels(0), pixels(1))
            && (w, h) != (extent[0], extent[1])
        {
            run.push(canvas_size(w, h));
        }
    }
    // Photoshop greys Canvas X/Y for documents without artboards.
    for label in [tl!("X"), tl!("Y")] {
        NumberField::length(label, Bind::given(Some(0.0)), lengths).read_only(true).show(ui);
    }
    let portrait = f.height > f.width;
    let square = f.width == f.height;
    ui.horizontal(|ui| {
        let to_portrait = ToggleButton::new(tl!("Portrait"), Bind::given(portrait)).show(ui);
        if to_portrait.events.committed() == Some(&true) && !portrait && !square {
            run.push(canvas_size(extent[1], extent[0]));
        }
        let to_landscape = ToggleButton::new(tl!("Landscape"), Bind::given(!portrait)).show(ui);
        if to_landscape.events.committed() == Some(&true) && portrait {
            run.push(canvas_size(extent[1], extent[0]));
        }
    });
    let resolution = crate::i18n::fmt(tl!("{dpi} pixels/inch"), &[("dpi", &f.dpi_text)]);
    Field::new(tl!("Resolution")).value(resolution.clone()).show(ui, |ui, _| {
        Text::new(resolution.clone()).show(ui);
    });
    let modes: Vec<Entry<ColorMode>> = MODES.iter().map(|(m, label, _)| Entry::new(*m, *label)).collect();
    let picked = Select::new(tl!("Color Mode"), Bind::given(f.mode), &modes).show(ui);
    if let Some(m) = picked.events.committed().cloned()
        && let Some((_, _, id)) = MODES.iter().find(|(x, _, _)| *x == m)
    {
        run.push(((*id).to_string(), Value::Null));
    }
    let depths: Vec<Entry<SampleType>> = DEPTHS.iter().map(|(d, label, _)| Entry::new(*d, *label)).collect();
    let picked = Select::new(tl!("Depth"), Bind::given(f.depth), &depths).show(ui);
    if let Some(d) = picked.events.committed().cloned()
        && let Some((_, _, id)) = DEPTHS.iter().find(|(x, _, _)| *x == d)
    {
        run.push(((*id).to_string(), Value::Null));
    }
    // The canvas fill is only editable for documents without a Background layer.
    let fills = [Entry::new(0u8, tl!("Background Color"))];
    Select::new(tl!("Fill"), Bind::given(0u8), &fills).enabled(!f.has_background).show(ui);
}

/// The Properties panel for the document itself (no layer selected): the Canvas section (size,
/// orientation, resolution, mode, depth, fill), then Rulers & Grids and Guides. Edits run as engine commands.
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(st) = app.session.active() else { return };
    let facts = Facts {
        width: st.doc.size.width,
        height: st.doc.size.height,
        mode: st.doc.mode,
        depth: st.doc.depth,
        ppi: f64::from(st.doc.resolution_dpi),
        dpi_text: st.doc.resolution_dpi.to_string(),
        has_background: st.doc.layers.first().is_some_and(|l| is_background(&st.doc, l)),
    };
    let ruler = app.session.prefs().units_and_rulers.rulers.name().to_string();
    let kind = ruler_kind(&ruler);
    let mut run: Vec<(String, Value)> = Vec::new();
    Section::new(tl!("Canvas")).show(ui, |ui, _| canvas_section(ui, &mut run, &facts, kind));
    Section::new(tl!("Rulers & Grids")).show(ui, |ui, _| {
        ui.horizontal(|ui| {
            for id in ["view.rulers", "view.show.grid", "view.show.pixelGrid"] {
                ToggleButton::command(CommandRef::new(id)).show(ui);
            }
        });
        let units: Vec<Entry<String>> = UNITS.iter().map(|(k, label)| Entry::new((*k).to_string(), *label)).collect();
        let picked = Select::new(tl!("Units"), Bind::given(ruler.clone()), &units).show(ui);
        if let Some(unit) = picked.events.committed().cloned() {
            run.push(("prefs.set".into(), json!({"path": "unitsAndRulers.rulers", "value": unit})));
        }
    });
    Section::new(tl!("Guides")).show(ui, |ui, _| {
        ui.horizontal(|ui| {
            for id in ["view.show.guides", "view.lockGuides"] {
                ToggleButton::command(CommandRef::new(id)).show(ui);
            }
            for id in ["view.newGuide", "view.clearGuides"] {
                Button::command(CommandRef::new(id)).show(ui);
            }
        });
    });
    let ctx = ui.ctx().clone();
    for (id, params) in run {
        let params = if params.is_null() { json!({}) } else { params };
        if let Err(e) = crate::menus::invoke(app, &ctx, &id, params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    /// #5661: a 1 x 1 px document at 300 ppi is 0.0033 in wide. With the rulers in inches the
    /// Width/Height fields clamped that to 0.01 in (3 px) and resized the document unasked.
    #[test]
    fn tiny_document_in_inches_is_not_resized_by_showing_it() {
        use egui_kittest::Harness;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 1, "height": 1, "resolution": 300, "background": "white"})).unwrap();
        app.session.prefs.edit(|p| p.units_and_rulers.rulers = photocraft_engine::prefs::Unit::Inches);
        let undo = app.session.active().unwrap().history.entries().len();
        let mut h = Harness::builder()
            .with_context(app.ctx().clone())
            .with_size(vec2(320.0, 600.0))
            .build_ui_state(|ui, app: &mut PhotocraftApp| properties(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
        h.run_steps(3);
        let st = h.state().session.active().unwrap();
        assert_eq!((st.doc.size.width, st.doc.size.height), (1, 1));
        assert_eq!(st.history.entries().len(), undo, "no history step");
    }

    #[test]
    fn document_dropdowns_map_to_live_mode_commands() {
        for id in MODES.iter().map(|m| m.2).chain(DEPTHS.iter().map(|d| d.2)) {
            assert!(photocraft_engine::commands::find(id).is_some(), "{id} is not an engine command");
        }
    }
}
