//! Arabic (right-to-left) menu rows: the label reads from the right, the shortcut and the
//! submenu arrow sit at the row's far end, and the keyboard follows the reading order. The twins
//! switch the language through the preferences — never injected styles — so the whole direction
//! plumbing is exercised.
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, i18n};
use serde_json::json;

fn harness(lang: &str) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).expect("new document");
        app
    });
    h.run_steps(2);
    h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": lang})).expect("language");
    h.run_steps(4);
    h
}

/// The painted text galleys with the absolute left edge of their glyphs.
fn painted(h: &Harness<'static, PhotocraftApp>) -> Vec<(String, f32)> {
    fn collect(shape: &egui::Shape, out: &mut Vec<(String, f32)>) {
        match shape {
            egui::Shape::Text(text) => {
                let left = text
                    .galley
                    .rows
                    .iter()
                    .map(|row| {
                        let row_left = row.glyphs.iter().map(|g| g.pos.x).fold(f32::INFINITY, f32::min);
                        row_left + row.pos.x + text.pos.x
                    })
                    .fold(f32::INFINITY, f32::min);
                out.push((text.galley.job.text.clone(), left));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut out);
    }
    out
}

fn galley_left(h: &Harness<'static, PhotocraftApp>, text: &str) -> Option<f32> {
    painted(h).into_iter().find(|(painted, _)| painted == text).map(|(_, x)| x)
}

#[test]
fn dump_painted() {
    for lang in ["ar", "en"] {
        let mut h = harness(lang);
        let tr = |s: &str| i18n::tr(i18n::current(), s).to_string();
        h.get_by_label(&tr("File")).click();
        h.run_steps(4);
        println!("=== {lang} ===");
        for (text, left) in painted(&h) {
            println!("{left:8.1}  {text:?}");
        }
    }
}

/// An Arabic row reads label first from the right with the shortcut at the far left; the English
/// row keeps the shortcut at the far right (decision 1: Arabic renders what English renders,
/// mirrored).
#[test]
fn the_shortcut_sits_at_the_reading_end_of_the_row() {
    for (lang, shortcut_first) in [("ar", true), ("en", false)] {
        let mut h = harness(lang);
        let tr = |s: &str| i18n::tr(i18n::current(), s).to_string();
        h.get_by_label(&tr("File")).click();
        h.run_steps(4);
        let label = format!("{}…", tr("New"));
        let label_left = galley_left(&h, &label).unwrap_or_else(|| panic!("{lang}: the label paints"));
        let shortcut_left =
            painted(&h).into_iter().find(|(text, _)| text == "Ctrl+N" || text == "⌘N").map(|(_, x)| x).unwrap_or_else(|| panic!("{lang}: the shortcut paints"));
        assert_eq!(shortcut_first, shortcut_left < label_left, "{lang}: the shortcut sits at the reading end");
    }
}

/// A submenu opens leftward in a right-to-left UI: its rows hang to the left of their header row
/// (the flag's default align; no explicit align is pinned).
#[test]
fn the_submenu_opens_to_the_left_of_its_row() {
    let mut h = harness("ar");
    let tr = |s: &str| i18n::tr(i18n::current(), s).to_string();
    h.get_by_label(&tr("Image")).click();
    h.run_steps(4);
    let mode = tr("Mode");
    h.get_by_label(&mode).click();
    h.run_steps(4);
    let child = h.get_by_label_contains(&tr("8 Bits/Channel")).rect();
    let parent = h.get_by_label(&mode).rect();
    assert!(child.right() <= parent.left() + 1.0, "the submenu hangs left of its row ({} vs {})", child.right(), parent.left());
}

/// The bar reads from the right in Arabic: File is the reading start, drawn to the right of
/// every other title, and Help ends the row on the left.
#[test]
fn the_bar_reads_from_the_right_in_arabic() {
    let h = harness("ar");
    let tr = |s: &str| i18n::tr(i18n::current(), s).to_string();
    let file = h.query_by_label(&tr("File")).expect("the File title").rect();
    let help = h.query_by_label(&tr("Help")).expect("the Help title").rect();
    assert!(file.left() > help.right(), "the File title sits right of Help ({} vs {})", file.left(), help.right());
    assert!(file.left() > 640.0, "the File title sits in the right half of the bar ({})", file.left());
}

/// The keyboard follows the reading order in Arabic: ↓ opens File, ← advances to Edit (the next
/// menu, drawn to the left), and the bar still drives everything.
#[test]
fn the_keyboard_follows_the_reading_order_in_arabic() {
    let mut h = harness("ar");
    let tr = |s: &str| i18n::tr(i18n::current(), s).to_string();
    h.key_press(egui::Key::F10);
    h.run_steps(2);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(3);
    assert!(h.query_all_by_label_contains(&tr("New")).next().is_some(), "↓ opens the lit File menu");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.key_press(egui::Key::F10);
    h.run_steps(2);
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(2);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(3);
    assert!(h.query_all_by_label_contains(&tr("Undo")).next().is_some(), "← advanced to Edit and ↓ opened it");
}
