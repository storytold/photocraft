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
        ..Default::default()
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
        assert_eq!(
            action.steps,
            vec![(
                "actions.menuItem".into(),
                json!({"id": "image.imageSize", "eventKind": kind,
            "_atnEvent":{"event":event.id(),"eventKind":kind,"name":"","dictionary":""}})
            )]
        );
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

/// Imported panel names retain their exact bytes; save metadata never becomes command input.
/// The gallery case exercises the existing user of the shared event/name representation.
#[test]
fn imported_menu_names_keep_raw_bytes_without_leaking_into_playback() {
    let gallery = Descriptor::new("null")
        .with("GEfk", D::Enumerated { type_id: Id::new("GEft"), value: Id::new("Ct") })
        .with("NmbL", D::Integer(6))
        .with("EdgS", D::Integer(4))
        .with("EdgF", D::Integer(2));
    let cases = [
        (AtnEvent::Code(*b"ImgS"), None, atn_cmds::MENU_ITEM, "image.imageSize"),
        (AtnEvent::name("imageSize"), None, atn_cmds::MENU_ITEM, "image.imageSize"),
        (AtnEvent::Code(*b"GsnB"), None, atn_cmds::MENU_ITEM, "filter.blur.gaussianBlur"),
        (AtnEvent::name("findEdges"), None, atn_cmds::MENU_ITEM, "filter.stylize.findEdges"),
        (AtnEvent::name("cutout"), Some(gallery), "filter.gallery.cutout", "filter.gallery.cutout"),
    ];
    for dictionary in ["画像サイズ".as_bytes().to_vec(), vec![0x83, 0x8a, 0xff, 0], Vec::new()] {
        for (event, descriptor, id, command) in &cases {
            let bytes = file(AtnItem { event: event.clone(), dictionary: dictionary.clone(), descriptor: descriptor.clone(), ..Default::default() });
            let imported = atn_cmds::import(&bytes).unwrap();
            assert!(imported.unsupported.is_empty());
            let (step_id, params) = &imported.actions[0].steps[0];
            assert_eq!(step_id, id);
            assert_eq!(params["_atnEvent"]["dictionary"], json!(photocraft_engine::paint::tile::b64_encode(&dictionary)));
            let mut session = Session::new();
            session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
            let execution = photocraft_engine::actions_cmds::playback_params(&session, step_id, params).unwrap();
            assert!(execution.get("_atnEvent").is_none());
            if *id == atn_cmds::MENU_ITEM {
                let kind = if matches!(event, AtnEvent::Code(_)) { "code" } else { "name" };
                assert_eq!(execution, json!({"id":command,"eventKind":kind}));
                assert_eq!(atn_cmds::menu_item_command(params).unwrap(), *command);
            }
            session.actions.list = imported.actions;
            let result = session.execute("actions.play", json!({"action": "Resize"})).unwrap();
            if *id == atn_cmds::MENU_ITEM && atn_cmds::menu_item_needs_settings(command) {
                assert_eq!(result["ran"], 0);
                assert!(result["failed"]["error"].as_str().unwrap().contains("interactive-required"));
            } else {
                assert_eq!(result["ran"], 1, "{result}");
                assert!(result.get("failed").is_none());
            }
            assert!(session.journal.iter().all(|(_, p)| p.get("_atnEvent").is_none()));
            let (saved, skipped) = atn_cmds::export("Menu", &[&session.actions.list[0]], &[], false).unwrap();
            assert!(skipped.is_empty());
            let reparsed = atn::parse(&saved).unwrap();
            let item = &reparsed.actions[0].items[0];
            assert_eq!(&item.event, event);
            assert_eq!(item.dictionary, dictionary);
            assert_eq!(item.descriptor.is_none(), descriptor.is_none());
        }
    }
}

/// Native choices keep canonical labels, and stale saved event metadata cannot retarget them.
#[test]
fn native_menu_names_stay_canonical_and_ignore_mismatched_saved_events() {
    let native = json!({"id":"image.imageSize"});
    for metadata in [None, Some(json!({"event":"GsnB","eventKind":"code"})), Some(json!({"event":"imageSize","eventKind":"name"}))] {
        let mut params = native.clone();
        if let Some(mut metadata) = metadata {
            params["eventKind"] = json!("code");
            metadata["dictionary"] = json!(photocraft_engine::paint::tile::b64_encode(b"wrong label"));
            params["_atnEvent"] = metadata;
        }
        let expected = if params.get("eventKind").is_some() { AtnEvent::Code(*b"ImgS") } else { AtnEvent::name("imageSize") };
        let action = Action::new("Native", vec![(atn_cmds::MENU_ITEM.into(), params)]);
        let (saved, skipped) = atn_cmds::export("Menu", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let parsed = atn::parse(&saved).unwrap();
        let item = &parsed.actions[0].items[0];
        assert_eq!(item.event, expected);
        assert_eq!(item.dictionary, b"Image Size");
        assert!(item.descriptor.is_none());
    }
}
