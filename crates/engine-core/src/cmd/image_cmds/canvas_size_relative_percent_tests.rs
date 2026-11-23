//! #14541: Canvas Size with Relative on takes Percent, as the reference application does. The
//! percentage is of each current side and is added to it, so +50 % on 400 x 300 px gives
//! 600 x 450 px and -50 % gives 200 x 150 px. A recorded step with `Rltv` and percent plays back
//! the same way, and the result is clamped to 1..=300000 px as in absolute mode.

use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use craft_psd::descriptor::{Descriptor, UnicodeString, Value as D};
use serde_json::{Value, json};

use crate::Session;
use crate::actions_cmds::{Action, ActionStep};

fn doc(width: u32, height: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": width, "height": height, "background": "white"})).unwrap();
    s
}

fn size(s: &Session) -> (u32, u32) {
    let d = &s.active().unwrap().doc;
    (d.size.width, d.size.height)
}

fn relative_percent(width: f64, height: f64) -> Value {
    json!({"units": "percent", "relative": true, "width": width, "height": height})
}

/// A recorded Canvas Size step with Relative on and both sides in percent.
fn recorded_step(width: f64, height: f64) -> Descriptor {
    Descriptor::new("null")
        .with("Rltv", D::Boolean(true))
        .with("Wdth", D::UnitFloat { unit: *b"#Prc", value: width })
        .with("Hght", D::UnitFloat { unit: *b"#Prc", value: height })
}

/// One action file holding a single recorded Canvas Size step.
fn action_file(descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Canvas"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Step"),
            items: vec![AtnItem { event: AtnEvent::Code(*b"CnvS"), dictionary: b"CnvS".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn a_relative_percentage_is_added_to_each_side() {
    let mut s = doc(400, 300);
    s.execute("image.canvasSize", relative_percent(50.0, 50.0)).unwrap();
    assert_eq!(size(&s), (600, 450));

    let mut s = doc(400, 300);
    s.execute("image.canvasSize", relative_percent(-50.0, -50.0)).unwrap();
    assert_eq!(size(&s), (200, 150));

    // An omitted side keeps its size.
    let mut s = doc(400, 300);
    s.execute("image.canvasSize", json!({"units": "percent", "relative": true, "width": 25})).unwrap();
    assert_eq!(size(&s), (500, 300));
}

#[test]
fn a_relative_percentage_is_clamped_like_an_absolute_size() {
    let mut s = doc(400, 300);
    s.execute("image.canvasSize", relative_percent(-100.0, -1000.0)).unwrap();
    assert_eq!(size(&s), (1, 1));

    let mut s = doc(1, 1);
    s.execute("image.canvasSize", json!({"units": "percent", "relative": true, "width": 1e12})).unwrap();
    assert_eq!(size(&s), (300_000, 1));
}

#[test]
fn a_relative_percentage_that_is_not_a_number_is_refused_without_a_change() {
    let mut s = doc(400, 300);
    for bad in [json!("50"), json!(true), json!([50])] {
        let err = s.execute("image.canvasSize", json!({"units": "percent", "relative": true, "width": bad})).unwrap_err();
        assert!(err.to_string().contains("`width`"), "{err}");
        assert_eq!(size(&s), (400, 300));
    }
}

#[test]
fn a_recorded_relative_percent_step_plays_back_as_the_command() {
    // A width-only step grows the width and keeps the omitted height.
    let width_only = Descriptor::new("null").with("Rltv", D::Boolean(true)).with("Wdth", D::UnitFloat { unit: *b"#Prc", value: 200.0 });
    for (case, step, expected) in
        [("+50 %", recorded_step(50.0, 50.0), (600, 450)), ("-50 %", recorded_step(-50.0, -50.0), (200, 150)), ("width +200 %", width_only, (1200, 300))]
    {
        let mut s = doc(400, 300);
        let data = photocraft_paint::tile::b64_encode(&action_file(step));
        let loaded = s.execute("actions.load", json!({"data": data})).unwrap();
        assert_eq!(loaded["unsupported"], json!([]), "{case}");
        let result = s.execute("actions.play", json!({"action": loaded["actions"][0]})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(size(&s), expected, "{case}");
    }
}

#[test]
fn a_relative_percent_step_saves_and_reloads_unchanged() {
    for pct in [50.0, -50.0] {
        let step = ActionStep::new("image.canvasSize", json!({"units": "percent", "relative": true, "width": pct, "height": pct, "anchor": "center"}));
        let action = Action::new("Recorded", vec![step.clone()]);
        let (bytes, _) = crate::atn_cmds::export("Canvas", &[&action], &[], false).unwrap();
        let imported = crate::atn_cmds::import(&bytes).unwrap();
        assert!(imported.unsupported.is_empty(), "{pct} %");
        assert_eq!(imported.actions[0].steps[0].command, "image.canvasSize", "{pct} %");
        let params = &imported.actions[0].steps[0].params;
        assert_eq!((&params["units"], &params["relative"]), (&json!("percent"), &json!(true)), "{pct} %");
        assert_eq!((params["width"].as_f64(), params["height"].as_f64()), (Some(pct), Some(pct)), "{pct} %");
    }
}
