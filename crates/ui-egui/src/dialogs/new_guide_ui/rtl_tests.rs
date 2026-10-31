//! The New Guide dialog's right-to-left twins: the rows read from the right while the data and
//! the behaviour stay the ones the left-to-right tests pin — only the direction is forced.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;

/// The window the body is drawn in: a mirrored full-width row is checked against its width.
const WINDOW: f32 = 640.0;

/// The rows the body lays out: the Orientation and Color captions, the Horizontal radio and the
/// Position field.
#[derive(Debug)]
struct Layout {
    orientation: egui::Rect,
    color: egui::Rect,
    horizontal: egui::Rect,
    position: egui::Rect,
}

/// A caption's rectangle: the plain text node, not the control that shares its name — the Color
/// dropdown is named after its row, as the caption is.
fn caption(h: &egui_kittest::Harness<'_, ()>, text: &str) -> egui::Rect {
    h.query_all_by_label(text).find(|n| n.accesskit_node().role() != Role::ComboBox).unwrap_or_else(|| panic!("the {text} caption")).rect()
}

/// The body of a Cyan, horizontal, "0 cm" dialog, drawn in `dir`.
fn rows(dir: crate::text::direction::Dir) -> Layout {
    let _dir = crate::text::direction::direction_scope(dir);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(WINDOW, 400.0)).with_max_steps(64).build_ui_state(
        |ui, _state| {
            // The named font families land in the second pass; drawing before that paints
            // text with an empty family and panics.
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let mut f = Map::new();
            f.insert("orientation".into(), json!("horizontal"));
            f.insert("position".into(), json!("0 cm"));
            f.insert("color".into(), json!("cyan"));
            body(ui, &mut f);
        },
        (),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    Layout {
        orientation: caption(&h, "Orientation"),
        color: caption(&h, "Color"),
        horizontal: h.get_by_label("Horizontal").rect(),
        position: h.query_all_by_role(Role::TextInput).next().unwrap_or_else(|| panic!("the Position field")).rect(),
    }
}

/// The left-to-right grid keeps its label column; the right-to-left rows read from the right,
/// each caption at the reading start and its editor beside it toward the reading end.
#[test]
fn the_new_guide_rows_mirror_in_a_right_to_left_ui() {
    let ltr = rows(crate::text::direction::Dir::Ltr);
    assert!((ltr.orientation.left() - ltr.color.left()).abs() < 1.0, "LTR: the captions share the grid column: {ltr:?}");
    assert!(ltr.horizontal.left() > ltr.orientation.right(), "LTR: the radios follow their label: {ltr:?}");
    assert!(ltr.position.left() > ltr.color.right(), "LTR: the Position field follows the captions: {ltr:?}");

    let rtl = rows(crate::text::direction::Dir::Rtl);
    assert!((rtl.orientation.right() - rtl.color.right()).abs() < 1.0, "RTL: the captions share the reading start: {ltr:?} / {rtl:?}");
    assert!(rtl.horizontal.right() < rtl.orientation.left(), "RTL: the radios lead toward the reading end: {ltr:?} / {rtl:?}");
    assert!(rtl.position.right() < rtl.color.left(), "RTL: the Position field leads toward the reading end: {ltr:?} / {rtl:?}");
    // Each caption mirrors about the window's centre: only the direction is forced, the labels
    // and fonts are the English ones in both runs.
    for (name, ltr, rtl) in [("Orientation", ltr.orientation, rtl.orientation), ("Color", ltr.color, rtl.color)] {
        assert!((ltr.center().x + rtl.center().x - WINDOW).abs() < 2.0, "{name} mirrors: {ltr:?} / {rtl:?}");
    }
}
