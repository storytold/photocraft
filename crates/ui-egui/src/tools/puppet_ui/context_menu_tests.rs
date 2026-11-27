//! #16734: a right-click in Edit › Puppet Warp opens the pin menu. On the mesh away from the pins it offers Add Pin;
//! on a pin it selects the pin and lists Delete Pin, Set Auto Rotation, Reset Depth, Bring Forward and Send Backward;
//! both end with Remove All Pins, Select All Pins and Hide Mesh / Show Mesh. Off the mesh it opens nothing.

use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::canvas::testing::to_screen;
use crate::canvas_tool_menu::tests::{harness, right_click};

/// A 400×300 document with an opaque block from (50,50) to (350,250), Puppet Warp open on it and a pin placed and held
/// at each of `pins`.
fn puppet(pins: &[[f64; 2]]) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(50, 50, 350, 250), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx, "edit.puppetWarp").unwrap();
    for &at in pins {
        let s = app.distort.puppet.as_ref().unwrap();
        let rest = s.solver.mesh().rest_of(point(at)).expect("on the mesh");
        let before = s.warp.pins().to_vec();
        let mut after = before.clone();
        after.push(Pin::new(rest, point(at)));
        session_apply(&mut app, PinEdit::replace(&before, after, "Add Pin")).unwrap();
    }
    app
}

fn session(app: &PhotocraftApp) -> &PuppetSession {
    app.distort.puppet.as_ref().expect("Puppet Warp stays open")
}

fn pins_of(app: &PhotocraftApp) -> Vec<Pin> {
    session(app).warp.pins().to_vec()
}

fn past_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// A menu's rows as labels, "---" for a separator, and the labels of the greyed ones.
fn labels(rows: &Value) -> (Vec<String>, Vec<String>) {
    let rows = rows.as_array().expect("rows");
    let label = |row: &Value| if row["separator"] == true { "---".to_string() } else { row["label"].as_str().unwrap().to_string() };
    let greyed = rows.iter().filter(|row| row["enabled"] == false).map(label).collect();
    (rows.iter().map(label).collect(), greyed)
}

/// The rows of the menu the control channel opens with `menu`.
fn menu(app: &mut PhotocraftApp, menu: Value) -> (Vec<String>, Vec<String>) {
    labels(&control(app, &json!({"menu": menu})).unwrap()["menu"])
}

/// Picks row `id` of the menu `menu` names, through the control channel.
fn pick(app: &mut PhotocraftApp, mut menu: Value, id: &str) -> Result<Value, String> {
    menu["pick"] = json!(id);
    control(app, &json!({"menu": menu}))
}

/// The rows of the menu open on the canvas.
fn open_rows(h: &Harness<'static, Box<PhotocraftApp>>) -> Option<(Vec<String>, Vec<String>)> {
    session(h.state()).menu.map(|_| labels(&session(h.state()).describe()["menu"]))
}

fn right_click_doc(h: &mut Harness<'static, Box<PhotocraftApp>>, at: [f64; 2]) {
    let p = to_screen(h.state(), at);
    right_click(h, p, Modifiers::NONE);
}

const PIN_ROWS: [&str; 6] = ["Delete Pin", "Set Auto Rotation", "Reset Depth", "---", "Bring Forward", "Send Backward"];
const SHARED_ROWS: [&str; 4] = ["---", "Remove All Pins", "Select All Pins", "Hide Mesh"];

/// A right-click on the mesh with no pin there opens the mesh menu and no canvas tool menu; Add Pin puts a pin
/// exactly where the right-click was, one session step and no document step.
#[test]
fn right_click_on_the_mesh_offers_add_pin_and_it_lands_at_the_click() {
    let mut h = harness(puppet(&[]));
    let past = past_len(h.state());
    let clicked = [150.0, 120.0];
    right_click_doc(&mut h, clicked);
    let (rows, greyed) = open_rows(&h).expect("the pin menu opened");
    assert_eq!(rows, [&["Add Pin"][..], &SHARED_ROWS].concat());
    assert_eq!(greyed, ["Remove All Pins", "Select All Pins"], "no pin: nothing to remove or select");
    assert!(h.state().ui.canvas_tool_menu.is_none(), "the tool's own menu stays shut");

    h.get_by_label("Add Pin").click();
    h.run_steps(2);
    assert!(session(h.state()).menu.is_none(), "choosing a row closes the menu");
    let pins = pins_of(h.state());
    assert_eq!(pins.len(), 1);
    let off = (pins[0].at - point(clicked)).hypot();
    assert!(off < 1.0, "the pin lands at the right-click point, {off} px away");
    assert_eq!(past_len(h.state()), past, "pin edits stay in the session");
    assert!(undo(h.state_mut()));
    assert!(pins_of(h.state()).is_empty(), "Add Pin is one session step");
}

/// A right-click on a pin selects it and lists the pin's rows above the shared ones. An Auto pin greys Set Auto
/// Rotation; with two pins nothing else is greyed.
#[test]
fn right_click_on_a_pin_selects_it_and_lists_the_pin_rows() {
    let mut h = harness(puppet(&[[100.0, 100.0], [300.0, 200.0]]));
    assert_eq!(session(h.state()).warp.selected(), [1], "the placed pin is selected");
    right_click_doc(&mut h, [100.0, 100.0]);
    let (rows, greyed) = open_rows(&h).expect("the pin menu opened");
    assert_eq!(rows, [&PIN_ROWS[..], &SHARED_ROWS].concat());
    assert_eq!(greyed, ["Set Auto Rotation"]);
    assert_eq!(session(h.state()).warp.selected(), [0], "the right-clicked pin is selected");
}

/// Off the mesh a right-click opens no pin menu and Puppet Warp stays open; the tool's own menu opens there as before.
/// Escape closes an open pin menu and leaves the session and its pins as they were.
#[test]
fn right_click_off_the_mesh_opens_no_pin_menu_and_escape_only_closes_the_menu() {
    let mut h = harness(puppet(&[[100.0, 100.0]]));
    right_click_doc(&mut h, [380.0, 20.0]);
    assert!(session(h.state()).menu.is_none(), "no pin menu off the mesh");
    assert!(h.state().ui.canvas_tool_menu.is_some(), "the Move tool's menu opens off the mesh, as it did");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none());

    let pins = pins_of(h.state());
    right_click_doc(&mut h, [200.0, 150.0]);
    assert!(session(h.state()).menu.is_some(), "the menu opens on the mesh");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(session(h.state()).menu.is_none(), "Escape closes the menu");
    assert_eq!(pins_of(h.state()), pins, "and changes nothing");
}

/// The open menu takes the keys: Enter chooses the focused row and does not commit the warp, as Enter on the pins does.
#[test]
fn enter_in_the_menu_chooses_the_row_and_keeps_the_session() {
    let mut h = harness(puppet(&[]));
    right_click_doc(&mut h, [150.0, 120.0]);
    assert!(session(h.state()).menu.is_some(), "the menu opened");
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().distort.puppet.is_some(), "Enter in the menu does not commit Puppet Warp");
    assert!(session(h.state()).menu.is_none(), "Enter chooses a row and closes the menu");
    assert_eq!(pins_of(h.state()).len(), 1, "the focused row, Add Pin, ran");
}

/// Remove All Pins needs a pin and Select All Pins two; a fixed pin among two greys nothing. A greyed row is refused.
#[test]
fn greyed_rows_follow_the_pins() {
    let mut app = puppet(&[[100.0, 100.0]]);
    let (_, greyed) = menu(&mut app, json!({"at": [200.0, 150.0]}));
    assert_eq!(greyed, ["Select All Pins"], "one pin: only Select All Pins is greyed on the mesh");
    let (_, greyed) = menu(&mut app, json!({"pin": 0}));
    assert_eq!(greyed, ["Set Auto Rotation", "Select All Pins"]);
    assert!(pick(&mut app, json!({"pin": 0}), "selectAllPins").is_err(), "a greyed row is refused");

    let mut app = puppet(&[[100.0, 100.0], [300.0, 200.0]]);
    let mut pins = pins_of(&app);
    pins[0].angle = Some(77.0);
    let before = pins_of(&app);
    session_apply(&mut app, PinEdit::replace(&before, pins, "Rotate")).unwrap();
    let (_, greyed) = menu(&mut app, json!({"pin": 0}));
    assert!(greyed.is_empty(), "a fixed pin among two: nothing greyed, got {greyed:?}");
    assert!(control(&mut app, &json!({"menu": {"at": [390.0, 10.0]}})).is_err(), "no menu off the mesh");
}

/// Each pin row changes what the bar's control or the key changes, as one session step; Hide Mesh turns the bar's Show
/// Mesh off and reads Show Mesh after.
#[test]
fn rows_do_what_the_bar_and_keys_do() {
    let mut app = puppet(&[[100.0, 100.0], [300.0, 200.0]]);
    let mut pins = pins_of(&app);
    pins[0].angle = Some(77.0);
    pins[0].depth = 2;
    let fixed = pins_of(&app);
    session_apply(&mut app, PinEdit::replace(&fixed, pins.clone(), "Rotate")).unwrap();
    let on_pin = json!({"pin": 0});

    pick(&mut app, on_pin.clone(), "setAutoRotation").unwrap();
    assert_eq!(pins_of(&app)[0].angle, None, "Set Auto Rotation frees the rotation");
    assert!(undo(&mut app));
    assert_eq!(pins_of(&app), pins, "one session step");
    assert!(redo(&mut app));

    for (row, depth) in [("resetDepth", 0), ("bringForward", 1), ("bringForward", 2), ("sendBackward", 1)] {
        pick(&mut app, on_pin.clone(), row).unwrap();
        assert_eq!(pins_of(&app)[0].depth, depth, "{row}");
    }

    pick(&mut app, on_pin.clone(), "selectAllPins").unwrap();
    assert_eq!(session(&app).warp.selected(), [0, 1]);
    let all = pins_of(&app);
    pick(&mut app, json!({"pin": 1}), "deletePin").unwrap();
    assert!(pins_of(&app).is_empty(), "Delete Pin removes the selected pins, as Delete does");
    assert!(undo(&mut app));
    assert_eq!(pins_of(&app), all);
    session_apply(&mut app, PinEdit::Select(vec![])).unwrap();
    pick(&mut app, json!({"pin": 1}), "deletePin").unwrap();
    assert_eq!(pins_of(&app), all[..1], "the right-clicked pin is the one deleted");

    pick(&mut app, on_pin.clone(), "removeAllPins").unwrap();
    assert!(pins_of(&app).is_empty());

    assert!(session(&app).warp.show_mesh());
    pick(&mut app, json!({"at": [200.0, 150.0]}), "toggleMesh").unwrap();
    assert!(!session(&app).warp.show_mesh(), "Hide Mesh unticks Show Mesh");
    let (rows, _) = menu(&mut app, json!({"at": [200.0, 150.0]}));
    assert_eq!(rows.last().map(String::as_str), Some("Show Mesh"));
}
