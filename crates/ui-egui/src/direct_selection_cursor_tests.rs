//! #13220: Direct Selection shows its own hollow arrow over the canvas instead of today's
//! crosshair, Path Selection's solid arrow while Ctrl/Cmd is held, and keeps the crosshair only
//! when Preferences › Cursors › Other Cursors asks for Precise. Tests drive the real canvas
//! (`document_area`) and read the cursor it hands the platform.

use egui::CursorIcon;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The canvas at 100 % with `tool`, the pointer over its centre.
fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.tool = tool;
    app.sync_views();
    app.ui.extras.rulers = false;
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.fit_pending = false;
    h.run_steps(2);
    let p = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(3);
    h
}

/// The cursor the canvas asks the platform for this frame.
fn cursor(h: &Harness<'_, PhotocraftApp>) -> CursorIcon {
    h.output().platform_output.cursor_icon
}

/// Idle over the canvas the tool paints its own arrow, so no OS pointer shows through.
#[test]
fn direct_selection_idles_on_its_own_hollow_arrow() {
    let h = harness(Tool::DirectSelection);
    assert_ne!(cursor(&h), CursorIcon::Crosshair, "the crosshair is not Direct Selection's pointer");
    assert_eq!(cursor(&h), CursorIcon::None, "the arrow we draw stands in for the OS pointer");
}

/// Alt changes nothing: the reference keeps its hollow arrow with Alt held over a segment and over
/// empty canvas.
#[test]
fn alt_keeps_the_hollow_arrow() {
    let mut h = harness(Tool::DirectSelection);
    h.state_mut().ui.shell.sticky_alt = true;
    h.run_steps(2);
    assert_eq!(cursor(&h), CursorIcon::None, "Alt keeps the drawn arrow");
}

/// Precise Other Cursors asks for the crosshair, the one case the brief leaves as it was.
#[test]
fn precise_other_cursors_keep_the_crosshair() {
    let mut h = harness(Tool::DirectSelection);
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    h.run_steps(2);
    assert_eq!(cursor(&h), CursorIcon::Crosshair, "Precise: the setting was not measured, so it keeps today's crosshair");
}

/// The arrow the arm paints, without a pointer: idle and with Alt the hollow one, with Ctrl/Cmd the
/// solid one the reference shows over a segment, and no arrow at all for Precise.
#[test]
fn the_arrow_follows_precise_and_the_path_modifier_only() {
    use super::direct_selection_arrow;
    assert_eq!(direct_selection_arrow(false, false), Some(false), "idle: the hollow arrow");
    assert_eq!(direct_selection_arrow(false, true), Some(true), "Ctrl/Cmd: Path Selection's solid arrow");
    assert_eq!(direct_selection_arrow(true, false), None, "Precise: the crosshair");
    assert_eq!(direct_selection_arrow(true, true), None, "Precise wins over the modifier");
}
