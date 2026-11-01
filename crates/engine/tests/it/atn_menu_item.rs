//! Descriptor-free Image Size means an interactive menu step, not empty settings.
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D},
};
use serde_json::json;

/// Build a single-step ATN input independently of the engine's exporter.
fn file(item: AtnItem) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Menu"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("Resize"), items: vec![item], ..Default::default() }],
        expanded: true,
        warnings: vec![],
    })
    .unwrap()
}

/// Absent settings retain menu intent and event kind, and refuse headless edits.
#[test]
fn absent_image_size_settings_round_trip_and_require_interaction_without_edits() {
    for (event, kind) in [(AtnEvent::Code(*b"ImgS"), "code"), (AtnEvent::name("imageSize"), "name")] {
        let bytes = file(AtnItem { event: event.clone(), descriptor: None, ..Default::default() });
        let imported = atn_cmds::import(&bytes).unwrap();
        assert!(imported.unsupported.is_empty(), "{:?}", imported.unsupported);
        let action = &imported.actions[0];
        assert_eq!(action.steps, vec![("actions.menuItem".into(), json!({"id": "image.imageSize", "eventKind": kind}))]);
        let (saved, skipped) = atn_cmds::export("Menu", &[action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let reparsed = atn::parse(&saved).unwrap();
        let step = &reparsed.actions[0].items[0];
        assert_eq!(step.event, event);
        assert!(step.descriptor.is_none());

        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
        session.actions.list = imported.actions;
        let revision = session.active().unwrap().revision;
        let journal = session.journal.len();
        let result = session.execute("actions.play", json!({"action": "Resize"})).unwrap();
        assert_eq!(result["ran"], 0);
        assert!(result["failed"]["error"].as_str().is_some_and(|s| s.contains("interactive-required")), "{result}");
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(session.journal.len(), journal);
    }
}

/// Empty and target descriptors, other events and recorded settings keep their meaning.
#[test]
fn empty_target_and_other_event_forms_keep_their_existing_meaning() {
    let empty = Descriptor::new("null");
    let target = Descriptor {
        items: vec![(
            Id::Code(*b"null"),
            D::Reference(vec![ReferenceItem::Enumerated {
                class: Class { name: UnicodeString::default(), class_id: Id::Code(*b"Lyr ") },
                type_id: Id::Code(*b"Ordn"),
                value: Id::Code(*b"Trgt"),
            }]),
        )],
        ..empty.clone()
    };
    for (event, descriptor) in [
        (AtnEvent::name("imageSize"), Some(empty)),
        (AtnEvent::name("imageSize"), Some(target)),
        (AtnEvent::name("ImgS"), None),
        (AtnEvent::name("invert"), None),
    ] {
        let imported = atn_cmds::import(&file(AtnItem { event, descriptor: descriptor.clone(), ..Default::default() })).unwrap();
        assert_ne!(imported.actions[0].steps[0].0, "actions.menuItem");
        if imported.actions[0].steps[0].0 == atn_cmds::PHOTOSHOP_STEP {
            let (saved, _) = atn_cmds::export("Menu", &[&imported.actions[0]], &[], false).unwrap();
            assert_eq!(atn::parse(&saved).unwrap().actions[0].items[0].descriptor, descriptor);
        }
    }
    // A wrapper cannot turn arbitrary commands or recorded settings into a menu dispatch.
    for params in [json!({"id": "file.open"}), json!({"id": "image.imageSize", "width": 2}), json!({"id": "image.imageSize", "eventKind": 0})] {
        let action = Action::new("Invalid", vec![("actions.menuItem".into(), params.clone())]);
        assert!(atn_cmds::export("Menu", &[&action], &[], false).is_err());
        let mut session = Session::new();
        assert!(session.execute("actions.menuItem", params).is_err());
    }
    // A normal recorded Image Size keeps its supplied settings and executes headlessly.
    let action = Action::new("Recorded", vec![("image.imageSize".into(), json!({"width": 4, "height": 3, "resample": "nearest"}))]);
    let (saved, _) = atn_cmds::export("Menu", &[&action], &[], false).unwrap();
    assert!(atn::parse(&saved).unwrap().actions[0].items[0].descriptor.is_some());
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    session.actions.list = atn_cmds::import(&saved).unwrap().actions;
    let result = session.execute("actions.play", json!({"action": "Recorded"})).unwrap();
    assert_eq!(result["ran"], 1, "{result}");
    assert_eq!(session.active().unwrap().doc.size, photocraft_doc::Size::new(4, 3));
}
