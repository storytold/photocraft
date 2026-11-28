//! Free Transform's contextual task bar through the real canvas and existing commands.

use egui::{Key, Rect, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::canvas::testing;
use crate::control::{ControlRequest, Reply, handle};
use crate::state::Tool;
use craft::behavior::geometry::{BoxPart, CropBox};
use craft::behavior::kurbo::{Point, Rect as DocRect};
use std::cell::Cell;

type AppHarness = Harness<'static, Box<PhotocraftApp>>;

thread_local! {
    static SECTION_GAP: Cell<f32> = const { Cell::new(0.0) };
    static HANDLE_SIDE: Cell<f32> = const { Cell::new(0.0) };
}

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    harness_tool(Tool::Move)
}

/// The same document with a chosen tool selected (e.g. Hand, #2302).
fn harness_tool(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = tool;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1000.0, 700.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            egui::CentralPanel::default().show(ui, |ui| {
                let env = craft::ui::runtime::env::Env::of(ui);
                SECTION_GAP.with(|x| x.set(env.tokens.space.gap.section));
                HANDLE_SIDE.with(|x| x.set(env.tokens.size.handle.md));
                crate::canvas::testing::document_area(app, ui);
            });
        },
        app,
    );
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.clear_pending();
    h.run_steps(2);
    assert!(SECTION_GAP.with(Cell::get) > 0.0 && HANDLE_SIDE.with(Cell::get) > 0.0, "the fixture uses actual current framework metrics");
    h
}

fn begin(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(2);
}

fn set_crop_frame(h: &mut AppHarness) {
    control(h, "ui.set", json!({"tool": "crop"}));
    h.run_steps(2);
    h.state_mut().ui.tool_options.crop.shield.classic_mode = true;
    h.state_mut().canvas_tools.crop.tool.crop_to(CropBox::new(DocRect::new(100.0, 100.0, 200.0, 160.0), 0.0));
}

fn view(app: &PhotocraftApp) -> craft::behavior::CanvasView {
    testing::view(app).expect("the primary canvas has shown its view")
}

/// The shared BoxState geometry, with the largest standard handle/pivot reach.
/// The real pointer gesture below must additionally rotate (not scale) the box.
fn outside_box(app: &PhotocraftApp, at: egui::Pos2) -> BoxPart {
    let v = view(app);
    let reach = f64::from(HANDLE_SIDE.with(Cell::get)) * 1.5 / v.zoom;
    app.ui.transform.as_ref().unwrap().session.state().part_at(v.to_doc(at), reach)
}

/// Settle the floating area's measured size before exercising its controls.
fn open_box() -> AppHarness {
    let mut h = harness();
    begin(&mut h);
    h.run_steps(4);
    h
}

/// Send a synchronous command through the same control path as the offscreen example.
fn queue_control(h: &mut AppHarness, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (request, _reply) = ControlRequest::new(method, params);
    let Reply::Done(reply) = handle(h.state_mut(), &ctx, &request) else { panic!("{method} should reply immediately") };
    assert_eq!(reply["ok"], true, "{method}: {reply}");
    reply["result"].clone()
}

fn control(h: &mut AppHarness, method: &str, params: Value) -> Value {
    let reply = queue_control(h, method, params);
    if method == "ui.pointer" {
        h.run_steps(4);
    }
    reply
}

/// Read the bar's current visibility and position through the public inspection route.
fn bar(h: &mut AppHarness) -> Value {
    control(h, "ui.inspect", json!({}))["taskBar"].clone()
}

/// Count committed document edits, excluding changes made only inside the open box.
fn history(h: &AppHarness) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// Click the actual accessible widget and allow the canvas to redraw.
fn click(h: &mut AppHarness, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(4);
}

/// Each preset must match the real right-click entry and keep the document history untouched.
#[test]
fn transform_task_bar_presets_match_the_context_menu() {
    for (label, id) in [
        ("Rotate 90° Counter Clockwise", "edit.transform.rotate90Ccw"),
        ("Rotate 90° Clockwise", "edit.transform.rotate90Cw"),
        ("Flip Horizontal", "edit.transform.flipHorizontal"),
        ("Flip Vertical", "edit.transform.flipVertical"),
    ] {
        let mut toolbar = open_box();
        let mut menu = open_box();
        let mut command = open_box();
        let command_history = history(&command);
        let before = toolbar.state().ui.transform.as_ref().unwrap().quad_array();
        let (toolbar_history, menu_history) = (history(&toolbar), history(&menu));
        click(&mut toolbar, label);
        control(&mut menu, "ui.pointer", json!({"button": "secondary", "events": [{"kind": "down", "x": 150, "y": 130}, {"kind": "up", "x": 150, "y": 130}]}));
        assert!(menu.state().ui.canvas_tool_menu.is_none(), "the transform owns its menu, not a tool-mode list");
        menu.get_by_role_and_label(egui::accesskit::Role::MenuItem, label).click();
        menu.run_steps(4);
        control(&mut command, "ui.menu.invoke", json!({"id": id}));
        assert_eq!(history(&command), command_history);
        let quad = toolbar.state().ui.transform.as_ref().unwrap().quad_array();
        assert_ne!(quad, before, "{label}");
        assert_eq!(quad, menu.state().ui.transform.as_ref().unwrap().quad_array(), "{label}");
        assert_eq!(quad, command.state().ui.transform.as_ref().unwrap().quad_array(), "{id}");
        assert_eq!(history(&toolbar), toolbar_history);
        assert_eq!(history(&menu), menu_history);
    }
}

/// Done commits once like Enter, while Cancel discards the box like Escape.
#[test]
fn transform_task_bar_done_and_cancel_match_the_keys() {
    let pixels = |h: &AppHarness| {
        let st = h.state().session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
    };
    for (label, key, edits) in [("Done", Key::Enter, 1), ("Cancel", Key::Escape, 0)] {
        let mut toolbar = open_box();
        let mut keys = open_box();
        let original = pixels(&toolbar);
        for h in [&mut toolbar, &mut keys] {
            click(h, "Rotate 90° Clockwise");
        }
        let (toolbar_history, key_history) = (history(&toolbar), history(&keys));
        click(&mut toolbar, label);
        keys.key_press(key);
        keys.run_steps(4);
        assert!(toolbar.state().ui.transform.is_none() && keys.state().ui.transform.is_none(), "{label}");
        assert_eq!(history(&toolbar), toolbar_history + edits);
        assert_eq!(history(&keys), key_history + edits);
        let actual = pixels(&toolbar);
        assert!(actual == pixels(&keys), "{label} should produce the same pixels as the key");
        assert_eq!(actual != original, edits != 0, "{label}");
        assert_eq!(bar(&mut toolbar)["visible"], false);
    }
}

/// The unpinned bar uses the current shared TaskBar section gap below the actual quad.
#[test]
fn transform_task_bar_tracks_the_rotated_quad() {
    let mut h = open_box();
    for turn in [false, true] {
        if turn {
            click(&mut h, "Rotate 90° Clockwise");
        }
        let app = h.state();
        let xf = view(app);
        let quad = app.ui.transform.as_ref().unwrap().session.quad().map(|p| xf.to_screen(p));
        let anchor = Rect::from_points(&quad);
        let [x0, y0, x1, y1] = crate::transform_task_bar::shown_rect(app, &h.ctx).unwrap();
        let shown = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        assert!((shown.center().x - anchor.center().x).abs() <= 1.0, "{shown:?} below {anchor:?}");
        assert!((shown.top() - anchor.bottom() - SECTION_GAP.with(Cell::get)).abs() <= 1.0, "{shown:?} below {anchor:?}");
        assert_eq!(bar(&mut h)["visible"], true);
    }
}

/// Warp and Content-Aware Scale keep their own controls, and the Crop bar still draws.
#[test]
fn transform_task_bar_keeps_warp_content_aware_scale_and_crop_separate() {
    let mut warp = open_box();
    control(&mut warp, "ui.menu.invoke", json!({"id": "edit.transform.warp"}));
    warp.run_steps(4);
    assert!(warp.state().ui.transform.as_ref().unwrap().session.warp().is_some());
    assert_eq!(bar(&mut warp)["visible"], false);

    let mut scale = harness();
    control(&mut scale, "ui.menu.invoke", json!({"id": "edit.contentAwareScale"}));
    scale.run_steps(4);
    assert!(scale.state().ui.transform.as_ref().unwrap().content_aware.is_some());
    assert_eq!(bar(&mut scale)["visible"], false);

    let mut crop = harness();
    control(&mut crop, "ui.set", json!({"tool": "crop"}));
    set_crop_frame(&mut crop);
    crop.run_steps(4);
    assert!(crate::crop_task_bar::eligible(crop.state()));
    assert!(!crate::transform_task_bar::eligible(crop.state()));
    assert_eq!(bar(&mut crop)["visible"], true);
}

/// The shared menu pins, resets and hides the bar, and the Window command restores it.
#[test]
fn transform_task_bar_uses_the_shared_menu_and_window_toggle() {
    let mut h = open_box();
    click(&mut h, "More options");
    click(&mut h, "Pin bar position");
    assert_eq!(bar(&mut h)["pinned"], true);
    assert!(h.state().ui.panels.task_bar.at.is_some());

    click(&mut h, "More options");
    click(&mut h, "Reset bar position");
    assert_eq!(bar(&mut h)["pinned"], false);
    assert!(h.state().ui.panels.task_bar.at.is_none());

    click(&mut h, "More options");
    click(&mut h, "Hide bar");
    assert_eq!(bar(&mut h)["enabled"], false);
    assert_eq!(bar(&mut h)["visible"], false);
    assert!(h.state().ui.transform.is_some());
    control(&mut h, "ui.menu.invoke", json!({"id": "window.panel.contextualTaskBar"}));
    h.run_steps(4);
    assert_eq!(bar(&mut h)["visible"], true);
}

/// A switch before the next canvas pass must not expose the previous document's Done action.
#[test]
fn transform_task_bar_ignores_a_foreign_preview_after_a_document_switch() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().run("document.activate", json!({"document": 0})).unwrap();
    begin(&mut h);
    h.run_steps(4);
    let owner = h.state().transform_preview.as_ref().unwrap().doc.id;
    let quad = h.state().ui.transform.as_ref().unwrap().quad_array();
    assert_eq!(bar(&mut h)["visible"], true);

    control(&mut h, "ui.menu.invoke", json!({"id": "window.nextDocument"}));
    assert_ne!(h.state().session.active().unwrap().doc.id, owner);
    assert_eq!(h.state().transform_preview.as_ref().unwrap().doc.id, owner);
    assert_eq!(bar(&mut h)["visible"], false);

    assert_eq!(h.state().ui.transform.as_ref().unwrap().quad_array(), quad);
}

#[test]
fn transform_task_bar_hides_while_a_shell_dialog_is_open() {
    let mut h = open_box();
    let session = h.state().ui.transform.as_ref().unwrap().id;
    assert_eq!(bar(&mut h)["visible"], true);
    control(&mut h, "ui.menu.invoke", json!({"id": "window.workspace.newWorkspace"}));
    assert!(h.state().dialog_open(), "the workspace dialog is hosted and modal");
    assert_eq!(bar(&mut h)["visible"], false);
    h.run_steps(2);
    assert!(h.query_by_label("Rotate 90° Clockwise").is_none());
    control(&mut h, "ui.dialog.cancel", json!({"dialog": "top"}));
    assert!(bar(&mut h)["rect"].is_null(), "the last pass did not draw the bar");
    h.run_steps(4);
    assert_eq!(h.state().ui.transform.as_ref().unwrap().id, session);
    assert_eq!(bar(&mut h)["visible"], true);
}

#[test]
fn transform_task_bar_inspection_keeps_crop_and_free_transform_rects_separate_before_redraw() {
    let mut h = open_box();
    assert_eq!(bar(&mut h)["visible"], true);
    control(&mut h, "ui.set", json!({"tool": "crop"}));
    queue_control(
        &mut h,
        "ui.pointer",
        json!({"tool": "crop", "events": [
        {"kind": "down", "x": 40, "y": 40}, {"kind": "move", "x": 230, "y": 170}, {"kind": "up", "x": 230, "y": 170}]}),
    );
    // Current ui.pointer is queued: before its real canvas pass neither bar can inherit the previous rect.
    assert!(!crate::transform_task_bar::eligible(h.state()));
    assert!(bar(&mut h)["rect"].is_null(), "Crop must wait for its own draw");
    h.run_steps(4);
    assert!(crate::crop_task_bar::eligible(h.state()));
    assert_eq!(bar(&mut h)["visible"], true);

    // An untouched Crop frame can leave the tool without committing a document edit.
    let mut h = harness();
    control(&mut h, "ui.set", json!({"tool": "crop"}));
    h.run_steps(4);
    assert_eq!(bar(&mut h)["visible"], true);
    control(&mut h, "ui.set", json!({"tool": "move"}));
    control(&mut h, "ui.menu.invoke", json!({"id": "edit.freeTransform"}));
    assert!(crate::transform_task_bar::eligible(h.state()));
    assert!(bar(&mut h)["rect"].is_null(), "Free Transform must wait for its own draw");
    h.run_steps(4);
    assert_eq!(bar(&mut h)["visible"], true);
}

/// Accessibility can deliver Hide and Done during the same pass.
#[test]
fn transform_task_bar_commits_when_done_and_hide_arrive_together() {
    let mut h = open_box();
    click(&mut h, "Rotate 90° Clockwise");
    let edits = history(&h);
    click(&mut h, "More options");

    // The harness runs queued clicks in separate frames. Deliver both actions in one input.
    let events = ["Hide bar", "Done"].map(|label| {
        let (target_node, target_tree) = h.get_by_label(label).accesskit_node().locate();
        egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest { action: egui::accesskit::Action::Click, target_node, target_tree, data: None })
    });
    h.input_mut().events.extend(events);
    h.run_steps(4);

    assert!(!h.state().ui.panels.context_task_bar);
    assert!(h.state().ui.transform.is_none());
    assert_eq!(history(&h), edits + 1);
}

/// A queued Crop gesture after real Cancel cannot inherit the Transform rect.
#[test]
fn transform_task_bar_does_not_report_its_rect_as_crop_after_cancel() {
    let mut h = open_box();
    let prior_rect = crate::transform_task_bar::shown_rect(h.state(), &h.ctx).unwrap();
    let cancel = h.get_by_label("Cancel").rect().center();
    assert!(Rect::from_min_max(pos2(prior_rect[0], prior_rect[1]), pos2(prior_rect[2], prior_rect[3])).contains(cancel));
    // The real primary-canvas TaskBar owns the pointer and cancels through its production action.
    h.event(egui::Event::PointerMoved(cancel));
    h.event(egui::Event::PointerButton { pos: cancel, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.event(egui::Event::PointerButton { pos: cancel, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    assert!(h.state().ui.transform.is_none());
    assert!(crate::transform_task_bar::shown_rect(h.state(), &h.ctx).is_none());

    control(&mut h, "ui.set", json!({"tool": "crop"}));
    queue_control(
        &mut h,
        "ui.pointer",
        json!({"tool": "crop", "events": [
        {"kind": "down", "x": 40, "y": 40}, {"kind": "move", "x": 230, "y": 170}, {"kind": "up", "x": 230, "y": 170}]}),
    );
    // No pass has consumed the queued Crop gesture: never expose the canceled Transform rectangle.
    let crop_bar = bar(&mut h);
    assert_eq!(crop_bar["visible"], false);
    assert!(crop_bar["rect"].is_null());
    h.run_steps(4);
    assert!(crate::crop_task_bar::eligible(h.state()));
    assert_eq!(bar(&mut h)["visible"], true);
}

/// The frame owns its press even when egui gives the underlying canvas the drag sense.
#[test]
fn transform_task_bar_frame_drag_preserves_the_open_transform() {
    for edge in ["top", "bottom", "right"] {
        let mut h = open_box();
        let app = h.state();
        let transform = app.ui.transform.as_ref().unwrap();
        let before = (transform.id, transform.session.pivot(), transform.quad_array());
        let before_history = history(&h);
        let [x0, y0, x1, y1] = crate::transform_task_bar::shown_rect(app, &h.ctx).unwrap();
        let frame = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        let from = match edge {
            "top" => pos2(frame.left() + 1.0, frame.top() + 1.0),
            "bottom" => pos2(frame.center().x, frame.bottom() - 1.0),
            _ => pos2(frame.right() - 1.0, frame.center().y),
        };
        let to = from + egui::vec2(35.0, 25.0);
        let xf = view(app);
        assert!(xf.rect.contains(from) && xf.rect.contains(to), "frame drag must stay on canvas");
        assert!(frame.contains(from), "frame drag must begin inside the visible bar");
        let hit = outside_box(app, from);
        assert!(matches!(hit, BoxPart::Outside), "frame sample must be outside transform controls: {edge} {hit:?}");
        task_bar_screen_drag(&mut h, from, to);
        let after = h.state().ui.transform.as_ref().map(|t| (t.id, t.session.pivot(), t.quad_array()));
        assert_eq!(after, Some(before), "dragging the bar's {edge} frame must leave the open transform unchanged");
        assert_eq!(history(&h), before_history, "frame drag must not commit an edit");
    }
}

/// The frame guard must still allow a canvas rotation gesture with an outside-bar origin.
#[test]
fn transform_task_bar_frame_guard_preserves_off_bar_rotation() {
    for via_controls in [false, true] {
        let mut h = harness();
        if via_controls {
            h.state_mut().ui.tool_options.move_show_transform = true;
            h.run_steps(2);
        } else {
            begin(&mut h);
        }
        let edits = history(&h);
        // The current handle's reach is theme-scaled; choose a point strictly inside that reach.
        // This replaces the legacy off-corner magic coordinate without weakening scale/no-rotate assertions.
        let xf = view(h.state());
        let offset = egui::vec2(HANDLE_SIDE.with(Cell::get) * 0.4, HANDLE_SIDE.with(Cell::get) * 0.4);
        let from = xf.to_screen(Point::new(200.0, 160.0)) + offset;
        let to = xf.to_screen(Point::new(250.0, 190.0)) + offset;
        task_bar_screen_drag(&mut h, from, to);
        let q = h.state().ui.transform.as_ref().expect("the real handle opens or retains Free Transform").quad_array();
        assert!((q[0][1] - q[1][1]).abs() < 1e-6 && (q[1][0] - q[2][0]).abs() < 1e-6, "via controls {via_controls}: not rotated: {q:?}");
        assert!(q[2][0] > 230.0, "via controls {via_controls}: scaled: {q:?}");
        assert_eq!(history(&h), edits);

        let app = h.state();
        let transform = app.ui.transform.as_ref().unwrap();
        let before = (transform.id, transform.session.pivot(), transform.quad_array());
        let before_history = history(&h);
        let xf = view(app);
        let corner = xf.to_screen(Point::new(before.2[1][0], before.2[1][1]));
        let from = corner + egui::vec2(20.0, -20.0);
        let to = corner + egui::vec2(45.0, 0.0);
        let [x0, y0, x1, y1] = crate::transform_task_bar::shown_rect(app, &h.ctx).unwrap();
        assert!(!Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)).contains(from));
        assert!(xf.rect.contains(from) && xf.rect.contains(to));
        let hit = outside_box(app, from);
        assert!(matches!(hit, BoxPart::Outside), "control must start in the rotation zone: {hit:?}");
        task_bar_screen_drag(&mut h, from, to);
        let after = h.state().ui.transform.as_ref().unwrap();
        assert_eq!((after.id, after.session.pivot()), (before.0, before.1));
        assert_ne!(after.quad_array(), before.2, "a real off-bar canvas drag must rotate the transform");
        let q = after.quad_array();
        assert!((q[0][1] - q[1][1]).abs() > 1.0, "via controls {via_controls}: rotated: {q:?}");
        assert_eq!(history(&h), before_history);
    }
}

fn task_bar_screen_drag(h: &mut AppHarness, from: egui::Pos2, to: egui::Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=6 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (step as f32 / 6.0)));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

fn task_bar_screen_rect(h: &mut AppHarness) -> Rect {
    let shown = bar(h);
    assert_eq!(shown["visible"], true);
    let [x0, y0, x1, y1]: [f32; 4] = serde_json::from_value(shown["rect"].clone()).unwrap();
    Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
}

/// Hit the current shared bar's named grip, not an inspection command.
fn drag_task_bar_grip(h: &mut AppHarness) -> Rect {
    let before = task_bar_screen_rect(h);
    let grip = h.get_by_label("Move bar").rect();
    assert!(grip.is_positive(), "the named shared grip has real positive bounds: {grip:?}");
    assert!(before.contains_rect(grip), "the pointer must hit the grip inside the visible bar");
    let fresh = h.state().ui.panels.task_bar;
    assert!(fresh.at.is_none() && fresh.offset.is_none(), "a fresh bar follows its frame before the drag");
    let area = view(h.state()).rect;
    let from = grip.center();
    let delta = egui::vec2(40.0, -24.0);
    assert!(area.contains(from) && area.contains(from + delta));
    let document = h.state().session.active().unwrap().doc.clone();
    let edits = history(h);
    task_bar_screen_drag(h, from, from + delta);
    let after = task_bar_screen_rect(h);
    assert_eq!(h.get_by_label("Move bar").rect().size(), grip.size(), "moving retains the current grip metrics");
    assert_eq!(after.size(), before.size(), "a grip drag moves the bar without resizing it");
    let movement = after.min - before.min;
    assert!(movement.length() > 12.0 && movement.dot(delta) > 0.0, "the grip must move the bar: {before:?} -> {after:?}");
    // An unpinned bar keeps the drop as its offset from its unmoved place and goes on following the frame.
    let place = h.state().ui.panels.task_bar;
    assert!(place.at.is_none(), "an unpinned bar keeps no fixed position: {place:?}");
    let [x, y] = place.offset.expect("a grip drag stores the bar's offset from its unmoved place");
    assert!((movement - egui::vec2(x, y)).length() <= 1.0, "the displayed bar must use the saved offset: {movement:?} vs {place:?}");
    assert_eq!(bar(h)["offset"], json!([x, y]));
    // This is the same serialized panel-layout field used across restarts, not a transient Area rect.
    let restored: crate::state::TaskBarPlace = serde_json::from_value(serde_json::to_value(place).unwrap()).unwrap();
    assert_eq!(restored, place);
    h.run_steps(4);
    assert_eq!(h.state().ui.panels.task_bar, place, "the drop stays saved on later canvas passes");
    assert!((task_bar_screen_rect(h).min - after.min).length() <= 1.0);
    assert!(std::sync::Arc::ptr_eq(&document, &h.state().session.active().unwrap().doc));
    assert_eq!(history(h), edits, "moving the bar must not commit a document edit");
    after
}

#[test]
fn transform_task_bar_grip_drag_moves_the_saved_bar_without_changing_the_box() {
    let mut h = open_box();
    let transform = h.state().ui.transform.as_ref().unwrap();
    let before = (transform.id, transform.session.pivot(), transform.quad_array());
    let edits = history(&h);
    let shown = drag_task_bar_grip(&mut h);
    let transform = h.state().ui.transform.as_ref().expect("the same transform stays open");
    assert_eq!((transform.id, transform.session.pivot(), transform.quad_array()), before);
    // A real pointer click after the grip drop still reaches a child button through the Area sense.
    let button = h.get_by_label("Rotate 90° Clockwise").rect().center();
    assert!(shown.contains(button));
    click(&mut h, "Rotate 90° Clockwise");
    let transform = h.state().ui.transform.as_ref().unwrap();
    assert_eq!((transform.id, transform.session.pivot()), (before.0, before.1));
    assert_ne!(transform.quad_array(), before.2, "the relocated rotation button still works");
    assert_eq!(history(&h), edits);
}

#[test]
fn crop_task_bar_grip_drag_moves_the_saved_bar_without_changing_the_crop() {
    let mut h = harness();
    control(&mut h, "ui.set", json!({"tool": "crop"}));
    set_crop_frame(&mut h);
    h.run_steps(4);
    assert!(crate::crop_task_bar::eligible(h.state()));
    let before = crate::crop_ui::frame(h.state());
    let edits = history(&h);
    let shown = drag_task_bar_grip(&mut h);
    assert_eq!(crate::crop_ui::frame(h.state()), before);
    let cancel = h.get_by_label("Cancel").rect().center();
    assert!(shown.contains(cancel));
    click(&mut h, "Cancel");
    assert!(!crate::crop_ui::pending(h.state()), "the relocated Cancel button still dismisses the pending crop");
    assert_ne!(crate::crop_ui::frame(h.state()), before, "the drawn frame was discarded");
    if crate::crop_ui::frame(h.state()).is_some() {
        assert!(crate::crop_ui::is_default_frame(h.state()), "a redrawn frame is the untouched current-tool default");
    }
    assert_eq!(history(&h), edits);
}

/// Crop uses the same floating frame and canvas drag routing as Free Transform.
#[test]
fn crop_task_bar_frame_drag_preserves_crop_but_off_bar_drag_rotates() {
    for on_bar in [true, false] {
        let mut h = harness();
        control(&mut h, "ui.set", json!({"tool": "crop"}));
        set_crop_frame(&mut h);
        h.run_steps(4);
        let app = h.state();
        assert!(crate::crop_task_bar::eligible(app));
        let before = crate::crop_ui::frame(app);
        let before_history = history(&h);
        let [x0, y0, x1, y1] = app.ui.task_bar_shown.expect("Crop bar must be visible");
        let frame = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        let xf = view(app);
        let (from, to) = if on_bar {
            let from = pos2(frame.center().x, frame.bottom() - 1.0);
            (from, from + egui::vec2(35.0, 25.0))
        } else {
            (xf.to_screen(Point::new(250.0, 130.0)), xf.to_screen(Point::new(150.0, 230.0)))
        };
        assert_eq!(frame.contains(from), on_bar, "origin must identify the intended owner");
        assert!(xf.rect.contains(from) && xf.rect.contains(to));
        task_bar_screen_drag(&mut h, from, to);
        let after = crate::crop_ui::frame(h.state());
        if on_bar {
            assert_eq!(after, before, "dragging the Crop bar frame must not edit the crop");
        } else {
            assert_ne!(after.unwrap().angle, before.unwrap().angle, "the off-bar Crop rotation control must still work");
        }
        assert_eq!(history(&h), before_history, "an uncommitted crop drag must not edit document history");
    }
}
