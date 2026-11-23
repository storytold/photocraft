//! A family installed while the app runs, picked from the Type tool's font menu.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::{PhotocraftApp, Services};

/// JetBrains Mono renamed to `family` in its name table (ASCII and UTF-16BE records; `family` has the same length).
fn renamed_font(family: &str) -> Vec<u8> {
    let mut bytes = photocraft_text::fonts::JETBRAINS_MONO_REGULAR.clone();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(b"JetBrains Mono".to_vec(), family.as_bytes().to_vec()), (utf16("JetBrains Mono"), utf16(family))] {
        let mut i = 0;
        while let Some(at) = bytes[i..].windows(from.len()).position(|w| w == from) {
            bytes[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    bytes
}

/// A pointer press and release at the centre of the row labelled `label`, as a mouse click.
fn click_row(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str) {
    let at = h.get_by_role_and_label(Role::TreeItem, label).rect().center();
    h.hover_at(at);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    h.run();
}

fn layer_font(app: &PhotocraftApp) -> String {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => t.font_family.clone(),
        _ => panic!("type layer"),
    }
}

#[test]
fn a_family_installed_while_the_menu_is_open_is_picked_on_the_first_click() {
    let family = "Zetbrains Pick";
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
    app.ui.tool = crate::state::Tool::Type;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
        app
    });
    h.run_steps(4);
    h.query_all_by_label("Font family").next().expect("the family field").click();
    h.run();
    h.query_all_by_label("Font family").next().expect("the family field").type_text("Zetbrains");
    h.run();

    // The launcher installs the family into the store while the list is open.
    let dir = std::env::temp_dir().join(format!("photocraft-pick-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ZetbrainsPick-Regular.ttf");
    std::fs::write(&path, renamed_font(family)).unwrap();
    photocraft_text::fonts::catalog().add_file(&path, craft::font_catalog::Source::Store, &craft::font_catalog::Limits::default()).unwrap();
    super::follow_catalog(h.state_mut());
    h.run();

    click_row(&mut h, family);
    assert_eq!(layer_font(h.state()), family);
    let _ = std::fs::remove_dir_all(&dir);
}
