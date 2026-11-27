//! The Color Picker dialog has no screen eyedropper button, as in the reference application (#17017): whichever chip
//! opens it, it samples by a click on the canvas only.

use craft::color_ui::ui::StillCapture;
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};

use super::*;

const SCREEN_BUTTON: &str = "Sample color from the screen";

/// The whole app with a screen capture service, so the platform could sample the screen and the button would show.
fn app_harness(setup: impl FnOnce(&mut photocraft_engine::Session) + 'static) -> Harness<'static, Box<PhotocraftApp>> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":160,"height":120})).unwrap();
    setup(&mut session);
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), session, Default::default());
        app.services.screen_capture = Some(std::sync::Arc::new(|| Box::new(StillCapture::color(Srgba::rgb(1.0, 136.0 / 255.0, 0.0)))));
        app
    });
    h.run_steps(8);
    h
}

/// The picker is drawn in its full layout (Only Web Colors under the field) and has no screen eyedropper button.
fn assert_no_screen_button(h: &Harness<'static, Box<PhotocraftApp>>, opener: &str) {
    assert!(h.query_by_label("Only Web Colors").is_some(), "{opener}: the full Color Picker is shown");
    assert!(h.query_by_label(SCREEN_BUTTON).is_none(), "{opener}: the Color Picker shows a screen eyedropper button");
}

#[test]
fn foreground_and_background_pickers_have_no_screen_button() {
    for target in ["foreground", "background"] {
        let mut h = app_harness(|_| {});
        open(h.state_mut(), target);
        h.run_steps(6);
        assert_no_screen_button(&h, target);
    }
}

/// The Type options bar's swatch opens the full picker for `type.setStyle`; OK still applies the picked colour.
#[test]
fn text_swatch_picker_has_no_screen_button_and_ok_applies() {
    let layer = std::rc::Rc::new(std::cell::Cell::new(0));
    let created = layer.clone();
    let mut h = app_harness(move |session| {
        let reply = session.execute("type.create", json!({"text":"Hello world","color":"#000000","size":24,"x":10,"y":60})).unwrap();
        created.set(reply["layer"].as_u64().unwrap());
    });
    h.state_mut().ui.tool = crate::state::Tool::Type;
    h.run_steps(4);
    h.get_all_by_role_and_label(egui::accesskit::Role::ColorWell, "Text color").next().expect("the text swatch").click();
    h.run_steps(6);
    let picker = top(h.state()).expect("the text swatch opens the Color Picker");
    assert!(matches!(target(h.state(), picker), Some(PickTarget::Command { command, .. }) if command == "type.setStyle"));
    assert_no_screen_button(&h, "text swatch");
    testing::set_color(h.state_mut(), picker, "#ff8800");
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(4);
    let doc = &h.state().session.active().unwrap().doc;
    let photocraft_doc::LayerContent::Text(text) = &doc.layer(photocraft_doc::LayerId(layer.get())).unwrap().content else { panic!("not text") };
    assert_eq!(text.color.to_rgba8(), [255, 136, 0, 255]);
}

#[test]
fn layer_style_picker_has_no_screen_button() {
    let mut h = app_harness(|_| {});
    let parent = crate::layer_style::open(h.state_mut(), Some("colorOverlay")).expect("the Layer Style dialog opens");
    h.run_steps(4);
    crate::dialogs::layer_style::color_picker::open(h.state_mut(), parent.value(), "fx1", "color", None).expect("the picker opens");
    h.run_steps(6);
    assert_no_screen_button(&h, "Layer Style");
}

#[test]
fn text_style_picker_has_no_screen_button() {
    let style = std::rc::Rc::new(std::cell::Cell::new(0));
    let created = style.clone();
    let mut h = app_harness(move |session| {
        let layer = session.execute("type.create", json!({"x":10,"y":55,"text":"Shared color","size":30})).unwrap()["layer"].as_u64().unwrap();
        let id = session.execute("type.characterStyle.new", json!({"fromSelection":false,"attrs":{"color":"#ff0000"}})).unwrap()["id"].as_u64().unwrap();
        session.execute("type.characterStyle.apply", json!({"id":id,"layer":layer})).unwrap();
        created.set(u32::try_from(id).unwrap());
    });
    let kind = crate::type_panels_ui::color_picker::Kind::Character;
    crate::type_panels_ui::color_picker::open(h.state_mut(), kind, style.get()).expect("the text style picker opens");
    h.run_steps(6);
    assert_no_screen_button(&h, "Character Style Options");
}
