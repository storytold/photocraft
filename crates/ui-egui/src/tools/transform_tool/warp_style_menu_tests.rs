//! #12500: the Warp style menu starts at Custom (no None) and groups the styles as the reference
//! does, Custom ― Arc · Arc Lower · Arc Upper ― Arch · Bulge · Shell Lower · Shell Upper ― Flag ·
//! Wave · Fish · Rise ― Fisheye · Inflate · Squeeze · Twist ― Cylinder, each with an icon of its
//! own shape. A warp opened as None shows as Custom; the type layer list is unchanged.

use egui_kittest::kittest::{NodeT, Queryable};

use super::super::tests::bar_harness;
use super::*;

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

const ORDER: [WarpStyle; 17] = [
    WarpStyle::Custom,
    WarpStyle::Arc,
    WarpStyle::ArcLower,
    WarpStyle::ArcUpper,
    WarpStyle::Arch,
    WarpStyle::Bulge,
    WarpStyle::ShellLower,
    WarpStyle::ShellUpper,
    WarpStyle::Flag,
    WarpStyle::Wave,
    WarpStyle::Fish,
    WarpStyle::Rise,
    WarpStyle::Fisheye,
    WarpStyle::Inflate,
    WarpStyle::Squeeze,
    WarpStyle::Twist,
    WarpStyle::Cylinder,
];

/// Warp mode on the test box with `w` (a function of the box).
fn warp_bar(w: impl FnOnce([f64; 4]) -> Warp) -> Harness {
    bar_harness(|app| {
        let t = app.ui.transform.as_mut().unwrap();
        t.warp = Some(w(t.rect));
    })
}

fn open_list(h: &mut Harness) {
    let at = h
        .query_all_by_role(egui::accesskit::Role::ComboBox)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Warp"))
        .expect("the Warp dropdown")
        .rect()
        .center();
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
}

/// The open list's row named exactly `label` (the names are padded for their icons).
fn row<'a>(h: &'a Harness, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(egui::accesskit::Role::Button)
        .rfind(|n| n.accesskit_node().label().is_some_and(|l| l.trim() == label))
        .unwrap_or_else(|| panic!("the {label} row"))
}

/// Each option row's top, by its name.
fn row_top(h: &Harness, label: &str) -> f32 {
    row(h, label).rect().top()
}

#[test]
fn the_list_starts_at_custom_and_has_no_none() {
    assert_eq!(warp_style_options(false).collect::<Vec<_>>(), ORDER);
}

#[test]
fn the_open_list_groups_the_styles_with_separators() {
    let mut h = warp_bar(|b| Warp::custom(BezierMesh::identity(b, 1, 1), b));
    open_list(&mut h);
    let tops: Vec<f32> = ORDER.iter().map(|s| row_top(&h, s.label())).collect();
    let gaps: Vec<f32> = tops.windows(2).map(|w| w[1] - w[0]).collect();
    let row = gaps.iter().copied().fold(f32::MAX, f32::min);
    // A separator adds to the step before Arc, Arch, Flag, Fisheye and Cylinder, and only there.
    for (i, g) in gaps.iter().enumerate() {
        let next = ORDER[i + 1];
        let separated = matches!(next, WarpStyle::Arc | WarpStyle::Arch | WarpStyle::Flag | WarpStyle::Fisheye | WarpStyle::Cylinder);
        assert_eq!(*g > row + 2.0, separated, "before {:?}: step {g} vs row {row}", next);
    }
    assert!(h.query_all_by_label_contains("None").next().is_none(), "no None entry");
}

#[test]
fn each_style_has_its_own_icon() {
    let ctx = egui::Context::default();
    let icon = |s: WarpStyle| -> Vec<egui::Shape> {
        let mut out = Vec::new();
        let mut o = ctx.run_ui(Default::default(), |ui| {
            let painter = ui.painter().clone();
            warp_style_icon(&s, &painter, egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(14.0, 14.0)));
        });
        o.textures_delta.clear();
        out.extend(o.shapes.into_iter().map(|c| c.shape));
        out
    };
    let mut seen: Vec<String> = Vec::new();
    for s in ORDER {
        let shapes = icon(s);
        assert!(shapes.len() >= 8, "{s:?}: a warped grid of lines, {} shapes", shapes.len());
        let key = format!("{shapes:?}");
        assert!(!seen.contains(&key), "{s:?}: its icon differs from the others");
        seen.push(key);
    }
}

#[test]
fn a_warp_opened_as_none_shows_as_custom() {
    let h = warp_bar(Warp::none);
    let shown = h.query_all_by_role(egui::accesskit::Role::ComboBox).any(|n| {
        let node = n.accesskit_node();
        node.label().as_deref() == Some("Warp") && node.value().is_some_and(|v| v.contains("Custom"))
    });
    assert!(shown, "the list shows Custom for an untouched None warp");
}

#[test]
fn picking_a_preset_sets_bend_50_and_custom_keeps_its_shape() {
    let mut h = warp_bar(|b| Warp::custom(BezierMesh::identity(b, 1, 1), b));
    open_list(&mut h);
    row(&h, "Arc Upper").click();
    h.run_steps(3);
    let w = h.state().ui.transform.as_ref().unwrap().warp.clone().unwrap();
    assert_eq!((w.style, w.bend, w.h_distort, w.v_distort), (WarpStyle::ArcUpper, 50.0, 0.0, 0.0));
    open_list(&mut h);
    row(&h, "Custom").click();
    h.run_steps(3);
    let c = h.state().ui.transform.as_ref().unwrap().warp.clone().unwrap();
    assert_eq!(c.style, WarpStyle::Custom);
    let (x, y) = (w.bounds[0] + (w.bounds[2] - w.bounds[0]) / 2.0, w.bounds[1]);
    let (a, b) = (w.map(x, y), c.map(x, y));
    assert!((a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6, "Custom keeps the Arc Upper shape");
}

#[test]
fn the_type_layer_list_is_unchanged() {
    let on_type: Vec<WarpStyle> = warp_style_options(true).collect();
    assert_eq!(on_type.first(), Some(&WarpStyle::None));
    assert!(!on_type.contains(&WarpStyle::Custom) && !on_type.contains(&WarpStyle::Cylinder));
    assert_eq!(on_type.len(), 16, "None and the fifteen presets");
}
