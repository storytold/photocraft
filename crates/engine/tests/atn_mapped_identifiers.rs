//! Recorded identifiers must match Photoshop, even when a local round trip would accept a typo.

use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D},
};
use serde_json::json;

fn class(id: &str) -> Class {
    Class { name: UnicodeString::new_nul(""), class_id: Id::new(id) }
}

fn enm(ty: &str, value: &str) -> D {
    D::Enumerated { type_id: Id::new(ty), value: Id::new(value) }
}

fn blue() -> Descriptor {
    Descriptor::new("RGBC").with("Rd  ", D::Double(0.0)).with("Grn ", D::Double(0.0)).with("Bl  ", D::Double(255.0))
}

/// Load a reference-shaped descriptor through the public ATN parser.
fn recorded(event: AtnEvent, d: Descriptor) -> Action {
    let bytes = atn::write(&AtnFile {
        name: UnicodeString::new_nul("Identifiers"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Recorded"),
            items: vec![AtnItem { event, descriptor: Some(d), ..Default::default() }],
            ..Default::default()
        }],
        warnings: vec![],
    })
    .unwrap();
    atn_cmds::import(&bytes).unwrap().actions.remove(0)
}

/// Parse the actual saved bytes, so matching import/export mistakes cannot cancel out.
fn saved(action: &Action) -> AtnItem {
    let (bytes, skipped) = atn_cmds::export("Identifiers", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    let file = atn::parse(&bytes).unwrap();
    assert_eq!(file.actions[0].items.len(), 1);
    file.actions[0].items[0].clone()
}

/// Check the enum type and value in saved bytes independently of the command parameters.
fn enum_value<'a>(d: &'a Descriptor, key: &str, ty: &str) -> &'a Id {
    let Some(D::Enumerated { type_id, value }) = d.get(key) else { panic!("missing {key} enumeration") };
    assert_eq!(*type_id, Id::new(ty));
    value
}

/// Canonical event names and old code imports share the existing gradient parameters.
#[test]
fn gradient_map_writes_the_named_event_and_keeps_legacy_code_imports() {
    let stop = |location, color| {
        D::Descriptor(Descriptor::new("Clrt").with("Lctn", D::Integer(location)).with("Mdpn", D::Integer(50)).with("Clr ", D::Descriptor(color)))
    };
    let red = Descriptor::new("RGBC").with("Rd  ", D::Double(255.0)).with("Grn ", D::Double(0.0)).with("Bl  ", D::Double(0.0));
    let d = Descriptor::new("GdMp")
        .with("Grad", D::Descriptor(Descriptor::new("Grdn").with("Clrs", D::List(vec![stop(0, blue()), stop(4096, red)]))))
        .with("Rvrs", D::Boolean(true))
        .with("Dthr", D::Boolean(false));
    for event in [AtnEvent::name("gradientMapEvent"), AtnEvent::Code(*b"GdMp")] {
        let action = recorded(event, d.clone());
        assert_eq!(action.steps, vec![("image.adjustments.gradientMap".into(), json!({"stops":[[0,"#0000ff"],[1,"#ff0000"]],"reverse":true,"dither":false}))]);
        let written = saved(&action);
        assert_eq!(written.event, AtnEvent::name("gradientMapEvent"));
        assert!(written.descriptor.unwrap().class_id.is("GdMp"));
    }
    assert_eq!(recorded(AtnEvent::name("GdMp"), d).steps[0].0, atn_cmds::PHOTOSHOP_STEP);
}

/// The measured outside identifier changes while inside, centre and unknown values stay safe.
#[test]
fn stroke_outside_writes_otsd_without_changing_inside_or_center() {
    for (input, location, canonical) in [("Otsd", "outside", "Otsd"), ("OtsF", "outside", "Otsd"), ("Insd", "inside", "Insd"), ("Cntr", "center", "Cntr")] {
        let d = Descriptor::new("null")
            .with("Wdth", D::UnitFloat { unit: *b"#Pxl", value: 2.0 })
            .with("Lctn", enm("StrL", input))
            .with("Clr ", D::Descriptor(blue()));
        let action = recorded(AtnEvent::Code(*b"Strk"), d);
        assert_eq!(action.steps[0].0, "edit.stroke");
        assert_eq!(action.steps[0].1["location"], location);
        assert_eq!(*enum_value(saved(&action).descriptor.as_ref().unwrap(), "Lctn", "StrL"), Id::new(canonical));
    }
    let unknown = Descriptor::new("null").with("Lctn", enm("StrL", "futureLocation"));
    let action = recorded(AtnEvent::Code(*b"Strk"), unknown.clone());
    assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
    assert_eq!(saved(&action).descriptor, Some(unknown));
}

/// A group blend uses Photoshop's actual identifier and reaches the native group command.
#[test]
fn pass_through_uses_the_string_identifier_and_replays_on_a_group() {
    for input in ["passThrough", "PsTh"] {
        let target = D::Reference(vec![ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Group") }]);
        let d = Descriptor::new("null").with("null", target).with("T   ", D::Descriptor(Descriptor::new("Lyr ").with("Md  ", enm("BlnM", input))));
        let action = recorded(AtnEvent::Code(*b"setd"), d);
        assert_eq!(action.steps, vec![("layer.setProps".into(), json!({"layer":"Group","blend":"Pass Through"}))]);
        let written = saved(&action).descriptor.unwrap();
        let Some(D::Descriptor(props)) = written.get("T   ") else { panic!("layer properties lost") };
        assert_eq!(*enum_value(props, "Md  ", "BlnM"), Id::new("passThrough"));
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4})).unwrap();
        s.execute("layer.new.group", json!({"name":"Group"})).unwrap();
        s.execute("layer.setProps", json!({"blend":"Normal"})).unwrap();
        let (id, p) = &action.steps[0];
        s.execute(id, p.clone()).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().blend, photocraft_color::BlendMode::PassThrough);
    }
}

fn canvas() -> Descriptor {
    Descriptor::new("null")
        .with("Wdth", D::UnitFloat { unit: *b"#Pxl", value: 6.0 })
        .with("Hght", D::UnitFloat { unit: *b"#Pxl", value: 4.0 })
        .with("Hrzn", enm("HrzL", "Left"))
        .with("Vrtc", enm("VrtL", "Top "))
}

/// Foreground and custom extension colours select visibly different new border pixels.
#[test]
fn canvas_extension_identifiers_preserve_foreground_and_custom_color() {
    for (input, custom, canonical, expected) in [
        ("FrgC", false, "FrgC", [1.0, 0.0, 0.0, 1.0]),
        ("Frgd", false, "FrgC", [1.0, 0.0, 0.0, 1.0]),
        ("Clr ", true, "Clr ", [0.0, 0.0, 1.0, 1.0]),
        ("Othr", true, "Clr ", [0.0, 0.0, 1.0, 1.0]),
    ] {
        let mut d = canvas().with("canvasExtensionColorType", enm("canvasExtensionColorType", input));
        if custom {
            d = d.with("canvasExtensionColor", D::Descriptor(blue()));
        }
        let action = recorded(AtnEvent::Code(*b"CnvS"), d);
        assert_eq!(action.steps[0].0, "image.canvasSize");
        assert_eq!(*enum_value(saved(&action).descriptor.as_ref().unwrap(), "canvasExtensionColorType", "canvasExtensionColorType"), Id::new(canonical));
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4,"background":"white"})).unwrap();
        s.execute("tools.setColors", json!({"foreground":"#ff0000","background":"#00ff00"})).unwrap();
        let (id, p) = &action.steps[0];
        s.execute(id, p.clone()).unwrap();
        let pixel: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x":5,"y":1})).unwrap()).unwrap();
        assert_eq!(pixel, expected);
    }
    for d in [
        canvas().with("canvasExtensionColorType", enm("canvasExtensionColorType", "futureColor")).with("canvasExtensionColor", D::Descriptor(blue())),
        canvas().with("canvasExtensionColorType", D::Integer(1)),
        canvas().with("canvasExtensionColorType", enm("canvasExtensionColorType", "Clr ")),
    ] {
        let action = recorded(AtnEvent::Code(*b"CnvS"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor, Some(d));
    }
}

/// Absolute intent writes the measured code and unknown intent values remain preserved steps.
#[test]
fn absolute_colorimetric_writes_aclr_and_rejects_unknown_intents() {
    let profile = || Descriptor::new("null").with("T   ", D::Text(UnicodeString::new_nul("sRGB IEC61966-2.1")));
    for input in ["AClr", "AbsC"] {
        let action = recorded(AtnEvent::name("convertToProfile"), profile().with("Inte", enm("Inte", input)));
        assert_eq!(action.steps, vec![("edit.convertToProfile".into(), json!({"profile":"srgb","intent":"absolute"}))]);
        assert_eq!(*enum_value(saved(&action).descriptor.as_ref().unwrap(), "Inte", "Inte"), Id::new("AClr"));
    }
    for value in [enm("Inte", "futureIntent"), D::Integer(1)] {
        let d = profile().with("Inte", value);
        let action = recorded(AtnEvent::name("convertToProfile"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor, Some(d));
    }
}

/// Wrap uses opening order after tab moves, with overflow-safe offsets and valid ATN references.
#[test]
fn document_offsets_wrap_in_opening_order_and_zero_atn_offsets_stay_kept() {
    let mut s = Session::new();
    for name in ["a", "b", "c"] {
        s.execute("file.new", json!({"width":2,"height":2,"name":name})).unwrap();
    }
    s.execute("document.move", json!({"document":2,"to":0})).unwrap();
    for (offset, expected) in [(-2, "c"), (2, "a"), (5, "a"), (-5, "c"), (i64::MIN, "c"), (i64::MAX, "c"), (0, "b")] {
        s.execute("document.activate", json!({"document":2})).unwrap();
        s.execute("document.activate", json!({"offset":offset})).unwrap();
        assert_eq!(s.active().unwrap().doc.name, expected, "{offset}");
    }
    for offset in [-2, 2] {
        let d = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Offset { class: class("Dcmn"), offset }]));
        let action = recorded(AtnEvent::Code(*b"slct"), d);
        assert_eq!(action.steps[0].0, "document.activate");
        s.execute("document.activate", json!({"document":2})).unwrap();
        let (id, p) = &action.steps[0];
        s.execute(id, p.clone()).unwrap();
        assert_eq!(s.active().unwrap().doc.name, if offset < 0 { "c" } else { "a" });
        let written = saved(&action).descriptor.unwrap();
        assert!(matches!(written.get("null"),Some(D::Reference(r)) if matches!(r.as_slice(),[ReferenceItem::Offset{offset:o,..}] if *o==offset)));
    }
    let zero = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Offset { class: class("Dcmn"), offset: 0 }]));
    let action = recorded(AtnEvent::Code(*b"slct"), zero.clone());
    assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
    assert_eq!(saved(&action).descriptor, Some(zero));
    assert!(atn_cmds::export("Identifiers", &[&Action::new("Zero", vec![("document.activate".into(), json!({"offset":0}))])], &[], false).is_err());
}
