//! #14523: Image › Calculations' Blending list holds the reference application's 22 modes in
//! six groups with a line between each group (with Darker Color and Lighter Color omitted until
//! #14513 adds them to the engine), and defaults to Multiply.

use egui::accesskit::Role;
use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::menus::invoke as menu;

/// The reference application's Calculations Blending groups, omitting Darker Color and Lighter
/// Color until the engine accepts them (#14513).
const GROUPS: [&[&str]; 6] = [
    &["Normal"],
    &["Darken", "Multiply", "Color Burn", "Linear Burn"],
    &["Lighten", "Screen", "Color Dodge", "Linear Dodge (Add)"],
    &["Overlay", "Soft Light", "Hard Light", "Vivid Light", "Linear Light", "Pin Light", "Hard Mix"],
    &["Add", "Subtract"],
    &["Difference", "Exclusion", "Divide"],
];

fn app_with_doc() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
    app.sync_views();
    (app, egui::Context::default())
}

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
        app.sync_views();
        menu(&mut app, &cc.egui_ctx, "image.calculations", json!({})).unwrap();
        app
    });
    h.run_steps(8);
    h
}

fn open_list(h: &Harness<'_, PhotocraftApp>) -> Option<Rect> {
    h.ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)))
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    for (pressed, frames) in [(true, 1), (false, 4)] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.run_steps(frames);
    }
}

#[test]
fn calculations_defaults_to_multiply_and_apply_image_is_unchanged() {
    let (mut app, ctx) = app_with_doc();
    menu(&mut app, &ctx, "image.calculations", json!({})).unwrap();
    let calc = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(calc["blending"], json!("multiply"));

    menu(&mut app, &ctx, "image.applyImage", json!({})).unwrap();
    let apply = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(
        apply["__choices"]["blending"],
        json!([
            "multiply",
            "screen",
            "normal",
            "overlay",
            "softLight",
            "hardLight",
            "darken",
            "lighten",
            "colorDodge",
            "colorBurn",
            "linearBurn",
            "linearDodge",
            "difference",
            "exclusion",
            "add",
            "subtract",
        ])
    );
}

#[test]
fn every_listed_calculations_blending_mode_is_accepted_by_the_engine() {
    let (mut app, ctx) = app_with_doc();
    menu(&mut app, &ctx, "image.calculations", json!({})).unwrap();
    let fields = &app.ui.dialogs.last().unwrap().fields;
    let choices: Vec<String> = fields["__choices"]["blending"].as_array().unwrap().iter().filter_map(Value::as_str).map(str::to_string).collect();
    assert_eq!(choices.len(), GROUPS.concat().len());
    for mode in choices {
        let res = app.run(
            "image.calculations",
            json!({
                "source1": {"channel": "red"},
                "source2": {"channel": "green"},
                "blending": mode,
                "result": "newChannel",
            }),
        );
        assert!(res.is_ok(), "engine rejected Calculations blending mode `{mode}`: {res:?}");
    }
}

#[test]
fn calculations_blending_dropdown_shows_groups_in_reference_order_with_separators() {
    let mut h = harness();
    let at = h
        .query_all_by_role(Role::ComboBox)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Blending"))
        .expect("Calculations Blending dropdown")
        .rect()
        .center();
    click(&mut h, at);
    assert!(open_list(&h).is_some(), "the Blending list is open");

    let expected: Vec<&str> = GROUPS.concat();
    let rows: Vec<Rect> = expected
        .iter()
        .map(|label| h.get_all_by_label(label).map(|n| n.rect()).next_back().unwrap_or_else(|| panic!("{label:?} is missing from the list")))
        .collect();
    for w in rows.windows(2) {
        assert!(w[1].top() > w[0].bottom(), "rows out of order: {w:?}");
    }
    let gaps: Vec<f32> = rows.windows(2).map(|w| w[1].top() - w[0].bottom()).collect();
    let row_gap = gaps.iter().copied().fold(f32::INFINITY, f32::min);
    let lines: Vec<(&str, &str)> = gaps.iter().enumerate().filter(|(_, g)| **g > row_gap + 2.0).map(|(i, _)| (expected[i], expected[i + 1])).collect();
    assert_eq!(
        lines,
        [("Normal", "Darken"), ("Linear Burn", "Lighten"), ("Linear Dodge (Add)", "Overlay"), ("Hard Mix", "Add"), ("Subtract", "Difference"),],
        "row gaps {gaps:?}"
    );
}
