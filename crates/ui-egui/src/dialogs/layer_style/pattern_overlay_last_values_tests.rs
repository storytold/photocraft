use super::tests::{cancel, draft, edit, first_of, layer_effects, ok};
use super::*;

const PATTERN: EffectKind = EffectKind::PatternOverlay;

fn new_document(app: &mut PhotocraftApp) {
    app.run("file.new", json!({"width": 20, "height": 20})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
}

fn custom_pattern(app: &mut PhotocraftApp) -> String {
    new_document(app);
    app.run("edit.fill", json!({"color": "#123456"})).unwrap();
    app.run("edit.definePattern", json!({"name": "Custom Overlay Tile"})).unwrap();
    app.session.patterns.items.iter().find(|pattern| pattern.name == "Custom Overlay Tile").unwrap().id.clone()
}

fn open_pattern_overlay(app: &mut PhotocraftApp) -> DialogId {
    open(app, Some(PATTERN.key())).unwrap()
}

fn params(app: &PhotocraftApp, id: DialogId) -> Value {
    let fx = first_of(app, id, PATTERN);
    style_params_json(&draft(app, id).style.effect(fx).unwrap().params)
}

fn set_overlay(app: &mut PhotocraftApp, dialog: DialogId, pattern: &str) {
    let fx = first_of(app, dialog, PATTERN);
    edit(app, dialog, |draft| {
        for (key, value) in [
            ("pattern", StyleValue::Text(pattern.to_owned())),
            ("blend", StyleValue::Text("Multiply".into())),
            ("opacity", StyleValue::Number(40.0)),
            ("scale", StyleValue::Number(50.0)),
            ("angle", StyleValue::Number(25.0)),
            ("link", StyleValue::Bool(false)),
            ("phaseX", StyleValue::Number(7.0)),
            ("phaseY", StyleValue::Number(4.0)),
        ] {
            draft.set_param(fx, key, value);
        }
    });
}

#[test]
fn reuses_last_committed_pattern_overlay_in_a_new_document() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let pattern = custom_pattern(&mut app);
    let first = open_pattern_overlay(&mut app);
    set_overlay(&mut app, first, &pattern);
    let expected = params(&app, first);
    ok(&mut app, first);
    let applied = layer_effects(&app.session);
    let [Effect::PatternOverlay { id, phase, .. }] = applied.as_slice() else { panic!("committed Pattern Overlay") };
    assert_eq!((id.as_str(), *phase), (pattern.as_str(), (7.0, 4.0)));

    new_document(&mut app);
    let second = open_pattern_overlay(&mut app);
    assert_eq!(params(&app, second), expected);
    cancel(&mut app, second);
}

fn commit_custom_overlay(app: &mut PhotocraftApp) -> (String, Value) {
    let pattern = custom_pattern(app);
    let dialog = open_pattern_overlay(app);
    set_overlay(app, dialog, &pattern);
    let expected = params(app, dialog);
    ok(app, dialog);
    (pattern, expected)
}

#[test]
fn persists_pattern_overlay_params_across_app_restarts() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let (pattern, expected) = commit_custom_overlay(&mut app);
    app.framework.move_memories(&mut app.session);

    let mut session = Session::new();
    session.load_prefs_json(&app.session.prefs_to_json()).unwrap();
    let saved_pattern = app.session.patterns.items.iter().find(|item| item.id == pattern).unwrap().clone();
    session.patterns.upsert(saved_pattern);
    drop(app);
    let mut reopened = PhotocraftApp::new(session, crate::Services::default());
    new_document(&mut reopened);

    let dialog = open_pattern_overlay(&mut reopened);
    assert_eq!(params(&reopened, dialog), expected);
    ok(&mut reopened, dialog);
    let applied = layer_effects(&reopened.session);
    let [Effect::PatternOverlay { id, phase, .. }] = applied.as_slice() else { panic!("committed Pattern Overlay") };
    assert_eq!((id.as_str(), *phase), (pattern.as_str(), (7.0, 4.0)));
}

#[test]
fn cancel_and_script_commands_leave_the_last_confirmed_overlay_unchanged() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let (pattern, expected) = commit_custom_overlay(&mut app);

    new_document(&mut app);
    let dialog = open_pattern_overlay(&mut app);
    let fx = first_of(&app, dialog, PATTERN);
    edit(&mut app, dialog, |draft| draft.set_param(fx, "opacity", StyleValue::Number(80.0)));
    cancel(&mut app, dialog);

    app.run("layer.layerStyle.patternOverlay", json!({"pattern": pattern, "opacity": 66})).unwrap();
    let unrelated = open(&mut app, Some(BLENDING)).unwrap();
    ok(&mut app, unrelated);

    new_document(&mut app);
    let next = open_pattern_overlay(&mut app);
    assert_eq!(params(&app, next), expected);
}

#[test]
fn a_deleted_pattern_falls_back_to_the_first_library_pattern() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let (pattern, mut expected) = commit_custom_overlay(&mut app);
    let first_library_pattern = app.session.patterns.items[0].id.clone();
    app.run("pattern.delete", json!({"pattern": pattern})).unwrap();
    new_document(&mut app);
    expected["pattern"] = json!(first_library_pattern);

    let dialog = open_pattern_overlay(&mut app);
    assert_eq!(params(&app, dialog), expected);
    ok(&mut app, dialog);
    let applied = layer_effects(&app.session);
    let [Effect::PatternOverlay { id, .. }] = applied.as_slice() else { panic!("committed Pattern Overlay") };
    assert_eq!(id, &first_library_pattern);
}

#[test]
fn reset_uses_factory_or_made_defaults_without_replacing_the_last_ok() {
    use egui_kittest::{Harness, kittest::Queryable};

    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let (_, expected) = commit_custom_overlay(&mut app);
    new_document(&mut app);
    let next = open_pattern_overlay(&mut app);

    let mut harness = Harness::builder().with_size(egui::vec2(1500.0, 1000.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            app.framework.dialogs.run(ui);
            take(app, ui.ctx());
        },
        app,
    );
    harness.run_steps(4);
    harness.get_by_label("Reset to Default").click();
    harness.run_steps(4);
    let reset = params(harness.state(), next);
    assert_eq!(reset["opacity"], json!(100.0));
    assert_eq!(reset["scale"], json!(100.0));
    assert_eq!(reset["link"], json!(true));
    assert_eq!(reset["phaseX"], json!(0.0));
    assert_eq!(reset["phaseY"], json!(0.0));

    let fx = first_of(harness.state(), next, PATTERN);
    edit(harness.state_mut(), next, |draft| {
        draft.set_param(fx, "opacity", StyleValue::Number(27.0));
        draft.set_param(fx, "scale", StyleValue::Number(60.0));
    });
    harness.get_by_label("Make Default").click();
    harness.run_steps(4);
    cancel(harness.state_mut(), next);

    let next = open_pattern_overlay(harness.state_mut());
    assert_eq!(params(harness.state(), next), expected);
    harness.run_steps(4);
    harness.get_by_label("Reset to Default").click();
    harness.run_steps(4);
    let reset = params(harness.state(), next);
    assert_eq!(reset["opacity"], json!(27.0));
    assert_eq!(reset["scale"], json!(60.0));
}

#[test]
fn rejected_or_disabled_overlays_do_not_replace_the_last_ok() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let (_, expected) = commit_custom_overlay(&mut app);
    new_document(&mut app);

    let rejected = open_pattern_overlay(&mut app);
    let fx = first_of(&app, rejected, PATTERN);
    edit(&mut app, rejected, |draft| draft.set_param(fx, "pattern", StyleValue::Text("missing".into())));
    ok(&mut app, rejected);
    assert!(app.ui.status_error);
    assert!(layer_effects(&app.session).is_empty());

    let disabled = open_pattern_overlay(&mut app);
    assert_eq!(params(&app, disabled), expected);
    let fx = first_of(&app, disabled, PATTERN);
    edit(&mut app, disabled, |draft| {
        draft.set_param(fx, "opacity", StyleValue::Number(80.0));
        draft.toggle(fx);
    });
    ok(&mut app, disabled);
    let applied = layer_effects(&app.session);
    assert_eq!(applied.len(), 1);
    assert!(!applied[0].enabled());

    new_document(&mut app);
    let next = open_pattern_overlay(&mut app);
    assert_eq!(params(&app, next), expected);
}

fn open_with_two_overlays(app: &mut PhotocraftApp) -> DialogId {
    let (pattern, _) = commit_custom_overlay(app);
    new_document(app);
    let effects = [40.0, 80.0].map(|opacity| json!({"kind": PATTERN.key(), "params": {"pattern": pattern.as_str(), "opacity": opacity}}));
    app.run("layer.layerStyle.replace", json!({"effects": effects})).unwrap();
    open_pattern_overlay(app)
}

fn assert_remembered_opacity(app: &mut PhotocraftApp, opacity: f64) {
    new_document(app);
    let next = open_pattern_overlay(app);
    assert_eq!(params(app, next)["opacity"], json!(opacity));
    cancel(app, next);
}

#[test]
fn remembers_an_overlay_edited_to_match_its_sibling() {
    use egui_kittest::{Harness, kittest::Queryable};

    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let dialog = open_with_two_overlays(&mut app);
    let first = first_of(&app, dialog, PATTERN);
    edit(&mut app, dialog, |draft| draft.set_param(first, "opacity", StyleValue::Number(80.0)));

    let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 680.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            app.framework.dialogs.run(ui);
            take(app, ui.ctx());
        },
        app,
    );
    harness.run_steps(4);
    harness.get_by_label("OK").click();
    harness.run_steps(4);

    assert_remembered_opacity(harness.state_mut(), 80.0);
}

#[test]
fn remembers_a_new_overlay_that_matches_an_existing_one() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let dialog = open_with_two_overlays(&mut app);
    let style = &draft(&app, dialog).style;
    let second = style.effect(style.instances(PATTERN).nth(1).unwrap()).unwrap();
    let duplicate = style_params(PATTERN, &style_params_json(&second.params), None);

    edit(&mut app, dialog, |draft| {
        draft.add_instance(PATTERN, &duplicate);
    });
    ok(&mut app, dialog);

    assert_remembered_opacity(&mut app, 80.0);
}

#[test]
fn removing_an_overlay_does_not_remember_its_unchanged_sibling() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let dialog = open_with_two_overlays(&mut app);
    let first = first_of(&app, dialog, PATTERN);

    edit(&mut app, dialog, |draft| draft.remove_instance(first));
    ok(&mut app, dialog);

    assert_remembered_opacity(&mut app, 40.0);
}

fn two_overlay_ids(app: &PhotocraftApp, dialog: DialogId) -> [EffectId; 2] {
    [0, 1].map(|index| draft(app, dialog).style.instances(PATTERN).nth(index).unwrap())
}

fn edit_on_pass(app: &mut PhotocraftApp, dialog: DialogId, change: impl FnOnce(&mut LayerStyleDraft)) {
    edit(app, dialog, change);
    let ctx = app.ctx().clone();
    take(app, &ctx);
}

#[test]
fn remembers_the_last_edit_when_both_overlay_values_change() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let dialog = open_with_two_overlays(&mut app);
    let [first, second] = two_overlay_ids(&app, dialog);

    edit_on_pass(&mut app, dialog, |draft| draft.set_param(first, "opacity", StyleValue::Number(50.0)));
    edit_on_pass(&mut app, dialog, |draft| draft.set_param(second, "opacity", StyleValue::Number(90.0)));
    ok(&mut app, dialog);

    assert_remembered_opacity(&mut app, 90.0);
}

#[test]
fn remembers_the_last_edit_in_the_ok_result_even_when_it_is_first_in_the_effect_list() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let dialog = open_with_two_overlays(&mut app);
    let [first, second] = two_overlay_ids(&app, dialog);

    edit_on_pass(&mut app, dialog, |draft| draft.set_param(second, "opacity", StyleValue::Number(90.0)));
    edit(&mut app, dialog, |draft| draft.set_param(first, "opacity", StyleValue::Number(50.0)));
    ok(&mut app, dialog);

    assert_remembered_opacity(&mut app, 50.0);
}

#[test]
fn an_undone_or_disabled_last_edit_does_not_replace_the_previous_edit() {
    for undo in [true, false] {
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        let dialog = open_with_two_overlays(&mut app);
        let [first, second] = two_overlay_ids(&app, dialog);

        edit_on_pass(&mut app, dialog, |draft| draft.set_param(first, "opacity", StyleValue::Number(50.0)));
        edit_on_pass(&mut app, dialog, |draft| draft.set_param(second, "opacity", StyleValue::Number(90.0)));
        edit(&mut app, dialog, |draft| {
            if undo {
                draft.set_param(second, "opacity", StyleValue::Number(80.0));
            } else {
                draft.toggle(second);
            }
        });
        ok(&mut app, dialog);

        assert_remembered_opacity(&mut app, 50.0);
    }
}

#[test]
fn remembers_a_new_overlay_that_reuses_a_deleted_overlays_id() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    let dialog = open_with_two_overlays(&mut app);
    let [first, second] = two_overlay_ids(&app, dialog);
    let style = &draft(&app, dialog).style;
    let defaults = style_params(PATTERN, &style_params_json(&style.effect(second).unwrap().params), None);

    edit_on_pass(&mut app, dialog, |draft| draft.set_param(first, "opacity", StyleValue::Number(50.0)));
    edit_on_pass(&mut app, dialog, |draft| draft.remove_instance(second));
    edit(&mut app, dialog, |draft| assert_eq!(draft.add_instance(PATTERN, &defaults), second));
    ok(&mut app, dialog);

    assert_remembered_opacity(&mut app, 80.0);
}
