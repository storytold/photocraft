//! #13815: the Fade dialog puts OK, Cancel and Preview in a column right of Opacity, its slider and
//! Mode, shows `%` after the Opacity field, and takes Enter as OK and Escape as Cancel.

use egui::{Key, Rect, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;

use crate::{PhotocraftApp, Services, dialogs::show};

fn open(command: &str) -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    app.run("image.adjustments.invert", json!({})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, command, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(6);
    (h, id)
}

fn rect(h: &Harness<'_, PhotocraftApp>, label: &str) -> Rect {
    h.get_by_label(label).rect()
}

fn spin(h: &Harness<'_, PhotocraftApp>, label: &str) -> Rect {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).expect("the field").rect()
}

fn history(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    h.state().session.active().unwrap().history.entries().iter().map(|e| e.to_string()).collect()
}

fn open_dialog(h: &Harness<'_, PhotocraftApp>, id: u64) -> bool {
    h.state().ui.dialogs.iter().any(|d| d.id == id)
}

#[test]
fn ok_cancel_and_preview_stand_in_a_column_right_of_the_fields() {
    let (h, _) = open("edit.fade");
    let (ok, cancel, preview) = (rect(&h, "OK"), rect(&h, "Cancel"), rect(&h, "Preview"));
    let opacity = spin(&h, "Opacity");
    let mode = h.query_all_by_role(Role::ComboBox).next().expect("the Mode list").rect();
    // One column, top to bottom: OK, Cancel, Preview.
    assert!((ok.left() - cancel.left()).abs() < 1.0, "OK {ok:?} and Cancel {cancel:?} line up");
    assert!(ok.bottom() <= cancel.top(), "OK above Cancel");
    assert!(cancel.bottom() <= preview.top(), "Cancel above Preview");
    // Preview's label starts after its box, inside the buttons' width.
    assert!(preview.left() >= ok.left() && preview.left() < ok.center().x, "Preview under the buttons: {ok:?} {preview:?}");
    // The column sits right of the fields, starting at the top.
    for field in [opacity, mode] {
        assert!(field.right() < ok.left(), "{field:?} is left of the buttons");
    }
    assert!(ok.top() <= opacity.bottom(), "OK starts beside Opacity, not under Mode");
    assert!(opacity.bottom() <= mode.top(), "Opacity above Mode");
}

#[test]
fn a_percent_sign_follows_the_opacity_field() {
    let (h, _) = open("edit.fade");
    let field = spin(&h, "Opacity");
    // The unit is painted right of the number, in the field's box.
    let percent = h.output().shapes.iter().any(|c| match &c.shape {
        egui::Shape::Text(t) => {
            let at = t.pos + t.galley.rect.center().to_vec2();
            t.galley.text() == "%" && at.x > field.center().x && at.x < field.right() + 24.0 && (field.top()..=field.bottom()).contains(&at.y)
        }
        _ => false,
    });
    assert!(percent, "`%` after the Opacity field {field:?}");
}

#[test]
fn enter_is_ok_and_escape_is_cancel() {
    let (mut h, id) = open("edit.fade");
    let before = history(&h);
    h.key_press(Key::Escape);
    h.run_steps(4);
    assert!(!open_dialog(&h, id), "Escape closes the dialog");
    assert_eq!(history(&h), before, "and fades nothing");

    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.fade", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(6);
    h.key_press(Key::Enter);
    h.run_steps(6);
    assert!(!open_dialog(&h, id), "Enter closes the dialog");
    assert!(history(&h).last().is_some_and(|e| e.starts_with("Fade")), "and fades: {:?}", history(&h));
}

#[test]
fn other_filter_dialogs_keep_their_buttons_under_the_fields() {
    let (h, _) = open("filter.blur.gaussianBlur");
    let (ok, cancel, preview) = (rect(&h, "OK"), rect(&h, "Cancel"), rect(&h, "Preview"));
    assert!((ok.top() - cancel.top()).abs() < 1.0, "OK and Cancel share a row");
    assert!(preview.bottom() <= ok.top(), "Preview above the button row");
}
