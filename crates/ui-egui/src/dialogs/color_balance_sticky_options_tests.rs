use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const COMMAND: &str = "image.adjustments.colorBalance";

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    session.execute("edit.fill", json!({"color": "#b04020"})).unwrap();
    let h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), PhotocraftApp::new(session, crate::Services::default()));
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h
}

fn show(h: &mut Harness<'static, PhotocraftApp>, command: &str) -> u64 {
    let id = open(h.state_mut(), command).unwrap();
    h.run_steps(3);
    id
}

fn option(h: &Harness<'static, PhotocraftApp>, id: u64, key: &str) -> bool {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields[key].as_bool().unwrap()
}

fn click(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

#[test]
fn preserve_luminosity_only_remembers_ok_even_when_it_creates_no_history() {
    let mut h = harness();
    let before = h.state().session.active().unwrap();
    let document = before.doc.clone();
    let before_revision = before.revision;
    let before_history = before.history.past_len();

    let id = show(&mut h, COMMAND);
    assert!(option(&h, id, "preserveLuminosity"));
    assert!(option(&h, id, "__preview"));
    click(&mut h, "Preserve Luminosity");
    assert!(!option(&h, id, "preserveLuminosity"));
    click(&mut h, "OK");

    let id = show(&mut h, COMMAND);
    assert!(!option(&h, id, "preserveLuminosity"));
    click(&mut h, "Preserve Luminosity");
    click(&mut h, "Cancel");

    let id = show(&mut h, COMMAND);
    assert!(!option(&h, id, "preserveLuminosity"));
    click(&mut h, "Preserve Luminosity");
    click(&mut h, "OK");

    let after = h.state().session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&after.doc, &document));
    assert_eq!(after.revision, before_revision);
    assert_eq!(after.history.past_len(), before_history);
    let id = show(&mut h, COMMAND);
    assert!(option(&h, id, "preserveLuminosity"));
}

#[test]
fn preview_remembers_escape_controls_the_canvas_and_stays_local_to_color_balance() {
    let mut h = harness();
    let id = show(&mut h, COMMAND);
    click(&mut h, "Preview");
    assert!(!option(&h, id, "__preview"));
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());

    let other = show(&mut h, "image.adjustments.photoFilter");
    assert!(option(&h, other, "__preview"));
    click(&mut h, "Cancel");

    let id = show(&mut h, COMMAND);
    assert!(!option(&h, id, "__preview"));
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("midtones".into(), json!([75, 0, 0]));
    let idx = h.state().session.active_index().unwrap();
    assert!(crate::adjust_preview::display_doc(h.state_mut(), idx).is_none());
    click(&mut h, "Preview");
    assert!(option(&h, id, "__preview"));
    assert!(crate::adjust_preview::display_doc(h.state_mut(), idx).is_some());
    click(&mut h, "Cancel");

    let id = show(&mut h, COMMAND);
    assert!(option(&h, id, "__preview"));
    assert_eq!(h.state().ui.dialogs[0].fields["midtones"], json!([0.0, 0.0, 0.0]));
}
