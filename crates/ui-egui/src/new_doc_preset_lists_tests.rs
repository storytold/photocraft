//! #14040: the Photo, Print and Art & Illustration tabs of File › New list the reference
//! application's blank presets, in its order, and the pixel grids are sized in pixels.

use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use photocraft_engine::prefs::UnitsAndRulers;

use crate::PhotocraftApp;
use crate::dialogs::new_doc_ui::{CATEGORIES, Preset, apply_preset, at_pref_resolutions};
use crate::state::{DialogKind, UiState};

fn presets(cat: &str) -> &'static [Preset] {
    CATEGORIES.iter().find(|c| c.0 == cat).map(|c| c.1).unwrap()
}

fn names(cat: &str) -> Vec<&'static str> {
    presets(cat).iter().map(|p| p.0).collect()
}

fn find(cat: &str, name: &str) -> Preset {
    *presets(cat).iter().find(|p| p.0 == name).unwrap()
}

#[test]
fn photo_lists_the_default_size_then_landscape_then_portrait_at_300_ppi() {
    assert_eq!(
        presets("Photo"),
        &[
            ("Default PhotoCraft Size", 2100, 1500, 300.0),
            ("Landscape, 3 x 2", 900, 600, 300.0),
            ("Landscape, 6 x 4", 1800, 1200, 300.0),
            ("Landscape, 7 x 5", 2100, 1500, 300.0),
            ("Landscape, 10 x 8", 3000, 2400, 300.0),
            ("Portrait, 2 x 3", 600, 900, 300.0),
            ("Portrait, 4 x 6", 1200, 1800, 300.0),
            ("Portrait, 5 x 7", 1500, 2100, 300.0),
            ("Portrait, 8 x 10", 2400, 3000, 300.0),
        ][..]
    );
    let mut f = UiState::new_document_fields();
    for p in presets("Photo") {
        apply_preset(&mut f, p, &UnitsAndRulers::default());
        assert_eq!(f["__unit"], "in", "{}", p.0);
    }
}

#[test]
fn print_lists_us_iso_a_b_and_c_sizes_at_300_ppi() {
    assert_eq!(names("Print"), ["Letter", "Legal", "Tabloid", "A4", "A6", "A5", "A3", "B5", "B4", "B3", "C4", "C5", "C6", "DL"]);
    assert!(presets("Print").iter().all(|p| p.3 == 300.0));
    // ISO 216 / ISO 269 sizes in mm at 300 ppi; Letter keeps 8.5 x 11 in, Legal 8.5 x 14, Tabloid 11 x 17.
    let mm = |v: f32| (v / 25.4 * 300.0).round() as u32;
    for (name, w, h) in [
        ("A3", 297.0, 420.0),
        ("A4", 210.0, 297.0),
        ("A5", 148.0, 210.0),
        ("A6", 105.0, 148.0),
        ("B3", 353.0, 500.0),
        ("B4", 250.0, 353.0),
        ("B5", 176.0, 250.0),
        ("C4", 229.0, 324.0),
        ("C5", 162.0, 229.0),
        ("C6", 114.0, 162.0),
        ("DL", 110.0, 220.0),
    ] {
        assert_eq!(find("Print", name), (name, mm(w), mm(h), 300.0));
    }
    assert_eq!(find("Print", "Letter"), ("Letter", 2550, 3300, 300.0));
    assert_eq!(find("Print", "Legal"), ("Legal", 2550, 4200, 300.0));
    assert_eq!(find("Print", "Tabloid"), ("Tabloid", 3300, 5100, 300.0));
}

#[test]
fn art_and_illustration_lists_pixel_grids_poster_postcard_and_video_sizes() {
    assert_eq!(
        presets("Art & Illustration"),
        &[
            ("1000 pixel grid", 1000, 1000, 300.0),
            ("2000 pixel grid", 2000, 2000, 300.0),
            ("Poster", 5400, 7200, 300.0),
            ("Postcard", 1200, 1800, 300.0),
            ("1080p", 1920, 1080, 72.0),
            ("720p", 1280, 720, 72.0),
        ][..]
    );
}

#[test]
fn the_other_tabs_keep_their_presets() {
    assert_eq!(names("Recent"), ["Default PhotoCraft Size", "HDTV 1080p"]);
    assert_eq!(names("Web"), ["Web Most Common", "Web Minimum", "Web Large", "MacBook Pro 16\"", "iMac 24\""]);
    assert_eq!(names("Mobile"), ["iPhone 16", "iPhone 16 Pro Max", "iPad Pro 13\"", "Android 1080p", "Apple Watch 45mm"]);
    assert_eq!(names("Film & Video"), ["HDTV 1080p", "HDTV 720p", "UHD 4K", "DCI 4K", "UHD 8K"]);
}

/// Before #14040 every 300 ppi preset was shown and applied in inches (or the picked physical
/// unit), so the pixel grids would have read 3.33 x 3.33 in.
#[test]
fn pixel_grids_apply_their_pixels_whatever_unit_is_picked() {
    for unit in ["px", "in", "mm"] {
        let mut f = UiState::new_document_fields();
        f.insert("__unit".into(), json!(unit));
        apply_preset(&mut f, &find("Art & Illustration", "2000 pixel grid"), &UnitsAndRulers::default());
        assert_eq!((f["width"].as_u64(), f["height"].as_u64(), f["resolution"].as_f64()), (Some(2000), Some(2000), Some(300.0)), "{unit}");
        assert_eq!(f["__unit"], "px", "{unit}");
        // Poster is still a print preset in the picked physical unit.
        apply_preset(&mut f, &find("Art & Illustration", "Poster"), &UnitsAndRulers::default());
        assert_eq!(f["__unit"], if unit == "px" { "in" } else { unit });
    }
    // At other preset resolutions a pixel grid keeps its pixels, at the Print Resolution.
    let units = UnitsAndRulers { print_resolution: 200.0, screen_resolution: 96.0, ..Default::default() };
    assert_eq!(at_pref_resolutions(&find("Art & Illustration", "1000 pixel grid"), &units), ("1000 pixel grid", 1000, 1000, 200.0));
    assert_eq!(at_pref_resolutions(&find("Art & Illustration", "Postcard"), &units), ("Postcard", 800, 1200, 200.0));
    assert_eq!(at_pref_resolutions(&find("Art & Illustration", "720p"), &units), ("720p", 1280, 720, 96.0));
}

fn harness() -> Harness<'static, PhotocraftApp> {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
    h.run_steps(3);
    h
}

fn open_tab(h: &mut Harness<'static, PhotocraftApp>, tab: &str) {
    h.get_by_label(tab).click();
    h.run_steps(2);
}

/// Where the card text `text` is painted (cards paint their titles and sizes without a label).
fn painted_at(h: &Harness<'static, PhotocraftApp>, text: &str) -> Option<egui::Pos2> {
    fn find(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(t) if t.galley.job.text == text => Some(t.visual_bounding_rect().center()),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, text)),
            _ => None,
        }
    }
    h.output().shapes.iter().find_map(|shape| find(&shape.shape, text))
}

#[test]
fn tab_headings_count_the_presets() {
    let mut h = harness();
    for (tab, n) in [("Photo", 9), ("Print", 14), ("Art & Illustration", 6)] {
        open_tab(&mut h, tab);
        let heading = format!("BLANK DOCUMENT PRESETS ({n})");
        assert!(h.query_by_label(&heading).is_some(), "{tab}: {heading}");
    }
}

#[test]
fn a_pixel_grid_card_shows_and_creates_its_pixels() {
    let mut h = harness();
    open_tab(&mut h, "Art & Illustration");
    assert!(painted_at(&h, "1000 x 1000 px @ 300 ppi").is_some(), "the card's size line is in pixels");
    assert!(painted_at(&h, "4 x 6 in @ 300 ppi").is_some(), "Postcard is 4 x 6 in portrait");
    let at = painted_at(&h, "1000 pixel grid").expect("visible card");
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(2);
    let f = h.state().ui.dialogs.first().map(|d| d.fields.clone()).unwrap();
    assert_eq!((f["__preset"].as_str(), f["__unit"].as_str()), (Some("1000 pixel grid"), Some("px")));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let d = &h.state().session.active().expect("a new document").doc;
    assert_eq!((d.size.width, d.size.height, d.resolution_dpi), (1000, 1000, 300.0));
}
