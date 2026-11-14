//! #14183: Check All Layers keeps the state Check Spelling was last closed with.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::tests::app_with_text;
use super::*;

const ALL_LAYERS: &str = "Check All Layers";

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app| {
            // The fonts are set up after the harness's first frame.
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                windows(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    h
}

/// Edit › Check Spelling… as the menu runs it.
fn open(h: &mut Harness<'static, PhotocraftApp>, params: Value) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.checkSpelling", params).unwrap();
    h.run_steps(3);
}

fn ticked(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.get_by_label(ALL_LAYERS).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

fn click(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

#[test]
fn the_next_open_keeps_check_all_layers_as_it_was_closed() {
    let mut h = harness(app_with_text("Teh cat"));
    open(&mut h, json!({}));
    assert!(ticked(&h), "the first open is on");
    click(&mut h, ALL_LAYERS);
    assert!(!ticked(&h));
    click(&mut h, "Done");
    assert!(h.state().ui.type_panels.spell.is_none(), "Done closed the dialog");
    open(&mut h, json!({}));
    assert!(!ticked(&h), "unticked as it was closed");
    click(&mut h, ALL_LAYERS);
    click(&mut h, "Done");
    open(&mut h, json!({}));
    assert!(ticked(&h), "ticked again as it was closed");
}

#[test]
fn an_explicit_all_layers_wins_over_the_remembered_state() {
    let mut h = harness(app_with_text("Teh cat"));
    open(&mut h, json!({}));
    click(&mut h, ALL_LAYERS);
    click(&mut h, "Done");
    open(&mut h, json!({"allLayers": true}));
    assert!(ticked(&h), "the param wins");
    click(&mut h, "Done");
    open(&mut h, json!({}));
    assert!(ticked(&h), "the param's run was closed ticked");
}

#[test]
fn a_played_check_spelling_step_neither_reads_nor_changes_the_remembered_state() {
    use photocraft_engine::actions_cmds::Action;
    let mut h = harness(app_with_text("Teh cat"));
    open(&mut h, json!({}));
    click(&mut h, ALL_LAYERS);
    click(&mut h, "Done");
    let mut a = Action::new("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": true}))]);
    a.dialogs.insert(0);
    h.state_mut().session.actions.list.push(a);
    crate::actions::play(h.state_mut(), &json!({"action": "Spell"})).unwrap();
    h.run_steps(3);
    assert!(h.state().ui.type_panels.spell.as_ref().is_some_and(|d| d.resume.is_some()), "the step paused at its dialog");
    assert!(ticked(&h), "the step opens with its own Check All Layers");
    click(&mut h, "Done");
    open(&mut h, json!({}));
    assert!(!ticked(&h), "the menu still opens with the state its own run was closed with");
}

#[test]
fn without_an_active_layer_a_remembered_off_opens_on_as_before() {
    let mut h = harness(app_with_text("Teh cat"));
    open(&mut h, json!({}));
    click(&mut h, ALL_LAYERS);
    click(&mut h, "Done");
    h.state_mut().run("select.deselectLayers", json!({})).unwrap();
    open(&mut h, json!({}));
    assert!(h.state().ui.type_panels.spell.is_some(), "the dialog opens");
    assert!(ticked(&h), "off needs a layer to check");
}

#[test]
fn the_menu_reopening_an_open_dialog_keeps_its_state() {
    let mut h = harness(app_with_text("Teh cat"));
    open(&mut h, json!({}));
    click(&mut h, ALL_LAYERS);
    open(&mut h, json!({}));
    assert!(!ticked(&h));
}

#[test]
fn recording_with_a_remembered_off_records_it() {
    let mut h = harness(app_with_text("Teh cat"));
    open(&mut h, json!({}));
    click(&mut h, ALL_LAYERS);
    click(&mut h, "Done");
    h.state_mut().run("actions.record", json!({"name": "Spell"})).unwrap();
    open(&mut h, json!({}));
    click(&mut h, "Done");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let steps = &h.state().session.actions.list[0].steps;
    assert_eq!(steps, &vec![("edit.checkSpelling".to_string(), json!({"allLayers": false}))]);
}
