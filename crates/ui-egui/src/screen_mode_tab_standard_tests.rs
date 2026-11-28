//! Screen-mode Tab in Standard Screen Mode (#16841): the real app window, the real `view.screenMode.*` commands
//! and the real Tab key. Tab hides the options bar, the tool strip, the panel dock and the contextual task bar;
//! the menu bar, the document tabs, the status bar, the rulers and the scroll bars stay. Every check is relative:
//! what stays in the accessibility tree, and the view (zoom and centre) unchanged before and after; no literal
//! rects. Full Screen Mode With Menu Bar keeps today's behaviour, Esc leaves the mode, and no Tab journals a
//! history step.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use craft::behavior::geometry::CropBox;
use craft::behavior::kurbo::Rect as DocRect;

use crate::PhotocraftApp;
use crate::control::{ControlRequest, Reply, handle};
use crate::theme::AppTheme;

type AppHarness = Harness<'static, Box<PhotocraftApp>>;

/// The real app window with one document, every panel on.
fn app_window() -> AppHarness {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "name": "tab check.png"})).unwrap();
    let theme = AppTheme::pro();
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme.clone());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.save_theme(&theme).unwrap();
        app
    });
    h.run_steps(6);
    h
}

/// Runs a command through the control protocol, as an agent does.
fn invoke(h: &mut AppHarness, id: &str) -> Value {
    let ctx = h.ctx.clone();
    let (request, _reply) = ControlRequest::new("ui.menu.invoke", json!({"id": id}));
    let Reply::Done(reply) = handle(h.state_mut(), &ctx, &request) else { panic!("{id}: expected an immediate reply") };
    assert_eq!(reply["ok"], true, "{id}: {reply}");
    reply["result"].clone()
}

fn inspect(h: &mut AppHarness) -> Value {
    let ctx = h.ctx.clone();
    let (request, _reply) = ControlRequest::new("ui.inspect", json!({}));
    let Reply::Done(reply) = handle(h.state_mut(), &ctx, &request) else { panic!("ui.inspect: expected an immediate reply") };
    assert_eq!(reply["ok"], true, "ui.inspect: {reply}");
    reply["result"].clone()
}

/// The window's own parts that the reference keeps after Tab: the menu bar's titles, the document tab of
/// "tab check.png" (the dock's panel tabs share the Tab role, so count the document's by its name) and the
/// status bar's zoom field.
fn kept_visible(h: &AppHarness) -> (bool, usize, bool) {
    let menus = h.query_by_label("File").is_some() && h.query_by_label("Help").is_some();
    let tabs = h.query_all_by_role(Role::Tab).filter(|n| n.accesskit_node().label().is_some_and(|label| label.contains("tab check"))).count();
    let status = h.query_by_label("Zoom").is_some();
    (menus, tabs, status)
}

/// The editor's parts that Tab hides: an options-bar field, a tool-strip button and the dock's footer button.
fn hidden_by_tab(h: &AppHarness) -> (bool, bool, bool) {
    (
        h.query_all_by_label_contains("Auto-Select").next().is_some(),
        h.query_all_by_label_contains("Move Tool").next().is_some(),
        h.query_all_by_label_contains("Create a new layer").next().is_some(),
    )
}

/// Press Tab as the user does, and let the shell settle.
fn press_tab(h: &mut AppHarness) {
    h.key_press(egui::Key::Tab);
    h.run_steps(6);
}

/// The primary canvas's rect from the last pass of the real layout, as `ui.inspect` reports it.
fn canvas_rect(h: &mut AppHarness) -> Value {
    let rect = inspect(h)["rects"]["canvas.0"].clone();
    assert!(rect.is_array(), "the primary canvas reports its rect");
    rect
}

/// The rect of a named widget in the accessibility tree, to pin "keeps its place" relatively.
fn widget_rect(h: &AppHarness, label: &str) -> egui::Rect {
    h.query_by_label(label).unwrap_or_else(|| panic!("{label} is drawn")).rect()
}

/// The Crop tool with its frame open, so the contextual task bar shows.
fn open_crop_frame(h: &mut AppHarness) {
    invoke_set_tool(h, "crop");
    h.state_mut().canvas_tools.crop.tool.crop_to(CropBox::new(DocRect::new(100.0, 100.0, 300.0, 200.0), 0.0));
    h.run_steps(3);
    assert!(crate::crop_task_bar::eligible(h.state()), "the crop frame and the tool make the task bar eligible");
}

/// Select the tool through the control protocol's `ui.set`.
fn invoke_set_tool(h: &mut AppHarness, tool: &str) {
    let ctx = h.ctx.clone();
    let (request, _reply) = ControlRequest::new("ui.set", json!({"tool": tool}));
    let Reply::Done(reply) = handle(h.state_mut(), &ctx, &request) else { panic!("ui.set: expected an immediate reply") };
    assert_eq!(reply["ok"], true, "ui.set tool {tool}: {reply}");
    h.run_steps(2);
}

#[test]
fn tab_in_standard_hides_only_the_editor_chrome_and_keeps_the_window_chrome() {
    let mut h = app_window();
    invoke(&mut h, "view.screenMode.standard");
    h.run_steps(2);
    assert_eq!(inspect(&mut h)["view"]["screen_mode"], json!("standard"), "the command puts the window in standard mode");
    let kept_before = kept_visible(&h);
    assert!(kept_before.0 && kept_before.1 > 0 && kept_before.2, "the window shows its menu bar, its document tab and its status bar");
    assert_eq!(hidden_by_tab(&h), (true, true, true), "the options bar, the tool strip and the dock show too");
    let views = inspect(&mut h)["views"].clone();
    let ruler_on = h.state().ui.extras.rulers;
    let canvas = canvas_rect(&mut h);
    let file = widget_rect(&h, "File");
    let zoom = widget_rect(&h, "Zoom");

    press_tab(&mut h);
    let kept = kept_visible(&h);
    assert_eq!(kept, kept_before, "Tab keeps the menu bar, the document tabs and the status bar shown");
    assert_eq!(hidden_by_tab(&h), (false, false, false), "Tab hides the options bar, the tool strip and the dock");
    assert_eq!(h.state().ui.extras.rulers, ruler_on, "View > Show > Rulers is untouched: the rulers stay");
    assert_eq!(inspect(&mut h)["views"], views, "the canvas keeps its zoom and view centre while its area grows");
    assert_eq!(widget_rect(&h, "File"), file, "the menu bar keeps its rect");
    assert_eq!(widget_rect(&h, "Zoom"), zoom, "the status bar keeps its rect");
    let grown = canvas_rect(&mut h);
    let (c, g) = (canvas.as_array().unwrap(), grown.as_array().unwrap());
    let side = |v: &[Value], i: usize| v[i].as_f64().expect("a canvas edge in points");
    assert!(side(g, 1) < side(c, 1), "the canvas reaches up into the room the options bar freed");
    assert_eq!(side(g, 3), side(c, 3), "its bottom stays put: the status bar keeps its room");
    assert!(side(g, 2) > side(c, 2), "and it reaches into the room the panel dock freed");

    press_tab(&mut h);
    assert_eq!(kept_visible(&h), kept_before, "the window chrome is still there after Tab again");
    assert_eq!(hidden_by_tab(&h), (true, true, true), "Tab again restores the same layout");
    assert_eq!(inspect(&mut h)["views"], views, "and the same view");
    assert_eq!(canvas_rect(&mut h), canvas, "down to the very canvas rect the layout had before Tab");
}

#[test]
fn tab_in_standard_hides_the_contextual_task_bar_and_tab_again_shows_it() {
    let mut h = app_window();
    invoke(&mut h, "view.screenMode.standard");
    open_crop_frame(&mut h);
    assert_eq!(inspect(&mut h)["taskBar"]["visible"], json!(true), "the task bar shows over the crop frame");

    press_tab(&mut h);
    assert_eq!(inspect(&mut h)["taskBar"]["visible"], json!(false), "Tab takes the task bar down with the options bar, the tool strip and the dock");
    assert!(!crate::crop_task_bar::eligible(h.state()), "and the bar is not eligible in the hidden state");

    press_tab(&mut h);
    assert_eq!(inspect(&mut h)["taskBar"]["visible"], json!(true), "Tab again brings the task bar back with the rest");
}

#[test]
fn tab_in_full_screen_with_menu_bar_keeps_todays_behaviour() {
    let mut h = app_window();
    invoke(&mut h, "view.screenMode.fullScreenWithMenuBar");
    h.run_steps(3);
    let kept = kept_visible(&h);
    assert!(kept.0 && kept.2, "the mode keeps the menu bar and the status bar");
    assert_eq!(kept.1, 0, "and hides the document tabs, as the mode does");
    assert_eq!(hidden_by_tab(&h), (true, true, true), "the bars and the dock show in the mode");
    open_crop_frame(&mut h);
    assert_eq!(inspect(&mut h)["taskBar"]["visible"], json!(true), "the task bar shows while the crop frame is open");

    press_tab(&mut h);
    assert_eq!(kept_visible(&h), (false, 0, false), "Tab there hides everything, the menu bar and the status bar included, as today");
    assert_eq!(hidden_by_tab(&h), (false, false, false), "and the editor's parts too");
    assert_eq!(inspect(&mut h)["taskBar"]["visible"], json!(true), "the task bar keeps today's behaviour: Tab in this mode leaves it alone");

    press_tab(&mut h);
    assert_eq!(kept_visible(&h), kept, "Tab again shows the window's parts as they were in the mode");
    // The Crop tool's options bar names its own controls, so check the two tool-agnostic parts.
    assert!(
        h.query_all_by_label_contains("Move Tool").next().is_some() && h.query_all_by_label_contains("Create a new layer").next().is_some(),
        "and the tool strip and the dock are back"
    );
}

#[test]
fn esc_leaves_either_full_screen_mode_back_to_standard() {
    let mut h = app_window();
    for mode in ["fullScreenWithMenuBar", "fullScreen"] {
        invoke(&mut h, &format!("view.screenMode.{mode}"));
        h.run_steps(3);
        assert_eq!(inspect(&mut h)["view"]["screen_mode"], json!(mode), "{mode}: the command set the mode");
        h.key_press(egui::Key::Escape);
        h.run_steps(4);
        assert_eq!(inspect(&mut h)["view"]["screen_mode"], json!("standard"), "{mode}: Esc leaves it");
    }
    // In standard mode the Tab hide and Esc interact as today: Esc leaves the screen mode alone.
    press_tab(&mut h);
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert_eq!(inspect(&mut h)["view"]["screen_mode"], json!("standard"), "Esc in standard mode keeps the mode");
}

#[test]
fn the_tab_chrome_toggle_journals_no_history_step() {
    let mut h = app_window();
    invoke(&mut h, "view.screenMode.standard");
    let before = h.state().session.active().unwrap().history.past_len();
    press_tab(&mut h);
    press_tab(&mut h);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before, "hiding and showing the chrome journals nothing");
}
