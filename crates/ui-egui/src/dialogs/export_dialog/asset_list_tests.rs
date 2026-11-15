//! #13736: Export As lists what it exports. Select All and the document's one row (checkbox,
//! thumbnail, name, format, output size, estimate) start ticked; the row follows the settings;
//! nothing ticked greys Export. The estimate reads `N bytes` below 1 KB, else KB with at most one
//! decimal.

use super::*;
use egui::accesskit::{Role, Toggled};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::{Color, Layer, SampleType, Size};

#[test]
fn the_estimate_reads_bytes_then_kb_with_one_decimal() {
    assert_eq!(crate::sizing::estimate_text(556.0), "556 bytes");
    assert_eq!(crate::sizing::estimate_text(1023.0), "1023 bytes");
    assert_eq!(crate::sizing::estimate_text(1024.0), "1 KB");
    assert_eq!(crate::sizing::estimate_text(7.0 * 1024.0), "7 KB");
    assert_eq!(crate::sizing::estimate_text(5939.0), "5.8 KB");
    assert_eq!(crate::sizing::estimate_text(7577.0), "7.4 KB");
    let mb = 2.5 * 1024.0 * 1024.0;
    assert_eq!(crate::sizing::estimate_text(mb), crate::sizing::human_bytes(mb), "1 MB and up keep today's text");
    assert_eq!(crate::sizing::human_bytes(556.0), "1K", "human_bytes itself is unchanged");
}

#[test]
fn only_export_as_is_greyed_by_its_asset_list() {
    assert!(can_confirm(&Map::new()), "other dialogs are not affected");
    let mut f = json!({"__export": true}).as_object().unwrap().clone();
    assert!(can_confirm(&f), "ticked when the dialog opens");
    f.insert("__asset".into(), json!(false));
    assert!(!can_confirm(&f));
}

/// 200 x 150 RGB/8 named `Rows.psd`, with a second layer selected.
fn harness() -> Harness<'static, PhotocraftApp> {
    Harness::builder().with_size(egui::vec2(1100.0, 1000.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut doc = Document::with_background("Rows.psd", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let fmt = doc.pixel_format();
        let layer = Layer::raster("Second", fmt);
        let id = layer.id;
        doc.layers.push(layer);
        app.session.add_document(doc, None);
        app.session.active_mut().unwrap().active_layer = Some(id);
        app
    })
}

fn open_dialog(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(6);
}

fn ticked(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::CheckBox, label).accesskit_node().toggled() == Some(Toggled::True)
}

fn export_enabled(h: &Harness<'_, PhotocraftApp>) -> bool {
    !h.get_by_role_and_label(Role::Button, "Export").accesskit_node().is_disabled()
}

fn click(h: &mut Harness<'_, PhotocraftApp>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run_steps(6);
}

/// A label node's text (egui puts a label's text in the node's value).
fn node_text(n: &egui_kittest::Node<'_>) -> String {
    let node = n.accesskit_node();
    node.value().or_else(|| node.label()).unwrap_or_default().to_string()
}

/// Steps until the asset row shows an estimate (the exact one comes from a worker thread).
fn row_text(h: &mut Harness<'_, PhotocraftApp>, starts: &str) -> String {
    for _ in 0..200 {
        h.run_steps(1);
        let text = h.query_all_by_label_contains(starts).map(|n| node_text(&n)).find(|t| !t.ends_with('—'));
        if let Some(text) = text.filter(|t| t.ends_with("bytes") || t.ends_with("KB")) {
            return text;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("no estimate for {starts}");
}

#[test]
fn one_ticked_row_follows_the_settings_and_unticking_greys_export() {
    let mut h = harness();
    open_dialog(&mut h);
    assert!(ticked(&h, "Select All") && ticked(&h, "Rows"), "both ticked at open");
    assert_eq!(
        h.query_all_by_role(Role::CheckBox).filter(|n| n.accesskit_node().label().is_some_and(|l| l == "Rows" || l == "Second")).count(),
        1,
        "one row, for the document, with a layer selected"
    );
    let png = row_text(&mut h, "PNG  200 x 150  ");
    assert!(png.ends_with("bytes") || png.ends_with("KB"), "{png}");

    click(&mut h, Role::ComboBox, "Format");
    h.get_by_label("JPG").click();
    h.run_steps(6);
    row_text(&mut h, "JPG  200 x 150  ");
    h.get_by_role_and_label(Role::SpinButton, "Scale").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    for ch in "50".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    row_text(&mut h, "JPG  100 x 75  ");

    assert!(export_enabled(&h));
    click(&mut h, Role::CheckBox, "Rows");
    assert!(!ticked(&h, "Select All") && !export_enabled(&h), "nothing ticked greys Export");
    click(&mut h, Role::CheckBox, "Select All");
    assert!(ticked(&h, "Rows") && export_enabled(&h));
    click(&mut h, Role::CheckBox, "Select All");
    assert!(!export_enabled(&h), "Select All off greys Export too");
}
