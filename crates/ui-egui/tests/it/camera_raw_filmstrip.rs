//! The open-time Camera Raw dialog's filmstrip through the real app shell: clicks with Shift and
//! ⌘ select raws, ⌘A selects all of them and never the document behind the dialog.
use egui::{Key, Modifiers, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_ui_egui::{PhotocraftApp, Services, theme::ThemeKind};
use serde_json::json;

const NAMES: [&str; 3] = ["IMG_0.dng", "IMG_1.dng", "IMG_2.dng"];

/// The app with the real importer, three synthetic raws opened together and the dialog up.
fn fixture() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1200.0, 800.0)).build_eframe(|cc| {
        use photocraft_raw::testgen::{DngSpec, mosaic, scene};
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let (w, h) = (48, 32);
        let dng = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000)).build();
        for name in NAMES {
            app.open_bytes(name, &dng).unwrap();
        }
        app
    });
    h.run_steps(4);
    h
}

fn selected(h: &Harness<'_, PhotocraftApp>) -> Vec<bool> {
    NAMES.iter().map(|n| h.get_by_label(n).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)).collect()
}

fn no_document_selection(h: &Harness<'_, PhotocraftApp>) -> bool {
    h.state().session.documents().iter().all(|st| st.doc.selection.is_none())
}

#[test]
fn filmstrip_clicks_and_command_a_select_raws_not_the_document() {
    let mut h = fixture();
    assert_eq!(selected(&h), [true, false, false], "the shown raw is selected");
    h.get_by_label(NAMES[2]).click_modifiers(Modifiers::SHIFT);
    h.run_steps(3);
    assert_eq!(selected(&h), [true, true, true], "Shift-click selects the range");
    h.get_by_label(NAMES[1]).click_modifiers(Modifiers::COMMAND);
    h.run_steps(3);
    assert_eq!(selected(&h), [true, false, true], "⌘-click takes one out");
    h.get_by_label(NAMES[1]).click();
    h.run_steps(3);
    assert_eq!(selected(&h), [false, true, false], "a click selects only that raw");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(3);
    assert_eq!(selected(&h), [true, true, true], "⌘A selects every raw in the filmstrip");
    assert!(no_document_selection(&h), "and nothing in the document behind the dialog");
    // Without the dialog, ⌘A is the document's Select All.
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().session.documents().is_empty(), "Cancel closed the raws");
    h.state_mut().run("file.new", json!({"width": 16, "height": 16})).unwrap();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    assert!(!no_document_selection(&h));
}
