//! The vertical type tools' cursors (#12063): the Vertical Type tool shows an I-beam turned on its
//! side inside a dotted box, the Vertical Type Mask tool the sideways I-beam alone. The horizontal
//! tools keep the upright I-beam and Preferences › Cursors › Other Cursors = Precise keeps the
//! crosshair. Tests drive the real canvas (`document_area`) and read the cursor it asks for.

use egui::CursorIcon;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// A cursor icon, and its bitmap's size, hotspot and pixels.
type Cursor = (CursorIcon, Option<([u16; 2], [u16; 2], Vec<u8>)>);

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

fn cursor(h: &Harness<'_, PhotocraftApp>) -> Cursor {
    let out = h.output();
    let image = out.platform_output.cursor_image.as_ref().map(|i| (i.size, i.hotspot, i.rgba.to_vec()));
    (out.platform_output.cursor_icon, image)
}

fn precise(h: &mut Harness<'_, PhotocraftApp>) {
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    h.run_steps(2);
}

/// Whether the bitmap draws something at (`dx`, `dy`) logical points from its hotspot.
fn drawn(c: &Cursor, scale: f32, dx: f32, dy: f32) -> bool {
    let Some((size, hot, rgba)) = &c.1 else { return false };
    let x = f32::from(hot[0]) + (dx * scale).round();
    let y = f32::from(hot[1]) + (dy * scale).round();
    if x < 0.0 || y < 0.0 || x >= f32::from(size[0]) || y >= f32::from(size[1]) {
        return false;
    }
    let i = (y as usize * usize::from(size[0]) + x as usize) * 4 + 3;
    rgba.get(i).is_some_and(|a| *a > 200)
}

/// Whether the bitmap draws something within a pixel of (`dx`, `dy`), allowing for where the
/// hotspot falls inside its pixel.
fn near(c: &Cursor, scale: f32, dx: f32, dy: f32) -> bool {
    [-1.0, 0.0, 1.0].into_iter().any(|ox| [-1.0, 0.0, 1.0].into_iter().any(|oy| drawn(c, scale, dx + ox / scale, dy + oy / scale)))
}

#[test]
fn vertical_type_shows_a_sideways_i_beam_in_a_dotted_box() {
    let h = harness(Tool::VerticalType);
    let c = cursor(&h);
    // Our own bitmap: the end of the pass reports it with the crosshair as the fallback icon.
    assert!(c.1.is_some(), "the cursor bitmap, not a system cursor");
    let scale = crate::tool_cursor::native_scale(&h.ctx);
    // The stem runs across through the hotspot, not up and down.
    assert!(near(&c, scale, 0.0, 0.0), "the stem under the hotspot");
    assert!(near(&c, scale, -4.0, 0.0) && near(&c, scale, 4.0, 0.0), "the stem left and right of the hotspot");
    assert!(!drawn(&c, scale, 0.0, -4.0) && !drawn(&c, scale, 0.0, 4.0), "nothing above or below the hotspot");
    // The serifs stand up and down at the stem's ends.
    assert!(near(&c, scale, -5.5, -2.0) && near(&c, scale, 5.5, 2.0), "the serifs");
    // The dotted box: its corners 9 left/right of the hotspot, 4 above and 12 below.
    for (dx, dy) in [(-9.0, -4.0), (9.0, -4.0), (-9.0, 12.0), (9.0, 12.0)] {
        assert!(near(&c, scale, dx, dy), "a box corner dot at ({dx}, {dy})");
    }
    // Dotted, not a line: gaps between the dots on the bottom edge.
    assert!(!drawn(&c, scale, -6.0, 12.0) && !drawn(&c, scale, 6.0, 12.0), "gaps between the dots");
    assert_ne!(c, cursor(&harness(Tool::Type)), "not the horizontal tool's cursor");
}

#[test]
fn vertical_type_mask_shows_the_sideways_i_beam_without_a_box() {
    let h = harness(Tool::VerticalTypeMask);
    let c = cursor(&h);
    assert_eq!(c.0, CursorIcon::VerticalText);
    assert!(c.1.is_none(), "the system's sideways I-beam, no bitmap");
}

#[test]
fn the_horizontal_type_tools_keep_the_upright_i_beam() {
    for tool in [Tool::Type, Tool::HorizontalTypeMask] {
        let c = cursor(&harness(tool));
        assert_eq!(c.0, CursorIcon::Text, "{tool:?}");
        assert!(c.1.is_none(), "{tool:?}: no bitmap");
    }
}

#[test]
fn precise_other_cursors_keep_the_crosshair_for_both_vertical_tools() {
    let mut reference = harness(Tool::Type);
    precise(&mut reference);
    let crosshair = cursor(&reference);
    assert_ne!(crosshair.0, CursorIcon::Text, "Precise: not the I-beam");
    for tool in [Tool::VerticalType, Tool::VerticalTypeMask] {
        let mut h = harness(tool);
        let standard = cursor(&h);
        precise(&mut h);
        let shown = cursor(&h);
        assert_eq!(shown, crosshair, "{tool:?}: the same crosshair as the other tools");
        assert_ne!(shown, standard, "{tool:?}: not the sideways I-beam");
    }
}
