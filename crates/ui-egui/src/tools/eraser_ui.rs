//! UI for the Eraser group's colour-aware tools: Magic Eraser (click → `paint.magicEraser`) and
//! Background Eraser (stroke → `paint.backgroundEraser`), with their options bars.
//!
//! A Magic Eraser press in an Indexed Color or Bitmap document shows the reference's alert (message
//! + OK) and runs nothing: the document keeps its pixels and its History (#12032).

use photocraft_engine::eraser_cmds::{MAGIC_ERASER_BITMAP_ERROR, MAGIC_ERASER_INDEXED_ERROR};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};
use crate::theme::Tokens;
use crate::{icons, widgets};

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
    app.ui.open_dialog(DialogKind::Error, json!({"message": tl!(message)}).as_object().cloned().unwrap_or_default());
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
        "tolerance": o.bg_tolerance,
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
            opt(ui, tl!("Tolerance:"));
            widgets::value_field(ui, &mut o.bg_tolerance, 0.0..=100.0, "%", 62.0);
            widgets::checkbox(ui, &mut o.bg_protect_fg, tl!("Protect Foreground Color"));
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};

    /// The message of every open error dialog, in the order they were opened.
    fn alerts(app: &PhotocraftApp) -> Vec<&str> {
        app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(|m| m.as_str())).collect()
    }

    /// The app with a white 64x48 document in `mode` open and the Magic Eraser picked: a click on
    /// it would wipe the flat canvas out.
    fn app_in(mode: ColorMode) -> PhotocraftApp {
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
            // OK closes it (`DialogKind::Error` asks for nothing else), and the next press answers
            // the same way: the document is no more erasable than it was.
            let open: Vec<u64> = app.ui.dialogs.iter().map(|d| d.id).collect();
            for dialog in open {
                app.ui.close_dialog(dialog);
            }
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
            assert!(app.ui.dialogs.iter().all(|d| d.kind != DialogKind::Error), "{case}: no alert");
            let st = app.session.active().unwrap();
            let id = st.active_layer.unwrap();
            assert_eq!(st.doc.layer(id).unwrap().name, "Layer 0", "{case}: the Background became a layer");
            assert_eq!(st.doc.layer(id).unwrap().surface().unwrap().rgba(2, 2)[3], 0.0, "{case}: erased to transparency");
            assert_eq!(st.history.past_len(), steps + 1, "{case}: one undo step");
        }
    }
}
