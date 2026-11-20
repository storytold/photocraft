//! #14753: Hue/Saturation picks its range with seven round swatches (Master, then Red, Yellow,
//! Green, Cyan, Blue, Magenta) instead of a dropdown; an edited range shows its colour before and
//! after with a dot under it, a label above the Hue field gives the range's centre, and Colorize
//! collapses the row into one disc in the colorize colour.

use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::super::EditorCx;
use super::{SWATCH_LABELS, colorize_color, hue_saturation, range_centre_label, range_swatch};

fn cx(mem: egui::Id) -> EditorCx {
    EditorCx {
        mem,
        hist: None,
        gray: false,
        swatches: [[0.0; 3], [1.0; 3]],
        gradients: Vec::new(),
        dialog: false,
        dynamic_color_sliders: true,
        isolated_curve: None,
    }
}

fn mem() -> egui::Id {
    egui::Id::unique("hue-saturation-swatches-test")
}

/// The Hue/Saturation editor over `v`, as the Properties panel hosts it.
fn harness(v: Value) -> Harness<'static, Value> {
    let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(
        |ui, v: &mut Value| {
            // The app's fonts load over a few frames; draw once they're in.
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                hue_saturation(ui, v, &cx(mem()));
            }
        },
        v,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn swatches(h: &Harness<'_, Value>) -> Vec<(String, bool)> {
    h.query_all_by_role(Role::RadioButton)
        .map(|n| {
            let a = n.accesskit_node();
            (a.label().unwrap_or_default(), a.toggled() == Some(Toggled::True))
        })
        .collect()
}

fn text_rect(h: &Harness<'_, Value>, text: &str) -> Option<egui::Rect> {
    h.query_all_by_role(Role::Label)
        .find(|n| {
            let a = n.accesskit_node();
            a.value().as_deref() == Some(text) || a.label().as_deref() == Some(text)
        })
        .map(|n| n.rect())
}

fn has_text(h: &Harness<'_, Value>, text: &str) -> bool {
    text_rect(h, text).is_some()
}

fn range(h: &Harness<'_, Value>) -> Option<usize> {
    h.ctx.data(|d| d.get_temp(mem().with("hs-range")))
}

#[test]
fn seven_swatches_replace_the_dropdown_with_master_selected() {
    let h = harness(json!({}));
    let want: Vec<(String, bool)> = SWATCH_LABELS.iter().enumerate().map(|(i, l)| (l.to_string(), i == 0)).collect();
    assert_eq!(swatches(&h), want, "Master, Red, Yellow, Green, Cyan, Blue, Magenta; Master selected");
    assert!(h.query_all_by_role(Role::ComboBox).next().is_none(), "no range dropdown");
}

#[test]
fn a_click_selects_that_range_and_shows_its_centre() {
    let mut h = harness(json!({}));
    assert!(!has_text(&h, "0°") && !has_text(&h, "240°"), "no centre label for Master");
    h.query_all_by_role_and_label(Role::RadioButton, "Blue").next().expect("the Blue swatch").click();
    h.run_steps(3);
    assert_eq!(range(&h), Some(5), "Blues selected");
    assert!(swatches(&h).iter().all(|(l, on)| *on == (l == "Blue")), "only Blue is selected: {:?}", swatches(&h));
    // Right-aligned just above the Hue row (title, field and slider), which now edits Blues.
    let label = text_rect(&h, "240°").expect("the Blues centre");
    let hue = h.query_all_by_role(Role::Slider).find(|n| n.accesskit_node().label().as_deref() == Some("Hue")).expect("the Hue slider").rect();
    assert!(label.bottom() <= hue.top() + 1.0 && hue.top() - label.bottom() < 48.0, "label {label:?} right above Hue {hue:?}");
    assert!(label.center().x > 160.0, "right-aligned: {label:?}");

    h.query_all_by_role_and_label(Role::RadioButton, "Red").next().expect("the Red swatch").click();
    h.run_steps(3);
    assert_eq!(range(&h), Some(1));
    assert!(has_text(&h, "0°"), "the Reds centre");

    h.query_all_by_role_and_label(Role::RadioButton, "Master").next().expect("the Master swatch").click();
    h.run_steps(3);
    assert_eq!(range(&h), Some(0));
    assert!(!has_text(&h, "0°"), "no centre label for Master");
}

#[test]
fn reds_after_a_hue_shift_label_both_centres() {
    let mut h = harness(json!({"reds": {"hue": 30}}));
    h.query_all_by_role_and_label(Role::RadioButton, "Red").next().expect("the Red swatch").click();
    h.run_steps(3);
    assert!(has_text(&h, "0° / 30°"));
}

/// Circles stroked in `color` whose centre lies inside `rect`.
fn rings_in(h: &Harness<'_, Value>, rect: egui::Rect, color: egui::Color32) -> usize {
    h.output()
        .shapes
        .iter()
        .filter(|s| matches!(&s.shape, egui::Shape::Circle(c) if c.stroke.color == color && c.stroke.width > 0.0 && rect.contains(c.center)))
        .count()
}

#[test]
fn a_focused_swatch_shows_focus_apart_from_the_selection() {
    let mut h = harness(json!({}));
    let accent = crate::theme::Tokens::get(&h.ctx).accent;
    let blue = h.query_all_by_role_and_label(Role::RadioButton, "Blue").next().expect("the Blue swatch").rect();
    let master = h.query_all_by_role_and_label(Role::RadioButton, "Master").next().expect("the Master swatch").rect();
    assert_eq!(rings_in(&h, blue, accent), 0, "no focus ring before focus");

    h.query_all_by_role_and_label(Role::RadioButton, "Blue").next().expect("the Blue swatch").focus();
    h.run_steps(3);
    assert_eq!(range(&h), Some(0), "focus alone keeps Master selected");
    assert_eq!(rings_in(&h, blue, accent), 1, "the focused Blue swatch has a focus ring");
    assert_eq!(rings_in(&h, master, accent), 0, "the selected Master swatch has none");
    assert!(swatches(&h).iter().all(|(l, on)| *on == (l == "Master")), "selection unchanged: {:?}", swatches(&h));

    h.key_press(egui::Key::Space);
    h.run_steps(3);
    assert_eq!(range(&h), Some(5), "Space selects the focused swatch");
}

#[test]
fn colorize_collapses_the_row_into_one_disc() {
    let h = harness(json!({"colorize": true, "hue": 120, "saturation": 25}));
    assert!(swatches(&h).is_empty(), "no range swatches with Colorize on");
    assert_eq!(h.query_all_by_role_and_label(Role::Image, "Colorize").count(), 1, "one colorize disc");
}

#[test]
fn centre_labels() {
    let v = json!({});
    assert_eq!(range_centre_label(&v, 0), None, "Master");
    assert_eq!(range_centre_label(&v, 1).as_deref(), Some("0°"), "Reds");
    assert_eq!(range_centre_label(&v, 2).as_deref(), Some("60°"), "Yellows");
    assert_eq!(range_centre_label(&v, 5).as_deref(), Some("240°"), "Blues");
    assert_eq!(range_centre_label(&v, 6).as_deref(), Some("300°"), "Magentas");
    assert_eq!(range_centre_label(&v, 7), None, "past the ranges: no label, no panic");
    assert_eq!(range_centre_label(&json!({"reds": {"hue": 30}}), 1).as_deref(), Some("0° / 30°"));
    assert_eq!(range_centre_label(&json!({"reds": {"hue": -30}}), 1).as_deref(), Some("0° / 330°"));
    // A moved range: its centre is the middle of its fully-on span.
    assert_eq!(range_centre_label(&json!({"blues": {"range": [190, 220, 240, 270]}}), 5).as_deref(), Some("230°"));
    // Saturation or Lightness alone moves no hue.
    assert_eq!(range_centre_label(&json!({"blues": {"saturation": 50}}), 5).as_deref(), Some("240°"));
}

#[test]
fn an_edited_range_shows_before_and_after() {
    let close = |a: egui::Color32, b: [u8; 3]| [a.r(), a.g(), a.b()].iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2);
    let (before, after) = range_swatch(&json!({}), 0);
    assert!(close(before, [255, 0, 0]), "Reds: red, {before:?}");
    assert_eq!(after, None, "an untouched range has no after half");

    let (before, after) = range_swatch(&json!({"reds": {"hue": 30}}), 0);
    assert!(close(before, [255, 0, 0]));
    let after = after.expect("Hue +30 edits Reds");
    assert!(close(after, [255, 128, 0]), "red turned 30° towards yellow: {after:?}");

    let (_, after) = range_swatch(&json!({"blues": {"lightness": -100}}), 4);
    assert!(close(after.expect("Lightness edits Blues"), [0, 0, 0]), "Blues at Lightness -100 is black");

    // Master and the other ranges leave a range's swatch alone.
    assert_eq!(range_swatch(&json!({"hue": 90, "greens": {"saturation": -50}}), 0).1, None);
    // Reset puts every range back to zero: the after halves go.
    let reset = json!({"reds": {"hue": 0, "saturation": 0, "lightness": 0}});
    assert_eq!(range_swatch(&reset, 0).1, None);
    assert_eq!(range_swatch(&json!({}), 6), (egui::Color32::TRANSPARENT, None), "past the ranges: no panic");
}

#[test]
fn the_colorize_disc_takes_the_colorize_colour() {
    let c = colorize_color(&json!({"colorize": true, "hue": 120, "saturation": 100}));
    assert!(c.g() > c.r() + 64 && c.g() > c.b() + 64, "green: {c:?}");
    let c = colorize_color(&json!({"colorize": true, "hue": 240, "saturation": 100}));
    assert!(c.b() > c.r() + 64 && c.b() > c.g() + 64, "blue: {c:?}");
}
