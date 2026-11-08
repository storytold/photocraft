//! UI for the Eraser group's colour-aware tools: Magic Eraser (click → `paint.magicEraser`) and
//! Background Eraser (stroke → `paint.backgroundEraser`), with their options bars.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::Tokens;
use crate::{icons, widgets};

fn report(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Magic Eraser click at document `(x, y)`. Returns false for other tools.
pub fn click(app: &mut PhotocraftApp, tool: Tool, x: f64, y: f64) -> bool {
    if tool != Tool::MagicEraser {
        return false;
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
    report(app, r);
    true
}

/// Finishes a Background Eraser stroke. Returns false for other tools.
pub fn finish_stroke(app: &mut PhotocraftApp, tool: Tool, points: &[[f64; 3]]) -> bool {
    if tool != Tool::BackgroundEraser {
        return false;
    }
    let r = app.run("paint.backgroundEraser", background_eraser_params(app, points));
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

/// The Eraser's Mode dropdown (Brush / Pencil / Block). Returns whether it changed.
pub fn mode_dropdown(ui: &mut egui::Ui, mode: &mut String) -> bool {
    opt(ui, tl!("Mode:"));
    let modes = [("brush".to_string(), tl!("Brush")), ("pencil".to_string(), tl!("Pencil")), ("block".to_string(), tl!("Block"))];
    widgets::dropdown(ui, "eraser-mode", tl!("Mode"), mode, &modes, 96.0)
}

fn opt(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let label = ui.label(egui::RichText::new(tl!(&text)).color(t.text_dim).size(12.0));
    crate::widgets::scrub_option_label(ui, &label);
}

/// Options bar for the Magic Eraser and Background Eraser. Returns false for other tools.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    let o = &mut app.ui.tool_options;
    match tool {
        Tool::MagicEraser => {
            opt(ui, tl!("Tolerance:"));
            widgets::value_field(ui, &mut o.tolerance, 0.0..=255.0, "", 58.0);
            widgets::checkbox(ui, &mut o.anti_alias, tl!("Anti-alias"));
            widgets::checkbox(ui, &mut o.contiguous, tl!("Contiguous"));
            widgets::checkbox(ui, &mut o.sample_all.magic_eraser, tl!("Sample All Layers"));
            widgets::vline(ui, 22.0);
            opt(ui, tl!("Opacity:"));
            widgets::value_field(ui, &mut o.magic_eraser_opacity, 1.0..=100.0, "%", 62.0);
        }
        Tool::BackgroundEraser => {
            // Sampling: Continuous / Once / Background Swatch, as icon toggles (Photoshop).
            ui.spacing_mut().item_spacing.x = 2.0;
            for (k, icon, tip) in [
                ("continuous", "pipette", tl!("Sampling: Continuous")),
                ("once", "circle-dot", tl!("Sampling: Once")),
                ("backgroundSwatch", "square", tl!("Sampling: Background Swatch")),
            ] {
                if icons::button(ui, icon, 24.0, o.bg_sampling == k, tip).clicked() {
                    o.bg_sampling = k.into();
                }
            }
            ui.spacing_mut().item_spacing.x = 8.0;
            widgets::vline(ui, 22.0);
            opt(ui, tl!("Limits:"));
            let limits = [
                ("discontiguous".to_string(), tl!("Discontiguous")),
                ("contiguous".to_string(), tl!("Contiguous")),
                ("findEdges".to_string(), tl!("Find Edges")),
            ];
            widgets::dropdown(ui, "bg-eraser-limits", tl!("Limits:"), &mut o.bg_limits, &limits, 110.0);
            // The dropdown's button, by the id its combo box takes from "bg-eraser-limits".
            if let Some(limits) = ui.ctx().read_response(ui.make_persistent_id(egui::IdSalt::new("bg-eraser-limits"))) {
                limits.on_hover_text(tl!("How far to let the erasing spread"));
            }
            opt(ui, tl!("Tolerance:"));
            if tolerance_field(ui, &mut o.bg_tolerance) {
                open_range_alert(app);
            }
            let o = &mut app.ui.tool_options;
            widgets::checkbox(ui, &mut o.bg_protect_fg, tl!("Protect Foreground Color")).on_hover_text(tl!("Do not erase the foreground swatch color"));
        }
        _ => return false,
    }
    true
}

/// The alert for a Background Eraser Tolerance typed out of range (#11909).
pub const TOLERANCE_RANGE_ALERT: &str = "An integer between 1% and 100% is required. Closest value inserted.";

/// The Background Eraser's Tolerance: whole numbers in 1..=100, as the reference application takes
/// them (#11909). A typed fraction becomes the nearest whole number, a value past the range the
/// closest one. Returns true when the field is left with a value typed out of range, so the
/// caller shows [`TOLERANCE_RANGE_ALERT`].
fn tolerance_field(ui: &mut egui::Ui, value: &mut f32) -> bool {
    whole_tolerance_field(ui, value, "bg-eraser-tolerance-typed", tl!("Erase range of similar colors"))
}

/// [`tolerance_field`]'s field for any tool's whole-number Tolerance (the Color Replacement tool's
/// too, #11930): `draft` keys its typed-but-not-left text, `hover` is its tooltip.
pub(crate) fn whole_tolerance_field(ui: &mut egui::Ui, value: &mut f32, draft: &str, hover: &str) -> bool {
    // A restored setting (an older 0, a fraction) shows and erases as the field would store it.
    *value = stored_tolerance(*value);
    // While the field has the keyboard it gets back what was typed, not the stored value, so
    // its text is left alone mid-entry (`0` on the way to `05` must not turn into `1`).
    let draft_id = ui.make_persistent_id(egui::IdSalt::new(draft));
    // The field's own range is wide, so the typed value reaches here unclamped.
    // A draft only stands while the setting is still the one it stored: a value set elsewhere
    // meanwhile (a tool preset while the bar was hidden) wins.
    let draft = ui.ctx().data(|d| d.get_temp::<(f32, f32)>(draft_id));
    let mut typed = draft.filter(|(_, stored)| *stored == *value).map_or(*value, |(typed, _)| typed);
    let r = widgets::value_field(ui, &mut typed, -1.0e9..=1.0e9, "%", 62.0).on_hover_text(hover);
    let out_id = r.id.with("out-of-range");
    // An edit the field never got to finish (the options bar was hidden while it had the
    // keyboard) has nothing left to say once the field is entered again.
    if r.gained_focus() {
        ui.ctx().data_mut(|d| d.remove_temp::<bool>(out_id));
    }
    if r.changed() && typed.is_finite() {
        *value = typed.round().clamp(1.0, 100.0);
        // Only what was typed alerts, also an expression committed as the field is left
        // (`50*4` and Enter); a scrub past the ends (the field's or its caption's, even while
        // the field has the keyboard) just stops there.
        let typing = (r.has_focus() || r.lost_focus()) && ui.ctx().dragged_id().is_none();
        let out = typing && !(1.0..=100.0).contains(&typed);
        ui.ctx().data_mut(|d| d.insert_temp(out_id, out));
        if typing && r.has_focus() {
            ui.ctx().data_mut(|d| d.insert_temp(draft_id, (typed, *value)));
        } else {
            ui.ctx().data_mut(|d| d.remove_temp::<(f32, f32)>(draft_id));
        }
    }
    if !r.has_focus() {
        ui.ctx().data_mut(|d| d.remove_temp::<(f32, f32)>(draft_id));
    }
    r.lost_focus() && ui.ctx().data_mut(|d| d.remove_temp::<bool>(out_id)).unwrap_or(false)
}

/// A Tolerance as the field stores it: a whole number in 1..=100, the default 50 for a value
/// that is no number. Strokes use it too, so a restored setting erases the same before and after
/// the options bar shows it.
pub(crate) fn stored_tolerance(v: f32) -> f32 {
    if v.is_finite() { v.round().clamp(1.0, 100.0) } else { 50.0 }
}

/// Shows [`TOLERANCE_RANGE_ALERT`]: a modal with OK, once even if more than one pass asks.
pub(crate) fn open_range_alert(app: &mut PhotocraftApp) {
    let message = tl!(TOLERANCE_RANGE_ALERT).to_owned();
    let shown = app.ui.dialogs.iter().any(|d| d.kind == crate::state::DialogKind::Error && d.fields.get("message").and_then(|m| m.as_str()) == Some(&message));
    if !shown {
        app.ui.open_dialog(crate::state::DialogKind::Error, json!({ "message": message }).as_object().cloned().unwrap_or_default());
    }
}
