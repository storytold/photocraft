//! The options-bar members of the selection tools: the marquees, the lassos and the Magic Wand.

use std::ops::RangeInclusive;

use craft::behavior::PixelSize;
use craft::behavior::geometry::{Constraint, SelectionOperation};
use craft::imaging::ui::select::{SelectionOpWords, selection_operations};
use craft::ui::bars::ConstraintFields;
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing, clicked};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::Field;
use craft::ui::runtime::commands::{CommandRef, CommandState};
use craft::ui::runtime::env::{Env, Label, UnitKind};
use craft::ui::text::NumberField;
use photocraft_engine::prefs::Unit;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The ruler units Fixed Size can show, with the framework's kind of each.
const FIXED_SIZE_UNITS: [(Unit, UnitKind); 7] = [
    (Unit::Pixels, UnitKind::Pixels),
    (Unit::Inches, UnitKind::Inches),
    (Unit::Centimeters, UnitKind::Centimetres),
    (Unit::Millimeters, UnitKind::Millimetres),
    (Unit::Points, UnitKind::Points),
    (Unit::Picas, UnitKind::Picas),
    (Unit::Percent, UnitKind::Percent),
];

/// A divider between two groups of members.
fn divider(ui: &mut egui::Ui) {
    Divider::vertical().show(ui);
}

/// The text of `english`, translated, as a label.
fn label(english: &str) -> Label {
    Label::from(tl!(english).to_string())
}

/// The selection operation `selection_mode` (0 New, 1 Add, 2 Subtract, 3 Intersect) stands for.
fn operation_of(selection_mode: u8) -> SelectionOperation {
    SelectionOperation::ALL.get(usize::from(selection_mode)).copied().unwrap_or_default()
}

/// The `selection_mode` of an operation; the inverse of [`operation_of`].
fn selection_mode_of(operation: SelectionOperation) -> u8 {
    SelectionOperation::ALL.iter().position(|o| *o == operation).map_or(0, |at| at as u8)
}

/// The New, Add, Subtract and Intersect segments over `app.ui.selection_mode`: the one binding of the
/// mode for every tool that shows the operations. `enabled` is false while Content-Aware Move's
/// transform box is up.
pub(crate) fn operation_segments(app: &mut PhotocraftApp, ui: &mut egui::Ui, enabled: bool) {
    let words = Env::of(ui).words::<SelectionOpWords>();
    let entries = selection_operations(&words);
    let current = operation_of(app.ui.selection_mode);
    let picked = Segmented::new(label("Selection mode"), Bind::given(current), &entries).enabled(enabled).show(ui).value().copied();
    if let Some(operation) = picked {
        app.ui.selection_mode = selection_mode_of(operation);
    }
}

/// A captioned number cell over `value`; the new value when the user edited it. A body can run more
/// than once in a pass and only the run that reads the input reports, so an edit is kept once seen.
fn number(
    ui: &mut egui::Ui,
    key: &'static str,
    caption: Label,
    value: f64,
    range: RangeInclusive<f64>,
    unit: Option<UnitKind>,
    tip: Option<Label>,
) -> Option<f64> {
    let mut edited = None;
    Field::new(caption.clone()).side(SideLabel::new().scrub(true)).id_salt(key).sizing(Sizing::Content).show(ui, |ui, _| {
        let mut field = NumberField::new(caption.clone(), Bind::given(Some(value))).range(range.clone()).commit_decimals(0).scrub_by_label();
        if let Some(kind) = unit {
            field = field.unit_kind(kind);
        }
        if let Some(tip) = tip.clone() {
            field = field.tooltip(tip);
        }
        if let Some(next) = field.show(ui).value().copied().flatten() {
            edited = Some(next);
        }
    });
    edited
}

/// A checkbox over `value`; the new value when the user flipped it.
fn check(ui: &mut egui::Ui, caption: Label, value: bool, enabled: bool) -> Option<bool> {
    Checkbox::new(Bind::given(value)).label(caption).enabled(enabled).show(ui).value().copied()
}

/// The Feather field, shared by the marquees and the lassos.
fn feather(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if let Some(next) = number(ui, "feather", label("Feather"), f64::from(app.ui.tool_options.feather), 0.0..=1000.0, Some(UnitKind::Pixels), None) {
        app.ui.tool_options.feather = next as f32;
    }
}

/// The Anti-alias checkbox over the shared tool option.
fn anti_alias(app: &mut PhotocraftApp, ui: &mut egui::Ui, enabled: bool) {
    if let Some(next) = check(ui, label("Anti-alias"), app.ui.tool_options.anti_alias, enabled) {
        app.ui.tool_options.anti_alias = next;
    }
}

/// The command the Select and Mask… button runs.
const SELECT_AND_MASK: &str = "select.selectAndMask";

/// The Select and Mask… button's state for the frame input: the command's, as the Select menu reads it.
pub(crate) fn command_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    vec![(CommandRef::new(SELECT_AND_MASK), CommandState::new().enabled(crate::menus::is_enabled(app, SELECT_AND_MASK)))]
}

/// [`command_states`], derived from what the rules read.
pub(crate) fn source_states(app: &PhotocraftApp, name: &'static str) -> std::sync::Arc<crate::app::framework::SourceStates> {
    crate::app::framework::derived_states(app, name, &crate::menus::facts::RuleFacts::of(app), || command_states(app))
}

/// The Select and Mask… button; its enabled state is [`command_states`]'.
fn select_and_mask(ui: &mut egui::Ui) {
    Button::command(SELECT_AND_MASK).show(ui);
}

/// The marquee's Style, Width, swap and Height cells over the stored tool options.
fn constraint_of(app: &PhotocraftApp) -> Constraint {
    let o = &app.ui.tool_options;
    let (w, h) = (f64::from(o.marquee_width), f64::from(o.marquee_height));
    match o.marquee_style.as_str() {
        "fixedRatio" => Constraint::Ratio(w, h),
        "fixedSize" => Constraint::Size(w, h),
        _ => Constraint::Free,
    }
}

/// The Fixed Size cells: they keep exact pixels and show them in the stored unit.
fn constraint_fields(app: &mut PhotocraftApp, ui: &mut egui::Ui, enabled: bool) {
    let mut constraint = constraint_of(app);
    let unit = Unit::parse(&app.ui.tool_options.marquee_unit).unwrap_or_default();
    let kind = FIXED_SIZE_UNITS.iter().find(|(u, _)| *u == unit).map_or(UnitKind::Pixels, |(_, k)| *k);
    let entries: Vec<Entry<UnitKind>> = FIXED_SIZE_UNITS.iter().map(|(u, k)| Entry::new(*k, label(unit_text(*u)))).collect();
    let (dpi, size) = app
        .session
        .active()
        .map_or((72.0, PixelSize::new(1, 1)), |d| (f64::from(d.doc.resolution_dpi.max(1.0)), PixelSize::new(d.doc.size.width, d.doc.size.height)));
    let out = ConstraintFields::new(Bind::bound(&mut constraint)).enabled(enabled).size_units(kind, &entries, dpi, size).show(ui);
    if out.changed {
        let (style, w, h) = match constraint {
            Constraint::Ratio(w, h) => ("fixedRatio", w, h),
            Constraint::Size(w, h) => ("fixedSize", w, h),
            _ => ("normal", 1.0, 1.0),
        };
        let o = &mut app.ui.tool_options;
        o.marquee_style = style.into();
        (o.marquee_width, o.marquee_height) = (w as f32, h as f32);
    }
    if let Some(picked) = out.unit_picked
        && let Some((unit, _)) = FIXED_SIZE_UNITS.iter().find(|(_, k)| *k == picked)
    {
        app.ui.tool_options.marquee_unit = unit.name().into();
    }
}

/// The English name of a ruler unit in the units menu.
fn unit_text(unit: Unit) -> &'static str {
    match unit {
        Unit::Inches => "Inches",
        Unit::Centimeters => "Centimeters",
        Unit::Millimeters => "Millimeters",
        Unit::Points => "Points",
        Unit::Picas => "Picas",
        Unit::Percent => "Percent",
        _ => "Pixels",
    }
}

/// The marquees: operations, Feather, Anti-alias, the constraint cells and Select and Mask.
fn marquee(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) {
    operation_segments(app, ui, true);
    divider(ui);
    feather(app, ui);
    // Photoshop greys Anti-alias for the Rectangular Marquee (its edges are always hard).
    anti_alias(app, ui, tool == Tool::EllipseMarquee);
    divider(ui);
    // A single row or column has no style or size: Photoshop greys them.
    constraint_fields(app, ui, matches!(tool, Tool::RectMarquee | Tool::EllipseMarquee));
    divider(ui);
    select_and_mask(ui);
}

/// A faint hint after the members.
fn hint(ui: &mut egui::Ui, text: String) {
    Text::new(text).shade(Shade::Faint).show(ui);
}

/// The lassos: operations, Feather, Anti-alias, the Magnetic Lasso's settings, Select and Mask and
/// the hint for what is being drawn.
fn lasso(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) {
    operation_segments(app, ui, true);
    divider(ui);
    feather(app, ui);
    anti_alias(app, ui, true);
    divider(ui);
    if tool == Tool::MagneticLasso {
        magnetic_settings(app, ui);
        divider(ui);
    }
    select_and_mask(ui);
    let enter = crate::shortcuts::pretty("Enter");
    // The framework's selection tools own the open outline the hints follow.
    if tool == Tool::MagneticLasso {
        if app.canvas_tools.selection.outline_open() {
            let del = crate::shortcuts::pretty("Backspace");
            hint(
                ui,
                crate::i18n::fmt(tl!("Click the first point or press {key} to close · {del} removes a point · Esc cancels"), &[("key", &enter), ("del", &del)]),
            );
        }
    } else if app.canvas_tools.selection.outline_open() {
        hint(ui, crate::i18n::fmt(tl!("Click the first point or press {key} to close · Esc cancels"), &[("key", &enter)]));
    } else if tool == Tool::Lasso {
        hint(ui, tl!("Hold Alt while drawing for straight segments").to_string());
    }
}

/// The Magnetic Lasso's Width, Contrast, Frequency and pen-pressure toggle.
fn magnetic_settings(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let o = &app.ui.tool_options;
    let (width, contrast, frequency, pressure) =
        (f64::from(o.magnetic.width), f64::from(o.magnetic.contrast), f64::from(o.magnetic.frequency), o.magnetic.pressure_width);
    let width = number(
        ui,
        "magnetic-width",
        label("Width"),
        width,
        1.0..=256.0,
        Some(UnitKind::Pixels),
        Some(label("Follows only edges this close to the pointer ([ and ] change it)")),
    );
    let contrast = number(
        ui,
        "magnetic-contrast",
        label("Contrast"),
        contrast,
        1.0..=100.0,
        Some(UnitKind::Percent),
        Some(label("Higher values follow only edges that contrast sharply with their surroundings")),
    );
    let frequency =
        number(ui, "magnetic-frequency", label("Frequency"), frequency, 0.0..=100.0, None, Some(label("How often fastening points are placed by themselves")));
    let toggled = ToggleButton::new(label("Use tablet pressure to change pen width"), Bind::given(pressure))
        .icon(craft::behavior::IconId::new("target"))
        .icon_only()
        .show(ui)
        .value()
        .copied();
    let o = &mut app.ui.tool_options;
    if let Some(v) = width {
        o.magnetic.width = v as f32;
    }
    if let Some(v) = contrast {
        o.magnetic.contrast = v as f32;
    }
    if let Some(v) = frequency {
        o.magnetic.frequency = v as f32;
    }
    if let Some(on) = toggled {
        o.magnetic.pressure_width = on;
    }
}

/// The Magic Wand: operations, Tolerance, Anti-alias, Contiguous, Sample All Layers, Select Subject
/// and Select and Mask.
fn wand(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    operation_segments(app, ui, true);
    divider(ui);
    if let Some(next) = number(ui, "tolerance", label("Tolerance"), f64::from(app.ui.tool_options.tolerance), 0.0..=255.0, None, None) {
        app.ui.tool_options.tolerance = next as f32;
    }
    anti_alias(app, ui, true);
    if let Some(next) = check(ui, label("Contiguous"), app.ui.tool_options.contiguous, true) {
        app.ui.tool_options.contiguous = next;
    }
    if let Some(next) = check(ui, label("Sample All Layers"), app.ui.tool_options.sample_all.magic_wand, true) {
        app.ui.tool_options.sample_all.magic_wand = next;
    }
    divider(ui);
    let subject = Button::new(label("Select Subject")).show(ui);
    if clicked(&subject.response) {
        let result = app.run("select.subject", serde_json::json!({}));
        crate::retouch_ui::report_smart_result(app, result, tl!("No subject found — try Object Selection or the Lasso"));
    }
    select_and_mask(ui);
}

/// Shows the members of `tool`'s bar; false for a tool without selection members.
pub(super) fn members(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    match tool {
        Tool::RectMarquee | Tool::EllipseMarquee | Tool::SingleRowMarquee | Tool::SingleColumnMarquee => marquee(app, ui, tool),
        Tool::Lasso | Tool::PolygonLasso | Tool::MagneticLasso => lasso(app, ui, tool),
        Tool::MagicWand => wand(app, ui),
        _ => return false,
    }
    true
}
