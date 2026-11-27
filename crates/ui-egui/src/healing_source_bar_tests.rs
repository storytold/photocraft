//! #12104: the Healing Brush's options bar gains the Source pair - a Sampled · Pattern segmented
//! control after Mode, with the pattern picker beside it - and the Clone Stamp's bar keeps the
//! cells it had. Sampled by default with the picker greyed; Pattern enables it and the next stroke
//! carries `sourceMode:"pattern"` plus the picker's pattern, with no "set the source" hint and no
//! no-source message. An Alt-clicked clone source wins: it sets Source back to Sampled. Reset Tool
//! returns Source to Sampled and keeps the chosen pattern.

use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// A Healing Brush app with a document; the caller's tool changes are applied afterwards.
fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    a.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    a.ui.tool = tool;
    a
}

/// The bar of `a`, on the harness clock.
fn bar(a: Box<PhotocraftApp>) -> H {
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_app(|ui, app| crate::panels::options_bar(app, ui), a);
    h.run_steps(4);
    h
}

/// The bar's segment names, in order (the Source pair is this tool's only radio group).
fn segments(h: &H) -> Vec<String> {
    h.query_all_by_role(Role::RadioButton).filter_map(|n| n.accesskit_node().label()).collect()
}

fn segment(h: &H, name: &str) -> bool {
    segments(h).iter().any(|n| n == name)
}

fn picked(h: &H, name: &str) -> bool {
    h.query_all_by_role(Role::RadioButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(name))
        .is_some_and(|n| n.accesskit_node().toggled() == Some(Toggled::True))
}

/// Whether the bar shows the Pattern picker, and whether it is greyed.
fn picker(h: &H) -> Option<bool> {
    h.query_all_by_role(Role::ComboBox)
        .find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == "Pattern"))
        .map(|n| n.accesskit_node().is_disabled())
}

/// The params the next Healing Brush stroke would carry.
fn stroke_params(a: &PhotocraftApp) -> Option<serde_json::Value> {
    crate::retouch_ui::clone_params(a)
}

/// The library pattern the picker holds when the tool has chosen none.
fn first_pattern(a: &PhotocraftApp) -> String {
    a.session.patterns.items[0].id.clone()
}

#[test]
fn the_healing_bar_shows_source_sampled_by_default_with_a_greyed_picker() {
    let h = bar(app(Tool::Healing));
    assert!(segment(&h, "Sampled") && segment(&h, "Pattern"), "the Source segments: {:?}", segments(&h));
    assert!(picked(&h, "Sampled"), "Source starts at Sampled");
    assert_eq!(h.state().ui.tool_options.healing_source, "sampled");
    assert_eq!(picker(&h), Some(true), "the pattern picker is greyed while Source is Sampled");
    assert!(stroke_params(h.state()).is_none(), "with no clone source set there is no sampled stroke");
    // A pick of the segment works and reads back on the next pass.
    let mut h = h;
    h.query_all_by_role(Role::RadioButton).find(|n| n.accesskit_node().label().as_deref() == Some("Sampled")).expect("the Sampled segment").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.healing_source, "sampled", "picking the segment already chosen keeps Sampled");
    // The Clone Stamp's bar keeps its own cells: no Source control appears there.
    let c = bar(app(Tool::CloneStamp));
    assert!(!segment(&c, "Sampled") && !segment(&c, "Pattern"), "the Clone Stamp bar: {:?}", segments(&c));
    assert_eq!(picker(&c), None, "no pattern picker on the Clone Stamp");
    // Reset Tool returns Source to Sampled and keeps the chosen pattern (the measured row): the
    // bar's Pattern pick stores the pattern, Reset Tool puts only the Source back.
    h.query_all_by_role(Role::RadioButton).find(|n| n.accesskit_node().label().as_deref() == Some("Pattern")).expect("the Pattern segment").click();
    h.run_steps(3);
    let kept = h.state().ui.tool_options.healing_pattern.clone();
    assert_eq!(kept, first_pattern(h.state()), "the Pattern pick stores the picker's pattern");
    let ctx = h.state().ctx().clone();
    crate::menus::invoke(h.state_mut(), &ctx, crate::tool_reset::RESET, json!({})).expect("Reset Tool runs");
    assert_eq!(h.state().ui.tool_options.healing_source, "sampled", "Reset Tool returns Source to Sampled");
    assert_eq!(h.state().ui.tool_options.healing_pattern, kept, "and keeps the chosen pattern");
    assert_eq!(h.state().ui.tool_options.healing_pattern, first_pattern(h.state()), "the kept pattern is still the picker's");
    assert!(stroke_params(h.state()).is_none(), "and after the reset the bar asks the next stroke for a source again");
}

#[test]
fn pattern_source_sends_the_pattern_and_silences_the_source_hint() {
    let mut h = bar(app(Tool::Healing));
    let before = h.state().session.active().unwrap().history.past_len();
    h.query_all_by_role(Role::RadioButton).find(|n| n.accesskit_node().label().as_deref() == Some("Pattern")).expect("the Pattern segment").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.healing_source, "pattern", "the segment sets the option");
    assert!(picked(&h, "Pattern"), "and the segment reads picked");
    assert_eq!(picker(&h), Some(false), "the picker is enabled with Pattern");
    let params = stroke_params(h.state()).expect("a pattern stroke needs no Alt-set source");
    assert_eq!(params["sourceMode"], json!("pattern"), "the next stroke carries the source");
    assert_eq!(params["pattern"], json!(first_pattern(h.state())), "and the picker's pattern");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before, "the pick itself adds no History step");
    // The "Alt-click to set the source" hint is away, and a stroke asks for no source.
    assert!(h.query_all_by_label("Alt-click to set the source").next().is_none(), "no source hint with Pattern");
    h.state_mut().ui.status.clear();
    crate::retouch_ui::finish_stroke(h.state_mut(), Tool::Healing, &[[16.0, 16.0, 1.0]], Default::default(), false);
    assert_ne!(h.state().ui.status, "Alt-click to define a source point to clone from", "no no-source message");
    let (_, params) = h.state().session.journal.iter().rev().find(|(id, _)| id == "paint.healingBrush").expect("the stroke ran");
    assert_eq!(params["sourceMode"], json!("pattern"), "the journaled params: {params}");
}

#[test]
fn an_alt_set_source_wins_and_returns_the_control_to_sampled() {
    let mut a = app(Tool::Healing);
    a.ui.tool_options.healing_source = "pattern".into();
    a.ui.tool_options.healing_pattern = first_pattern(&a);
    crate::retouch_ui::set_source(&mut a, 8.0, 8.0);
    assert_eq!(a.ui.tool_options.healing_source, "sampled", "the Alt-click sets Source back to Sampled");
    let p = stroke_params(&a).expect("the sampled stroke");
    assert!(p.get("sourceMode").is_none() && p.get("pattern").is_none(), "the next stroke samples: {p}");
    // And when a source is set by any route, leaving the control on Pattern still samples.
    let mut b = app(Tool::Healing);
    b.ui.tool_options.healing_pattern = first_pattern(&b);
    crate::retouch_ui::set_source(&mut b, 8.0, 8.0);
    b.ui.tool_options.healing_source = "pattern".into();
    let p = stroke_params(&b).expect("the sampled stroke");
    assert!(p.get("sourceMode").is_none(), "a set clone source keeps the stroke sampled: {p}");
    // A stroke with the source set paints sampled, not the pattern.
    let before = b.session.active().unwrap().history.past_len();
    crate::retouch_ui::finish_stroke(&mut b, Tool::Healing, &[[16.0, 16.0, 1.0]], Default::default(), false);
    assert_eq!(b.session.active().unwrap().history.past_len(), before + 1, "the sampled stroke ran");
    let (_, params) = b.session.journal.iter().rev().find(|(id, _)| id == "paint.healingBrush").expect("the stroke ran");
    assert!(params.get("sourceMode").is_none(), "{params}");
}

#[test]
fn switching_back_to_sampled_greys_the_picker_and_keeps_the_pattern() {
    let mut h = bar(app(Tool::Healing));
    h.query_all_by_role(Role::RadioButton).find(|n| n.accesskit_node().label().as_deref() == Some("Pattern")).expect("the Pattern segment").click();
    h.run_steps(3);
    let chosen = h.state().ui.tool_options.healing_pattern.clone();
    assert!(!chosen.is_empty(), "the picker holds the library's current pattern");
    h.query_all_by_role(Role::RadioButton).find(|n| n.accesskit_node().label().as_deref() == Some("Sampled")).expect("the Sampled segment").click();
    h.run_steps(3);
    assert_eq!(picker(&h), Some(true), "the picker is greyed again");
    assert_eq!(h.state().ui.tool_options.healing_pattern, chosen, "the chosen pattern stays");
    assert!(stroke_params(h.state()).is_none(), "with no clone source and Sampled there is no stroke to run");
}
