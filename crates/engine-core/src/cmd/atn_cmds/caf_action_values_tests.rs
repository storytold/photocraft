//! Recorded `cafWorkspace` steps: every measured value maps to the command and back, and an
//! empty step plays with the workspace's remembered settings (#14254).
use super::*;
use crate::actions_cmds::{ActionList, ActionStep, ActionsExt};

const CMD: &str = "edit.contentAwareFill";
const MARK: &str = "_atnCafRemembered";

/// A seven-key step with these enumerations and every boolean off.
fn workspace(sampling: &str, color: &str, rotation: &str, output: &str) -> Descriptor {
    desc("null")
        .with("cafSamplingRegion", enm("cafSamplingRegion", sampling))
        .with("cafSampleAllLayers", D::Boolean(false))
        .with("cafColorAdaptationLevel", enm("cafColorAdaptationLevel", color))
        .with("cafRotationAmount", enm("cafRotationAmount", rotation))
        .with("cafScale", D::Boolean(false))
        .with("cafMirror", D::Boolean(false))
        .with("cafOutput", enm("cafOutput", output))
}

/// The params [`workspace`] imports as.
fn params(sampling: &str, color: &str, rotation: &str, output: &str) -> Value {
    json!({"sampling":sampling,"sampleAllLayers":false,"colorAdaptation":color,
        "rotationAdaptation":rotation,"scale":false,"mirror":false,"output":output})
}

fn item(d: Descriptor) -> AtnItem {
    AtnItem { event: AtnEvent::name("cafWorkspace"), dictionary: b"Content-Aware Fill".to_vec(), descriptor: Some(d), ..Default::default() }
}

fn file(d: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("S"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items: vec![item(d)], ..Default::default() }],
        ..Default::default()
    })
    .unwrap()
}

/// The single step a one-step file imports as.
fn load(d: Descriptor) -> Action {
    let loaded = import(&file(d)).unwrap();
    assert!(loaded.unsupported.is_empty(), "{:?}", loaded.unsupported);
    loaded.actions.into_iter().next().unwrap()
}

/// The item Save Actions writes for one step, or `None` when it refuses the step. A save that succeeds must
/// write a readable file holding the item.
fn exported(p: Value) -> Option<AtnItem> {
    let a = Action::new("A", vec![ActionStep::new(CMD, p)]);
    let (bytes, skipped) = export("S", &[&a], &[], false).ok()?;
    assert!(skipped.is_empty());
    let saved = atn::parse(&bytes).expect("a successful save writes a readable file");
    Some(saved.actions.into_iter().next().and_then(|a| a.items.into_iter().next()).expect("the saved file holds the step"))
}

#[test]
fn every_measured_value_imports_to_its_param_and_saves_back_unchanged() {
    let base = ("cafSamplingRegionRectangular", "cafColorAdaptationDefault", "cafRotationAmountNone", "cafOutputToNewLayer");
    let mut cases = Vec::new();
    for (wire, value) in [("cafSamplingRegionAuto", "auto"), ("cafSamplingRegionRectangular", "rectangular"), ("cafSamplingRegionCustom", "custom")] {
        cases.push((workspace(wire, base.1, base.2, base.3), params(value, "default", "none", "new")));
    }
    for (wire, value) in [
        ("cafColorAdaptationNone", "none"),
        ("cafColorAdaptationDefault", "default"),
        ("cafColorAdaptationHigh", "high"),
        ("cafColorAdaptationVeryHigh", "veryHigh"),
    ] {
        cases.push((workspace(base.0, wire, base.2, base.3), params("rectangular", value, "none", "new")));
    }
    for (wire, value) in [
        ("cafRotationAmountNone", "none"),
        ("cafRotationAmountLow", "low"),
        ("cafRotationAmountMedium", "medium"),
        ("cafRotationAmountHigh", "high"),
        ("cafRotationAmountFull", "full"),
    ] {
        cases.push((workspace(base.0, base.1, wire, base.3), params("rectangular", "default", value, "new")));
    }
    for (wire, value) in [("cafOutputToCurrentLayer", "current"), ("cafOutputToNewLayer", "new"), ("cafOutputToDuplicateLayer", "duplicate")] {
        cases.push((workspace(base.0, base.1, base.2, wire), params("rectangular", "default", "none", value)));
    }
    for (d, p) in cases {
        let action = load(d.clone());
        assert_eq!(action.steps, vec![ActionStep::new(CMD, p.clone())]);
        assert_eq!(exported(p.clone()), Some(item(d.clone())), "{p}");
        let bytes = export("S", &[&action], &[], false).unwrap().0;
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0], item(d));
    }
}

#[test]
fn an_empty_step_imports_as_remembered_and_saves_back_empty() {
    let action = load(desc("null"));
    assert_eq!(action.steps, vec![ActionStep::new(CMD, json!({MARK: true}))]);
    assert_eq!(exported(json!({MARK: true})), Some(item(desc("null"))));
    // A differently named empty descriptor comes back as recorded.
    let mut named = desc("null");
    named.name = UnicodeString::new_nul("Recorded CAF");
    let action = load(named.clone());
    assert_eq!(action.steps[0].params[MARK], json!(true));
    let bytes = export("S", &[&action], &[], false).unwrap().0;
    assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0], item(named));
}

#[test]
fn a_remembered_step_with_values_of_its_own_is_not_saved_as_an_empty_step() {
    for p in [json!({MARK: true, "output": "current"}), json!({MARK: false}), json!({MARK: "yes"}), json!({MARK: true, "seed": 2})] {
        assert_eq!(exported(p.clone()), None, "{p}");
    }
}

/// White 8 x 8 with a red square selected for the fill.
fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width":8,"height":8,"background":"white"})).unwrap();
    s.execute("select.rect", json!({"x":3,"y":3,"width":2,"height":2})).unwrap();
    // Painted directly: Edit › Fill is registered by another engine crate.
    s.edit("paint", |doc, active| {
        let surface = doc.layer_mut(active.ok_or(crate::EngineError::NoDocument)?).and_then(|l| l.surface_mut()).ok_or(crate::EngineError::NoDocument)?;
        for (x, y) in [(3, 3), (4, 3), (3, 4), (4, 4)] {
            surface.write_pixel(x, y, &[1.0, 0.0, 0.0, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    s
}

fn remember(s: &mut Session, settings: Value) {
    s.edit_prefs(|p| p.dialogs.insert(CAF_WORKSPACE_PREFS.into(), settings));
}

/// Play `d` as a one-step action and return the result.
fn play(s: &mut Session, d: Descriptor) -> Value {
    s.actions_mut().list = ActionList::new();
    s.actions_mut().list.push(load(d)).unwrap();
    let result = s.execute("actions.play", json!({"action":0})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(result["ran"], 1);
    result
}

/// Every layer's pixels, in stack order.
fn pixels(s: &Session) -> Vec<Vec<[f32; 4]>> {
    let doc = &s.active().unwrap().doc;
    doc.layers.iter().map(|l| (0..64).map(|i| l.surface().unwrap().rgba(i % 8, i / 8)).collect()).collect()
}

/// What running the command itself with `p` leaves.
fn native(p: Value) -> Vec<Vec<[f32; 4]>> {
    let mut s = session();
    s.execute(CMD, p).unwrap();
    pixels(&s)
}

#[test]
fn an_empty_step_fills_a_new_layer_before_anything_is_remembered() {
    let mut s = session();
    let layers = s.active().unwrap().doc.layers.len();
    let past = s.active().unwrap().history.past_len();
    play(&mut s, desc("null"));
    assert_eq!(s.active().unwrap().doc.layers.len(), layers + 1);
    assert_eq!(s.active().unwrap().history.past_len(), past + 1);
    assert_eq!(pixels(&s), native(json!({"output":"new"})));
    assert!(!s.prefs().dialogs.contains_key(CAF_WORKSPACE_PREFS), "playing remembers nothing");
}

#[test]
fn an_empty_step_plays_with_the_remembered_settings_and_steps_never_change_them() {
    let settings = json!({"showOverlay":false,"opacity":20.0,"sampling":"rectangular","sampleAllLayers":false,
        "colorAdaptation":"high","rotationAdaptation":"full","scale":true,"mirror":true,"output":"current"});
    let mut s = session();
    remember(&mut s, settings.clone());
    let layers = s.active().unwrap().doc.layers.len();
    play(&mut s, desc("null"));
    assert_eq!(s.active().unwrap().doc.layers.len(), layers, "remembered Current Layer");
    let used = json!({"sampling":"rectangular","sampleAllLayers":false,"colorAdaptation":"high",
        "rotationAdaptation":"full","scale":true,"mirror":true,"output":"current"});
    assert_eq!(pixels(&s), native(used));
    assert_eq!(s.prefs().dialogs.get(CAF_WORKSPACE_PREFS), Some(&settings));

    // A step's own values are not remembered: the next empty step still uses Current Layer.
    let mut s = session();
    remember(&mut s, settings.clone());
    let layers = s.active().unwrap().doc.layers.len();
    play(&mut s, workspace("cafSamplingRegionAuto", "cafColorAdaptationNone", "cafRotationAmountLow", "cafOutputToNewLayer"));
    assert_eq!(s.active().unwrap().doc.layers.len(), layers + 1);
    assert_eq!(s.prefs().dialogs.get(CAF_WORKSPACE_PREFS), Some(&settings));
    let next = crate::actions_cmds::playback_params(&s, CMD, &json!({MARK: true})).unwrap();
    assert_eq!(next["output"], "current");
}

#[test]
fn remembered_settings_fill_only_what_the_step_does_not_name() {
    let resolve = |s: &Session, p: Value| crate::actions_cmds::playback_params(s, CMD, &p).unwrap();
    let mut s = crate::session();
    assert_eq!(resolve(&s, json!({MARK: true})), json!({"output":"new"}));
    // Only the command's settings come over, never the overlay's.
    remember(&mut s, json!({"showOverlay":true,"opacity":50.0,"color":[1,2,3],"excluded":true,"sampling":"auto","output":"duplicate"}));
    assert_eq!(resolve(&s, json!({MARK: true})), json!({"sampling":"auto","output":"duplicate"}));
    assert_eq!(resolve(&s, json!({MARK: true, "output": "current"})), json!({"sampling":"auto","output":"current"}));
    // A mark that isn't `true` is dropped without bringing the remembered settings in.
    assert_eq!(resolve(&s, json!({MARK: false, "output": "current"})), json!({"output":"current"}));
    // Steps without the mark and other commands are left alone.
    assert_eq!(resolve(&s, json!({"output":"current"})), json!({"output":"current"}));
    let other = crate::actions_cmds::playback_params(&s, "edit.fill", &json!({MARK: true})).unwrap();
    assert_eq!(other, json!({MARK: true}));
}

#[test]
fn damaged_remembered_settings_are_skipped_without_a_panic() {
    let resolve = |s: &Session| crate::actions_cmds::playback_params(s, CMD, &json!({MARK: true})).unwrap();
    let mut s = crate::session();
    for damaged in [json!(3), json!(null), json!([]), json!({"output":5,"scale":"yes","sampling":true,"mirror":null})] {
        remember(&mut s, damaged.clone());
        assert_eq!(resolve(&s), json!({"output":"new"}), "{damaged}");
    }
    // A remembered value the command refuses fails the step with an error, not a panic.
    let mut s = session();
    remember(&mut s, json!({"output":"sideways"}));
    s.actions_mut().list = ActionList::new();
    s.actions_mut().list.push(load(desc("null"))).unwrap();
    let before = s.active().unwrap().doc.clone();
    let result = s.execute("actions.play", json!({"action":0})).unwrap();
    assert!(result.get("failed").is_some(), "{result}");
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn folder_batch_plays_an_empty_step_with_the_remembered_settings() {
    let dir = tempfile::tempdir().unwrap();
    let join = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
    let mut source = session();
    source.execute("file.saveACopy", json!({"path": join("in.png")})).unwrap();
    let steps = json!([["select.rect", {"x":3,"y":3,"width":2,"height":2}], [CMD, {MARK: true}]]);
    // The layer count of each output: Current Layer fills the background, New Layer adds one.
    let layers = |s: &mut Session, out: &str| {
        let result = s.execute("file.automate.batch", json!({"input":[join("in.png")],"output":join(out),"format":"psd","steps":steps})).unwrap();
        assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
        let bytes = std::fs::read(dir.path().join(out).join("in.psd")).unwrap();
        photocraft_io::import("in.psd", &bytes).unwrap().document.layers.len()
    };
    let mut s = crate::session();
    assert_eq!(layers(&mut s, "default"), 2, "nothing remembered: a new layer");
    remember(&mut s, json!({"output":"current"}));
    assert_eq!(layers(&mut s, "remembered"), 1, "the remembered Current Layer");
    // Playing in Batch remembers nothing in the caller either.
    assert_eq!(s.prefs().dialogs.get(CAF_WORKSPACE_PREFS), Some(&json!({"output":"current"})));
}
