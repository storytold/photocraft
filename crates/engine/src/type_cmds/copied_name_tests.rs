// Included in type_cmds::tests: exercise the command seam, not a suffix parser.
fn via_copy_text(s: &mut Session, id: u64) -> u64 {
    s.execute("layer.select", json!({"layer": id})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none());
    assert_eq!(s.active().unwrap().selected_layers().len(), 1);
    s.execute("layer.new.layerViaCopy", json!({})).unwrap()["layer"].as_u64().unwrap()
}

#[test]
fn layer_via_copy_auto_name_follows_edits_and_history() {
    let mut s = session();
    let original = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let steps = s.active().unwrap().history.entries().len();
    let copy = via_copy_text(&mut s, original);
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    assert_eq!(layer_name_of(&s, copy), "Heading copy");
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(LayerId(copy)).is_none());
    assert!(s.redo());
    let steps = s.active().unwrap().history.entries().len();
    s.execute("type.edit", json!({"layer": copy, "text": "Changed"})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Changed");
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    assert!(s.undo());
    assert_eq!(layer_name_of(&s, copy), "Heading copy");
    assert_eq!(text_layer(&s, copy).text, "Heading");
    assert!(s.redo());
    assert_eq!(layer_name_of(&s, copy), "Changed");
    assert_eq!(text_layer(&s, copy).text, "Changed");
    assert!(s.undo());
    s.execute("type.edit", json!({"layer": copy, "text": "Branched"})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Branched");
    assert_eq!(layer_name_of(&s, original), "Heading");
}

#[test]
fn layer_via_copy_repeated_auto_names_refresh_provenance() {
    for first_command in ["layer.new.layerViaCopy", "layer.duplicate"] {
        let mut s = session();
        let original = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
        let first = s.execute(first_command, json!({})).unwrap()["layer"].as_u64().unwrap();
        let second = via_copy_text(&mut s, first);
        let third = via_copy_text(&mut s, second);
        assert_eq!(layer_name_of(&s, second), "Heading copy 2");
        assert_eq!(layer_name_of(&s, third), "Heading copy 3");
        s.execute("type.edit", json!({"layer": third, "size": 20})).unwrap();
        s.execute("type.pasteLoremIpsum", json!({"layer": third, "at": 0})).unwrap();
        assert_eq!(layer_name_of(&s, third), layer_name(&text_layer(&s, third).text), "{first_command}");
        assert!(s.undo());
        assert_eq!(layer_name_of(&s, third), "Heading copy 3");
        s.execute("type.edit", json!({"layer": third, "text": "Changed"})).unwrap();
        assert_eq!(layer_name_of(&s, third), "Changed", "{first_command}");
        assert_eq!(layer_name_of(&s, original), "Heading");
        assert_eq!(layer_name_of(&s, first), "Heading copy");
        assert_eq!(layer_name_of(&s, second), "Heading copy 2");
    }
}

#[test]
fn layer_via_copy_custom_labels_stay_custom() {
    for custom in ["Logo", "Heading copy", "Heading copy 2"] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": "Heading", "name": custom})).unwrap()["layer"].as_u64().unwrap();
        let first = via_copy_text(&mut s, id);
        let second = via_copy_text(&mut s, first);
        let name = layer_name_of(&s, second);
        s.execute("type.edit", json!({"layer": second, "text": "Changed"})).unwrap();
        assert_eq!(layer_name_of(&s, second), name);
    }
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let copy = via_copy_text(&mut s, id);
    s.execute("layer.renameLayer", json!({"layer": copy, "name": "Heading copy"})).unwrap();
    let next = via_copy_text(&mut s, copy);
    let name = layer_name_of(&s, next);
    s.execute("type.edit", json!({"layer": next, "text": "Changed"})).unwrap();
    assert_eq!(layer_name_of(&s, next), name);
}

#[test]
fn layer_via_copy_auto_name_survives_native_reopen() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let first = copied_text(&mut s, id);
    let copy = via_copy_text(&mut s, first);
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &photocraft_format::SaveOptions::default()).unwrap();
    s.active_mut().unwrap().doc = Arc::new(photocraft_format::load_from_bytes(&bytes).unwrap());
    s.execute("type.edit", json!({"layer": copy, "text": "Reopened"})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Reopened");
}

#[test]
fn layer_via_copy_still_unlocks_the_copy_not_the_source() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": id, "locks": {"all": true}})).unwrap();
    let copy = via_copy_text(&mut s, id);
    assert!(s.active().unwrap().doc.layer(LayerId(id)).unwrap().locks.all);
    assert_eq!(s.active().unwrap().doc.layer(LayerId(copy)).unwrap().locks, Default::default());
    s.execute("type.edit", json!({"layer": copy, "text": "Unlocked"})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Unlocked");
}
// Explicit intent must win over equality with the text-derived name, not just copy labels.
#[test]
fn explicit_equal_name_at_creation_survives_edits_and_copy() {
    for copy_command in ["layer.duplicate", "layer.new.layerViaCopy"] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": "Heading", "name": "Heading"})).unwrap()["layer"].as_u64().unwrap();
        let copy = s.execute(copy_command, json!({"layer": id})).unwrap()["layer"].as_u64().unwrap();
        let name = layer_name_of(&s, copy);
        s.execute("type.edit", json!({"layer": copy, "text": "Copy changed"})).unwrap();
        assert_eq!(layer_name_of(&s, copy), name, "{copy_command}");
        s.execute("type.edit", json!({"layer": id, "text": "Changed"})).unwrap();
        assert_eq!(layer_name_of(&s, id), "Heading");
    }
}

#[test]
fn explicit_equal_name_rename_routes_survive_native_reopen() {
    for command in ["layer.renameLayer", "layer.setProps", "type.edit"] {
        for edit_command in ["type.edit", "type.pasteLoremIpsum"] {
            let mut s = session();
            let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
            let copy = copied_text(&mut s, id);
            s.execute("type.edit", json!({"layer": copy, "text": "Edited copy"})).unwrap();
            s.execute(command, json!({"layer": copy, "name": "Edited copy"})).unwrap();
            let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &photocraft_format::SaveOptions::default()).unwrap();
            s.active_mut().unwrap().doc = Arc::new(photocraft_format::load_from_bytes(&bytes).unwrap());
            s.execute(edit_command, json!({"layer": copy, "text": "Changed", "at": 0})).unwrap();
            assert_eq!(layer_name_of(&s, copy), "Edited copy", "{command}, {edit_command}");
            let next = via_copy_text(&mut s, copy);
            let name = layer_name_of(&s, next);
            s.execute("type.edit", json!({"layer": next, "text": "Changed again"})).unwrap();
            assert_eq!(layer_name_of(&s, next), name);
        }
    }
}

#[test]
fn explicit_equal_name_duplicate_override_survives_reopen() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let copy = s.execute("layer.duplicate", json!({"layer": id, "name": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &photocraft_format::SaveOptions::default()).unwrap();
    s.active_mut().unwrap().doc = Arc::new(photocraft_format::load_from_bytes(&bytes).unwrap());
    s.execute("type.edit", json!({"layer": copy, "text": "Changed"})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Heading");
}

#[test]
fn explicit_equal_name_history_restores_automatic_intent() {
    for command in ["layer.renameLayer", "layer.setProps", "type.edit"] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
        let steps = s.active().unwrap().history.entries().len();
        s.execute(command, json!({"layer": id, "name": "Heading"})).unwrap();
        assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
        assert!(s.undo());
        assert!(s.redo());
        s.execute("type.edit", json!({"layer": id, "text": "Changed"})).unwrap();
        assert_eq!(layer_name_of(&s, id), "Heading", "{command}");
        assert!(s.undo());
        assert!(s.undo());
        s.execute("type.edit", json!({"layer": id, "text": "Branched"})).unwrap();
        assert_eq!(layer_name_of(&s, id), "Branched");
    }
}

#[test]
fn explicit_equal_name_legacy_unmarked_layers_keep_inference() {
    // Default TextLayer models imported/old documents with no naming metadata.
    for (name, expected) in [("Heading", "Changed"), ("Logo", "Logo"), ("Heading copy", "Heading copy")] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        let l = doc.layer_mut(LayerId(id)).unwrap();
        l.name = name.into();
        l.content = LayerContent::Text(TextLayer { text: "Heading".into(), ..Default::default() });
        let bytes = photocraft_format::save_to_bytes(doc, &photocraft_format::SaveOptions::default()).unwrap();
        s.active_mut().unwrap().doc = Arc::new(photocraft_format::load_from_bytes(&bytes).unwrap());
        s.execute("type.edit", json!({"layer": id, "text": "Changed"})).unwrap();
        assert_eq!(layer_name_of(&s, id), expected);
    }
}

fn copied_text(s: &mut Session, id: u64) -> u64 {
    s.execute("layer.duplicate", json!({"layer": id})).unwrap()["layer"].as_u64().unwrap()
}

#[test]
fn copied_auto_name_follows_edits_and_history() {
    let mut s = session();
    let original = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let first = copied_text(&mut s, original);
    let second = copied_text(&mut s, first);
    assert_eq!(layer_name_of(&s, first), "Heading copy");
    assert_eq!(layer_name_of(&s, second), "Heading copy 2");
    let steps = s.active().unwrap().history.entries().len();
    s.execute("type.edit", json!({"layer": second, "text": "New heading"})).unwrap();
    assert_eq!(layer_name_of(&s, second), "New heading");
    assert_eq!(layer_name_of(&s, original), "Heading");
    assert_eq!(layer_name_of(&s, first), "Heading copy");
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    assert!(s.undo());
    assert_eq!(layer_name_of(&s, second), "Heading copy 2");
    assert_eq!(text_layer(&s, second).text, "Heading");
    assert!(s.redo());
    assert_eq!(layer_name_of(&s, second), "New heading");
    assert_eq!(text_layer(&s, second).text, "New heading");
    assert!(s.undo());
    s.execute("type.edit", json!({"layer": second, "replace": {"start": 0, "end": 7, "text": "Again"}})).unwrap();
    assert_eq!(layer_name_of(&s, second), "Again");
}

#[test]
fn copied_auto_name_survives_non_text_edits_and_lorem_uses_same_policy() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let copy = copied_text(&mut s, id);
    s.execute("type.edit", json!({"layer": copy, "size": 20})).unwrap();
    s.execute("type.convertToParagraphText", json!({"layer": copy})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Heading copy", "formatting must not consume the copy name");
    s.execute("type.pasteLoremIpsum", json!({"layer": copy, "at": 0})).unwrap();
    assert_eq!(layer_name_of(&s, copy), layer_name(&text_layer(&s, copy).text));
    assert!(s.undo());
    assert_eq!(layer_name_of(&s, copy), "Heading copy");
    assert!(s.redo());
    assert_eq!(layer_name_of(&s, copy), layer_name(&text_layer(&s, copy).text));
}

#[test]
fn copied_custom_names_are_not_inferred_from_copy_suffixes() {
    for custom in ["Logo", "Heading copy", "Heading copy 2"] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": "Heading", "name": custom})).unwrap()["layer"].as_u64().unwrap();
        let first = copied_text(&mut s, id);
        let second = copied_text(&mut s, first);
        let name = layer_name_of(&s, second);
        s.execute("type.edit", json!({"layer": second, "text": "Changed"})).unwrap();
        s.execute("type.pasteLoremIpsum", json!({"layer": second, "at": 0})).unwrap();
        assert_eq!(layer_name_of(&s, second), name);
    }
}

#[test]
fn copied_auto_name_can_be_explicitly_customized() {
    for command in ["layer.renameLayer", "layer.setProps", "type.edit"] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
        let copy = copied_text(&mut s, id);
        // Even choosing exactly the generated name is an explicit custom label.
        s.execute(command, json!({"layer": copy, "name": "Heading copy"})).unwrap();
        s.execute("type.edit", json!({"layer": copy, "text": "Changed"})).unwrap();
        assert_eq!(layer_name_of(&s, copy), "Heading copy", "{command}");
        let again = copied_text(&mut s, copy);
        let name = layer_name_of(&s, again);
        s.execute("type.edit", json!({"layer": again, "text": "Changed again"})).unwrap();
        assert_eq!(layer_name_of(&s, again), name);
    }
}

#[test]
fn copied_auto_name_survives_native_save_and_reopen() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let copy = copied_text(&mut s, id);
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &photocraft_format::SaveOptions::default()).unwrap();
    let doc = photocraft_format::load_from_bytes(&bytes).unwrap();
    s.active_mut().unwrap().doc = Arc::new(doc);
    s.execute("type.edit", json!({"layer": copy, "text": "Reopened"})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Reopened");
}

#[test]
fn copied_auto_name_with_explicit_duplicate_label_stays_custom() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let copy = s.execute("layer.duplicate", json!({"layer": id, "name": "Heading copy"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.edit", json!({"layer": copy, "text": "Changed"})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Heading copy");
}

#[test]
fn copied_auto_names_use_the_normal_unicode_blank_and_length_rules() {
    for initial in ["", "\n  \nGröße ✓\nsecond", "A very long heading that exceeds forty characters easily", "Heading copy 2"] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": initial})).unwrap()["layer"].as_u64().unwrap();
        let copy = copied_text(&mut s, id);
        s.execute("type.edit", json!({"layer": copy, "text": "\n 新しい見出し\nsecond"})).unwrap();
        assert_eq!(layer_name_of(&s, copy), "新しい見出し");
        s.execute("type.edit", json!({"layer": copy, "text": ""})).unwrap();
        assert_eq!(layer_name_of(&s, copy), "Type Layer");
    }
}
