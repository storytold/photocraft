use std::sync::Arc;

use super::*;

// The PSD round trip in #13191 produced two "??" names. Keep the geometries apart so
// choosing the first match or falling back to the Work Path cannot pass unnoticed.
fn duplicate_paths(depth: u32) -> Session {
    let mut s = session(100, 100, depth);
    let d = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    d.paths = vec![
        NamedPath { name: "??".into(), path: parse_path(&json!({"subpaths":[{"knots":[[10,10],[30,10],[20,30]]}]})).unwrap(), psd_raw: None },
        NamedPath { name: "??".into(), path: vector::shapes::rect(60.0, 10.0, 20.0, 20.0), psd_raw: None },
    ];
    d.work_path = Some(vector::shapes::rect(10.0, 60.0, 20.0, 20.0));
    s
}

#[test]
fn path_index_lists_and_reads_each_duplicate_independently() {
    let mut s = duplicate_paths(8);
    let paths = doc(&s).paths.clone();
    let list = s.execute("path.list", json!({})).unwrap();
    for (i, path) in paths.iter().enumerate() {
        assert_eq!(list["paths"][i]["pathIndex"], json!(i));
        let info = s.execute("path.info", json!({"pathIndex":i})).unwrap();
        assert_eq!(info["name"], "??");
        assert_eq!(info["pathIndex"], json!(i));
        assert_eq!(info["path"], path_json(&path.path));
    }
}

#[test]
fn path_index_rename_changes_only_second_and_undo_restores_order() {
    let mut s = duplicate_paths(8);
    let before = s.active().unwrap().doc.clone();
    s.execute("path.rename", json!({"pathIndex":1,"to":"Right"})).unwrap();
    assert_eq!(doc(&s).paths.len(), 2);
    assert_eq!(doc(&s).paths[0], before.paths[0]);
    assert_eq!(doc(&s).paths[1].name, "Right");
    assert_eq!(doc(&s).paths[1].path, before.paths[1].path);
    assert_eq!(doc(&s).work_path, before.work_path);
    assert!(s.undo());
    assert_eq!(doc(&s), before.as_ref());
    assert!(s.redo());
    assert_eq!(doc(&s).paths[1].name, "Right");
}

#[test]
fn path_index_delete_removes_exactly_one_and_undo_redo_keep_order() {
    let mut s = duplicate_paths(8);
    let before = s.active().unwrap().doc.clone();
    s.execute("path.delete", json!({"pathIndex":1})).unwrap();
    assert_eq!(doc(&s).paths, vec![before.paths[0].clone()]);
    assert_eq!(doc(&s).work_path, before.work_path);
    assert!(s.undo());
    assert_eq!(doc(&s), before.as_ref());
    assert!(s.redo());
    assert_eq!(doc(&s).paths, vec![before.paths[0].clone()]);
}

#[test]
fn path_index_is_relisted_after_deletion_and_never_falls_back() {
    let mut s = duplicate_paths(8);
    let second = doc(&s).paths[1].clone();
    s.execute("path.delete", json!({"pathIndex":0})).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert_eq!(doc(&s).paths, vec![second.clone()]);
    assert!(s.execute("path.delete", json!({"pathIndex":1})).is_err());
    assert_eq!(doc(&s), before.as_ref());
    let list = s.execute("path.list", json!({})).unwrap();
    assert_eq!(list["paths"].as_array().unwrap().len(), 1);
    assert_eq!(list["paths"][0]["pathIndex"], 0);
    let info = s.execute("path.info", json!({"pathIndex":0})).unwrap();
    assert_eq!(info["path"], path_json(&second.path));
    s.execute("path.delete", json!({"pathIndex":0})).unwrap();
    assert!(s.execute("path.info", json!({"pathIndex":0})).is_err());
    assert_eq!(doc(&s).work_path, before.work_path);
}

#[test]
fn path_index_set_returns_the_second_paths_geometry_and_can_append() {
    let mut s = duplicate_paths(8);
    let before = s.active().unwrap().doc.clone();
    let replacement = vector::shapes::rect(65.0, 15.0, 10.0, 10.0);
    let result = s.execute("path.set", json!({"pathIndex":1,"path":path_json(&replacement)})).unwrap();
    assert_eq!(result["path"], path_json(&replacement));
    assert_eq!(result["name"], "??");
    assert_eq!(result["pathIndex"], 1);
    assert_eq!(doc(&s).paths[1].path, replacement);
    s.execute("path.set", json!({"pathIndex":1,"path":path_json(&replacement),"op":"subtract"})).unwrap();
    assert_eq!(doc(&s).paths[1].path.subpaths.len(), 2);
    assert_eq!(doc(&s).paths[1].path.subpaths[1].op, PathOp::Subtract);
    assert_eq!(doc(&s).paths[0], before.paths[0]);
    assert_eq!(doc(&s).work_path, before.work_path);
}

#[test]
fn path_index_selection_uses_second_geometry_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = duplicate_paths(depth);
        s.execute("path.toSelection", json!({"pathIndex":1,"antiAlias":false})).unwrap();
        let mask = selection_mask(&s);
        assert_eq!(mask[20 * 100 + 70], 1.0, "depth {depth}: second rectangle");
        assert_eq!(mask[15 * 100 + 20], 0.0, "depth {depth}: first triangle");
        assert_eq!(mask[70 * 100 + 20], 0.0, "depth {depth}: Work Path");
    }
}

#[test]
fn path_index_invalid_targets_leave_document_history_and_journal_unchanged() {
    let mut s = duplicate_paths(8);
    let geometry = path_json(&doc(&s).paths[1].path);
    let calls = [
        ("path.info", json!({})),
        ("path.set", json!({"path":geometry})),
        ("path.rename", json!({"to":"Right"})),
        ("path.delete", json!({})),
        ("path.transform", json!({"translateX":5})),
        ("path.toSelection", json!({})),
        ("path.addToSelection", json!({})),
        ("path.subtractFromSelection", json!({})),
        ("path.intersectWithSelection", json!({})),
        ("path.fill", json!({"color":"#ff0000"})),
        ("path.stroke", json!({"size":2})),
        ("path.toShape", json!({})),
        ("layer.vectorMask.add", json!({})),
        ("layer.vectorMask.fromPath", json!({})),
        ("layer.vectorMask.currentPath", json!({})),
        ("path.moveAnchors", json!({"anchors":[[0,0]],"move":[1,1]})),
        ("path.moveHandle", json!({"subpath":0,"knot":0,"handle":"out","to":[65,5]})),
        ("path.bendSegment", json!({"subpath":0,"knot":0,"move":[0,2]})),
        ("path.convertPoint", json!({"subpath":0,"knot":0})),
        ("path.addAnchor", json!({"subpath":0,"knot":0,"t":0.5})),
        ("path.deleteAnchor", json!({"subpath":0,"knot":0})),
        ("path.deleteSegment", json!({"subpath":0,"knot":0})),
        ("path.mergeShapeComponents", json!({})),
        ("path.setOperation", json!({"subpaths":[0],"op":"subtract"})),
    ];
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let journal = s.journal.clone();
    for (cmd, params) in calls {
        for index in [json!(-1), json!(0.5), json!("1"), Value::Null, json!(true), json!(2), json!(u64::MAX)] {
            let mut params = params.clone();
            params["pathIndex"] = index;
            let error = s.execute(cmd, params.clone()).unwrap_err();
            assert!(error.to_string().contains("pathIndex"), "{cmd} {params}: {error}");
            assert_eq!(doc(&s), before.as_ref(), "{cmd} {params}");
            assert_eq!(s.active().unwrap().history.past_len(), history);
            assert_eq!(s.journal, journal);
        }
    }
}

#[test]
fn path_index_rejects_conflicting_sources_before_editing() {
    let mut s = duplicate_paths(8);
    let before = s.active().unwrap().doc.clone();
    let layer = s.active().unwrap().active_layer.unwrap().0;
    for (cmd, p) in [
        ("path.set", json!({"name":"??","path":path_json(&before.paths[1].path)})),
        ("path.info", json!({"name":null})),
        ("path.rename", json!({"name":"work","to":"Other"})),
        ("path.delete", json!({"name":"??"})),
        ("path.transform", json!({"layer":layer,"translateX":5})),
        ("path.transform", json!({"vectorMask":true,"translateX":5})),
        ("path.moveAnchors", json!({"typePath":true,"anchors":[[0,0]],"move":[1,1]})),
        ("path.toShape", json!({"path":path_json(&before.paths[0].path)})),
        ("path.toShape", json!({"consumeWorkPath":true})),
        ("layer.vectorMask.add", json!({"layer":layer,"path":path_json(&before.paths[0].path)})),
        ("path.clippingPath.set", json!({"name":"??"})),
    ] {
        let mut p = p;
        p["pathIndex"] = json!(1);
        let history = s.active().unwrap().history.past_len();
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
        assert_eq!(doc(&s), before.as_ref());
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn path_index_transform_and_point_edits_keep_the_first_and_work_paths() {
    for (cmd, p) in [
        ("path.transform", json!({"translateX":5})),
        ("path.transform", json!({"matrix":[1,0,0,1,5,0]})),
        ("path.transform", json!({"quad":[[60,10],[85,10],[80,35],[60,30]]})),
        ("path.moveAnchors", json!({"anchors":[[0,0]],"move":[1,1]})),
        ("path.moveHandle", json!({"subpath":0,"knot":0,"handle":"out","to":[65,5]})),
        ("path.bendSegment", json!({"subpath":0,"knot":0,"move":[0,2]})),
        ("path.convertPoint", json!({"subpath":0,"knot":0,"out":[65,5]})),
        ("path.addAnchor", json!({"subpath":0,"knot":0,"t":0.5})),
        ("path.deleteAnchor", json!({"subpath":0,"knot":0})),
        ("path.deleteSegment", json!({"subpath":0,"knot":0})),
    ] {
        let mut s = duplicate_paths(8);
        let before = s.active().unwrap().doc.clone();
        let mut p = p;
        p["pathIndex"] = json!(1);
        let result = s.execute(cmd, p.clone()).unwrap();
        assert_eq!(result["pathIndex"], 1, "{cmd}");
        assert_eq!(result["name"], "??", "{cmd}");
        assert_ne!(doc(&s).paths[1].path, before.paths[1].path, "{cmd} {p}");
        assert_eq!(doc(&s).paths[0], before.paths[0]);
        assert_eq!(doc(&s).work_path, before.work_path);
        assert!(s.undo());
        assert_eq!(doc(&s), before.as_ref());
    }
}

#[test]
fn path_index_saved_path_can_lose_its_last_anchor() {
    let mut s = duplicate_paths(8);
    s.execute("path.set", json!({"pathIndex":1,"path":{"subpaths":[{"knots":[[70,20]]}]}})).unwrap();
    s.execute("path.deleteAnchor", json!({"pathIndex":1,"subpath":0,"knot":0})).unwrap();
    assert_eq!(doc(&s).paths.len(), 2);
    assert!(doc(&s).paths[1].path.is_empty());
    assert!(!doc(&s).paths[0].path.is_empty());
    assert!(s.undo());
    assert_eq!(doc(&s).paths[1].path.subpaths[0].knots.len(), 1);
}

#[test]
fn path_index_does_not_interpret_a_saved_name_as_work_or_layer() {
    for name in ["work", "Work Path", "layer", ""] {
        let mut s = duplicate_paths(8);
        Arc::make_mut(&mut s.active_mut().unwrap().doc).paths[1].name = name.into();
        let before = s.active().unwrap().doc.clone();
        let result = s.execute("path.moveAnchors", json!({"pathIndex":1,"anchors":[[0,0]],"move":[5,0]})).unwrap();
        assert_eq!(result["name"], name);
        assert_eq!(doc(&s).paths[1].path.subpaths[0].knots[0].anchor.x, 65.0);
        assert_eq!(doc(&s).work_path, before.work_path);
        s.execute("path.rename", json!({"pathIndex":1,"to":"Right"})).unwrap();
        assert_eq!(doc(&s).paths.len(), 2);
        assert_eq!(doc(&s).paths[1].name, "Right");
        assert_eq!(doc(&s).paths[0], before.paths[0]);
    }
}

#[test]
fn path_index_to_shape_and_vector_mask_copy_only_the_second_path() {
    for depth in [8, 16, 32] {
        for cmd in ["path.toShape", "layer.vectorMask.add", "layer.vectorMask.fromPath", "layer.vectorMask.currentPath"] {
            let mut s = duplicate_paths(depth);
            let before = s.active().unwrap().doc.clone();
            let layer = s.active().unwrap().active_layer.unwrap().0;
            let mut params = json!({"pathIndex":1});
            if cmd != "path.toShape" {
                // The layer parameter is a destination; another active layer must stay untouched.
                s.execute("layer.new.layer", json!({"name":"Other"})).unwrap();
                params["layer"] = json!(layer);
            }
            let result = s.execute(cmd, params).unwrap();
            let out = doc(&s).layer(LayerId(result["layer"].as_u64().unwrap())).unwrap();
            let path = match &out.content {
                LayerContent::Shape(sh) => &sh.path,
                _ => &out.vector_mask.as_ref().unwrap().path,
            };
            assert_eq!(path, &before.paths[1].path, "{cmd}, depth {depth}");
            assert_eq!(doc(&s).paths, before.paths);
            assert_eq!(doc(&s).work_path, before.work_path);
        }
    }
}

#[test]
fn path_index_fill_and_stroke_allow_a_destination_layer_at_every_depth() {
    for depth in [8, 16, 32] {
        for cmd in ["path.fill", "path.stroke"] {
            let mut s = duplicate_paths(depth);
            let before = s.active().unwrap().doc.clone();
            let destination = s.active().unwrap().active_layer.unwrap();
            let other = s.execute("layer.new.layer", json!({"name":"Other"})).unwrap()["layer"].as_u64().unwrap();
            let other_before = doc(&s).layer(LayerId(other)).unwrap().clone();
            s.execute(cmd, json!({"pathIndex":1,"layer":destination.0,"color":"#ff0000","size":4,"hardness":1,"opacity":100})).unwrap();
            let surface = doc(&s).layer(destination).unwrap().surface().unwrap();
            let rgba = |x, y| photocraft_raster::to_rgba(&surface.format(), &surface.pixel(x, y));
            let y = if cmd == "path.fill" { 20 } else { 10 };
            assert!(rgba(70, y)[1] < 0.1, "{cmd}, depth {depth}: second path is red");
            assert!(rgba(20, 15)[1] > 0.9, "first path stays white");
            assert!(rgba(20, 60)[1] > 0.9, "Work Path stays white");
            assert_eq!(doc(&s).layer(LayerId(other)).unwrap(), &other_before);
            assert_eq!(doc(&s).paths, before.paths);
            assert_eq!(doc(&s).work_path, before.work_path);
        }
    }
}

#[test]
fn path_index_clipping_reference_is_updated_only_when_unambiguous() {
    let mut s = duplicate_paths(8);
    s.execute("path.clippingPath.set", json!({"name":"??"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    for cmd in ["path.rename", "path.delete"] {
        let e = s.execute(cmd, json!({"pathIndex":1,"to":"Right"})).unwrap_err();
        assert!(e.to_string().contains("clipping path"));
        assert_eq!(doc(&s), before.as_ref());
    }
    s.execute("path.clippingPath.clear", json!({})).unwrap();
    s.execute("path.rename", json!({"pathIndex":1,"to":"Right"})).unwrap();
    s.execute("path.clippingPath.set", json!({"name":"Right"})).unwrap();
    s.execute("path.rename", json!({"pathIndex":1,"to":"Outline"})).unwrap();
    assert_eq!(doc(&s).clipping_path.as_ref().unwrap().name, "Outline");
    s.execute("path.delete", json!({"pathIndex":1})).unwrap();
    assert!(doc(&s).clipping_path.is_none());
    assert_eq!(doc(&s).paths, vec![before.paths[0].clone()]);
}

#[test]
fn path_index_actions_record_positions_and_refuse_lossy_atn_export() {
    let mut s = duplicate_paths(8);
    let before = doc(&s).paths.clone();
    s.execute("actions.record", json!({"name":"Rename second"})).unwrap();
    s.execute("path.rename", json!({"pathIndex":1,"to":"Right"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action":"Rename second"})).unwrap()["steps"].clone();
    assert_eq!(steps[0], json!(["path.rename",{"pathIndex":1,"to":"Right"}]));
    s.execute("file.new", json!({"width":100,"height":100})).unwrap();
    let d = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    d.paths = before;
    d.paths[0].name = "Alpha".into();
    d.paths[1].name = "Beta".into();
    let played = s.execute("actions.play", json!({"action":"Rename second"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(doc(&s).paths[0].name, "Alpha");
    assert_eq!(doc(&s).paths[1].name, "Right", "replay targets a position, not the old display name");

    for (cmd, p) in [
        ("path.set", json!({"pathIndex":1,"path":path_json(&doc(&s).paths[1].path)})),
        ("path.rename", json!({"pathIndex":1,"to":"Right"})),
        ("layer.vectorMask.fromPath", json!({"pathIndex":1})),
        ("layer.vectorMask.currentPath", json!({"pathIndex":1})),
        ("layer.vectorMask.add", json!({"pathIndex":1})),
    ] {
        let action = crate::actions_cmds::Action::new("Indexed", vec![craft::automation::actions::ActionStep::new(cmd, p)]);
        assert!(crate::atn_cmds::export("Test", &[&action], &[], false).is_err(), "{cmd}: strict export must refuse");
        let (_, skipped) = crate::atn_cmds::export("Test", &[&action], &[], true).unwrap();
        assert_eq!(skipped.len(), 1, "{cmd}: explicit skip mode reports the loss");
    }
}

#[test]
fn path_index_ui_consumers_capture_the_second_path() {
    let mut s = duplicate_paths(8);
    let expected = doc(&s).paths[1].path.clone();
    s.actions.list.push(crate::actions_cmds::Action::new("Capture", vec![])).unwrap();
    s.execute("actions.insertPath", json!({"action":"Capture","pathIndex":1})).unwrap();
    assert_eq!(s.actions.list.actions().last().unwrap().steps[0].params["path"], path_json(&expected));
    s.execute("paint.symmetryFromPath", json!({"pathIndex":1})).unwrap();
    let Some(crate::symmetry_cmds::PaintingSymmetry::Path(axis)) = &s.active().unwrap().symmetry else { panic!("path symmetry") };
    assert!(axis.segments.iter().flatten().all(|point| point[0] >= 60.0));
    for command in ["actions.insertPath", "paint.symmetryFromPath"] {
        for index in [json!(-1), json!(2), json!("1"), Value::Null] {
            assert!(s.execute(command, json!({"action":"Capture","pathIndex":index})).is_err());
        }
        assert!(s.execute(command, json!({"action":"Capture","pathIndex":1,"name":"??"})).is_err());
    }
    assert_eq!(s.actions.list.actions().last().unwrap().steps.len(), 1);
    s.execute("edit.defineCustomShape", json!({"name":"Right","path":path_json(&expected)})).unwrap();
    assert_eq!(s.edit_state.custom_shapes.last().unwrap().path, expected);
}
