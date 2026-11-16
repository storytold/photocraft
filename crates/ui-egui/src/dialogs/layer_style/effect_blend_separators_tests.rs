//! #14941: the Blend Mode lists of the Layer Style effect pages part the 27 modes into the
//! reference's six families with five lines, as the Layers panel does; a pick stores the mode's
//! name as before.
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

type State = (PhotocraftApp, Map<String, Value>);

/// The reference's list, a family per row.
const GROUPS: [&[&str]; 6] = [
    &["Normal", "Dissolve"],
    &["Darken", "Multiply", "Color Burn", "Linear Burn", "Darker Color"],
    &["Lighten", "Screen", "Color Dodge", "Linear Dodge (Add)", "Lighter Color"],
    &["Overlay", "Soft Light", "Hard Light", "Vivid Light", "Linear Light", "Pin Light", "Hard Mix"],
    &["Difference", "Exclusion", "Subtract", "Divide"],
    &["Hue", "Saturation", "Color", "Luminosity"],
];

/// The Layer Style dialog on `kind`'s page, for a layer that has that effect.
fn page(kind: &str) -> Harness<'static, State> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run(&format!("layer.layerStyle.{kind}"), json!({})).unwrap();
    let fields = {
        let state = app.session.active().unwrap();
        initial_fields(state.doc.layer(state.active_layer.unwrap()).unwrap(), Some(kind), 120.0)
    };
    let mut h = Harness::builder().with_size(vec2(1000.0, 2000.0)).build_ui_state(
        |ui, state: &mut State| {
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                body(&mut state.0, ui, &mut state.1);
            }
        },
        (app, fields),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

/// Opens the dropdown named `name` and returns its rows' rects, in the reference's order.
fn open_list(h: &mut Harness<'static, State>, name: &str) -> Vec<egui::Rect> {
    h.get_by_role_and_label(Role::ComboBox, name).click();
    h.run_steps(4);
    let list = h.ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)));
    let Some(list) = list else { panic!("the {name} list is open") };
    // Other widgets share some labels (the effect list's "Color Overlay" row, the field's own
    // text), so each row is the nearest match below the previous one in the same column.
    let mut rows: Vec<egui::Rect> = Vec::new();
    for l in GROUPS.concat() {
        let mut found = h.get_all_by_label(l).map(|n| n.rect()).filter(|r| list.contains_rect(*r));
        let row = match rows.last() {
            None => found.next_back(),
            Some(prev) => found.filter(|r| (r.left() - prev.left()).abs() < 2.0 && r.top() >= prev.bottom() - 0.5).min_by(|a, b| a.top().total_cmp(&b.top())),
        };
        rows.push(row.unwrap_or_else(|| panic!("{l} is in the {name} list")));
    }
    rows
}

/// Asserts the rows are parted exactly where the reference's families start.
fn assert_grouped(rows: &[egui::Rect], what: &str) {
    let gaps: Vec<f32> = rows.windows(2).map(|w| w[1].top() - w[0].bottom()).collect();
    let plain = gaps.iter().copied().fold(f32::INFINITY, f32::min);
    let starts: Vec<usize> = GROUPS.iter().scan(0, |at, g| Some(std::mem::replace(at, *at + g.len()))).skip(1).collect();
    for (i, gap) in gaps.iter().enumerate() {
        // gaps[i] lies above row i + 1.
        let parted = *gap > plain + 0.5;
        assert_eq!(parted, starts.contains(&(i + 1)), "{what}: line above {}: {gaps:?}", GROUPS.concat()[i + 1]);
    }
    assert_eq!(gaps.iter().filter(|g| **g > plain + 0.5).count(), 5, "{what}: five lines");
}

#[test]
fn the_inner_shadow_blend_mode_list_shows_the_five_lines() {
    let mut h = page("innerShadow");
    let rows = open_list(&mut h, "Blend Mode");
    assert_grouped(&rows, "Inner Shadow");
}

#[test]
fn every_effect_page_with_a_blend_mode_shows_the_same_lines() {
    for kind in ["dropShadow", "innerGlow", "outerGlow", "satin", "colorOverlay", "gradientOverlay", "patternOverlay", "stroke"] {
        let mut h = page(kind);
        let rows = open_list(&mut h, "Blend Mode");
        assert_grouped(&rows, kind);
    }
    for name in ["Highlight Mode", "Shadow Mode"] {
        let mut h = page("bevelEmboss");
        let rows = open_list(&mut h, name);
        assert_grouped(&rows, name);
    }
}

#[test]
fn a_pick_stores_the_mode_name_as_before() {
    let mut h = page("innerShadow");
    open_list(&mut h, "Blend Mode");
    h.get_all_by_label("Linear Dodge (Add)").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().1["effects"][0]["params"]["blend"], "Linear Dodge (Add)");
    open_list(&mut h, "Blend Mode");
    h.get_all_by_label("Hue").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().1["effects"][0]["params"]["blend"], "Hue");
}
