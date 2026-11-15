//! #15003: the "+" on the selected Stroke row inserts a copy of that stroke (every value, a new
//! id) directly above it and selects the copy, as measured in the reference. The fx menu, the "+"
//! on the Stroke row of a layer without a stroke, the "+" on a row that is not selected (not
//! measured) and the other kinds keep adding fresh values.
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};

use super::*;

/// Fields with one instance of `kind` per `params`, each over its fresh values, `selected` on
/// the instance at that index.
fn fields(kind: &str, params: &[Value], selected: usize) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("effects".into(), json!([]));
    for p in params {
        add_instance(&mut f, kind);
        let id = f["selected"].as_str().unwrap().to_string();
        for (k, v) in p.as_object().unwrap() {
            set_param(&mut f, &id, k, v.clone());
        }
    }
    let id = f["effects"][selected]["id"].clone();
    f.insert("selected".into(), id);
    f
}

fn blue() -> Value {
    json!({"size": 5.0, "position": "outside", "color": "#0000ff"})
}

fn red() -> Value {
    json!({"size": 10.0, "position": "outside", "color": "#ff0000"})
}

/// The effect list, as the dialog draws it.
fn list(f: Map<String, Value>) -> Harness<'static, Map<String, Value>> {
    let mut h = Harness::builder().with_size(vec2(240.0, 600.0)).build_ui_state(
        |ui, f: &mut Map<String, Value>| {
            let selected = f.get("selected").and_then(Value::as_str).unwrap_or_default().to_string();
            effect_list(ui, f, &selected);
        },
        f,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

/// Clicks the "+" of the `row`th instance row (the rightmost 18 px button on it).
fn click_plus(h: &mut Harness<'static, Map<String, Value>>, row: usize) {
    let mut buttons: Vec<egui::Rect> = h
        .query_all(by())
        .filter_map(|n| n.accesskit_node().bounding_box())
        .map(|r| egui::Rect::from_min_max(egui::pos2(r.x0 as f32, r.y0 as f32), egui::pos2(r.x1 as f32, r.y1 as f32)))
        .filter(|r| (r.width() - 18.0).abs() < 0.5 && (r.height() - 18.0).abs() < 0.5)
        .collect();
    buttons.sort_by(|a, b| a.center().y.total_cmp(&b.center().y).then(b.center().x.total_cmp(&a.center().x)));
    buttons.dedup_by(|b, a| (a.center().y - b.center().y).abs() < 1.0);
    let Some(plus) = buttons.get(row).copied() else { panic!("row {row} has a +: {buttons:?}") };
    let pos = plus.center();
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

fn effects(h: &Harness<'static, Map<String, Value>>) -> Vec<Value> {
    h.state()["effects"].as_array().unwrap().clone()
}

fn color(e: &Value) -> &str {
    e["params"]["color"].as_str().unwrap_or_default()
}

#[test]
fn plus_on_the_selected_blue_row_puts_a_copy_above_it() {
    let mut h = list(fields("stroke", &[blue(), red()], 0));
    let before = effects(&h);
    click_plus(&mut h, 0);
    let after = effects(&h);
    assert_eq!(after.iter().map(color).collect::<Vec<_>>(), ["#0000ff", "#0000ff", "#ff0000"]);
    assert_eq!(after[0]["params"], before[0]["params"], "the copy has every value of its source");
    assert_ne!(after[0]["id"], before[0]["id"], "the copy has its own id");
    assert_eq!(after[1], before[0], "the source is unchanged");
    assert_eq!(h.state()["selected"], after[0]["id"], "the copy is selected");
}

#[test]
fn plus_on_the_selected_red_row_puts_a_copy_above_it() {
    let mut h = list(fields("stroke", &[blue(), red()], 1));
    let before = effects(&h);
    click_plus(&mut h, 1);
    let after = effects(&h);
    assert_eq!(after.iter().map(color).collect::<Vec<_>>(), ["#0000ff", "#ff0000", "#ff0000"]);
    assert_eq!(after[1]["params"], before[1]["params"]);
    assert_eq!(h.state()["selected"], after[1]["id"], "the copy is selected");
    assert_eq!(&after[2], &before[1]);
}

#[test]
fn the_copy_takes_every_value_and_changes_on_its_own() {
    let gradient = json!({"size": 7.0, "position": "center", "blend": "Multiply", "opacity": 40.0, "overprint": true,
        "fillType": "gradient", "color": "#00ff00", "stops": [{"at": 0.0, "color": "#000000"}, {"at": 1.0, "color": "#ffffff"}],
        "style": "radial", "angle": 30.0, "scale": 80.0, "reverse": true});
    let f = fields("stroke", &[gradient], 0);
    let source = f["effects"][0].clone();
    let mut f = f;
    copy_instance(&mut f, source["id"].as_str().unwrap());
    let copy = f["effects"][0].clone();
    assert_eq!(copy["params"], source["params"], "every value, the gradient's too");
    assert_eq!(copy["on"], source["on"]);
    assert_eq!(copy["kind"], "stroke");
    let copy_id = copy["id"].as_str().unwrap().to_string();
    set_param(&mut f, &copy_id, "size", json!(20.0));
    set_param(&mut f, &copy_id, "color", json!("#123456"));
    assert_eq!(f["effects"][1], source, "changing the copy leaves the source as it was");
}

#[test]
fn plus_on_a_stroke_row_that_is_not_selected_keeps_adding_fresh_values() {
    let mut h = list(fields("stroke", &[blue(), red()], 0));
    click_plus(&mut h, 1);
    let after = effects(&h);
    assert_eq!(after.len(), 3);
    assert_eq!(after[2]["params"], fresh_params("stroke", 120.0, None), "a fresh stroke after the last");
    assert_eq!(h.state()["selected"], after[2]["id"]);
}

/// Clicks the greyed (empty) Stroke row of a list that has no effects, as a user does.
fn click_empty_stroke_row(h: &mut Harness<'static, Map<String, Value>>) {
    let mut rows: Vec<egui::Rect> = h
        .query_all(by())
        .filter_map(|n| n.accesskit_node().bounding_box())
        .map(|r| egui::Rect::from_min_max(egui::pos2(r.x0 as f32, r.y0 as f32), egui::pos2(r.x1 as f32, r.y1 as f32)))
        .filter(|r| (r.height() - 26.0).abs() < 0.5 && r.width() > 100.0)
        .collect();
    rows.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
    rows.dedup_by(|b, a| (a.center().y - b.center().y).abs() < 1.0);
    // Styles and Blending Options come first, then one row per kind, all greyed here.
    let index = 2 + KINDS.iter().position(|k| k.0 == "stroke").unwrap_or(0);
    let Some(row) = rows.get(index).copied() else { panic!("row {index} exists: {rows:?}") };
    let pos = row.left_center() + vec2(60.0, 0.0);
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn the_empty_stroke_row_adds_exactly_one_fresh_stroke() {
    let mut f = Map::new();
    f.insert("effects".into(), json!([]));
    f.insert("selected".into(), json!(BLENDING));
    let mut h = list(f);
    click_empty_stroke_row(&mut h);
    let after = effects(&h);
    assert_eq!(after.len(), 1, "one entry more, not a copy of anything: {after:?}");
    assert_eq!(after[0]["kind"], "stroke");
    assert_eq!(after[0]["params"], fresh_params("stroke", 120.0, None), "default values");
    assert_eq!(h.state()["selected"], after[0]["id"]);
}

#[test]
fn the_fx_menu_adds_exactly_one_fresh_stroke_after_the_last() {
    let mut f = fields("stroke", &[blue()], 0);
    pick_from_fx_menu(&mut f, "stroke");
    let a = f["effects"].as_array().unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(a[1]["params"], fresh_params("stroke", 120.0, None), "the fx menu adds fresh values: {a:?}");
    assert_eq!(a[0]["params"]["color"], "#0000ff", "the existing stroke is untouched");
}

#[test]
fn a_source_that_is_not_an_object_is_left_alone() {
    let mut f = Map::new();
    f.insert("effects".into(), json!([5]));
    copy_instance(&mut f, "fx1");
    assert_eq!(f["effects"], json!([5]));
    let mut f = Map::new();
    f.insert("effects".into(), json!([{"id": "fx1"}, 7]));
    f.insert("selected".into(), json!("fx1"));
    copy_instance(&mut f, "fx1");
    assert_eq!(f["effects"].as_array().map(Vec::len), Some(3), "an object source still copies");
}

#[test]
/// Pins the scope of #15003 (Stroke only), not measured parity: a later change may copy other kinds too.
fn plus_on_a_selected_drop_shadow_row_keeps_adding_fresh_values_in_this_scope() {
    let shadow = json!({"distance": 12.0, "size": 9.0});
    let mut h = list(fields("dropShadow", &[shadow.clone(), shadow], 0));
    click_plus(&mut h, 0);
    let after = effects(&h);
    assert_eq!(after.len(), 3);
    assert_eq!(after[2]["params"], fresh_params("dropShadow", 120.0, None), "a fresh shadow after the last");
    assert_eq!(h.state()["selected"], after[2]["id"]);
}
