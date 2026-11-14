//! #14038: File › New › Recent is headed "YOUR RECENT ITEMS (n)", lays its cards out 4 per row and
//! keeps every created document, duplicates included, as Photoshop does.

use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::new_doc_ui::{CATEGORIES, recent_cards};
use crate::state::{DialogKind, UiState};

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

/// Open New Document, set `fields`, press Create.
fn create(app: &mut PhotocraftApp, fields: Value) {
    let f = app.new_document_fields();
    let id = app.ui.open_dialog(DialogKind::NewDocument, f);
    let d = app.ui.dialog_mut(id).unwrap();
    for (k, v) in fields.as_object().unwrap() {
        d.fields.insert(k.clone(), v.clone());
    }
    crate::dialogs::confirm(app, id).unwrap();
}

fn create_px(app: &mut PhotocraftApp, w: u32, h: u32) {
    create(app, json!({"width": w, "height": h, "resolution": 72.0, "__unit": "px"}));
}

fn widths(app: &PhotocraftApp) -> Vec<u32> {
    app.session.presets.recent_documents.iter().map(|r| r.width).collect()
}

#[test]
fn equal_documents_give_two_cards_newest_first_after_a_reload() {
    let mut app = app();
    create_px(&mut app, 300, 200);
    create_px(&mut app, 640, 480);
    create_px(&mut app, 300, 200);
    assert_eq!(widths(&app), [300, 640, 300]);
    let cards = recent_cards(&app.new_document_fields(), &app.session.presets.recent_documents);
    let lines: Vec<String> = cards.iter().map(|c| c.size_line(&Default::default(), &serde_json::Map::new())).collect();
    assert_eq!(lines, ["300 x 200 px @ 72 ppi", "640 x 480 px @ 72 ppi", "300 x 200 px @ 72 ppi"]);
    // Both 300 x 200 entries survive a preferences reload.
    let mut restarted = photocraft_engine::Session::new();
    restarted.load_prefs_json(&app.session.prefs_to_json()).unwrap();
    assert_eq!(restarted.presets.recent_documents, app.session.presets.recent_documents);
}

#[test]
fn the_list_is_capped_at_twenty() {
    let mut app = app();
    for _ in 0..25 {
        create_px(&mut app, 300, 200);
    }
    assert_eq!(widths(&app).len(), 20);
    let mut restarted = photocraft_engine::Session::new();
    restarted.load_prefs_json(&app.session.prefs_to_json()).unwrap();
    assert_eq!(restarted.presets.recent_documents.len(), 20);
}

#[test]
fn recent_items_are_listed_without_built_in_presets() {
    let mut app = app();
    let fields = app.new_document_fields();
    // No recent items yet: today's built-in defaults.
    let titles = |cards: &[crate::new_doc_ui::Card]| cards.iter().map(|c| c.title()).collect::<Vec<_>>();
    let defaults: Vec<&str> = CATEGORIES[0].1.iter().map(|p| p.0).collect();
    assert_eq!(titles(&recent_cards(&fields, &[])), defaults);
    // A document of a default's size is listed once, as the recent item; the other default is gone.
    create(&mut app, json!({"width": 1920, "height": 1080, "resolution": 72.0, "__unit": "px"}));
    create_px(&mut app, 600, 400);
    assert_eq!(titles(&recent_cards(&app.new_document_fields(), &app.session.presets.recent_documents)), ["Custom", "HDTV 1080p"]);
}

/// The size lines painted in the dialog, with their screen rects, in paint order.
fn size_lines(h: &Harness<'static, PhotocraftApp>) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text.contains(" ppi") => {
                out.push((text.galley.job.text.replace('\n', " "), text.visual_bounding_rect()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output().shapes.iter().for_each(|shape| walk(&shape.shape, &mut out));
    out
}

fn dialog(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
    h.run_steps(3);
    h
}

#[test]
fn the_recent_tab_is_headed_your_recent_items_with_four_cards_per_row() {
    let mut app = app();
    for w in [601, 602, 603, 604, 605] {
        create_px(&mut app, w, 400);
    }
    // A long size line still fits its narrower card.
    create(&mut app, json!({"width": 30000, "height": 20000, "resolution": 300.0, "__unit": "px"}));
    let mut h = dialog(app);
    assert!(h.query_by_label("YOUR RECENT ITEMS (6)").is_some(), "the Recent heading counts the cards");
    assert!(h.query_by_label_contains("BLANK DOCUMENT PRESETS").is_none());
    let lines = size_lines(&h);
    let names: Vec<&str> = lines.iter().map(|(text, _)| text.as_str()).collect();
    assert_eq!(
        names,
        [
            "30000 x 20000 px @ 300 ppi",
            "605 x 400 px @ 72 ppi",
            "604 x 400 px @ 72 ppi",
            "603 x 400 px @ 72 ppi",
            "602 x 400 px @ 72 ppi",
            "601 x 400 px @ 72 ppi"
        ]
    );
    let rects: Vec<egui::Rect> = lines.iter().map(|(_, r)| *r).collect();
    let row = |i: usize| rects[i].center().y;
    assert!((1..4).all(|i| (row(i) - row(1)).abs() < 1.0 && rects[i].center().x > rects[i - 1].center().x), "four cards in the first row");
    assert!(row(4) > row(0) + 50.0 && (row(5) - row(4)).abs() < 1.0, "the fifth card starts the second row");
    let pitch = rects[2].center().x - rects[1].center().x;
    assert!(rects.iter().all(|r| r.width() <= pitch - 8.0), "every size line fits its card: {rects:?}");
    // The preset tabs keep their heading and three columns.
    h.get_by_label("Photo").click();
    h.run_steps(2);
    assert!(h.query_by_label_contains("BLANK DOCUMENT PRESETS").is_some());
    assert!(h.query_by_label_contains("YOUR RECENT ITEMS").is_none());
    let rects: Vec<egui::Rect> = size_lines(&h).into_iter().map(|(_, r)| r).collect();
    assert!((rects[3].center().y - rects[0].center().y) > 50.0 && (rects[2].center().y - rects[0].center().y).abs() < 1.0, "three per row");
}

#[test]
fn without_recent_items_the_tab_keeps_todays_defaults() {
    let h = dialog(app());
    let count = CATEGORIES[0].1.len();
    assert!(h.query_by_label(&format!("BLANK DOCUMENT PRESETS ({count})")).is_some());
    assert!(h.query_by_label_contains("YOUR RECENT ITEMS").is_none());
}
