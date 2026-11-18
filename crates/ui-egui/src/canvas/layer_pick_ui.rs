//! The canvas layer context menu (#307): right-click with the Move tool, or ⌘/Ctrl+right-click
//! with any tool, lists the layers with visible pixels under the pointer, topmost first, as
//! Photoshop does. Choosing one selects it.
//!
//! The hit test is `layer.pickAt` with `"list": true` (non-transparent pixels, text bounds,
//! layer masks, hidden layers and hidden groups), and choosing runs `layer.select`, so the menu is the same
//! two commands an agent would run. Painting tools keep their plain right-click (the Brush
//! Preset picker, #268); the ⌘/Ctrl variant reaches the layer list from them too. The menu is
//! `canvas_tool_menu::MenuKind::Layers`, shown by `canvas_tool_menu::run`.

use craft::keymap::Mods;
use craft::ui::contract::Entry; // PLAN: assumed path of the framework `Entry`
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

/// The id prefix of a layer row: `layer.select.<layer id>`.
pub const ROW: &str = "layer.select.";

/// The row id of layer `layer`.
pub fn row_id(layer: u64) -> String {
    format!("{ROW}{layer}")
}

/// Does a right-click with `tool` and `mods` open the layer menu (rather than the tool's own)?
pub fn is_gesture(tool: Tool, mods: Mods) -> bool {
    tool == Tool::Move || mods.cmd
}

/// The layers at document point (x, y), topmost first, including text between glyphs.
pub fn layers_at(app: &mut PhotocraftApp, x: f64, y: f64) -> Vec<(u64, String)> {
    if !(x.is_finite() && y.is_finite()) {
        return Vec::new();
    }
    let Ok(v) = app.session.execute("layer.pickAt", json!({"x": x, "y": y, "list": true})) else { return Vec::new() };
    v.get("layers")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|l| Some((l.get("layer")?.as_u64()?, l.get("name").and_then(Value::as_str).unwrap_or_default().to_string()))).collect())
        .unwrap_or_default()
}

/// The menu's rows: one per layer, the active one checked.
pub fn entries(layers: &[(u64, String)], active: Option<u64>) -> Vec<Entry<String>> {
    layers.iter().map(|(id, name)| Entry::new(row_id(*id), name.clone()).checked(Some(*id) == active)).collect()
}

/// Choose a layer from the menu: select it (one `layer.select`) and close the menu.
pub fn choose(app: &mut PhotocraftApp, layer: u64) {
    app.ui.canvas_tool_menu = None;
    if let Err(e) = app.run("layer.select", json!({"layer": layer, "mode": "replace"})) {
        app.ui.status = e;
    }
}

#[cfg(test)]
mod tests {
    use egui::{Modifiers, PointerButton, Pos2, pos2, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    use super::*;
    use crate::canvas_tool_menu::MenuKind;

    /// 200 × 200: a white Background, "Red" (a square in the middle), and "Hidden" (all over,
    /// hidden). Returns the app and the (background, red, hidden) ids.
    fn app() -> (PhotocraftApp, u64, u64, u64) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let bg = app.session.active().unwrap().active_layer.unwrap().0;
        app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
        app.run("select.rect", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let red = app.session.active().unwrap().active_layer.unwrap().0;
        app.run("layer.new.layer", json!({"name": "Hidden"})).unwrap();
        app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let hidden = app.session.active().unwrap().active_layer.unwrap().0;
        app.run("layer.setProps", json!({"layer": hidden, "visible": false})).unwrap();
        (app, bg, red, hidden)
    }

    fn layer_ids(app: &PhotocraftApp) -> Option<Vec<u64>> {
        match app.ui.canvas_tool_menu.as_ref().map(|m| &m.kind) {
            Some(MenuKind::Layers { layers }) => Some(layers.iter().map(|l| l.0).collect()),
            _ => None,
        }
    }

    #[test]
    fn lists_visible_layers_with_pixels_topmost_first() {
        let (mut a, bg, red, _) = app();
        assert_eq!(layers_at(&mut a, 100.0, 100.0).iter().map(|l| l.0).collect::<Vec<_>>(), [red, bg]);
        assert_eq!(layers_at(&mut a, 10.0, 10.0).iter().map(|l| l.1.as_str()).collect::<Vec<_>>(), ["Background"]);
        choose(&mut a, red);
        assert_eq!(a.session.active().unwrap().active_layer.unwrap().0, red);
        assert!(a.ui.canvas_tool_menu.is_none());
    }

    #[test]
    fn nothing_there_or_bad_input_opens_nothing() {
        let (mut a, _, _, _) = app();
        let open = |app: &mut PhotocraftApp, x: f64, y: f64| crate::canvas_tool_menu::open_at(app, Tool::Move, Mods::NONE, pos2(0.0, 0.0), [x, y], None);
        for (x, y) in [(-5.0, 10.0), (500.0, 500.0), (f64::NAN, 1.0), (1.0, f64::INFINITY), (1e300, -1e300)] {
            assert!(layers_at(&mut a, x, y).is_empty(), "({x}, {y})");
        }
        for (x, y) in [(f64::NAN, 1.0), (1.0, f64::INFINITY)] {
            assert!(!open(&mut a, x, y), "({x}, {y})");
            assert!(a.ui.canvas_tool_menu.is_none());
        }
        // Off the canvas a right-click lists no layers: it opens the pasteboard menu.
        for (x, y) in [(-5.0, 10.0), (500.0, 500.0), (1e300, -1e300)] {
            assert!(open(&mut a, x, y), "({x}, {y})");
            assert!(a.ui.canvas_tool_menu.as_ref().is_some_and(|m| matches!(m.kind, MenuKind::Pasteboard)), "({x}, {y})");
            a.ui.canvas_tool_menu = None;
        }
        // A transparent document: nothing to list.
        let mut b = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(!open(&mut b, 1.0, 1.0), "no document");
        b.run("file.new", json!({"width": 20, "height": 20, "background": "transparent"})).unwrap();
        assert!(!open(&mut b, 1.0, 1.0));
        // Choosing a layer that no longer exists reports it and keeps the app running.
        choose(&mut a, 987_654);
        assert!(!a.ui.status.is_empty());
    }

    fn harness(a: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
        let mut a = a;
        a.sync_views();
        let mut h = Harness::builder().with_context(a.ctx().clone()).with_size(vec2(1000.0, 700.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            a,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::default());
        h.run_steps(4);
        h
    }

    fn right_click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, modifiers: Modifiers) {
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers });
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers });
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(2);
    }

    /// ⌘/Ctrl+right-click beats the Erase preference: it lists the layers and erases nothing.
    #[test]
    fn erase_preference_does_not_take_the_command_right_click() {
        let (mut a, _, red, _) = app();
        a.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
        a.ui.tool = Tool::Brush;
        let mut h = harness(a);
        let c = h.state().last_canvas_rect.center();
        right_click(&mut h, c, Modifiers::COMMAND);
        assert_eq!(layer_ids(h.state()).and_then(|l| l.first().copied()), Some(red));
        assert!(!h.state().session.journal.iter().any(|(id, _)| id == "paint.stroke"), "nothing erased");
    }

    /// `ui.inspect` reports `layerMenu.layers` (docs/control-protocol.md). The injected press is
    /// delivered on the next pass, so the harness runs frames after `control::handle`.
    #[test]
    fn agents_open_it_with_a_right_pointer_press() {
        let (mut a, _, red, _) = app();
        a.ui.tool = Tool::Move;
        let mut h = harness(a);
        let ctx = h.ctx.clone();
        let events = json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "up", "x": 100, "y": 100}]);
        let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"tool": "move", "button": "right", "events": events}));
        let _ = crate::control::handle(h.state_mut(), &ctx, &req);
        h.run_steps(4);
        assert_eq!(layer_ids(h.state()).and_then(|l| l.first().copied()), Some(red));
        let (req, _rx) = crate::control::ControlRequest::new("ui.inspect", Value::Null);
        let crate::control::Reply::Done(v) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!("inspect") };
        assert_eq!(v["result"]["layerMenu"]["layers"][0]["name"], "Red");
    }
}
