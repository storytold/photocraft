//! View › New Guide Layout…: the framework's hosted form over the document's guides. Its preview is a pure guide
//! overlay, never an edit; OK runs `view.newGuideLayout`.
//!
//! The remembered layout (`app.ui.view.guide_layout`) keeps lengths as the pixels they applied, so a bare "5" typed
//! in px still means 5 px after the ruler unit changes; percent stays percent, of whatever the next target is.

use craft::behavior::Axis;
use craft::theme::Rgba;
use craft::ui::canvas::{Guide, GuideColor, GuideKey};
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::patterns::form_dialogs::{
    GuideGroup, GuideLayoutDialog, GuideLayoutOptions, GuideLayoutResult, GuideLayoutValues, GuideLength, GuideRect, GuideTarget, GuideUnits, HostRequests,
    HostedGuideLayoutDialog,
};
use craft::ui::runtime::env::UnitKind;
use egui::Color32;
use photocraft_doc::{Document, Guides};
use photocraft_engine::prefs::Unit;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

const COMMAND: &str = "view.newGuideLayout";

/// The most columns or rows a remembered layout may hold.
const MAX_COUNT: u64 = 1000;

/// A `"#rrggbb"` colour as drawn.
fn rgb(hex: &str) -> Option<Color32> {
    photocraft_engine::prefs::parse_hex(hex).map(|[r, g, b]| Color32::from_rgb(r, g, b))
}

fn ruler_unit(app: &PhotocraftApp) -> Unit {
    app.session.prefs().units_and_rulers.rulers
}

/// What the guide dialogs measure in: the active document's size and resolution and the ruler unit. With no document
/// it is a 1 px page at 72 ppi, so a dialog still opens.
pub(crate) fn guide_units(app: &PhotocraftApp) -> GuideUnits {
    let prefs = &app.session.prefs().units_and_rulers;
    let (width, height, dpi) = app
        .session
        .active()
        .map_or((1.0, 1.0, 72.0), |st| (f64::from(st.doc.size.width), f64::from(st.doc.size.height), f64::from(st.doc.resolution_dpi.max(1.0))));
    GuideUnits::new(crate::prefs::unit_kind(ruler_unit(app)), width, height, dpi).points_per_inch(prefs.point_size.per_inch())
}

/// The guide colour preference as a framework colour (Cyan when it does not parse).
pub(crate) fn preference_rgba(app: &PhotocraftApp) -> Rgba {
    rgb(&app.session.prefs().guides_grid_and_slices.guide_color).map_or(GuideColor::Cyan.rgba(), |c| Rgba::rgb(c.r(), c.g(), c.b()))
}

/// The document's guides as the framework's, keyed by the canvas's guide key rule.
pub(crate) fn document_guides(guides: &Guides) -> Vec<Guide> {
    let mut out = Vec::new();
    for (vertical, list) in [(false, &guides.horizontal), (true, &guides.vertical)] {
        for (index, &position) in list.iter().enumerate() {
            let mut guide = Guide::new(crate::guide_select::key(vertical, index), if vertical { Axis::Y } else { Axis::X }, position);
            guide.color = guides.color(vertical, position).map(|[r, g, b]| Rgba::rgb(r, g, b));
            out.push(guide);
        }
    }
    out
}

/// The key after every existing guide's, for the first guide a layout makes.
fn next_key(existing: &[Guide]) -> u64 {
    existing.iter().map(|guide: &Guide| GuideKey::get(guide.key) + 1).max().unwrap_or(0)
}

/// `px` pixels as a length in the ruler unit, so a reopened dialog shows it in the unit now chosen.
fn shown(px: f64, units: &GuideUnits, extent: f64) -> GuideLength {
    units.from_pixels(px, units.ruler, extent).map_or(GuideLength::pixels(px), |value| GuideLength::new(value, units.ruler))
}

/// A remembered length: a number is pixels, "n%" is percent.
fn length_of(value: &Value, units: &GuideUnits, extent: f64) -> Option<GuideLength> {
    match value {
        Value::Number(n) => n.as_f64().filter(|px| px.is_finite() && *px >= 0.0).map(|px| shown(px, units, extent)),
        Value::String(s) => {
            let text = s.trim();
            match text.strip_suffix('%') {
                Some(percent) => percent.trim().parse().ok().map(GuideLength::percent),
                None => text.parse::<f64>().ok().filter(|px| px.is_finite() && *px >= 0.0).map(|px| shown(px, units, extent)),
            }
        }
        _ => None,
    }
}

/// A remembered length as the engine takes it: percent stays percent, anything else is the pixels it applied.
fn length_json(length: GuideLength, units: &GuideUnits, extent: f64) -> Value {
    if length.unit == UnitKind::Percent { json!(format!("{}%", length.value)) } else { units.to_pixels(length, extent).map_or(Value::Null, |px| json!(px)) }
}

fn read_group(group: &mut GuideGroup, m: &Map<String, Value>, [count, flag, size, gap]: [&str; 4], extent: f64, units: &GuideUnits) {
    let number = m.get(count).and_then(Value::as_u64);
    if let Some(n) = number {
        if n > 0 {
            group.number = n.min(MAX_COUNT) as u32;
        }
        group.enabled = n > 0;
    }
    if let Some(on) = m.get(flag).and_then(Value::as_bool) {
        group.enabled = on;
    }
    if let Some(v) = m.get(size) {
        group.size = length_of(v, units, extent).filter(|l| l.value > 0.0);
    }
    if let Some(length) = m.get(gap).and_then(|v| length_of(v, units, extent)) {
        group.gutter = length;
    }
}

/// The remembered layout as options; whatever it lacks keeps the factory layout's value.
fn options_of(remembered: &Value, units: &GuideUnits) -> GuideLayoutOptions {
    let mut options = GuideLayoutOptions::default();
    let Some(m) = remembered.as_object() else { return options };
    let (w, h) = (units.width_px, units.height_px);
    read_group(&mut options.columns, m, ["columns", "columnsEnabled", "width", "gutter"], w, units);
    read_group(&mut options.rows, m, ["rows", "rowsEnabled", "height", "rowGutter"], h, units);
    // Top, left, bottom, right: left and right measure along x.
    let extents = [h, w, h, w];
    match m.get("margin") {
        Some(Value::Array(a)) if a.len() == 4 => {
            for ((slot, v), extent) in options.margin.iter_mut().zip(a).zip(extents) {
                if let Some(length) = length_of(v, units, extent) {
                    *slot = length;
                }
            }
        }
        Some(v) => {
            for (slot, extent) in options.margin.iter_mut().zip(extents) {
                if let Some(length) = length_of(v, units, extent) {
                    *slot = length;
                }
            }
        }
        None => {}
    }
    if let Some(on) = m.get("marginEnabled").and_then(Value::as_bool) {
        options.margin_enabled = on;
    }
    if let Some(on) = m.get("centerColumns").and_then(Value::as_bool) {
        options.center_columns = on;
    }
    if let Some(on) = m.get("clearExisting").and_then(Value::as_bool) {
        options.clear_existing = on;
    }
    if m.get("target").and_then(Value::as_str) == Some("selectedArtboards") {
        options.target = GuideTarget::SelectedArtboards;
    }
    options
}

/// The layout as the app remembers it: lengths as applied pixels, percent kept, the switches beside their counts.
fn remembered_json(o: &GuideLayoutOptions, units: &GuideUnits) -> Value {
    let (w, h) = (units.width_px, units.height_px);
    let size = |length: Option<GuideLength>, extent: f64| length.map_or(Value::Null, |l| length_json(l, units, extent));
    json!({
        "columns": o.columns.number,
        "columnsEnabled": o.columns.enabled,
        "rows": o.rows.number,
        "rowsEnabled": o.rows.enabled,
        "width": size(o.columns.size, w),
        "height": size(o.rows.size, h),
        "gutter": length_json(o.columns.gutter, units, w),
        "rowGutter": length_json(o.rows.gutter, units, h),
        "margin": [
            length_json(o.margin[0], units, h),
            length_json(o.margin[1], units, w),
            length_json(o.margin[2], units, h),
            length_json(o.margin[3], units, w),
        ],
        "marginEnabled": o.margin_enabled,
        "centerColumns": o.center_columns,
        "clearExisting": o.clear_existing,
        "target": if o.target == GuideTarget::SelectedArtboards { "selectedArtboards" } else { "document" },
    })
}

/// The parameters of `view.newGuideLayout`: a disabled axis or margin contributes nothing.
fn params(o: &GuideLayoutOptions, units: &GuideUnits) -> Value {
    let mut p = remembered_json(o, units);
    if let Some(m) = p.as_object_mut() {
        for (enabled, count, size, gap) in [(o.columns.enabled, "columns", "width", "gutter"), (o.rows.enabled, "rows", "height", "rowGutter")] {
            if !enabled {
                m.insert(count.into(), json!(0));
                m.insert(size.into(), Value::Null);
                m.insert(gap.into(), json!(0));
            }
        }
        if !o.margin_enabled {
            m.insert("margin".into(), json!(0));
        }
        for flag in ["columnsEnabled", "rowsEnabled", "marginEnabled"] {
            m.remove(flag);
        }
    }
    p
}

/// Opens New Guide Layout on the active document; `None` without one.
pub fn open(app: &mut PhotocraftApp) -> Option<DialogId> {
    let st = app.session.active()?;
    let doc_id = st.doc.id;
    let existing = document_guides(&st.doc.guides);
    let first_key = next_key(&existing);
    let artboards: Vec<GuideRect> = photocraft_engine::guide_layout::selected_artboards(&app.session)
        .iter()
        .map(|r| GuideRect::new(f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)))
        .collect();
    let units = guide_units(app);
    let document = GuideRect::new(0.0, 0.0, units.width_px, units.height_px);
    let mut options = options_of(&app.ui.view.guide_layout, &units);
    // A remembered Selected Artboards target needs a selected artboard.
    if options.target == GuideTarget::SelectedArtboards && artboards.is_empty() {
        options.target = GuideTarget::Document;
    }
    let color = GuideColor::from_rgba(preference_rgba(app));
    let dialog = HostedGuideLayoutDialog::new(GuideLayoutValues::new(options, color), move |bind| {
        GuideLayoutDialog::new(units, document, bind).artboards(artboards.clone()).existing(existing.clone()).first_key(first_key)
    });
    let id = app.framework.dialogs.open(dialog);
    app.guide_layout_doc = Some((id, doc_id));
    Some(id)
}

fn layout_requests(app: &mut PhotocraftApp, id: DialogId) -> Option<&mut HostRequests> {
    app.framework.dialogs.get_mut_by_id::<HostedGuideLayoutDialog>(id).map(HostedGuideLayoutDialog::requests)
}

/// Opens the colour picker for a hosted dialog's Custom… request and hands the picked colour back with the request's
/// key. `requests` finds the dialog's request queue.
pub(crate) fn pump_colors(app: &mut PhotocraftApp, id: DialogId, requests: for<'a> fn(&'a mut PhotocraftApp, DialogId) -> Option<&'a mut HostRequests>) {
    if let Some(request) = requests(app, id).and_then(HostRequests::take_color) {
        crate::color_picker_ui::open_for_request(app, id, request);
    }

    if let Some((key, color)) = crate::color_picker_ui::answer_for(app, id)
        && let Some(queue) = requests(app, id)
    {
        queue.answer_color(key, color);
    }
}

/// The dialog's colour requests and its answer, once per pass.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some((id, doc)) = app.guide_layout_doc else { return };
    pump_colors(app, id, layout_requests);
    let Some(result) = app.framework.dialogs.take_by_id::<HostedGuideLayoutDialog>(id) else { return };
    app.guide_layout_doc = None;
    if let DialogResult::Ok(layout) = result
        && let Err(error) = apply(app, doc, &layout)
    {
        app.ui.status = error;
    }
}

/// OK: the layout on the document the dialog opened on, the guides revealed, the colour preference and the layout
/// remembered.
fn apply(app: &mut PhotocraftApp, doc: photocraft_doc::DocId, result: &GuideLayoutResult) -> Result<(), String> {
    if app.session.active().map(|st| st.doc.id) != Some(doc) {
        return Err("the guide layout belongs to another document".into());
    }
    let units = guide_units(app);
    app.run(COMMAND, params(&result.options, &units))?;
    // The preview shows even when guides were hidden. Keep a committed layout visible too.
    app.ui.view.extras = true;
    app.ui.view.show.canvas_guides = true;
    app.ui.extras.guides = true;
    // Colour is the existing global guide display preference, not document data.
    let [r, g, b, _] = result.color.0;
    let hex = format!("#{r:02x}{g:02x}{b:02x}");
    app.session.prefs.edit(|prefs| prefs.guides_grid_and_slices.guide_color = hex);
    app.ui.view.guide_layout = remembered_json(&result.options, &units);
    Ok(())
}

/// Full replacement guide set for the dialog's own document. Cancel and removal drop it at once.
pub fn preview(app: &PhotocraftApp, doc: &Document) -> Option<(Guides, Color32)> {
    let (id, opened_on) = app.guide_layout_doc?;
    if opened_on != doc.id {
        return None;
    }
    let dialog = app.framework.dialogs.get_by_id::<HostedGuideLayoutDialog>(id)?;
    // The colour is the dialog's own, drawn for the whole set; OK commits guides without colours of their own.
    let mut guides = Guides::default();
    for line in dialog.preview()? {
        let vertical = line.along == Axis::Y;
        if vertical { &mut guides.vertical } else { &mut guides.horizontal }.push(line.position);
    }
    let [r, g, b, _] = dialog.values().color.rgba().0;
    Some((guides, Color32::from_rgb(r, g, b)))
}

#[cfg(test)]
mod tests;
