//! #2849: a complete secondary tap can arrive in one frame. Keep its opening press from
//! dismissing a canvas menu whose position rounds to a physical pixel.

use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::canvas_tool_menu::MenuKind;
use crate::{PhotocraftApp, Services, Tool};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Popup {
    Tool,
    Layers,
}

fn harness(tool: Tool, selection: bool, scale: f32) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_pixels_per_point(scale).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({"name": "Color"})).unwrap();
        app.run("edit.fill", json!({"color": "#cc6644"})).unwrap();
        if selection {
            app.run("select.rect", json!({"x": 40, "y": 40, "width": 200, "height": 100})).unwrap();
        }
        app.ui.tool = tool;
        app
    });
    h.run_steps(5);
    h
}

fn fractional_point(h: &Harness<'_, Box<PhotocraftApp>>, scale: f32) -> Pos2 {
    let center = crate::canvas::testing::view(h.state()).expect("the canvas shows a view").rect.center();
    pos2(((center.x * scale).floor() + 0.75) / scale, ((center.y * scale).floor() + 0.75) / scale)
}

fn press(h: &mut Harness<'_, Box<PhotocraftApp>>, at: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) {
    h.input_mut().events.push(Event::PointerButton { pos: at, button, pressed, modifiers });
}

fn tap(h: &mut Harness<'_, Box<PhotocraftApp>>, at: Pos2, modifiers: Modifiers, same_frame: bool) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(at));
    h.run_steps(2);
    for pressed in [true, false] {
        // Harness::event separates queued events into frames; RawInput preserves a quick tap.
        press(h, at, PointerButton::Secondary, pressed, modifiers);
        if !same_frame {
            h.run_steps(1);
        }
    }
    h.run_steps(3);
}

fn document_state(h: &Harness<'_, Box<PhotocraftApp>>) -> impl PartialEq + use<> {
    let state = h.state().session.active().unwrap();
    let strokes = h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").count();
    (state.doc.clone(), state.revision, state.history.past_len(), state.active_layer, state.selected_layers().to_vec(), strokes)
}

fn is_open(h: &Harness<'_, Box<PhotocraftApp>>, popup: Popup) -> bool {
    let layers = |m: &crate::canvas_tool_menu::CanvasToolMenu| matches!(m.kind, MenuKind::Layers { .. });
    match popup {
        Popup::Tool => h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| !layers(m)),
        Popup::Layers => h.state().ui.canvas_tool_menu.as_ref().is_some_and(layers),
    }
}

fn dismiss_outside(h: &mut Harness<'_, Box<PhotocraftApp>>, popup: Popup) {
    // The status-bar corner is outside every popup and does not edit the document.
    let at = pos2(1190.0, 790.0);
    h.event(Event::PointerMoved(at));
    h.run_steps(2);
    press(h, at, PointerButton::Primary, true, Modifiers::NONE);
    h.run_steps(1);
    assert!(!is_open(h, popup), "a later outside press closes immediately");
    press(h, at, PointerButton::Primary, false, Modifiers::NONE);
    h.run_steps(2);
}

/// A macOS Control-click reaches the canvas as raw Ctrl+primary presses; the runtime's input filter turns
/// them into a secondary click. It opens the same menu as a plain right-click and paints nothing. The
/// filter reads the shell's OS, which is the build's, so only a macOS build runs this.
#[cfg(target_os = "macos")]
#[test]
fn control_clicks_open_the_right_click_menus() {
    for (tool, popup) in [(Tool::RectMarquee, Popup::Tool), (Tool::Pen, Popup::Tool), (Tool::Move, Popup::Layers)] {
        let mut h = harness(tool, true, 1.0);
        let at = fractional_point(&h, 1.0);
        let before = document_state(&h);
        h.event(Event::ModifiersChanged(Modifiers::CTRL));
        h.event(Event::PointerMoved(at));
        h.run_steps(2);
        for pressed in [true, false] {
            press(&mut h, at, PointerButton::Primary, pressed, Modifiers::CTRL);
            h.run_steps(1);
        }
        h.run_steps(3);
        assert!(is_open(&h, popup), "{tool:?}");
        assert!(document_state(&h) == before, "{tool:?}: a Control-click paints nothing");
    }
}

#[test]
fn reopened_canvas_menus_still_choose_their_actions() {
    let mut h = harness(Tool::RectMarquee, true, 1.0);
    let at = fractional_point(&h, 1.0);
    tap(&mut h, at, Modifiers::NONE, true);
    assert!(is_open(&h, Popup::Tool));
    dismiss_outside(&mut h, Popup::Tool);
    tap(&mut h, at, Modifiers::NONE, true);
    assert!(is_open(&h, Popup::Tool));
    h.get_by_label(crate::i18n::t("Deselect")).click();
    h.run_steps(3);
    assert!(!is_open(&h, Popup::Tool));
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(2);
    tap(&mut h, at, Modifiers::NONE, true);
    assert!(is_open(&h, Popup::Layers));
    dismiss_outside(&mut h, Popup::Layers);
    tap(&mut h, at, Modifiers::NONE, true);
    assert!(is_open(&h, Popup::Layers));
    h.get_by_role_and_label(egui::accesskit::Role::MenuItem, "Background").click();
    h.run_steps(3);
    assert!(!is_open(&h, Popup::Layers));
    let state = h.state().session.active().unwrap();
    assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().name, "Background");
}

/// The automation request opens the menu through the frames the harness runs, as a user's click does.
fn control_tap(h: &mut Harness<'_, Box<PhotocraftApp>>, at: Pos2, popup: Popup) {
    use crate::control::{ControlRequest, Reply, handle};

    let doc = crate::canvas::document_display::active_view(h.state()).expect("the canvas shows a view").to_doc(at);
    let (x, y) = (doc.x, doc.y);
    let (req, _) = ControlRequest::new("ui.pointer", json!({"button":"secondary", "events":[{"kind":"down", "x":x, "y":y}, {"kind":"up", "x":x, "y":y}]}));
    let ctx = h.ctx.clone();
    let Reply::Done(reply) = handle(h.state_mut(), &ctx, &req) else { panic!("ui.pointer replies at once") };
    assert_eq!(reply["ok"], json!(true), "{reply:?}");
    h.run_steps(3);
    assert!(is_open(h, popup));
}

/// #7324: an agent's right-click on committed type opens the type layer menu, as on the canvas.
#[test]
fn control_channel_right_click_on_committed_type_opens_the_type_layer_menu() {
    let mut h = harness(Tool::Type, false, 1.0);
    h.state_mut().run("type.create", json!({"text": "Hello", "size": 72, "x": 100, "y": 180})).unwrap();
    h.run_steps(3);
    let at = fractional_point(&h, 1.0);
    let before = document_state(&h);
    control_tap(&mut h, at, Popup::Tool);
    let menu = h.state().ui.canvas_tool_menu.clone().unwrap();
    assert!(matches!(menu.kind, MenuKind::TypeLayer { .. }));
    assert!(h.state().ui.text_edit.is_none());
    // The new type is already the active layer, so the menu selects nothing either.
    assert!(document_state(&h) == before, "opening the menu changes nothing");
}
