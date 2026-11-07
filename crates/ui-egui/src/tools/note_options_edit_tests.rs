//! #11790: with a note selected, the Note tool's Author and Color edit that note ("Note Author",
//! "Note Color") and leave the tool's defaults alone; with none selected they set the defaults
//! for new notes, and the bar shows the defaults again once nothing is selected.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

const BLUE: [f32; 3] = [0.25, 0.5, 1.0];

/// A 200 × 100 document with one note (no author, blue) at 20,20, and the Note tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("notes.add", json!({"x": 20, "y": 20, "author": "", "color": BLUE})).unwrap();
    app.ui.tool = Tool::Note;
    app
}

fn select(app: &mut PhotocraftApp, i: usize) {
    app.ui.analysis.note_selected = Some(i);
    show_note_options(app, i);
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1200.0, 80.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn note(app: &PhotocraftApp, i: usize) -> photocraft_doc::Note {
    app.session.active().unwrap().doc.notes[i].clone()
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

#[test]
fn enter_in_author_re_authors_the_selected_note() {
    let mut app = app();
    select(&mut app, 0);
    let before = steps(&app);
    let mut h = harness(app);
    h.get_by_label("Author:").click();
    h.run_steps(2);
    h.get_by_label("Author:").type_text("Tester");
    h.run_steps(2);
    assert_eq!(note(h.state(), 0).author, "", "typing alone doesn't commit");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let app = h.state();
    assert_eq!(note(app, 0).author, "Tester");
    assert_eq!(steps(app), before + 1, "one history step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Note Author"));
    assert_eq!(app.ui.analysis.note_default_author, "", "the tool default is untouched");
}

#[test]
fn ok_in_the_picker_recolours_the_selected_note() {
    let mut app = app();
    select(&mut app, 0);
    let before = steps(&app);
    let shown = app.ui.analysis.note_color;
    let id = crate::color_picker_ui::open_for_target(&mut app, "noteColor", "Color", shown);
    app.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#80c0ff"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(close(note(&app, 0).color.to_rgb(), [128.0 / 255.0, 192.0 / 255.0, 1.0]), "{:?}", note(&app, 0).color.to_rgb());
    assert_eq!(steps(&app), before + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Note Color"));
    assert!(close(app.ui.analysis.note_default_color, [1.0, 1.0, 0.51]), "the tool default is untouched");
}

#[test]
fn with_nothing_selected_the_bar_shows_and_sets_the_defaults() {
    let mut app = app();
    // A selected note's values are shown while it is selected …
    select(&mut app, 0);
    assert!(close(app.ui.analysis.note_color, BLUE));
    // … and the defaults come back once nothing is selected.
    app.ui.analysis.note_selected = None;
    let mut h = harness(app);
    assert_eq!(h.state().ui.analysis.note_author, "");
    assert!(close(h.state().ui.analysis.note_color, [1.0, 1.0, 0.51]), "default colour, not the last note's");
    // Typing an Author with nothing selected sets the default, with no history step.
    let before = steps(h.state());
    h.get_by_label("Author:").click();
    h.run_steps(2);
    h.get_by_label("Author:").type_text("Ann");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().ui.analysis.note_default_author, "Ann");
    assert_eq!(steps(h.state()), before, "no history step");
    assert_eq!(note(h.state(), 0).author, "", "no note changed");
}

#[test]
fn the_picker_with_nothing_selected_sets_the_default_colour() {
    let mut app = app();
    let before = steps(&app);
    let shown = app.ui.analysis.note_color;
    let id = crate::color_picker_ui::open_for_target(&mut app, "noteColor", "Color", shown);
    app.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#ff8000"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(close(app.ui.analysis.note_default_color, [1.0, 128.0 / 255.0, 0.0]));
    assert_eq!(steps(&app), before, "no history step");
    assert!(close(note(&app, 0).color.to_rgb(), BLUE), "no note changed");
}

#[test]
fn a_new_note_takes_the_values_shown_in_the_bar() {
    // Made while the blue note is selected: blue, as in Photoshop.
    let mut app = app();
    select(&mut app, 0);
    create_note(&mut app, 120.0, 60.0).unwrap();
    assert!(close(note(&app, 1).color.to_rgb(), BLUE));
}

#[test]
fn notes_set_names_a_single_field_change() {
    let mut app = app();
    for (p, want) in [
        (json!({"index": 0, "author": "A"}), "Note Author"),
        (json!({"index": 0, "color": "#102030"}), "Note Color"),
        (json!({"index": 0, "author": "B", "color": "#203040"}), "Edit Note"),
        (json!({"index": 0, "text": "hi"}), "Edit Note"),
        (json!({"index": 0, "x": 40, "y": 30}), "Move Note"),
    ] {
        app.run("notes.set", p.clone()).unwrap();
        assert_eq!(app.session.active().unwrap().history.undo_label(), Some(want), "{p}");
    }
}
