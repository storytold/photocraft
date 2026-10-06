//! The canvas layer list (#307), like Photoshop's: with the Move tool, a right-click on the image
//! lists the layers with pixels under the pointer, topmost first (`layer.pickAt` with
//! `"list": true`, so hidden layers and the contents of hidden groups are skipped); choosing one
//! selects it with `layer.select`, journaled like any other selection. With another tool,
//! ⌘/Ctrl-right-click shows the same list, since ⌘/Ctrl temporarily acts as the Move tool. A
//! right-click outside the image, or over no pixels, opens nothing.

use egui::{Modifiers, Pos2, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

/// The open layer list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayerPickMenu {
    /// The document it lists layers of (it closes when another document becomes active).
    pub doc: u64,
    /// Where it opens: the pointer, in screen points.
    pub pos: [f32; 2],
    /// The layers under the point, topmost first: id and name.
    pub layers: Vec<(u64, String)>,
}

/// Does a right-click with `tool` (and `mods` held) open the layer list rather than the tool's
/// own right-click (the Brush Preset picker, or erasing)?
pub fn wanted(tool: Tool, mods: Modifiers) -> bool {
    tool == Tool::Move || mods.command
}

/// Open the list for the document point `at`, shown at the screen point `pos`. Nothing opens
/// without a document, outside the image or where no layer has pixels. Returns whether it opened.
pub fn open(app: &mut PhotocraftApp, pos: Pos2, at: [f64; 2]) -> bool {
    app.ui.layer_pick = None;
    let Some(st) = app.session.active() else { return false };
    let doc = st.doc.id.0;
    let (w, h) = (f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    // Written so NaN fails too.
    let inside = at[0] >= 0.0 && at[1] >= 0.0 && at[0] < w && at[1] < h;
    if !inside {
        return false;
    }
    let listed = match app.run("layer.pickAt", json!({"x": at[0], "y": at[1], "list": true})) {
        Ok(v) => v,
        Err(_) => return false, // `run` reported it in the status bar
    };
    let layers: Vec<(u64, String)> = listed
        .get("layers")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter().filter_map(|r| Some((r.get("layer")?.as_u64()?, r.get("name").and_then(Value::as_str).unwrap_or_default().to_string()))).collect()
        })
        .unwrap_or_default();
    if layers.is_empty() {
        return false;
    }
    app.ui.brush_picker = None;
    app.ui.layer_pick = Some(LayerPickMenu { doc, pos: [pos.x, pos.y], layers });
    true
}

/// `open` for a document point with no pointer (the control channel's `ui.pointer`): the list
/// shows where that point is on the active view.
pub fn open_at_doc_point(app: &mut PhotocraftApp, at: [f64; 2]) -> bool {
    let view = app.session.active_index().and_then(|i| app.ui.views.get(i)).cloned().unwrap_or_default();
    let xf = crate::canvas::ViewXform { rect: app.last_canvas_rect, zoom: view.zoom, center: view.center, flip: app.ui.view.flip_horizontal };
    let pos = xf.to_screen(at[0] as f32, at[1] as f32);
    open(app, pos, at)
}

/// Show the open list as a context menu at its position (`canvas` is the canvas response it
/// belongs to). Choosing a layer selects it; a click elsewhere or Escape closes it.
pub fn show(app: &mut PhotocraftApp, canvas: &Response) {
    let Some(menu) = app.ui.layer_pick.clone() else { return };
    let current = match app.session.active() {
        Some(st) if st.doc.id.0 == menu.doc => st.active_layer.map(|l| l.0),
        _ => {
            app.ui.layer_pick = None;
            return;
        }
    };
    let mut open = true;
    let mut chosen = None;
    egui::Popup::menu(canvas).id(egui::Id::new("canvas-layer-pick")).at_position(Pos2::new(menu.pos[0], menu.pos[1])).open_bool(&mut open).show(|ui| {
        ui.set_min_width(160.0);
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for (id, name) in &menu.layers {
                if ui.add(egui::Button::selectable(current == Some(*id), name)).clicked() {
                    chosen = Some(*id);
                    ui.close();
                }
            }
        });
    });
    if let Some(id) = chosen {
        // `run` reports a failure (say, the layer was deleted meanwhile) in the status bar.
        let _ = app.run("layer.select", json!({"layer": id}));
        open = false;
    }
    if !open {
        app.ui.layer_pick = None;
    }
}

#[cfg(test)]
mod tests {
    use egui::{Modifiers, PointerButton, Pos2, vec2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    use super::*;
    use crate::canvas::ViewXform;

    /// A 400 × 300 document: the Background, "Square" (a red square at 100..200 × 100..200) and an
    /// empty "Empty" layer on top, which the list skips.
    fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({"name": "Square"})).unwrap();
        app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.run("layer.new.layer", json!({"name": "Empty"})).unwrap();
        app.ui.tool = tool;
        app.sync_views();
        harness_for(app)
    }

    fn harness_for(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
        let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        h
    }

    /// The screen point of document point (x, y).
    fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
        let app = h.state();
        let v = &app.ui.views[0];
        ViewXform { rect: app.last_canvas_rect, zoom: v.zoom, center: v.center, flip: false }.to_screen(x, y)
    }

    fn right_click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, modifiers: Modifiers) {
        // Held for the whole click, like a real key.
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed, modifiers });
            h.run_steps(2);
        }
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(1);
    }

    fn names(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
        h.state().ui.layer_pick.as_ref().map(|m| m.layers.iter().map(|(_, n)| n.clone()).collect()).unwrap_or_default()
    }

    fn active_name(h: &Harness<'static, PhotocraftApp>) -> String {
        let st = h.state().session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().name.clone()
    }

    #[test]
    fn move_tool_right_click_lists_the_layers_under_the_pointer_and_selects_one() {
        let mut h = harness(Tool::Move);
        let p = screen(&h, 150.0, 150.0);
        right_click(&mut h, p, Modifiers::NONE);
        assert_eq!(names(&h), ["Square", "Background"], "topmost first, the empty layer skipped");
        assert_eq!(h.state().ui.brush_picker, None);
        assert_eq!(active_name(&h), "Empty", "opening the list selects nothing");
        // The menu shows at the pointer, listing the layers by name.
        h.run_steps(2);
        let shown = h.ctx.memory(|m| m.area_rect(egui::Id::new("canvas-layer-pick"))).expect("menu shown");
        assert!(shown.min.distance(p) < 2.0, "{shown:?} at {p:?}");
        h.get_by_label("Square");
        // Choosing one selects it with `layer.select` (journaled) and closes the menu.
        h.get_by_label("Background").click();
        h.run_steps(3);
        assert_eq!(active_name(&h), "Background");
        assert_eq!(h.state().ui.layer_pick, None, "closed");
        let last = h.state().session.journal.last().map(|(id, _)| id.clone());
        assert_eq!(last.as_deref(), Some("layer.select"));
        // The click on the menu never reached the Move tool underneath.
        assert!(h.state().session.journal.iter().all(|(id, _)| id != "layer.translate"));
        // Outside the square only the Background is under the pointer.
        let q = screen(&h, 300.0, 50.0);
        right_click(&mut h, q, Modifiers::NONE);
        assert_eq!(names(&h), ["Background"]);
        // Escape closes it without selecting anything.
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert_eq!(h.state().ui.layer_pick, None);
    }

    #[test]
    fn hidden_layers_are_not_listed() {
        let mut h = harness(Tool::Move);
        let square = h.state().session.active().unwrap().doc.layers.iter().find(|l| l.name == "Square").unwrap().id;
        h.state_mut().run("layer.setProps", json!({"layer": square.0, "visible": false})).unwrap();
        h.run_steps(2);
        let p = screen(&h, 150.0, 150.0);
        right_click(&mut h, p, Modifiers::NONE);
        assert_eq!(names(&h), ["Background"]);
    }

    #[test]
    fn command_right_click_with_the_brush_lists_layers_instead_of_the_brush_picker() {
        let mut h = harness(Tool::Brush);
        let p = screen(&h, 150.0, 150.0);
        right_click(&mut h, p, Modifiers::COMMAND);
        assert_eq!(names(&h), ["Square", "Background"]);
        assert_eq!(h.state().ui.brush_picker, None, "no Brush Preset picker");
        h.run_steps(2);
        h.get_by_label("Square").click();
        h.run_steps(3);
        assert_eq!(active_name(&h), "Square");
        assert!(h.state().session.journal.iter().all(|(id, _)| id != "paint.stroke"), "nothing painted");
        // A plain right-click still opens the Brush Preset picker.
        right_click(&mut h, p, Modifiers::NONE);
        assert_eq!(h.state().ui.layer_pick, None);
        assert!(h.state().ui.brush_picker.is_some());
    }

    #[test]
    fn command_right_click_with_the_erase_preference_does_not_erase() {
        let mut h = harness(Tool::Brush);
        h.state_mut().run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
        let p = screen(&h, 150.0, 150.0);
        right_click(&mut h, p, Modifiers::COMMAND);
        assert_eq!(names(&h), ["Square", "Background"]);
        assert!(h.state().session.journal.iter().all(|(id, _)| id != "paint.stroke"), "nothing erased");
    }

    #[test]
    fn right_click_off_the_image_or_without_a_document_opens_nothing() {
        let mut h = harness(Tool::Move);
        // On the pasteboard, outside the image.
        let r = h.state().last_canvas_rect;
        let off = r.min + vec2(4.0, 4.0);
        right_click(&mut h, off, Modifiers::NONE);
        assert_eq!(h.state().ui.layer_pick, None);
        // Without a document (the Home screen).
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.tool = Tool::Move;
        let mut h = harness_for(app);
        right_click(&mut h, Pos2::new(600.0, 400.0), Modifiers::NONE);
        right_click(&mut h, Pos2::new(600.0, 400.0), Modifiers::COMMAND);
        assert_eq!(h.state().ui.layer_pick, None);
        assert!(!open(h.state_mut(), Pos2::ZERO, [1.0, 1.0]));
        assert!(!open(h.state_mut(), Pos2::ZERO, [f64::NAN, 1.0]));
    }

    #[test]
    fn a_stale_list_closes_and_a_deleted_layer_fails_gracefully() {
        let mut h = harness(Tool::Move);
        let p = screen(&h, 150.0, 150.0);
        right_click(&mut h, p, Modifiers::NONE);
        // The listed layer disappears before it is chosen: an error in the status bar, no panic.
        let square = h.state().ui.layer_pick.as_ref().unwrap().layers[0].0;
        h.state_mut().run("layer.select", json!({"layer": square})).unwrap();
        h.state_mut().run("layer.delete", json!({})).unwrap();
        h.run_steps(2);
        h.get_by_label("Square").click();
        h.run_steps(3);
        assert!(h.state().ui.status_error, "reported: {}", h.state().ui.status);
        assert_eq!(h.state().ui.layer_pick, None);
        // A list for a document that is no longer active closes.
        h.state_mut().ui.layer_pick = Some(LayerPickMenu { doc: u64::MAX, pos: [p.x, p.y], layers: vec![(1, "X".into())] });
        h.run_steps(2);
        assert_eq!(h.state().ui.layer_pick, None);
    }

    #[test]
    fn the_control_channel_opens_the_list_with_a_secondary_pointer() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        let ctx = egui::Context::default();
        let call = |app: &mut PhotocraftApp, p: serde_json::Value| {
            let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", p);
            assert!(matches!(crate::control::handle(app, &ctx, &req), crate::control::Outcome::Done(_)));
        };
        let ev = json!([{"kind": "down", "x": 10, "y": 10}, {"kind": "up", "x": 10, "y": 10}]);
        call(&mut app, json!({"tool": "move", "button": "secondary", "events": ev}));
        assert_eq!(app.ui.layer_pick.as_ref().map(|m| m.layers.len()), Some(1));
        app.ui.layer_pick = None;
        // ⌘/Ctrl with another tool; without it the Brush opens its picker instead.
        call(&mut app, json!({"tool": "brush", "button": "secondary", "command": true, "events": ev}));
        assert!(app.ui.layer_pick.is_some() && app.ui.brush_picker.is_none());
        app.ui.layer_pick = None;
        call(&mut app, json!({"tool": "brush", "button": "secondary", "events": ev}));
        assert!(app.ui.layer_pick.is_none() && app.ui.brush_picker.is_some());
    }
}
