//! These fixtures spell the measured wire layout directly; no production writer builds the oracle.
use super::*;
use crate::Session;
use serde_json::json;

fn be(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn text(value: &str) -> Vec<u8> {
    let units: Vec<_> = value.encode_utf16().collect();
    let mut out = Vec::new();
    be(&mut out, units.len() as u32);
    for unit in units {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}
fn id(out: &mut Vec<u8>, value: &str) {
    be(out, if value.len() == 4 { 0 } else { value.len() as u32 });
    out.extend_from_slice(value.as_bytes());
}
fn object(fields: &[(&str, &[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut out = text("");
    id(&mut out, "null");
    be(&mut out, fields.len() as u32);
    for (key, kind, bytes) in fields {
        id(&mut out, key);
        out.extend_from_slice(*kind);
        out.extend(bytes);
    }
    out
}
fn list(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    be(&mut out, objects.len() as u32);
    for object in objects {
        out.extend_from_slice(b"Objc");
        out.extend(object);
    }
    out
}
fn raw(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    be(&mut out, bytes.len() as u32);
    out.extend_from_slice(bytes);
    out
}
fn action_entry(index: usize) -> Vec<u8> {
    let mut fields = vec![
        ("actionDescriptionKey", b"TEXT", text(if index == 0 { "First description\0" } else { "Second description\0" })),
        ("actionThumbnailTypeKey", b"long", (if index == 0 { 3i32 } else { 42i32 }).to_be_bytes().to_vec()),
        ("actionPreviewKey", b"bool", vec![2]), // Legal noncanonical bool must remain byte-exact.
        ("actionGenerativeKey", b"bool", vec![0]),
        ("actionPreconditionVal", b"long", 73i32.to_be_bytes().to_vec()),
        ("futureDouble", b"doub", 0x7ff8_0000_0000_0042u64.to_be_bytes().to_vec()),
        ("futureActionBytes", b"tdta", raw(&[0, 255, index as u8])),
    ];
    if index == 0 {
        fields.push(("actionIconKey", b"TEXT", text("future-icon\0")));
    }
    object(&fields)
}
fn set_body(out: &mut Vec<u8>, name: &str, key: Option<u16>) {
    out.extend(text(&format!("{name}\0")));
    out.push(0);
    be(out, u32::from(key.is_some()));
    if let Some(key) = key {
        action_body(out, key);
    }
}
fn action_body(out: &mut Vec<u8>, key: u16) {
    out.extend_from_slice(&key.to_be_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend(text("Same\0"));
    out.push(0);
    be(out, 0);
}
fn set_entry(index: usize, entries: &[Vec<u8>]) -> Vec<u8> {
    object(&[
        ("setColorIndex", b"long", (11 + index as i32 * 11).to_be_bytes().to_vec()),
        ("actions", b"VlLs", list(entries)),
        ("futureSet", b"tdta", raw(&[9, index as u8])),
    ])
}
fn sidecar_data(sets: &[Vec<u8>], note: &str) -> (Vec<u8>, Vec<u8>) {
    let root = object(&[("sets", b"VlLs", list(sets)), ("futureGlobal", b"bool", vec![2])]);
    let mut payload = 16u32.to_be_bytes().to_vec();
    payload.extend(root);
    let outer = object(&[("Msge", b"TEXT", text(note)), ("Cntn", b"bool", vec![1]), (MODERN, b"tdta", raw(&payload))]);
    (payload, outer)
}
struct Fixture {
    bytes: Vec<u8>,
    payload: Vec<u8>,
    outer: Vec<u8>,
    entries: Vec<Vec<u8>>,
}
fn fixture(names: &[&str], empty: bool, note: &str) -> Fixture {
    fixture_at(names, empty.then_some(names.len()), note)
}
fn fixture_at(names: &[&str], empty_at: Option<usize>, note: &str) -> Fixture {
    fixture_rows(&names.iter().map(|name| (*name, 1)).collect::<Vec<_>>(), empty_at, note)
}
fn fixture_rows(rows: &[(&str, usize)], empty_at: Option<usize>, note: &str) -> Fixture {
    let entries: Vec<_> = (0..rows.iter().map(|(_, count)| *count).sum::<usize>()).map(action_entry).collect();
    let mut at = 0;
    let mut sets: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(index, (_, count))| {
            let set = set_entry(index, &entries[at..at + *count]);
            at += *count;
            set
        })
        .collect();
    if let Some(slot) = empty_at {
        sets.insert(slot, object(&[("setColorIndex", b"long", 99i32.to_be_bytes().to_vec()), ("actions", b"VlLs", list(&[]))]));
    }
    let (payload, outer) = sidecar_data(&sets, note);
    let mut bytes = 16u32.to_be_bytes().to_vec();
    at = 0;
    for index in 0..=rows.len() {
        if empty_at == Some(index) {
            set_body(&mut bytes, "Empty", None);
        }
        if let Some((name, count)) = rows.get(index) {
            bytes.extend(text(&format!("{name}\0")));
            bytes.push(0);
            be(&mut bytes, *count as u32);
            for action in at..at + *count {
                action_body(&mut bytes, 6 + action as u16);
            }
            at += *count;
        }
    }
    bytes.extend(text("Action sidecar\0"));
    bytes.push(0);
    be(&mut bytes, 1);
    bytes.extend_from_slice(&[0; 6]);
    bytes.extend(text("Play for details\0"));
    bytes.push(0);
    be(&mut bytes, 1);
    bytes.extend_from_slice(&[0, 1, 1, 2]);
    bytes.extend_from_slice(b"TEXT");
    be(&mut bytes, 4);
    bytes.extend_from_slice(b"stop");
    be(&mut bytes, 4);
    bytes.extend_from_slice(b"Stop");
    bytes.extend_from_slice(&(-1i32).to_be_bytes());
    bytes.extend(&outer);
    Fixture { bytes, payload, outer, entries }
}
fn loaded(bytes: &[u8]) -> Session {
    let mut session = Session::new();
    session.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(bytes)})).unwrap();
    session
}
fn saved_bytes(session: &mut Session, params: serde_json::Value) -> Vec<u8> {
    let response = session.execute("actions.save", params).unwrap();
    photocraft_paint::tile::b64_decode(response["data"].as_str().unwrap()).unwrap()
}
fn saved(session: &mut Session, params: serde_json::Value) -> Vec<AtnFile> {
    atn::parse_sets(&saved_bytes(session, params)).unwrap()
}
fn panel_groups(state: &crate::actions_cmds::ActionState) -> Vec<(String, Option<usize>, Vec<String>)> {
    // Match the Actions panel's Folder identity/order: slotless names first, then each explicit
    // archive occurrence in physical slot order. Explicit duplicate captions remain distinct.
    let mut implicit = Vec::new();
    for action in &state.list {
        if action.set_slot.is_none() && action.set.is_some() && super::super::logical_set_name(action.set.as_deref()) != super::super::UNGROUPED_SET {
            let name = super::super::logical_set_name(action.set.as_deref()).to_string();
            if !implicit.contains(&name) {
                implicit.push(name);
            }
        }
    }
    for name in &state.sets {
        if !implicit.contains(name) {
            implicit.push(name.clone());
        }
    }
    let mut groups: Vec<_> = implicit
        .into_iter()
        .map(|name| {
            let actions = state
                .list
                .iter()
                .filter(|action| action.set_slot.is_none() && super::super::logical_set_name(action.set.as_deref()) == name)
                .map(|action| action.name.clone())
                .collect();
            (name, None, actions)
        })
        .collect();
    for (slot, name) in state.set_rows.iter().enumerate() {
        let actions = state
            .list
            .iter()
            .filter(|action| action.set_slot == Some(slot) && super::super::logical_set_name(action.set.as_deref()) == name)
            .map(|action| action.name.clone())
            .collect();
        groups.push((name.clone(), Some(slot), actions));
    }
    groups
}

fn action_names(actions: &[Action]) -> Vec<String> {
    actions.iter().map(|action| action.name.clone()).collect()
}
fn metadata(files: &[AtnFile]) -> RawDescriptor {
    RawDescriptor::new(payload(files.last().unwrap()).unwrap()).unwrap()
}
fn set_metadata(files: &[AtnFile], name: &str) -> RawDescriptor {
    let index = files.iter().position(|file| file.name.to_string_lossy() == name).unwrap();
    metadata(files).list("sets").unwrap()[index].clone()
}
fn integer(raw: &RawDescriptor, key: &str) -> i32 {
    match raw.parsed().unwrap().get(key).unwrap() {
        D::Integer(value) => *value,
        _ => panic!("integer expected"),
    }
}
fn assert_fields_equal(actual: &[u8], expected: &[u8], names: &[&str]) {
    let actual_descriptor = RawDescriptor::new(actual).unwrap();
    let expected_descriptor = RawDescriptor::new(expected).unwrap();
    let (_, actual_fields) = actual_descriptor.fields().unwrap();
    let (_, expected_fields) = expected_descriptor.fields().unwrap();
    for name in names {
        let actual_field = actual_fields.iter().find(|field| field.key.is(name)).unwrap();
        let expected_field = expected_fields.iter().find(|field| field.key.is(name)).unwrap();
        assert_eq!(actual_descriptor.source.get(actual_field.span.clone()), expected_descriptor.source.get(expected_field.span.clone()), "opaque field {name}");
    }
}

#[test]
fn public_load_save_all_keeps_positional_metadata_and_raw_descriptor_bytes() {
    let fixture = fixture(&["A", "B"], true, "Original note\0");
    let mut session = loaded(&fixture.bytes);
    assert_eq!(session.actions.list.len(), 2);
    assert_eq!(session.actions.list[0].description, "First description");
    assert_eq!(session.actions.list[0].icon, "future-icon");
    assert_eq!(session.actions.list[1].description, "Second description");
    assert_eq!(session.actions.list[1].icon, "");
    assert_eq!(session.actions.set_rows, ["A", "B", "Empty"]);
    let files = saved(&mut session, json!({}));
    assert_eq!(files.len(), 4);
    assert_eq!(files.last().unwrap().actions[0].items[0].descriptor.as_ref().unwrap().to_bytes(), fixture.outer);
    assert_eq!(payload(files.last().unwrap()).unwrap(), fixture.payload.get(4..).unwrap());
    assert_eq!(integer(&set_metadata(&files, "Empty"), "setColorIndex"), 99);
}

#[test]
fn unedited_empty_first_and_interleaved_rows_keep_whole_sidecar_bytes_and_positions() {
    for (fixture, expected_names, expected_actions, expected_colors) in [
        (fixture_at(&["A"], Some(0), "Empty first\0"), vec!["Empty", "A"], vec![vec![], vec!["Same"]], vec![99, 11]),
        (fixture(&["A", "B", "A"], false, "Interleaved\0"), vec!["A", "B", "A"], vec![vec!["Same"], vec!["Same 2"], vec!["Same 3"]], vec![11, 22, 33]),
    ] {
        let mut session = loaded(&fixture.bytes);
        let files = saved(&mut session, json!({}));
        assert_eq!(files.len(), expected_names.len() + 1);
        let real = &files[..expected_names.len()];
        assert_eq!(real.iter().map(|file| file.name.to_string_lossy()).collect::<Vec<_>>(), expected_names);
        for (file, expected) in real.iter().zip(expected_actions) {
            assert_eq!(file.actions.iter().map(|action| action.name.to_string_lossy()).collect::<Vec<_>>(), expected);
        }
        let outer = files.last().unwrap().actions[0].items[0].descriptor.as_ref().unwrap();
        assert_eq!(outer.to_bytes(), fixture.outer, "the whole unedited wire descriptor is the independent oracle");
        let Some(D::RawData(bytes)) = outer.get(MODERN) else { panic!("missing sidecar data") };
        assert_eq!(bytes, &fixture.payload);
        let descriptor = Descriptor::from_bytes(&bytes[4..]).unwrap();
        let Some(D::List(sets)) = descriptor.get("sets") else { panic!("missing physical set metadata") };
        assert_eq!(sets.len(), real.len());
        for (value, expected) in sets.iter().zip(expected_colors) {
            let D::Descriptor(set) = value else { panic!("expected set object") };
            assert_eq!(set.get("setColorIndex"), Some(&D::Integer(expected)));
        }
    }
}

#[test]
fn moving_a_later_physical_set_keeps_row_order_and_restores_panel_order_after_reload() {
    let note = "Interleaved move\0";
    let fixture = fixture(&["A", "B", "A"], false, note);
    let mut session = loaded(&fixture.bytes);
    session.execute("actions.move", json!({"action":"Same 3","to":0})).unwrap();
    let expected = vec![("A".into(), Some(0), vec!["Same".into()]), ("B".into(), Some(1), vec!["Same 2".into()]), ("A".into(), Some(2), vec!["Same 3".into()])];
    assert_eq!(panel_groups(&session.actions), expected);
    let bytes = saved_bytes(&mut session, json!({}));
    let files = atn::parse_sets(&bytes).unwrap();
    let original = atn::parse_sets(&fixture.bytes).unwrap();
    assert_eq!(files.len(), 4);
    let sets = metadata(&files).list("sets").unwrap();
    for (index, name, action, source) in [(0, "A", "Same", 0), (1, "B", "Same 2", 1), (2, "A", "Same 3", 2)] {
        assert_eq!(files[index].name.to_string_lossy(), name);
        assert_eq!(files[index].actions.len(), 1);
        assert_eq!(files[index].actions[0].name.to_string_lossy(), action);
        let mut expected_record = original[index].actions.clone();
        expected_record[0].name = UnicodeString::new_nul(action);
        assert_eq!(files[index].actions, expected_record, "only the portable duplicate-name projection may change the action name");
        assert_eq!(sets[index].bytes(), set_entry(source, std::slice::from_ref(&fixture.entries[source])));
    }
    let outer = files[3].actions[0].items[0].descriptor.as_ref().unwrap().to_bytes();
    assert_fields_equal(&outer, &fixture.outer, &["Msge", "Cntn"]);
    assert_fields_equal(metadata(&files).bytes(), fixture.payload.get(4..).unwrap(), &["futureGlobal"]);
    let external = loaded(&bytes);
    assert_eq!(panel_groups(&external.actions), expected, "portable import follows its explicit physical rows");
    let durable = crate::actions_cmds::library::encode(&session.actions, &[]).unwrap();
    let trusted = crate::actions_cmds::library::decode(&durable).unwrap();
    assert_eq!(action_names(&trusted.list), ["Same 3", "Same", "Same 2"], "trusted local restore keeps the moved global order");
    assert_eq!(panel_groups(&trusted), expected);
}

#[test]
fn moving_between_actions_keeps_physical_rows_and_restores_global_order_after_reload() {
    let note = "Source runs\0";
    let fixture = fixture_rows(&[("A", 2), ("A", 1)], None, note);
    let mut session = loaded(&fixture.bytes);
    session.execute("actions.move", json!({"action":"Same 3","to":1})).unwrap();
    let expected = vec![("A".into(), Some(0), vec!["Same".into(), "Same 2".into()]), ("A".into(), Some(1), vec!["Same 3".into()])];
    assert_eq!(panel_groups(&session.actions), expected);
    let bytes = saved_bytes(&mut session, json!({}));
    let files = atn::parse_sets(&bytes).unwrap();
    let original = atn::parse_sets(&fixture.bytes).unwrap();
    assert_eq!(files.len(), 3);
    let sets = metadata(&files).list("sets").unwrap();
    assert_eq!(files[0].name.to_string_lossy(), "A");
    assert_eq!(files[0].actions.iter().map(|action| action.name.to_string_lossy()).collect::<Vec<_>>(), ["Same", "Same 2"]);
    let mut expected_first = original[0].actions.clone();
    for (record, name) in expected_first.iter_mut().zip(["Same", "Same 2"]) {
        record.name = UnicodeString::new_nul(name);
    }
    assert_eq!(files[0].actions, expected_first, "the first physical row stays intact apart from its portable duplicate-name projection");
    assert_eq!(sets[0].bytes(), set_entry(0, &[fixture.entries[0].clone(), fixture.entries[1].clone()]));
    assert_eq!(files[1].name.to_string_lossy(), "A");
    assert_eq!(files[1].actions.iter().map(|action| action.name.to_string_lossy()).collect::<Vec<_>>(), ["Same 3"]);
    let mut expected_second = original[1].actions.clone();
    expected_second[0].name = UnicodeString::new_nul("Same 3");
    assert_eq!(files[1].actions, expected_second, "the second physical row stays intact apart from its portable duplicate-name projection");
    assert_eq!(sets[1].bytes(), set_entry(1, std::slice::from_ref(&fixture.entries[2])));
    let outer = files[2].actions[0].items[0].descriptor.as_ref().unwrap().to_bytes();
    assert_fields_equal(&outer, &fixture.outer, &["Msge", "Cntn"]);
    assert_fields_equal(metadata(&files).bytes(), fixture.payload.get(4..).unwrap(), &["futureGlobal"]);
    let external = loaded(&bytes);
    assert_eq!(panel_groups(&external.actions), expected, "untrusted import follows wire rows, not the private identity manifest");
    let durable = crate::actions_cmds::library::encode(&session.actions, &[]).unwrap();
    let trusted = crate::actions_cmds::library::decode(&durable).unwrap();
    assert_eq!(action_names(&trusted.list), ["Same", "Same 3", "Same 2"], "trusted local restore keeps the moved global order");
    assert_eq!(panel_groups(&trusted), expected);
}

#[test]
fn duplicate_names_are_renamed_after_metadata_and_shortcuts_bind_by_position() {
    let fixture = fixture(&["A", "B"], false, "Note\0");
    let mut session = Session::new();
    session.actions.list.push(Action::new("Same", vec![]));
    let response = session.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&fixture.bytes), "replace": false})).unwrap();
    assert_eq!(response["actions"], json!(["Same 2", "Same 3"]));
    assert_eq!(session.actions.list[1].description, "First description");
    assert_eq!(session.actions.list[2].description, "Second description");
    assert_eq!(session.prefs().shortcuts.get("actions.play:Same 2").map(String::as_str), Some("F6"));
    assert_eq!(session.prefs().shortcuts.get("actions.play:Same 3").map(String::as_str), Some("F7"));
    let files = saved(&mut session, json!({"action": "Same 3"}));
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].name.to_string_lossy(), "B");
    let sets = metadata(&files).list("sets").unwrap();
    assert_eq!(integer(&sets[0], "setColorIndex"), 22);
    assert_eq!(sets[0].list("actions").unwrap()[0].bytes(), fixture.entries[1]);
}

#[test]
fn selected_set_and_empty_set_export_rebuild_only_their_entries() {
    let fixture = fixture(&["A", "B"], true, "Note\0");
    let mut session = loaded(&fixture.bytes);
    let files = saved(&mut session, json!({"set": "B"}));
    assert_eq!(metadata(&files).list("sets").unwrap().len(), 1);
    assert_eq!(set_metadata(&files, "B").list("actions").unwrap()[0].bytes(), fixture.entries[1]);
    let empty = saved(&mut session, json!({"set": "Empty"}));
    assert!(empty[0].actions.is_empty());
    assert_eq!(integer(&set_metadata(&empty, "Empty"), "setColorIndex"), 99);
}

#[test]
fn edits_preserve_unknown_thumbnail_until_explicit_choice_and_all_other_raw_fields() {
    let fixture = fixture(&["A", "B"], false, "Note\0");
    let mut session = loaded(&fixture.bytes);
    let action = &mut session.actions.list[1];
    assert_eq!(*action, action.clone(), "raw NaN bytes do not break draft equality");
    action.description = "Edited".into();
    let edited = RawDescriptor::new(&entry(action).unwrap()).unwrap();
    assert_eq!(integer(&edited, "actionThumbnailTypeKey"), 42);
    assert_eq!(integer(&edited, "actionPreconditionVal"), 73);
    let (_, original_fields) = RawDescriptor::new(&fixture.entries[1]).unwrap().fields().unwrap();
    let (_, fields) = edited.fields().unwrap();
    let original = RawDescriptor::new(&fixture.entries[1]).unwrap();
    for key in ["futureDouble", "futureActionBytes", "actionPreviewKey"] {
        let before = original_fields.iter().find(|field| field.key.is(key)).unwrap();
        let after = fields.iter().find(|field| field.key.is(key)).unwrap();
        assert_eq!(original.source.get(before.span.clone()), edited.source.get(after.span.clone()));
    }
    explicit_icon(action);
    let reset = RawDescriptor::new(&entry(action).unwrap()).unwrap();
    assert_eq!(integer(&reset, "actionThumbnailTypeKey"), 0);
    assert!(reset.parsed().unwrap().get("actionIconKey").is_none());
    action.icon = "ClickAction".into();
    let explicit = RawDescriptor::new(&entry(action).unwrap()).unwrap();
    assert_eq!(integer(&explicit, "actionThumbnailTypeKey"), 3);
    assert_eq!(explicit.parsed().unwrap().get("actionIconKey"), Some(&D::Text(UnicodeString::new("ClickAction"))));
}

#[test]
fn clone_and_rename_keep_action_entry_and_assign_set_rebinds_only_set_metadata() {
    let fixture = fixture(&["A", "B"], true, "Note\0");
    let mut session = loaded(&fixture.bytes);
    let mut clone = session.actions.list[0].clone();
    clone.name = "Clone".into();
    assert_eq!(entry(&clone).unwrap(), fixture.entries[0]);
    session.actions.list.push(clone);
    session.execute("actions.assignSet", json!({"action": "Clone", "set": "B"})).unwrap();
    let files = saved(&mut session, json!({"action": "Clone"}));
    let set = set_metadata(&files, "B");
    assert_eq!(integer(&set, "setColorIndex"), 22);
    assert_eq!(set.list("actions").unwrap()[0].bytes(), fixture.entries[0]);
    let whole = saved(&mut session, json!({}));
    let b = set_metadata(&whole, "B");
    assert_eq!(integer(&b, "setColorIndex"), 22);
    let b_actions = b.list("actions").unwrap();
    let clone_index = whole
        .iter()
        .find(|file| file.name.to_string_lossy() == "B")
        .unwrap()
        .actions
        .iter()
        .position(|action| action.name.to_string_lossy() == "Clone")
        .unwrap();
    assert_eq!(b_actions[clone_index].bytes(), fixture.entries[0]);
    session.execute("actions.assignSet", json!({"action": "Clone", "set": "New"})).unwrap();
    let files = saved(&mut session, json!({"action": "Clone"}));
    assert_eq!(integer(&set_metadata(&files, "New"), "setColorIndex"), 0);
    let whole = saved(&mut session, json!({}));
    assert_eq!(integer(&set_metadata(&whole, "New"), "setColorIndex"), 0);

    // Import and confirmed replacement each bind action provenance and its physical row from
    // the same decoded archive. Action-only and full exports retain B's set metadata and bytes.
    let imported = self::fixture(&["A", "B"], false, "Note\0");
    for replace in [false, true] {
        let mut roundtrip = Session::new();
        let data = photocraft_paint::tile::b64_encode(&imported.bytes);
        if replace {
            roundtrip.execute("actions.replace", json!({"data":data,"confirm":true})).unwrap();
        } else {
            roundtrip.execute("actions.load", json!({"data":data,"replace":false})).unwrap();
        }
        let action_only = saved(&mut roundtrip, json!({"action":"Same 2"}));
        assert_eq!(integer(&set_metadata(&action_only, "B"), "setColorIndex"), 22);
        assert_eq!(set_metadata(&action_only, "B").list("actions").unwrap()[0].bytes(), imported.entries[1]);
        let whole = saved(&mut roundtrip, json!({}));
        assert_eq!(integer(&set_metadata(&whole, "B"), "setColorIndex"), 22);
        assert_eq!(set_metadata(&whole, "B").list("actions").unwrap()[0].bytes(), imported.entries[1]);
        roundtrip.execute("actions.assignSet", json!({"action":"Same 2","set":"New"})).unwrap();
        let action_only = saved(&mut roundtrip, json!({"action":"Same 2"}));
        assert_eq!(integer(&set_metadata(&action_only, "New"), "setColorIndex"), 0);
        assert_eq!(set_metadata(&action_only, "New").list("actions").unwrap()[0].bytes(), imported.entries[1]);
        let whole = saved(&mut roundtrip, json!({}));
        assert_eq!(integer(&set_metadata(&whole, "New"), "setColorIndex"), 0);
        assert_eq!(set_metadata(&whole, "New").list("actions").unwrap()[0].bytes(), imported.entries[1]);
    }
}

#[test]
fn last_action_removal_keeps_live_set_row_but_retired_rows_never_reattach_by_name() {
    let fixture = fixture(&["A", "B"], true, "Note\0");
    let mut session = loaded(&fixture.bytes);
    session.actions.list.remove(0);
    let files = saved(&mut session, json!({"set": "A"}));
    assert!(files[0].actions.is_empty());
    assert_eq!(integer(&set_metadata(&files, "A"), "setColorIndex"), 11);
    retire_sets(&mut session.actions, &std::collections::HashSet::from(["A".to_string()]));
    install_row(&mut session.actions, "A", None);
    assert!(groups(&session.actions, Some("A")).unwrap()[0].1.source.is_none());
    let files = saved(&mut session, json!({"set": "B"}));
    assert_eq!(integer(&set_metadata(&files, "B"), "setColorIndex"), 22, "surviving row slot was remapped");
}

#[test]
fn replacement_retires_only_target_rows_and_keeps_other_set_metadata() {
    let mut session = loaded(&fixture(&["A", "B"], true, "Note\0").bytes);
    let replacement = fixture(&["A"], false, "Note\0");
    session.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&replacement.bytes)})).unwrap();
    assert_eq!(session.actions.list.len(), 2);
    assert_eq!(session.actions.set_rows.iter().filter(|name| *name == "A").count(), 1);
    let files = saved(&mut session, json!({}));
    assert_eq!(integer(&set_metadata(&files, "B"), "setColorIndex"), 22);
    assert_eq!(integer(&set_metadata(&files, "Empty"), "setColorIndex"), 99);
}

#[test]
fn incompatible_archive_notes_refuse_import_before_mutation() {
    let mut session = loaded(&fixture(&["A"], false, "First note\0").bytes);
    let second = fixture(&["B"], false, "Second note\0");
    let before = (session.actions.list.clone(), session.actions.sets.clone(), session.actions.atn_set_rows.clone());
    let prefs = session.prefs().clone();
    let output = saved_bytes(&mut session, json!({"set":"A"}));
    let error = session.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&second.bytes), "replace": false})).unwrap_err();
    assert!(error.to_string().contains("conflict"), "{error}");
    assert_eq!(session.actions.list, before.0);
    assert_eq!(session.actions.sets, before.1);
    assert_eq!(session.actions.atn_set_rows, before.2);
    assert_eq!(session.prefs(), &prefs);
    for skip in [false, true] {
        assert_eq!(saved_bytes(&mut session, json!({"set":"A","skipUnsupported":skip})), output);
    }
}

#[test]
fn malformed_metadata_or_later_set_never_commits_a_partial_library() {
    let fixture = fixture(&["A", "B"], false, "Note\0");
    let mut session = Session::new();
    session.actions.list.push(Action::new("Keep", vec![]));
    let before = session.actions.list.clone();
    let mut files = atn::parse_sets(&fixture.bytes).unwrap();
    files[0].actions.clear(); // Deliberately mismatch the independently encoded sidecar.
    let invalid = atn::write_sets(&files).unwrap();
    for bytes in [&fixture.bytes[..fixture.bytes.len() - 1], invalid.as_slice()] {
        assert!(session.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(bytes)})).is_err());
        assert_eq!(session.actions.list, before);
        assert!(session.actions.sets.is_empty());
    }
}

#[test]
fn similarly_named_user_set_and_unknown_payload_version_remain_visible_and_opaque() {
    let mut files = atn::parse_sets(&fixture(&["A"], false, "Note\0").bytes).unwrap();
    files.last_mut().unwrap().actions[0].name = UnicodeString::new_nul("User action");
    let imported = super::super::import(&atn::write_sets(&files).unwrap()).unwrap();
    assert_eq!(imported.sets.len(), 2);
    assert_eq!(imported.actions.last().unwrap().steps[0].0, super::super::PHOTOSHOP_STEP);
    files.last_mut().unwrap().actions[0].name = UnicodeString::new_nul("Play for details");
    let descriptor = files.last_mut().unwrap().actions[0].items[0].descriptor.as_mut().unwrap();
    let (_, D::RawData(bytes)) = descriptor.items.iter_mut().find(|(key, _)| key.is(MODERN)).unwrap() else { panic!("raw payload") };
    bytes[..4].copy_from_slice(&17u32.to_be_bytes());
    let imported = super::super::import(&atn::write_sets(&files).unwrap()).unwrap();
    assert_eq!(imported.sets.len(), 2);
    assert!(imported.actions.iter().all(|action| action.atn_metadata.is_none()));
}

#[test]
fn new_default_has_no_sidecar_but_description_and_explicit_clickaction_have_canonical_metadata() {
    let mut action = Action::new("New", vec![]);
    let (bytes, _) = super::super::export("A", &[&action], &[], false).unwrap();
    assert_eq!(atn::parse_sets(&bytes).unwrap().len(), 1);
    action.description = "Description".into();
    let (bytes, _) = super::super::export("A", &[&action], &[], false).unwrap();
    let files = atn::parse_sets(&bytes).unwrap();
    let entry = set_metadata(&files, "A").list("actions").unwrap().remove(0);
    assert_eq!(integer(&entry, "actionThumbnailTypeKey"), 0);
    assert!(entry.parsed().unwrap().get("actionIconKey").is_none());
    action.icon = "ClickAction".into();
    let (bytes, _) = super::super::export("A", &[&action], &[], false).unwrap();
    let files = atn::parse_sets(&bytes).unwrap();
    assert_eq!(integer(&set_metadata(&files, "A").list("actions").unwrap()[0], "actionThumbnailTypeKey"), 3);
}

#[test]
fn empty_wire_category_and_legacy_none_share_one_group_and_export_empty_name() {
    let mut session = loaded(&fixture(&[""], false, "Note\0").bytes);
    assert_eq!(session.actions.list[0].set.as_deref(), Some(super::super::UNGROUPED_SET));
    session.actions.list.push(Action::new("Legacy", vec![]));
    assert_eq!(crate::actions_edit_cmds::set_names(&session), [super::super::UNGROUPED_SET]);
    assert_eq!(groups(&session.actions, None).unwrap().len(), 1);
    let old = session.actions.list[0].atn_metadata.clone();
    session.execute("actions.assignSet", json!({"action":0,"set":null})).unwrap();
    assert_eq!(session.actions.list[0].atn_metadata, old);
    assert_eq!(groups(&session.actions, None).unwrap().len(), 1, "keep the assigned action in its matching empty-wire row");
    let files = saved(&mut session, json!({"set": super::super::UNGROUPED_SET}));
    assert_eq!(files[0].name.to_string_lossy(), "");
    assert_eq!(files[0].actions.len(), 2);
    assert_eq!(integer(&set_metadata(&files, ""), "setColorIndex"), 11);

    // A fresh action may join one raw empty-name row, but multiple such rows remain distinct.
    let mut ambiguous = loaded(&fixture(&["", ""], false, "Ambiguous empty rows\0").bytes);
    ambiguous.actions.list.push(Action::new("Loose", vec![]));
    let portable = groups(&ambiguous.actions, None).unwrap();
    assert_eq!(portable.len(), 3, "do not choose arbitrarily between two physical empty-name rows");
    assert_eq!(portable.iter().filter(|(_, selection)| selection.raw.is_some()).count(), 2);
    assert_eq!(portable.iter().filter(|(_, selection)| selection.raw.is_none()).count(), 1);

    // Assigned actions from two physical empty-name rows stay unbound as a separate portable
    // group; the first row's provenance cannot claim the other row's action.
    let mut ambiguous_assigned = loaded(&fixture(&["", ""], false, "Ambiguous assigned rows\0").bytes);
    ambiguous_assigned.execute("actions.assignSet", json!({"action":0,"set":null})).unwrap();
    ambiguous_assigned.execute("actions.assignSet", json!({"action":1,"set":null})).unwrap();
    assert_eq!(groups(&ambiguous_assigned.actions, None).unwrap().len(), 3);

    // A source row whose logical caption was overridden is not the default category, even
    // though its retained raw file still has an empty wire name.
    let mut renamed = Session::new();
    renamed
        .execute(
            "actions.load",
            json!({
                "data": photocraft_paint::tile::b64_encode(&fixture(&[""], false, "Renamed empty row\0").bytes),
                "replace": false,
                "set": "Retouch"
            }),
        )
        .unwrap();
    renamed.actions.list.push(Action::new("Loose", vec![]));
    let renamed_groups = groups(&renamed.actions, None).unwrap();
    assert_eq!(renamed_groups.len(), 2, "do not merge a loose action into a renamed physical row");
    assert!(renamed_groups.iter().any(|(name, selection)| *name == "Retouch" && selection.raw.is_some()));
    assert!(renamed_groups.iter().any(|(name, selection)| *name == super::super::UNGROUPED_SET && selection.raw.is_none()));
}

#[test]
fn literal_reserved_group_name_never_merges_with_empty_wire_category() {
    let fixture = fixture(&["", super::super::UNGROUPED_SET, "Ungrouped Actions 2"], false, "Note\0");
    let imported = super::super::import(&fixture.bytes).unwrap();
    assert_eq!(imported.sets.iter().map(|set| set.name.as_str()).collect::<Vec<_>>(), ["Ungrouped Actions", "Ungrouped Actions 3", "Ungrouped Actions 2"]);
    assert!(!imported.warnings.is_empty());
    assert_eq!(imported.actions[1].atn_metadata.as_ref().unwrap().set.as_ref().unwrap().ordinal, 1);
}

#[test]
fn duplicate_physical_set_names_keep_distinct_positional_metadata_on_save() {
    let mut session = loaded(&fixture(&["A", "A"], false, "Note\0").bytes);
    let files = saved(&mut session, json!({"set": "A"}));
    assert_eq!(files.len(), 3);
    let sets = metadata(&files).list("sets").unwrap();
    assert_eq!(integer(&sets[0], "setColorIndex"), 11);
    assert_eq!(integer(&sets[1], "setColorIndex"), 22);
    assert_eq!(sets[0].list("actions").unwrap().len(), 1);
    assert_eq!(sets[1].list("actions").unwrap().len(), 1);
}

#[test]
fn sidecar_overhead_is_reserved_before_appending_a_physical_set() {
    let mut action = Action::new("Described", vec![]);
    action.description = "Details".into();
    let mut files = vec![AtnFile::default(); atn::MAX_SETS];
    let selections: Vec<_> =
        (0..atn::MAX_SETS).map(|index| Selection { actions: if index == 0 { vec![&action] } else { vec![] }, source: None, raw: None }).collect();
    assert!(append(&mut files, &selections).is_err());
    assert_eq!(files.len(), atn::MAX_SETS);

    // The hidden carrier also consumes one physical set slot; a full folder inventory cannot append it.
    let mut session = loaded(&fixture_rows(&[("A", 2), ("A", 1)], None, "Capacity\0").bytes);
    session.execute("actions.move", json!({"action":"Same 3","to":1})).unwrap();
    session.actions.set_rows.extend((0..atn::MAX_SETS - 2).map(|index| format!("Empty {index}")));
    assert_eq!(session.actions.set_rows.len(), atn::MAX_SETS);
    let before = (session.actions.list.clone(), session.actions.sets.clone(), session.actions.set_rows.clone(), session.actions.atn_set_rows.clone());
    let prefs = serde_json::to_value(session.prefs()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.atn");
    std::fs::write(&path, b"keep previous archive").unwrap();
    let error = session.execute("actions.save", json!({"path":path.to_str().unwrap()})).unwrap_err();
    assert!(error.to_string().contains("no space for its metadata sidecar"), "{error}");
    assert_eq!(session.actions.list, before.0);
    assert_eq!(session.actions.sets, before.1);
    assert_eq!(session.actions.set_rows, before.2);
    assert_eq!(session.actions.atn_set_rows, before.3);
    assert_eq!(serde_json::to_value(session.prefs()).unwrap(), prefs);
    assert_eq!(std::fs::read(path).unwrap(), b"keep previous archive");
}

#[test]
fn recognized_sidecar_without_real_sets_is_not_silently_discarded() {
    let files = atn::parse_sets(&fixture(&[], false, "Only note\0").bytes).unwrap();
    assert!(decode(&files).is_err());
}

#[test]
fn native_group_operations_keep_row_carriers_through_copy_move_delete_and_name_reuse() {
    let fixture = fixture(&["A", "B"], true, "Original note\0");
    let mut session = loaded(&fixture.bytes);
    session.execute("actions.duplicate", json!({"set":"A"})).unwrap();
    session.execute("actions.duplicate", json!({"set":"Empty"})).unwrap();
    let copied = saved(&mut session, json!({"set":"A copy"}));
    assert_eq!(integer(&set_metadata(&copied, "A copy"), "setColorIndex"), 11);
    assert_eq!(set_metadata(&copied, "A copy").list("actions").unwrap()[0].bytes(), fixture.entries[0]);
    let empty_copy = saved(&mut session, json!({"set":"Empty copy"}));
    assert!(empty_copy[0].actions.is_empty());
    assert_eq!(integer(&set_metadata(&empty_copy, "Empty copy"), "setColorIndex"), 99);
    session.execute("actions.move", json!({"action":"Same 2","to":0})).unwrap();
    let expected = vec![
        ("A copy".into(), None, vec!["Same copy".into()]),
        ("Empty copy".into(), None, vec![]),
        ("A".into(), Some(0), vec!["Same".into()]),
        ("B".into(), Some(1), vec!["Same 2".into()]),
        ("Empty".into(), Some(2), vec![]),
    ];
    assert_eq!(panel_groups(&session.actions), expected);
    let bytes = saved_bytes(&mut session, json!({}));
    let moved = atn::parse_sets(&bytes).unwrap();
    assert_eq!(moved.iter().take(moved.len() - 1).map(|file| file.name.to_string_lossy()).collect::<Vec<_>>(), ["A", "B", "Empty", "A copy", "Empty copy"]);
    assert_eq!(integer(&set_metadata(&moved, "B"), "setColorIndex"), 22);
    assert_eq!(set_metadata(&moved, "B").list("actions").unwrap()[0].bytes(), fixture.entries[1]);
    let external = loaded(&bytes);
    assert_eq!(
        panel_groups(&external.actions),
        [
            ("A".into(), Some(0), vec!["Same".into()]),
            ("B".into(), Some(1), vec!["Same 2".into()]),
            ("Empty".into(), Some(2), vec![]),
            ("A copy".into(), Some(3), vec!["Same copy".into()]),
            ("Empty copy".into(), Some(4), vec![]),
        ]
    );
    let durable = crate::actions_cmds::library::encode(&session.actions, &[]).unwrap();
    let trusted = crate::actions_cmds::library::decode(&durable).unwrap();
    assert_eq!(action_names(&trusted.list), ["Same 2", "Same", "Same copy"]);
    assert_eq!(panel_groups(&trusted), expected);
    session.execute("actions.delete", json!({"set":"A"})).unwrap();
    session.execute("actions.delete", json!({"action":"Same copy"})).unwrap();
    let live_empty = saved(&mut session, json!({"set":"A copy"}));
    assert!(live_empty[0].actions.is_empty());
    assert_eq!(integer(&set_metadata(&live_empty, "A copy"), "setColorIndex"), 11);
    session.execute("actions.delete", json!({"set":"A copy"})).unwrap();
    session.execute("actions.newSet", json!({"name":"A copy"})).unwrap();
    let reused = saved(&mut session, json!({"set":"A copy"}));
    assert_eq!(reused.len(), 1, "a reused empty folder has no retired sidecar");
    assert!(reused[0].actions.is_empty());
    for (name, color) in [("B", 22), ("Empty", 99), ("Empty copy", 99)] {
        let files = saved(&mut session, json!({"set":name}));
        assert_eq!(integer(&set_metadata(&files, name), "setColorIndex"), color, "survivor {name} follows its remapped row");
    }
}

#[test]
fn duplicating_one_logical_group_copies_each_distinct_physical_set_carrier() {
    let fixture = fixture(&["A", "A"], false, "Note\0");
    let mut session = loaded(&fixture.bytes);
    session.execute("actions.duplicate", json!({"set":"A"})).unwrap();
    session.execute("actions.delete", json!({"set":"A"})).unwrap();
    let files = saved(&mut session, json!({"set":"A copy"}));
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].name.to_string_lossy(), "A copy");
    assert_eq!(files[1].name.to_string_lossy(), "A copy");
    let sets = metadata(&files).list("sets").unwrap();
    assert_eq!(sets.len(), 2);
    for (index, color) in [(0, 11), (1, 22)] {
        assert_eq!(integer(&sets[index], "setColorIndex"), color);
        assert_eq!(sets[index].list("actions").unwrap()[0].bytes(), fixture.entries[index]);
    }
}

#[test]
fn actual_editor_icon_update_preserves_unknown_type_until_an_explicit_default_choice() {
    let mut session = loaded(&fixture(&["A", "B"], false, "Note\0").bytes);
    let opened = session.execute("actions.editor.open", json!({"action":"Same 2"})).unwrap();
    assert_eq!(opened["dirty"], false, "raw NaN metadata clones compare by bytes");
    let id = opened["draft"].as_u64().unwrap();
    session.execute("actions.editor.update", json!({"draft":id,"description":"Edited description"})).unwrap();
    session.execute("actions.editor.save", json!({"draft":id})).unwrap();
    let preserved = saved(&mut session, json!({"action":"Same 2"}));
    assert_eq!(integer(&set_metadata(&preserved, "B").list("actions").unwrap()[0], "actionThumbnailTypeKey"), 42);
    let reset = session.execute("actions.editor.update", json!({"draft":id,"icon":""})).unwrap();
    assert_eq!(reset["dirty"], true, "an explicit default click differs from leaving an unknown thumbnail alone");
    session.execute("actions.editor.save", json!({"draft":id})).unwrap();
    let reset = saved(&mut session, json!({"action":"Same 2"}));
    let entries = set_metadata(&reset, "B").list("actions").unwrap();
    let entry = &entries[0];
    assert_eq!(integer(entry, "actionThumbnailTypeKey"), 0);
    assert!(entry.parsed().unwrap().get("actionIconKey").is_none());
}

#[test]
fn duplicating_one_same_caption_occurrence_keeps_only_its_opaque_metadata() {
    let original = fixture(&["Same", "Same"], false, "shared archive note");
    let mut session = Session::new();
    session.actions = crate::actions_cmds::library::decode(&original.bytes).unwrap();
    let first = session.actions.atn_set_rows.iter().find(|row| row.slot == 0).unwrap().source.raw.bytes().to_vec();
    let second = session.actions.atn_set_rows.iter().find(|row| row.slot == 1).unwrap().source.raw.bytes().to_vec();
    assert_ne!(first, second);
    let ids = ["11111111111111111111111111111111", "22222222222222222222222222222222"];
    for (action, id) in session.actions.list.iter_mut().zip(ids) {
        action.action_id = Some(id.into());
    }
    session.edit_prefs(|prefs| {
        prefs.shortcuts.insert(crate::actions_cmds::occurrence_key(ids[1]), "F7".into());
    });
    let result = session.execute("actions.duplicate", json!({"set":"Same","setSlot":1})).unwrap();
    assert_eq!(result["actions"], 1);
    assert_eq!(result["setSlot"], 2);
    assert_eq!(session.actions.set_rows, ["Same", "Same", "Same copy"]);
    let copied = session.actions.list.iter().find(|action| action.set_slot == Some(2)).unwrap();
    assert_eq!(copied.description, "Second description");
    assert_eq!(copied.action_id, None);
    assert!(!copied.legacy_shortcut_owner);
    assert_eq!(session.prefs().shortcuts.get(&crate::actions_cmds::occurrence_key(ids[1])).map(String::as_str), Some("F7"));
    let bytes = crate::actions_cmds::library::encode(&session.actions, &[]).unwrap();
    let restored = crate::actions_cmds::library::decode(&bytes).unwrap();
    // The archive gains a copied set and local identity metadata; only these descriptor spans must stay exact.
    assert_eq!(restored.atn_set_rows.iter().find(|row| row.slot == 0).unwrap().source.raw.bytes(), first.as_slice());
    assert_eq!(restored.atn_set_rows.iter().find(|row| row.slot == 2).unwrap().source.raw.bytes(), second.as_slice());
}
