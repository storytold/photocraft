use craft::ui::dialog::DialogId;
use egui::{Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::*;
use crate::canvas::{ToolEvent, testing, tool_event};
use crate::canvas_tool_menu::GUIDE_CLEAR;
use crate::state::Tool;

type AppHarness = Harness<'static, Box<PhotocraftApp>>;

/// A 200 × 100 document with the Move tool, a vertical guide at 50 and a magenta horizontal one
/// at 30.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
    app.run("view.newGuide", json!({"orientation": "horizontal", "position": 30, "color": "magenta"})).unwrap();
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
}

fn guides(app: &PhotocraftApp) -> (Vec<f64>, Vec<f64>) {
    let g = &app.session.active().unwrap().doc.guides;
    (g.vertical.clone(), g.horizontal.clone())
}

#[test]
fn the_selection_lapses_when_its_guide_goes_or_guides_are_hidden_or_locked() {
    let mut app = app();
    click(&mut app, 50.0, 70.0);
    app.ui.extras.lock_guides = true;
    assert_eq!(selected(&app), None, "locked guides can't be selected");
    app.ui.extras.lock_guides = false;
    app.ui.extras.guides = false;
    assert_eq!(selected(&app), None, "nor hidden ones");
    app.ui.extras.guides = true;
    assert_eq!(selected(&app), Some((true, 0)));
    app.run("view.clearGuides", json!({})).unwrap();
    assert_eq!(selected(&app), None);
    app.session.undo();
    assert_eq!(selected(&app), Some((true, 0)), "undo brings the guide back, still selected");
    // Another document's guide at the same place isn't the selected one.
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
    assert_eq!(selected(&app), None);
}

/// Delete and Backspace on the canvas take the selected guide before Edit › Clear would take the
/// selected pixels.
#[test]
fn delete_removes_the_selected_guide_and_leaves_the_pixels() {
    let mut app = app();
    app.run("select.all", json!({})).unwrap();
    click(&mut app, 120.0, 30.0);
    testing::key(&mut app, Key::Delete, Modifiers::NONE);
    assert_eq!(guides(&app), (vec![50.0], vec![]));
    assert_eq!(selected(&app), None);
    click(&mut app, 50.0, 70.0);
    testing::key(&mut app, Key::Backspace, Modifiers::NONE);
    assert_eq!(guides(&app), (vec![], vec![]));
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo_label(), Some("Delete Guide"), "Edit › Clear didn't run");
    assert!(st.doc.selection.is_some());
    // One undoable step per guide.
    app.session.undo();
    assert_eq!(guides(&app), (vec![50.0], vec![]));
}

/// The canvas with its context menu and the hosted dialogs, as a frame runs them.
fn harness(app: Box<PhotocraftApp>) -> AppHarness {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1000.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            crate::dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::default());
    h.run_steps(4);
    h
}

fn right_click(h: &mut AppHarness, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// A point on the vertical guide at x = 100, below the document's middle.
fn guide_on_screen(h: &AppHarness) -> Pos2 {
    testing::to_screen(h.state(), [100.0, 70.0])
}

fn centred_guide_app() -> Box<PhotocraftApp> {
    let mut app = app();
    app.run("view.clearGuides", json!({})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 100})).unwrap();
    app
}

#[test]
fn guide_menu_rows_clear_the_selected_guide_and_lock_guides() {
    let mut h = harness(centred_guide_app());
    h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 20})).unwrap();
    let at = guide_on_screen(&h);
    right_click(&mut h, at);
    let ctx = h.ctx.clone();
    crate::canvas_tool_menu::choose(h.state_mut(), &ctx, GUIDE_CLEAR);
    assert_eq!(guides(h.state()).0, vec![20.0], "only the right-clicked guide goes");
    h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 100})).unwrap();
    h.run_steps(2);
    let at = guide_on_screen(&h);
    right_click(&mut h, at);
    crate::canvas_tool_menu::choose(h.state_mut(), &ctx, "view.lockGuides");
    assert!(h.state().ui.extras.lock_guides);
    assert_eq!(selected(h.state()), None, "locked guides aren't selected");
}

/// Opens Edit Selected Guides on the host and lets a frame show it.
fn open_edit(h: &mut AppHarness) -> DialogId {
    // PLAN: assumed dialogs-w55's `open_edit` answers the hosted dialog's `DialogId` (base: the legacy `u64` stack id).
    let id = crate::new_guide_ui::open_edit(h.state_mut()).expect("a selected guide");
    h.run_steps(2);
    id
}

/// The fields of the open dialog `id`, as the host lists them.
fn fields(app: &PhotocraftApp, id: DialogId) -> Value {
    let open = app.framework.dialogs.open_dialogs();
    open.into_iter().find(|d| d.id == id).and_then(|d| d.fields).expect("the New Guide dialog is open")
}

/// A colour field's `Srgba` as 8-bit channels.
fn rgb8(color: &Value) -> [u8; 3] {
    ["r", "g", "b"].map(|c| (color[c].as_f64().expect("a channel") * 255.0).round() as u8)
}

fn set_field(h: &mut AppHarness, id: DialogId, field: &str, value: Value) -> Result<(), String> {
    let ctx = h.ctx.clone();
    let answer = h.state_mut().framework.dialogs.set_json_field(&ctx, id, field, value);
    h.run_steps(1);
    answer
}

/// Presses OK on dialog `id`; the next frames' dialog run applies the answer.
fn confirm(h: &mut AppHarness, id: DialogId) {
    crate::dialogs::testing::confirm(h.state_mut(), id).expect("OK is enabled");
    h.run_steps(2);
}

#[test]
fn edit_selected_guides_opens_new_guide_on_the_guide_and_applies_the_edit() {
    let mut app = app();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "percent"})).unwrap();
    select(&mut app, false, 0);
    let mut h = harness(app);
    let id = open_edit(&mut h);
    let f = fields(h.state(), id);
    assert_eq!(f["orientation"].as_str(), Some("horizontal"));
    assert!((f["position"].as_f64().unwrap() - 30.0).abs() < 1e-6, "30 % of the height: {f}");
    assert_eq!(rgb8(&f["color"]), [0xff, 0x00, 0xff], "the guide's own magenta");
    // Turn it vertical at 30 % of the width; the colour isn't touched, so it keeps its own.
    set_field(&mut h, id, "orientation", json!("vertical")).unwrap();
    set_field(&mut h, id, "position", json!(30.0)).unwrap();
    confirm(&mut h, id);
    assert_eq!(guides(h.state()), (vec![50.0, 60.0], vec![]));
    let g = &h.state().session.active().unwrap().doc.guides;
    assert_eq!((g.color(true, 60.0), g.color(false, 30.0)), (Some([0xff, 0x00, 0xff]), None));
    assert_eq!(selected(h.state()), Some((true, 1)), "the edited guide stays selected");
}

/// Types `text` into the dialog's Position field and leaves it with Tab, which commits the text.
fn type_position(h: &mut AppHarness, text: &str) {
    h.get_by_role(egui::accesskit::Role::TextInput).click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_role(egui::accesskit::Role::TextInput).type_text(text);
    h.run_steps(1);
    h.key_press(Key::Tab);
    h.run_steps(2);
}

/// #10440: Edit Selected Guides' position takes arithmetic as New Guide's does, a leading bare
/// number in the ruler unit.
#[test]
fn edit_selected_guides_takes_arithmetic_in_the_position() {
    let mut app = app();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "percent"})).unwrap();
    select(&mut app, true, 0);
    let mut h = harness(app);
    let id = open_edit(&mut h);
    // "30 + 10" with percent rulers is 40 % of the 200 px width. Arithmetic is the Position field's, so it is typed:
    // the host's `position` field takes a number only.
    type_position(&mut h, "30 + 10");
    assert!((fields(h.state(), id)["position"].as_f64().unwrap() - 40.0).abs() < 1e-6);
    confirm(&mut h, id);
    assert_eq!(guides(h.state()), (vec![80.0], vec![30.0]));
    assert_eq!(selected(h.state()), Some((true, 0)), "the edited guide stays selected");
    // A unit on the first operand wins: 100 % of the width less 50 px.
    let id = open_edit(&mut h);
    type_position(&mut h, "100% - 50px");
    confirm(&mut h, id);
    assert_eq!(guides(h.state()), (vec![150.0], vec![30.0]));
}

#[test]
fn editing_an_uncoloured_guide_keeps_the_preference_colour_unless_one_is_chosen() {
    let mut app = app();
    select(&mut app, true, 0);
    let mut h = harness(app);
    let id = open_edit(&mut h);
    // The default preference colour is Cyan's, so a guide dragged from a ruler shows as Cyan.
    assert_eq!(rgb8(&fields(h.state(), id)["color"]), [0x4a, 0xff, 0xff]);
    // A refused position leaves the dialog open and the guide untouched.
    assert!(set_field(&mut h, id, "position", json!("far")).is_err());
    assert_eq!(guides(h.state()).0, vec![50.0]);
    assert_eq!(fields(h.state(), id)["orientation"].as_str(), Some("vertical"), "still open");
    // OK with the colour untouched keeps the guide uncoloured: it follows the preference.
    confirm(&mut h, id);
    assert_eq!(h.state().session.active().unwrap().doc.guides.color(true, 50.0), None);
    // A chosen colour becomes the guide's own.
    let id = open_edit(&mut h);
    set_field(&mut h, id, "color", json!({"r": 1.0, "g": 1.0, "b": 0.0, "a": 1.0})).unwrap();
    confirm(&mut h, id);
    assert_eq!(h.state().session.active().unwrap().doc.guides.color(true, 50.0), Some([0xff, 0xff, 0x00]));
    // A preference colour that isn't a named one shows as itself (Custom…).
    h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 120})).unwrap();
    h.state_mut().run("prefs.set", json!({"path": "guidesGridAndSlices.guideColor", "value": "#123456"})).unwrap();
    select(h.state_mut(), true, 1);
    let id = open_edit(&mut h);
    assert_eq!(rgb8(&fields(h.state(), id)["color"]), [0x12, 0x34, 0x56]);
}

#[test]
fn edit_selected_guides_needs_a_selected_guide() {
    let mut h = harness(app());
    assert!(crate::new_guide_ui::open_edit(h.state_mut()).is_err());
    h.run_steps(1);
    assert!(h.state().framework.dialogs.open_dialogs().is_empty(), "no dialog opens on the host");
}

/// ⌥-click turns a guide through the point clicked, keeping its colour, as one undoable step, and
/// leaves it selected; a locked guide can't be turned.
#[test]
fn alt_click_turns_a_guide_through_the_point_clicked() {
    let mut app = app();
    let alt = Modifiers::ALT;
    tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 30.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 30.0 }, alt);
    assert_eq!(guides(&app), (vec![50.0, 120.0], vec![]), "the magenta horizontal guide is now vertical at x = 120");
    let g = &app.session.active().unwrap().doc.guides;
    assert_eq!(g.color(true, 120.0), Some([0xff, 0x00, 0xff]));
    assert_eq!(selected(&app), Some((true, 1)));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Edit Guide"));
    app.session.undo();
    assert_eq!(guides(&app), (vec![50.0], vec![30.0]));
    app.ui.extras.lock_guides = true;
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 70.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 70.0 }, alt);
    assert_eq!(guides(&app), (vec![50.0], vec![30.0]), "locked guides stay put");
}
