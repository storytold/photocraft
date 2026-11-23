//! UI for the Eraser group's colour-aware tools: Magic Eraser (click → `paint.magicEraser`) and
//! Background Eraser (stroke → `paint.backgroundEraser`), with their options-bar members.
//!
//! A Magic Eraser press in an Indexed Color or Bitmap document shows the reference's alert (message
//! + OK) and runs nothing: the document keeps its pixels and its History (#12032).

use craft::behavior::IconId;
use craft::ui::buttons::ToggleButton;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use photocraft_engine::eraser_cmds::{MAGIC_ERASER_BITMAP_ERROR, MAGIC_ERASER_INDEXED_ERROR};
use serde_json::{Value, json};

use super::option_cells;
use crate::PhotocraftApp;
use crate::state::Tool;

fn report(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The alert this document gets instead of a Magic Eraser click (#12032): `Some(text)` means the
/// click is refused. The check is the engine's, so the press and `paint.magicEraser` can never
/// disagree about what is erased.
fn document_alert(app: &PhotocraftApp) -> Option<&'static str> {
    app.session.active().and_then(|st| photocraft_engine::eraser_cmds::magic_eraser_refusal(&st.doc))
}

/// A `paint.magicEraser` error as the alert it was meant to be (#12032), so a click that reaches
/// the command shows the same dialog the press shows rather than only the status bar.
fn error_alert(error: &str) -> Option<&'static str> {
    [MAGIC_ERASER_INDEXED_ERROR, MAGIC_ERASER_BITMAP_ERROR].into_iter().find(|text| *text == error)
}

/// Opens the alert: the message and one OK button, as a refused Move press does
/// (`move_lock::prompt`).
fn refuse(app: &mut PhotocraftApp, message: &str) {
    crate::dialogs::alerts::alert(app, alert_text(message));
}

/// The alert in the UI language. Literal keys, so the translation report lists both alerts for the
/// translators; `alert_keys_match_the_engine_texts` keeps them equal to the engine's texts.
fn alert_text(message: &str) -> &str {
    match message {
        MAGIC_ERASER_INDEXED_ERROR => {
            tl!("Could not use the magic eraser because it does not work with Indexed Color images (convert image to RGB Color mode to edit).")
        }
        MAGIC_ERASER_BITMAP_ERROR => {
            tl!("Could not use the magic eraser because it does not work with Bitmap images (convert image to Grayscale mode to edit).")
        }
        other => tl!(other),
    }
}

/// Magic Eraser click at document `(x, y)`. Returns false for other tools.
pub fn click(app: &mut PhotocraftApp, tool: Tool, x: f64, y: f64) -> bool {
    if tool != Tool::MagicEraser {
        return false;
    }
    // An Indexed Color or Bitmap document (#12032): the reference answers the click with its alert
    // and erases nothing, so the command is never reached - and the document keeps its History.
    if let Some(message) = document_alert(app) {
        refuse(app, message);
        return true;
    }
    let o = &app.ui.tool_options;
    let p = json!({
        "x": x.floor(),
        "y": y.floor(),
        "tolerance": o.tolerance,
        "antiAlias": o.anti_alias,
        "contiguous": o.contiguous,
        "sampleAllLayers": o.sample_all.magic_eraser,
        "opacity": o.magic_eraser_opacity,
    });
    let r = app.run("paint.magicEraser", p);
    if let Err(e) = &r
        && let Some(message) = error_alert(e)
    {
        refuse(app, message);
    }
    report(app, r);
    true
}

/// Finishes a Background Eraser stroke. Returns false for other tools.
pub fn finish_stroke(app: &mut PhotocraftApp, tool: Tool, points: &[[f64; 3]], line: bool) -> bool {
    if tool != Tool::BackgroundEraser {
        return false;
    }
    let mut p = background_eraser_params(app, points);
    crate::canvas::unsmoothed_line(&mut p, line);
    let r = app.run("paint.backgroundEraser", p);
    report(app, r);
    true
}

/// `paint.backgroundEraser` params for a stroke along `points`: shared by the live preview and
/// the commit, with the jitter seed from the first point (`retouch_ui::stroke_seed`).
pub(crate) fn background_eraser_params(app: &PhotocraftApp, points: &[[f64; 3]]) -> Value {
    let o = &app.ui.tool_options;
    json!({
        "points": points,
        "sampling": o.bg_sampling,
        "limits": o.bg_limits,
        "tolerance": stored_tolerance(o.bg_tolerance),
        "protectForegroundColor": o.bg_protect_fg,
        "seed": crate::retouch_ui::stroke_seed(points),
    })
}

/// The tool a drag with `tool` runs as: the History Brush for an Eraser that erases to history. ⌥
/// at the press reverses the option for that stroke, as in Photoshop.
pub fn drag_tool(app: &PhotocraftApp, tool: Tool, alt: bool) -> Tool {
    if tool == Tool::Eraser && app.ui.tool_options.erase_to_history != alt { Tool::HistoryBrush } else { tool }
}

/// Whether `tool` is the Eraser in Pencil mode: it erases hard, aliased pixels through
/// `paint.pencil` (`"erase": true`) and shows the Pencil's pixel-grid cursor (#2662).
pub fn pencil_mode(app: &PhotocraftApp, tool: Tool) -> bool {
    tool == Tool::Eraser && app.ui.tool_options.eraser_mode == "pencil"
}

/// Whether `tool` is the Eraser in Block mode: it erases a hard square of 16 screen pixels
/// (`16 / zoom` document pixels) through `paint.pencil` (`"block": true`); Size, Hardness,
/// Opacity and Flow don't apply (#2770).
pub fn block_mode(app: &PhotocraftApp, tool: Tool) -> bool {
    tool == Tool::Eraser && app.ui.tool_options.eraser_mode == "block"
}

/// The Block's edge in document pixels at the current zoom (what the engine erases).
pub fn block_size(app: &PhotocraftApp) -> f32 {
    photocraft_engine::brush_cmds::block_size(f64::from(app.point_zoom()))
}

/// A Tolerance as the field stores it: a whole number in 1..=100, the default 50 for a value
/// that is no number. Strokes use it too, so a restored setting erases the same before and after
/// the options bar shows it.
pub(crate) fn stored_tolerance(v: f32) -> f32 {
    if v.is_finite() { v.round().clamp(1.0, 100.0) } else { 50.0 }
}

/// A check box named `label` over `value`.
fn check(ui: &mut egui::Ui, label: &str, value: &mut bool, tip: Option<&str>) {
    let mut checkbox = Checkbox::new(Bind::bound(value)).label(label.to_owned());
    if let Some(tip) = tip {
        checkbox = checkbox.tooltip(tip.to_owned());
    }
    checkbox.show(ui);
}

/// The members of the Eraser group's options bars, added to the host's `Bar` body. Returns false
/// for other tools. The Eraser's own Mode is `PaintBar::eraser_mode`, not a member here.
pub fn options_bar(ui: &mut egui::Ui, app: &mut PhotocraftApp, tool: Tool) -> bool {
    match tool {
        Tool::MagicEraser => magic_eraser_members(ui, app),
        Tool::BackgroundEraser => background_eraser_members(ui, app),
        _ => return false,
    }
    true
}

fn magic_eraser_members(ui: &mut egui::Ui, app: &mut PhotocraftApp) {
    let o = &mut app.ui.tool_options;
    // The field clamps without an alert, as the reference's Magic Eraser Tolerance does.
    let _ = option_cells::number(tl!("Tolerance:"), &mut o.tolerance, 0.0..=255.0).show(ui);
    check(ui, tl!("Anti-alias"), &mut o.anti_alias, None);
    check(ui, tl!("Contiguous"), &mut o.contiguous, None);
    check(ui, tl!("Sample All Layers"), &mut o.sample_all.magic_eraser, None);
    Divider::vertical().show(ui);
    let _ = option_cells::number(tl!("Opacity:"), &mut o.magic_eraser_opacity, 1.0..=100.0).unit("%").show(ui);
}

fn background_eraser_members(ui: &mut egui::Ui, app: &mut PhotocraftApp) {
    let mut alerts = Vec::new();
    {
        let o = &mut app.ui.tool_options;
        let sampling = [
            Entry::new("continuous".to_string(), tl!("Sampling: Continuous")).icon(IconId::new("eyedropper")),
            Entry::new("once".to_string(), tl!("Sampling: Once")).icon(IconId::new("target")),
            Entry::new("backgroundSwatch".to_string(), tl!("Sampling: Background Swatch")).icon(IconId::new("square")),
        ];
        let picked = Segmented::new(tl!("Sampling"), Bind::given(o.bg_sampling.clone()), &sampling).show(ui).value().cloned();
        if let Some(sampling) = picked {
            o.bg_sampling = sampling;
        }
        Divider::vertical().show(ui);

        let limits = [
            Entry::new("discontiguous".to_string(), tl!("Discontiguous")),
            Entry::new("contiguous".to_string(), tl!("Contiguous")),
            Entry::new("findEdges".to_string(), tl!("Find Edges")),
        ];
        // In a `Field`, as Color Replacement's Limits: a bare control's label only names it (#16338).
        let picked = Field::new(tl!("Limits:"))
            .show(ui, |ui, _| {
                Select::new(tl!("Limits:"), Bind::given(o.bg_limits.clone()), &limits)
                    .tooltip(tl!("How far to let the erasing spread").to_owned())
                    .show(ui)
                    .events
                    .committed()
                    .cloned()
            })
            .inner
            .flatten();
        if let Some(limits) = picked {
            o.bg_limits = limits;
        }

        // Shown as the stroke reads it; written back only when the user edits it.
        let shown = stored_tolerance(o.bg_tolerance);
        let mut tolerance = shown;
        let alert = option_cells::number(tl!("Tolerance:"), &mut tolerance, 1.0..=100.0)
            .whole()
            .unit("%")
            .tooltip(tl!("Erase range of similar colors"))
            .show(ui)
            .out_of_range;
        if tolerance != shown {
            o.bg_tolerance = tolerance;
        }
        alerts.extend(alert);
    }

    // Angle and Pressure for Size edit the tool's own brush (the shell swaps one in per tool).
    let before = (*app.session.tools.brush).clone();
    let mut brush = before.clone();
    let mut angle = brush.angle;
    let angle_out = option_cells::number(tl!("Angle"), &mut angle, -180.0..=180.0)
        .whole()
        .unit("°")
        .icon(IconSpec::svg(IconId::new("rotate_cw")))
        .tooltip(tl!("Set the brush angle"))
        .show(ui);
    // One scrub is one gesture, so one history step.
    let mut key = angle_out.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e));
    brush.angle = angle;
    alerts.extend(angle_out.out_of_range);

    check(ui, tl!("Protect Foreground Color"), &mut app.ui.tool_options.bg_protect_fg, Some(tl!("Do not erase the foreground swatch color")));

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

    for alert in &alerts {
        option_cells::range_alert(app, ui, alert);
    }
}

#[cfg(test)]
mod tests {
    use super::super::brush_refusal::brush_refusal_alert_tests::alerts;
    use super::*;
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};

    /// #12032: the literal translation keys are the engine's alert texts, so an English UI shows
    /// exactly what `paint.magicEraser` answers and the catalogs translate the right string.
    #[test]
    fn alert_keys_match_the_engine_texts() {
        crate::i18n::with_language(crate::i18n::Lang::EN, || {
            for text in [MAGIC_ERASER_INDEXED_ERROR, MAGIC_ERASER_BITMAP_ERROR] {
                assert_eq!(alert_text(text), text);
            }
        });
    }

    /// The app with a white 64x48 document in `mode` open and the Magic Eraser picked: a click on
    /// it would wipe the flat canvas out.
    fn app_in(mode: ColorMode) -> Box<PhotocraftApp> {
        let mut s = photocraft_engine::Session::new();
        s.add_document(Document::with_background("p", Size::new(64, 48), mode, SampleType::U8, Color::WHITE), None);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.tool = Tool::MagicEraser;
        app
    }

    /// #12032: the press in an Indexed Color or a Bitmap document shows the reference's alert - the
    /// measured text and an OK button - and runs nothing: no History step, no erase, the Background
    /// stays a Background, and the alert rather than the status bar carries the message.
    #[test]
    fn a_press_in_an_indexed_or_bitmap_document_shows_the_alert_and_erases_nothing() {
        for (case, mode, want) in [("Indexed", ColorMode::Indexed, MAGIC_ERASER_INDEXED_ERROR), ("Bitmap", ColorMode::Bitmap, MAGIC_ERASER_BITMAP_ERROR)] {
            let mut app = app_in(mode);
            let st = app.session.active().unwrap();
            let (doc, steps) = (st.doc.clone(), st.history.past_len());
            let id = st.active_layer.unwrap();
            assert!(click(&mut app, Tool::MagicEraser, 2.0, 2.0), "{case}: the Magic Eraser takes the click");
            assert_eq!(alerts(&app), [want], "{case}: one alert with the reference's text");
            let st = app.session.active().unwrap();
            assert!(std::sync::Arc::ptr_eq(&doc, &st.doc), "{case}: the command never ran");
            assert_eq!(st.history.past_len(), steps, "{case}: no History step");
            assert_eq!(st.doc.layer(id).unwrap().name, "Background", "{case}: still a Background");
            assert!(st.doc.layer(id).unwrap().surface().unwrap().rgba(2, 2)[3] > 0.0, "{case}: nothing erased");
            assert!(!app.ui.status_error, "{case}: the alert is the answer, not the status bar");
            // OK closes it, and the next press answers the same way: the document is no more
            // erasable than it was.
            let id = app.framework.dialogs.open_dialogs().first().map(|d| d.id).unwrap();
            let ctx = app.ctx().clone();
            crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
            click(&mut app, Tool::MagicEraser, 10.0, 10.0);
            assert_eq!(alerts(&app), [want], "{case}: the next press is refused too");
            assert_eq!(app.session.active().unwrap().history.past_len(), steps, "{case}: still no History step");
        }
    }

    /// #12032: the new press check leaves every document the tool can edit exactly as it was: the
    /// click erases, no alert opens.
    #[test]
    fn a_press_in_an_editable_document_still_erases() {
        for (case, mode) in [
            ("RGB", ColorMode::Rgb),
            ("Grayscale", ColorMode::Grayscale),
            ("CMYK", ColorMode::Cmyk),
            ("Lab", ColorMode::Lab),
            ("Duotone", ColorMode::Duotone),
            ("Multichannel", ColorMode::Multichannel),
        ] {
            let mut app = app_in(mode);
            let steps = app.session.active().unwrap().history.past_len();
            assert!(click(&mut app, Tool::MagicEraser, 2.0, 2.0), "{case}");
            assert!(alerts(&app).is_empty(), "{case}: no alert");
            let st = app.session.active().unwrap();
            let id = st.active_layer.unwrap();
            assert_eq!(st.doc.layer(id).unwrap().name, "Layer 0", "{case}: the Background became a layer");
            assert_eq!(st.doc.layer(id).unwrap().surface().unwrap().rgba(2, 2)[3], 0.0, "{case}: erased to transparency");
            assert_eq!(st.history.past_len(), steps + 1, "{case}: one undo step");
        }
    }
}
