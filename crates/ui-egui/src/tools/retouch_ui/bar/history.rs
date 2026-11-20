//! The options-bar members of the History Brush, Art History Brush, Pattern Stamp and Red Eye
//! tools. Mode, Opacity and Flow come before them, from the bar's stamp arm.

use craft::behavior::IconId;
use craft::ui::buttons::ToggleButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::Field;
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use craft::ui::text::RangeAlert;
use serde_json::json;

use crate::PhotocraftApp;
use crate::tools::option_cells::{hint, number};

/// What the History Brush paints from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SourceCaption {
    /// The document's opening state.
    Opening,
    /// A snapshot or history state, by name.
    Named(String),
    /// The source no longer exists.
    Deleted,
}

/// The History Brush's source as the bar names it.
pub(crate) fn source_caption(app: &PhotocraftApp) -> SourceCaption {
    use photocraft_engine::history_source_cmds::{HistorySource, source};
    let Some(st) = app.session.active() else { return SourceCaption::Deleted };
    let named = || match source(st)? {
        HistorySource::Snapshot(id) => st.snapshots().iter().find(|s| s.id == id).map(|s| SourceCaption::Named(s.name.clone())),
        HistorySource::State(idx) => {
            let entries = st.history.entries();
            let name = if idx < entries.len() { entries.get(idx).cloned() } else { st.history.redo_labels().nth(idx - entries.len()).map(str::to_string) }?;
            // The opening state is "Open" for a file and "New" for a new document (#14029).
            Some(if idx == 0 && matches!(name.as_str(), "Open" | "New") { SourceCaption::Opening } else { SourceCaption::Named(name) })
        }
    };
    named().unwrap_or(SourceCaption::Deleted)
}

/// History Brush: the caption of its source.
pub(super) fn history_brush(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let text = match source_caption(app) {
        SourceCaption::Opening => tl!("Paints from the document's opening state").to_string(),
        SourceCaption::Named(name) => crate::i18n::fmt(tl!("Source: {name}"), &[("name", &name)]),
        SourceCaption::Deleted => tl!("The History Brush source was deleted: set a new source in the History panel").to_string(),
    };
    hint(ui, text);
    None
}

/// Art History Brush: Style, Area, Tolerance. Its alerts carry the reference's words, so it opens them itself.
pub(super) fn art_history(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let styles: Vec<Entry<String>> = [
        ("tightShort", tl!("Tight Short")),
        ("tightMedium", tl!("Tight Medium")),
        ("tightLong", tl!("Tight Long")),
        ("looseMedium", tl!("Loose Medium")),
        ("looseLong", tl!("Loose Long")),
        ("dab", tl!("Dab")),
        ("tightCurl", tl!("Tight Curl")),
        ("looseCurl", tl!("Loose Curl")),
    ]
    .into_iter()
    .map(|(k, l)| Entry::new(k.to_string(), l.to_string()))
    .collect();
    let o = &mut app.ui.tool_options;
    Field::new(tl!("Style")).side(SideLabel::new().scrub(true)).sizing(Sizing::Content).show(ui, |ui, _| {
        Select::new(tl!("Style"), Bind::bound(&mut o.art_history_style), &styles).show(ui);
    });
    // Area: whole pixels 0..500, no slider; Tolerance: whole percents 0..100 with a slider. A value
    // typed past either range becomes the closest one with the reference's own alert (#12630).
    let area = number(tl!("Area"), &mut o.art_history_area, 0.0..=500.0).whole().unit("px").tooltip(tl!("Painting area diameter")).show(ui).out_of_range;
    let tolerance = number(tl!("Tolerance"), &mut o.art_history_tolerance, 0.0..=100.0)
        .whole()
        .unit("%")
        .popup_slider()
        .tooltip(tl!("Painting tolerance"))
        .show(ui)
        .out_of_range;
    // Angle and Pressure for Size follow Tolerance (#12629); an Angle typed past its range gets
    // the bar's own alert.
    let angle = art_history_angle_pressure(ui, app);
    if area.is_some() {
        let _ = crate::dialogs::alerts::alert(app, tl!(ART_HISTORY_AREA_ALERT));
    } else if tolerance.is_some() {
        let _ = crate::dialogs::alerts::alert(app, tl!(ART_HISTORY_TOLERANCE_ALERT));
    }
    angle
}

/// The Art History Brush's Angle and Pressure for Size, after Tolerance (#12629). They edit the
/// tool's own brush, as the Background Eraser's do.
fn art_history_angle_pressure(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let before = app.session.tools.brush.clone();
    let mut brush = before.clone();
    let mut angle = brush.angle;
    let angle_out = number(tl!("Angle"), &mut angle, -180.0..=180.0)
        .whole()
        .unit("°")
        .icon(IconSpec::svg(IconId::new("rotate_cw")))
        .tooltip(tl!("Set the brush angle"))
        .show(ui);
    // One scrub is one gesture, so one history step.
    let mut key = angle_out.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e));
    brush.angle = angle;
    let pressure = ToggleButton::new(tl!("Always use pressure for size"), Bind::given(brush.pressure_size))
        .icon(IconId::new("target"))
        .icon_only()
        .tooltip(tl!("Always use Pressure for Size. When off, Brush Preset controls pressure").to_owned())
        .show(ui);
    if let Some(on) = pressure.value().copied() {
        brush.pressure_size = on;
        key = pressure.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e)).or(key);
    }
    crate::brush_panel::commit_keyed(app, &before, &brush, key);
    angle_out.out_of_range
}

/// The reference's alert for an Art History Brush Area typed past 0..500 px (#12630).
pub(crate) const ART_HISTORY_AREA_ALERT: &str = "An integer between 0 pixel and 500 Pixels is required. Closest value inserted.";

/// The reference's alert for an Art History Brush Tolerance typed past 0..100 (#12630).
pub(crate) const ART_HISTORY_TOLERANCE_ALERT: &str = "An integer between 0 and 100 is required. Closest value inserted.";

/// Pattern Stamp: Aligned, Impressionist, Pattern, Scale, Angle.
pub(super) fn pattern_stamp(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let patterns: Vec<Entry<String>> = app.session.patterns.items.iter().map(|p| Entry::new(p.id.clone(), p.display_name().to_string())).collect();
    let held = photocraft_engine::presets::patterns::current(&app.session).or(app.session.patterns.items.first()).map(|p| p.id.clone());
    let mut picked = held.clone().unwrap_or_default();
    let o = &mut app.ui.tool_options;
    Checkbox::new(Bind::bound(&mut o.pattern_stamp_aligned)).label(tl!("Aligned")).show(ui);
    Checkbox::new(Bind::bound(&mut o.pattern_stamp_impressionist)).label(tl!("Impressionist")).show(ui);
    Divider::vertical().show(ui);
    if patterns.is_empty() {
        Text::new(tl!("No patterns")).shade(Shade::Faint).show(ui);
    } else {
        Field::new(tl!("Pattern")).side(SideLabel::new().scrub(true)).sizing(Sizing::Content).show(ui, |ui, _| {
            Select::new(tl!("Pattern"), Bind::bound(&mut picked), &patterns).show(ui);
        });
    }
    let scale = number(tl!("Scale"), &mut o.pattern_stamp_scale, 1.0..=1000.0).unit("%").show(ui).out_of_range;
    let angle = number(tl!("Angle"), &mut o.pattern_stamp_angle, -180.0..=180.0).unit("°").show(ui).out_of_range;
    if !patterns.is_empty() && held.as_deref() != Some(picked.as_str()) {
        let _ = app.run("pattern.presets.select", json!({"pattern": picked}));
    }
    scale.or(angle)
}

/// Red Eye: Pupil Size and Darken Amount.
pub(super) fn red_eye(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let o = &mut app.ui.tool_options;
    let pupil = number(tl!("Pupil Size"), &mut o.red_eye_pupil_size, 1.0..=100.0).show(ui).out_of_range;
    let darken = number(tl!("Darken Amount"), &mut o.red_eye_darken, 0.0..=100.0).show(ui).out_of_range;
    Divider::vertical().show(ui);
    hint(ui, tl!("Click a red pupil to neutralize it"));
    pupil.or(darken)
}
