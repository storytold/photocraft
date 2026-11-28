//! Window › Arrange: measured entry order, separators, and dynamic "New Window for <doc>" label (#16888).

use serde_json::json;

use super::*;

#[cfg(not(target_arch = "wasm32"))]
fn new_doc(app: &mut PhotocraftApp, name: &str) {
    app.run("file.new", json!({"width": 64, "height": 64, "name": name})).unwrap();
}

/// The `Window › Arrange` entries returned by `ui.menu.list` as `(id, label, shortcut)`.
#[cfg(not(target_arch = "wasm32"))]
fn arrange_list(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<(String, String, Option<String>)> {
    let (req, _rx) = crate::control::ControlRequest::new("ui.menu.list", json!({}));
    let reply = match crate::control::handle(app, ctx, &req) {
        crate::control::Reply::Done(v) => v,
        _ => panic!("ui.menu.list: expected an immediate reply"),
    };
    reply["result"]
        .as_array()
        .expect("ui.menu.list returns an array")
        .iter()
        .filter(|it| it["path"] == json!(["Window", "Arrange"]))
        .map(|it| {
            let label = it["label"].as_str().unwrap_or_default().to_string();
            let id = if label == "---" { "---".to_string() } else { it["id"].as_str().unwrap_or_default().to_string() };
            let shortcut = it["shortcut"].as_str().map(str::to_string);
            (id, label, shortcut)
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn arrange_menu_matches_measured_order_and_active_document_label() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());

    // Without a document open, the final entry keeps its plain fallback label.
    let empty = arrange_list(&mut app, &ctx);
    assert_eq!(empty.last().map(|(id, label, _)| (id.as_str(), label.as_str())), Some(("window.arrange.newWindowForDocument", "New Window")));

    new_doc(&mut app, "W1");
    let got = arrange_list(&mut app, &ctx);
    let expected: &[(&str, &str)] = &[
        ("window.arrange.tileAllVertically", "Tile All Vertically"),
        ("window.arrange.tileAllHorizontally", "Tile All Horizontally"),
        ("window.arrange.twoUpHorizontal", "2-up Horizontal"),
        ("window.arrange.twoUpVertical", "2-up Vertical"),
        ("window.arrange.threeUpHorizontal", "3-up Horizontal"),
        ("window.arrange.threeUpVertical", "3-up Vertical"),
        ("window.arrange.threeUpStacked", "3-up Stacked"),
        ("window.arrange.fourUp", "4-up"),
        ("window.arrange.sixUp", "6-up"),
        ("window.arrange.consolidateAllToTabs", "Consolidate All to Tabs"),
        ("---", "---"),
        ("window.arrange.cascade", "Cascade"),
        ("window.arrange.tile", "Tile"),
        ("window.arrange.floatInWindow", "Float in Window"),
        ("window.arrange.floatAllInWindows", "Float All in Windows"),
        ("---", "---"),
        ("window.arrange.matchZoom", "Match Zoom"),
        ("window.arrange.matchLocation", "Match Location"),
        ("window.arrange.matchRotation", "Match Rotation"),
        ("window.arrange.matchAll", "Match All"),
        ("---", "---"),
        ("window.arrange.newWindowForDocument", "New Window for W1"),
    ];
    let pairs: Vec<(&str, &str)> = got.iter().map(|(id, label, _)| (id.as_str(), label.as_str())).collect();
    assert_eq!(pairs, expected);
    assert!(got.iter().all(|(_, _, shortcut)| shortcut.is_none()), "no Window › Arrange entry carries a shortcut: {got:?}");

    // Activating another document updates the dynamic label while keeping the command id.
    new_doc(&mut app, "W2");
    let second = arrange_list(&mut app, &ctx);
    assert_eq!(second.last().map(|(id, label, _)| (id.as_str(), label.as_str())), Some(("window.arrange.newWindowForDocument", "New Window for W2")));

    app.session.set_active(0);
    let back = arrange_list(&mut app, &ctx);
    assert_eq!(back.last().map(|(id, label, _)| (id.as_str(), label.as_str())), Some(("window.arrange.newWindowForDocument", "New Window for W1")));
}

#[test]
fn new_window_for_document_label_uses_translation_catalog_with_english_fallback() {
    let translated = crate::i18n::fixture_lang("fr", "\tNew Window for {name}\tNouvelle fenêtre pour {name}\n");
    let missing = crate::i18n::fixture_lang("fr", "");

    assert_eq!(translated_menu_label(translated, "window.arrange.newWindowForDocument", "New Window for W1"), "Nouvelle fenêtre pour W1");
    assert_eq!(translated_menu_label(missing, "window.arrange.newWindowForDocument", "New Window for W1"), "New Window for W1");
    assert_eq!(translated_menu_label(crate::i18n::Lang::EN, "window.arrange.newWindowForDocument", "New Window for W1"), "New Window for W1");
}
