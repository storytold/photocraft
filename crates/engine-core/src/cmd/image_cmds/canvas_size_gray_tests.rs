//! #14543: Canvas Size takes Gray as its extension colour, the reference application's 128,128,128,
//! from the command and from recorded `.atn` steps (`canvasExtensionColorType` `Gry `), and a
//! `gray` step is written back as Gray.

use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use craft_psd::descriptor::{Descriptor, Id, UnicodeString, Value as D};
use serde_json::{Value, json};

use crate::Session;
use crate::actions_cmds::{Action, ActionStep};

/// A 400 x 300 document at `depth` with a white Background and an empty layer "Top" above it.
fn doc(depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s
}

/// The 8-bit RGB the document shows at pixel (x, y), as the Eyedropper reads it.
fn read(s: &mut Session, x: i32, y: i32) -> [i64; 3] {
    let v = s.execute("document.sampleColor", json!({"x": f64::from(x) + 0.5, "y": f64::from(y) + 0.5})).unwrap();
    let c = |i: usize| (v[i].as_f64().unwrap() * 255.0).round() as i64;
    [c(0), c(1), c(2)]
}

/// The alpha of the layer "Top" at pixel (x, y).
fn top_alpha(s: &Session, x: i32, y: i32) -> f32 {
    let doc = &s.active().unwrap().doc;
    let top = doc.layers.iter().find(|l| l.name == "Top").unwrap();
    top.surface().unwrap().rgba(x, y)[3]
}

const GROW: fn(&str) -> Value = |color| json!({"width": 500, "height": 400, "anchor": "topLeft", "extensionColor": color});

#[test]
fn gray_extends_the_background_with_128_gray_at_8_and_16_bits() {
    for depth in [8, 16] {
        let mut s = doc(depth);
        s.execute("image.canvasSize", GROW("gray")).unwrap();
        assert_eq!(read(&mut s, 450, 350), [128, 128, 128], "depth {depth}: the new area");
        assert_eq!(read(&mut s, 10, 10), [255, 255, 255], "depth {depth}: the old Background keeps its pixels");
        assert_eq!(top_alpha(&s, 450, 350), 0.0, "depth {depth}: only the Background is filled");
    }
}

/// The Background's stored colour at pixel (x, y).
fn background_value(s: &Session, x: i32, y: i32) -> [f32; 4] {
    s.active().unwrap().doc.background().unwrap().surface().unwrap().rgba(x, y)
}

/// An Indexed Color document was not measured and keeps today's behaviour: Gray takes the
/// background colour there, as an unknown value does.
#[test]
fn gray_in_an_indexed_document_keeps_the_background_colour() {
    let grown = |color: &str| {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
        s.execute("image.mode.indexedColor", json!({})).unwrap();
        s.tools.background = [0.0, 0.0, 1.0, 1.0];
        s.execute("image.canvasSize", GROW(color)).unwrap();
        background_value(&s, 450, 350)
    };
    assert_eq!(grown("gray"), grown("background"), "Gray fills with the background colour");
    assert_ne!(grown("gray"), grown("white"), "the test tells the two apart");
}

#[test]
fn an_unknown_extension_colour_still_takes_the_background_colour() {
    let mut s = doc(8);
    s.execute("image.canvasSize", GROW("grey-ish")).unwrap();
    assert_eq!(read(&mut s, 450, 350), [255, 255, 255]);
}

#[test]
fn the_params_doc_lists_gray() {
    // The registry lists the commands once a session has installed them.
    let _ = crate::session();
    let spec = crate::registry::find("image.canvasSize").unwrap();
    assert!(spec.params.contains("|gray|"), "{}", spec.params);
}

/// A recorded Canvas Size step: +100 / +100 px, relative, from the top left, extension colour type `ty`.
fn recorded_step(ty: &str) -> Vec<u8> {
    let px = |v: f64| D::UnitFloat { unit: *b"#Pxl", value: v };
    let enm = |t: &str, v: &str| D::Enumerated { type_id: Id::new(t), value: Id::new(v) };
    let descriptor = Descriptor::new("null")
        .with("Wdth", px(100.0))
        .with("Hght", px(100.0))
        .with("Rltv", D::Boolean(true))
        .with("Hrzn", enm("HrzL", "Left"))
        .with("Vrtc", enm("VrtL", "Top "))
        .with("canvasExtensionColorType", enm("canvasExtensionColorType", ty));
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
fn a_recorded_gray_step_imports_as_gray_and_plays_back_128_gray() {
    let bytes = recorded_step("Gry ");
    let imported = crate::atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty(), "the Gray step is supported: {:?}", imported.unsupported);
    assert_eq!(imported.actions[0].steps[0].params["extensionColor"], json!("gray"));
    let mut s = doc(8);
    let loaded = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]));
    let played = s.execute("actions.play", json!({"action": loaded["actions"][0]})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(read(&mut s, 450, 350), [128, 128, 128]);
}

#[test]
fn a_gray_step_is_written_back_as_gray() {
    let action = Action::new("Step", vec![ActionStep::new("image.canvasSize", GROW("gray"))]);
    let (bytes, skipped) = crate::atn_cmds::export("Canvas", &[&action], &[], false).unwrap();
    assert!(skipped.is_empty(), "{} skipped", skipped.len());
    let file = atn::parse(&bytes).unwrap();
    let d = file.actions[0].items[0].descriptor.as_ref().unwrap();
    assert!(
        matches!(d.get("canvasExtensionColorType"), Some(D::Enumerated { value, .. }) if *value == Id::new("Gry ")),
        "{:?}",
        d.get("canvasExtensionColorType")
    );
    assert!(d.get("canvasExtensionColor").is_none(), "not a custom colour");
}
