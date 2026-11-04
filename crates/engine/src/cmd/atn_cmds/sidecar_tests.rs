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
fn panel_groups(session: &Session) -> Vec<(String, Vec<String>)> {
    crate::actions_edit_cmds::set_names(session)
        .into_iter()
        .map(|name| {
            let actions = session
                .actions
                .list
                .iter()
                .filter(|action| super::super::logical_set_name(action.set.as_deref()) == name)
                .map(|action| action.name.clone())
                .collect();
            (name, actions)
        })
        .collect()
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

#[test]
fn public_load_save_all_keeps_positional_metadata_and_raw_descriptor_bytes() {
    let fixture = fixture(&["A", "B"], true, "Original note\0");
    let mut session = loaded(&fixture.bytes);
    assert_eq!(session.actions.list.len(), 2);
    assert_eq!(session.actions.list[0].description, "First description");
    assert_eq!(session.actions.list[0].icon, "future-icon");
    assert_eq!(session.actions.list[1].description, "Second description");
    assert_eq!(session.actions.list[1].icon, "");
    assert_eq!(session.actions.sets, ["A", "B", "Empty"]);
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
fn moving_a_later_physical_set_keeps_panel_order_and_positional_metadata_after_reload() {
    let note = "Interleaved move\0";
    let fixture = fixture(&["A", "B", "A"], false, note);
    let mut session = loaded(&fixture.bytes);
    session.execute("actions.move", json!({"action":"Same 3","to":0})).unwrap();
    let expected: Vec<(String, Vec<String>)> = vec![("A".into(), vec!["Same 3".into(), "Same".into()]), ("B".into(), vec!["Same 2".into()])];
    assert_eq!(panel_groups(&session), expected);
    let bytes = saved_bytes(&mut session, json!({}));
    let files = atn::parse_sets(&bytes).unwrap();
    assert_eq!(files.len(), 4);
    let sets = metadata(&files).list("sets").unwrap();
    for (index, name, action, source) in [(0, "A", "Same 3", 2), (1, "A", "Same", 0), (2, "B", "Same 2", 1)] {
        assert_eq!(files[index].name.to_string_lossy(), name);
        assert_eq!(files[index].actions.len(), 1);
        assert_eq!(files[index].actions[0].name.to_string_lossy(), action);
        assert_eq!(sets[index].bytes(), set_entry(source, std::slice::from_ref(&fixture.entries[source])));
    }
    let (_, outer) = sidecar_data(
        &[
            set_entry(2, std::slice::from_ref(&fixture.entries[2])),
            set_entry(0, std::slice::from_ref(&fixture.entries[0])),
            set_entry(1, std::slice::from_ref(&fixture.entries[1])),
        ],
        note,
    );
    assert_eq!(files[3].actions[0].items[0].descriptor.as_ref().unwrap().to_bytes(), outer);
    assert_eq!(panel_groups(&loaded(&bytes)), expected);
}

#[test]
fn moving_between_actions_of_one_source_keeps_three_source_runs_after_reload() {
    let note = "Source runs\0";
    let fixture = fixture_rows(&[("A", 2), ("A", 1)], None, note);
    let mut session = loaded(&fixture.bytes);
    session.execute("actions.move", json!({"action":"Same 3","to":1})).unwrap();
    let expected: Vec<(String, Vec<String>)> = vec![("A".into(), vec!["Same".into(), "Same 3".into(), "Same 2".into()])];
    assert_eq!(panel_groups(&session), expected);
    let bytes = saved_bytes(&mut session, json!({}));
    let files = atn::parse_sets(&bytes).unwrap();
    assert_eq!(files.len(), 4);
    let sets = metadata(&files).list("sets").unwrap();
    let mut expected_sets = Vec::new();
    for (index, action, source, entry) in [(0, "Same", 0, 0), (1, "Same 3", 1, 2), (2, "Same 2", 0, 1)] {
        assert_eq!(files[index].name.to_string_lossy(), "A");
        assert_eq!(files[index].actions.len(), 1);
        assert_eq!(files[index].actions[0].name.to_string_lossy(), action);
        let raw = set_entry(source, std::slice::from_ref(&fixture.entries[entry]));
        assert_eq!(sets[index].bytes(), raw);
        expected_sets.push(raw);
    }
    let (_, outer) = sidecar_data(&expected_sets, note);
    assert_eq!(files[3].actions[0].items[0].descriptor.as_ref().unwrap().to_bytes(), outer);
    assert_eq!(panel_groups(&loaded(&bytes)), expected);
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
    session.execute("actions.assignSet", json!({"action": "Clone", "set": "New"})).unwrap();
    let files = saved(&mut session, json!({"action": "Clone"}));
    assert_eq!(integer(&set_metadata(&files, "New"), "setColorIndex"), 0);
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
    assert_eq!(session.actions.sets.iter().filter(|name| *name == "A").count(), 1);
    let files = saved(&mut session, json!({}));
    assert_eq!(integer(&set_metadata(&files, "B"), "setColorIndex"), 22);
    assert_eq!(integer(&set_metadata(&files, "Empty"), "setColorIndex"), 99);
}

#[test]
fn incompatible_archive_notes_refuse_export_even_when_skipping_unsupported() {
    let mut session = loaded(&fixture(&["A"], false, "First note\0").bytes);
    let second = fixture(&["B"], false, "Second note\0");
    session.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&second.bytes), "replace": false})).unwrap();
    let before = session.actions.list.clone();
    for skip in [false, true] {
        let error = session.execute("actions.save", json!({"skipUnsupported": skip})).unwrap_err().to_string();
        assert!(error.contains("conflict"), "{error}");
        assert_eq!(session.actions.list, before);
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
    let files = saved(&mut session, json!({"set": super::super::UNGROUPED_SET}));
    assert_eq!(files[0].name.to_string_lossy(), "");
    assert_eq!(files[0].actions.len(), 2);
    assert_eq!(integer(&set_metadata(&files, ""), "setColorIndex"), 11);
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

    // The move needs one more physical row for S1/S2/S1. Never merge it to fit.
    let mut session = loaded(&fixture_rows(&[("A", 2), ("A", 1)], None, "Capacity\0").bytes);
    session.execute("actions.move", json!({"action":"Same 3","to":1})).unwrap();
    session.actions.sets.extend((0..atn::MAX_SETS - 3).map(|index| format!("Empty {index}")));
    assert_eq!(session.actions.sets.len(), atn::MAX_SETS - 1);
    let before = (session.actions.list.clone(), session.actions.sets.clone(), session.actions.atn_set_rows.clone());
    let prefs = serde_json::to_value(session.prefs()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.atn");
    std::fs::write(&path, b"keep previous archive").unwrap();
    let error = session.execute("actions.save", json!({"path":path.to_str().unwrap()})).unwrap_err();
    assert!(error.to_string().contains("no space for its metadata sidecar"), "{error}");
    assert_eq!(session.actions.list, before.0);
    assert_eq!(session.actions.sets, before.1);
    assert_eq!(session.actions.atn_set_rows, before.2);
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
    let expected: Vec<(String, Vec<String>)> = vec![
        ("B".into(), vec!["Same 2".into()]),
        ("A".into(), vec!["Same".into()]),
        ("A copy".into(), vec!["Same copy".into()]),
        ("Empty".into(), vec![]),
        ("Empty copy".into(), vec![]),
    ];
    assert_eq!(panel_groups(&session), expected);
    let bytes = saved_bytes(&mut session, json!({}));
    let moved = atn::parse_sets(&bytes).unwrap();
    assert_eq!(moved[0].name.to_string_lossy(), "B");
    assert_eq!(moved.iter().take(moved.len() - 1).map(|file| file.name.to_string_lossy()).collect::<Vec<_>>(), ["B", "A", "A copy", "Empty", "Empty copy"]);
    assert_eq!(integer(&set_metadata(&moved, "B"), "setColorIndex"), 22);
    assert_eq!(set_metadata(&moved, "B").list("actions").unwrap()[0].bytes(), fixture.entries[1]);
    assert_eq!(panel_groups(&loaded(&bytes)), expected);
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

// Reuse this independently spelled wire oracle for the isolated placement prototype.
pub(crate) fn physical_placement_fixture() -> Vec<u8> {
    fixture_rows(&[("Same", 1), ("Same", 1), ("Same", 0)], None, "placement").bytes
}
pub(crate) fn physical_action_payload(action: &Action) -> Vec<u8> {
    action.atn_metadata.as_ref().unwrap().raw.bytes().to_vec()
}
pub(crate) fn physical_set_payload(state: &ActionState, slot: usize) -> Vec<u8> {
    empty_source(state, slot).unwrap().raw.bytes().to_vec()
}
