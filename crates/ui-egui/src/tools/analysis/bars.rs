//! Options bars of the Ruler, Count, Note and Color Sampler tools.

use craft::behavior::IconId;
use craft::behavior::kurbo::Point;
use craft::color_ui::ui::SamplerOptions;
use craft::draw::ui::measure::{RulerBar, RulerLine};
use craft::theme::Srgba;
use craft::ui::buttons::{Button, IconButton, ToggleButton};
use craft::ui::choice::Swatch;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Text};
use craft::ui::form::Field;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use photocraft_doc::Ruler;
use serde_json::{Value, json};

use super::{count_readout, note_alerts};
use crate::PhotocraftApp;

use crate::panels::panel_docking;
use crate::state::Tool;
use crate::tools::option_cells;

/// The Count bar's Marker Size range.
const MARKER_SIZE: std::ops::RangeInclusive<f64> = 1.0..=10.0;
/// The Count bar's Label Size range.
const LABEL_SIZE: std::ops::RangeInclusive<f64> = 8.0..=72.0;

/// What the Count bar's group menu picks: a group, or the Rename… row after the groups.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupPick {
    Group(usize),
    Rename,
}

/// `count.setGroup` params for the Marker Size and Label Size fields, sent as JSON integers (the fields hold
/// floats; `8.0` is a float on the wire). NaN gives the range's low end.
pub(super) fn count_size_params(marker: f32, label: f32) -> Value {
    let whole = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.round().clamp(lo, hi) as u32 } else { lo as u32 };
    json!({"markerSize": whole(marker, 1.0, 10.0), "labelSize": whole(label, 8.0, 72.0)})
}

/// Draws the options bar of `tool` when it is one of the analysis tools; false for any other tool. With no document
/// the bar is empty, so the host does not fall through to another tool's bar.
pub(crate) fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if !matches!(tool, Tool::Ruler | Tool::Count | Tool::Note | Tool::ColorSampler) {
        return false;
    }
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return true };
    match tool {
        Tool::Ruler => ruler(app, ui, &doc),
        Tool::Count => count(app, ui, &doc),
        Tool::Note => note(app, ui, &doc),
        _ => sampler(app, ui),
    }
    true
}

/// Puts a refusal on the status line.
fn report(app: &mut PhotocraftApp, error: String) {
    app.ui.status = error;
    app.ui.status_error = true;
}

/// Runs `command`; a refusal other than a cancelled dialog goes to the status line.
fn run(app: &mut PhotocraftApp, command: &str, params: Value) {
    if let Err(error) = app.run(command, params)
        && error != "cancelled"
    {
        report(app, error);
    }
}

fn ruler_line(r: &Ruler) -> RulerLine {
    let line = RulerLine::new(Point::new(r.start[0], r.start[1]), Point::new(r.end[0], r.end[1]));
    match r.protractor {
        Some(p) => line.with_protractor(Point::new(p[0], p[1])),
        None => line,
    }
}

fn ruler(app: &mut PhotocraftApp, ui: &mut egui::Ui, doc: &photocraft_doc::Document) {
    let factor = doc.measurement.scale.factor();
    let can_straighten = app.session.is_enabled("image.analysis.straightenLayer");
    let line = doc.measurement.ruler.as_ref().map(ruler_line);
    // PLAN: assumed `RulerBar::show(ui)` returns an output with `straighten_layer: bool` and `clear: bool`.
    let out = RulerBar::new(line, Bind::bound(&mut app.ui.analysis.use_measurement_scale)).factor(factor).can_straighten(can_straighten).show(ui);
    if out.straighten_layer
        && let Err(why) = app.run("image.analysis.straightenLayer", json!({}))
    {
        // A hidden target layer answers with the reference's alert. Every refusal also reaches the status line: one
        // raised before the command runs (the automation gate) would otherwise leave a dead button (#12148).
        if why == photocraft_engine::analysis_cmds::STRAIGHTEN_HIDDEN_ERROR {
            let hidden = photocraft_engine::analysis_cmds::STRAIGHTEN_HIDDEN_ERROR;
            // PLAN: assumed `dialogs::alerts::alert(app: &mut PhotocraftApp, message: &str)`.
            crate::dialogs::alerts::alert(app, tl!(hidden));
        }
        report(app, why);
    }
    if out.clear {
        run(app, "image.analysis.rulerTool", json!({"clear": true}));
    }
}

fn count(app: &mut PhotocraftApp, ui: &mut egui::Ui, doc: &photocraft_doc::Document) {
    let m = &doc.measurement;
    Text::new(Label::from(crate::i18n::fmt(tl!("Count: {n}"), &[("n", &count_readout(m))]))).show(ui);
    Divider::vertical().show(ui);
    // Photoshop's order (#11765): group, eye, folder, trash, Clear, colour, Marker Size, Label Size. With no group the
    // sizes show what the next new group gets.
    let group = (!m.count_groups.is_empty()).then(|| m.active_count_group.min(m.count_groups.len() - 1));
    let next = photocraft_doc::CountGroup::default();
    let g = group.map_or(&next, |gi| &m.count_groups[gi]);
    if let Some(gi) = group {
        let mut entries: Vec<Entry<GroupPick>> =
            m.count_groups.iter().enumerate().map(|(i, g)| Entry::new(GroupPick::Group(i), Label::from(g.name.clone()))).collect();
        entries.push(Entry::separator());
        entries.push(Entry::new(GroupPick::Rename, Label::from(tl!("Rename…"))));
        match Select::new(Label::from(tl!("Group")), Bind::given(GroupPick::Group(gi)), &entries).show(ui).value() {
            Some(GroupPick::Group(picked)) => run(app, "count.setGroup", json!({"group": *picked, "active": true})),
            Some(GroupPick::Rename) => {
                crate::count_group_name_ui::open_rename(app, gi);
            }
            None => {}
        }
    } else {
        let entries = [Entry::new(GroupPick::Group(0), Label::from(tl!("Count Group")))];
        Select::new(Label::from(tl!("Group")), Bind::given(GroupPick::Group(0)), &entries).enabled(false).show(ui);
    }
    let eye = IconId::new(if g.visible { "eye" } else { "eye_off" });
    if IconButton::new(eye).label(Label::from(tl!("Toggle visibility of count group"))).show(ui).clicked()
        && let Some(gi) = group
    {
        run(app, "count.setGroup", json!({"group": gi, "visible": !g.visible}));
    }
    if IconButton::new(IconId::new("new_folder")).label(Label::from(tl!("Create a new count group"))).show(ui).clicked() {
        crate::count_group_name_ui::open_new(app);
    }
    if IconButton::new(IconId::new("trash"))
        .label(Label::from(tl!("Delete the currently selected count group")))
        .enabled(m.count_groups.len() >= 2)
        .show(ui)
        .clicked()
        && let Some(gi) = group
    {
        run(app, "count.deleteGroup", json!({"group": gi}));
    }
    // Clear acts on a visible current group with markers.
    let clearable = group.is_some() && g.visible && !g.points.is_empty();
    if Button::new(Label::from(tl!("Clear"))).enabled(clearable).show(ui).clicked() {
        run(app, "count.clear", json!({}));
    }
    let rgb = g.color.to_rgb();
    if Swatch::new(Label::from(tl!("Count group color")), Srgba::from_rgb_array(rgb)).show(ui).clicked
        && let Some(gi) = group
    {
        crate::color_picker_ui::open_for_command(app, &crate::color_picker_ui::title_for(tl!("Count Color")), rgb, "count.setGroup", json!({"group": gi}));
    }
    Divider::vertical().show(ui);
    let (marker, label) = (g.marker_size as f32, g.label_size as f32);
    let (mut new_marker, mut new_label) = (marker, label);
    let marker_out = option_cells::number(tl!("Marker Size"), &mut new_marker, MARKER_SIZE).whole().show(ui);
    let label_out = option_cells::number(tl!("Label Size"), &mut new_label, LABEL_SIZE).whole().show(ui);
    // A typed size out of range takes the closest value and says so once (#13530).
    for alert in [&marker_out.out_of_range, &label_out.out_of_range].into_iter().flatten() {
        option_cells::range_alert(app, ui, alert);
    }
    let Some(gi) = group else { return };
    if new_marker != marker || new_label != label {
        let mut params = count_size_params(new_marker, new_label);
        params["group"] = json!(gi);
        // A scrub's edits share one key, so the drag is one history step.
        let events = marker_out.events.iter().chain(label_out.events.iter());
        if let Some(key) = events.into_iter().find_map(|e| crate::history_keys::coalesce("count-size", e)) {
            params["coalesce"] = json!(key);
        }
        run(app, "count.setGroup", params);
    }
}

fn note(app: &mut PhotocraftApp, ui: &mut egui::Ui, doc: &photocraft_doc::Document) {
    // In `Field`s: a bare control's label only names it, it is not drawn (#16338).
    Field::new(Label::from(tl!("Author"))).show(ui, |ui, _| TextField::new(Label::from(tl!("Author")), Bind::bound(&mut app.ui.analysis.note_author)).show(ui));
    let rgb = app.ui.analysis.note_color;
    let swatch = Field::new(Label::from(tl!("Color"))).show(ui, |ui, _| Swatch::new(Label::from(tl!("Color")), Srgba::from_rgb_array(rgb)).show(ui).clicked);
    if swatch.inner == Some(true)
        && let Err(message) = crate::color_picker_ui::open_for_target(app, "noteColor", &crate::color_picker_ui::title_for(tl!("Color")), rgb)
    {
        crate::notices::error(app, message);
    }
    // Clear All asks first (#11791); `notes.delete {"all": true}` itself never does.
    if Button::new(Label::from(tl!("Clear All"))).enabled(!doc.notes.is_empty()).show(ui).clicked() {
        note_alerts::ask_clear_notes(app);
    }
    let shown = panel_docking::is_shown(app, "notes");
    let toggle = ToggleButton::new(Label::from(tl!("Show or hide the Notes panel")), Bind::given(shown)).icon(IconId::new("note")).icon_only().show(ui);
    if toggle.response.clicked()
        && let Err(error) = panel_docking::show_panel(app, "notes", None)
    {
        report(app, error);
    }
}

fn sampler(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let n = app.session.active().map_or(0, |d| d.color_samplers.len());
    // One Sample Size setting, shared with the Eyedropper (#11732).
    // PLAN: assumed `SamplerOptions::show(ui)` returns an output with `clear_all: bool`.
    let out = SamplerOptions::new(n, Bind::bound(&mut app.ui.tool_options.eyedropper.size)).show(ui);
    if out.clear_all {
        run(app, "view.colorSamplers.clear", json!({}));
    }
}
