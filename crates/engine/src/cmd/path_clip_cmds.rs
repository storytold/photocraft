//! Clipboard for path components: what Edit › Copy / Cut / Paste do with the Path Selection
//! tool in Photoshop when a path component is selected. `path.copy` and `path.cut` take the
//! addressed subpaths of a layer's shape path, a vector mask, the work path or a saved path
//! onto the path clipboard (`Session::path_clipboard`); `path.paste` lays the clipboard down
//! centred on the given point — appended to an addressed path (`"name"`, like Photoshop's
//! paste into the path selected in the Paths panel), else as a new saved path "Pasted Path".
//! Like one real clipboard, a path clip replaces a pixel clip and is replaced by one
//! (`s.clipboard` stays the pixel clipboard). The UI routes the Edit menu's Copy/Cut/Paste
//! here while Path Selection has a component selected; agents drive these commands directly.

use photocraft_doc::{LayerContent, NamedPath, Path};
use photocraft_geom::{Affine, Point};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::vector_cmds::{bad, has_doc, is_work, layer_id, op_from, path_json, pt};
use crate::{EngineError, Result, Session};

/// The pasted path's name, as in Photoshop's Paths panel.
const PASTED: &str = "Pasted Path";

/// The path `path.copy` / `path.cut` address: `"layer"` with `"layer"`:id? (a shape layer's
/// path or its vector mask), else the work path, else a saved path by name.
fn target_path(s: &Session, cmd: &str, p: &Value) -> Result<Path> {
    let name = p.get("name").and_then(Value::as_str).unwrap_or("work").to_owned();
    let layer = if name == "layer" { Some(layer_id(s, p)?) } else { None };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    match layer {
        Some(id) => {
            let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            match &l.content {
                LayerContent::Shape(sh) => Ok(sh.path.clone()),
                _ => l.vector_mask.as_ref().map(|m| m.path.clone()).ok_or_else(|| bad(cmd, "the layer has no shape path or vector mask")),
            }
        }
        None if is_work(Some(&name)) => d.doc.work_path.clone().ok_or_else(|| bad(cmd, "no work path")),
        None => d.doc.paths.iter().find(|q| q.name == name).map(|q| q.path.clone()).ok_or_else(|| bad(cmd, format!("no path named \"{name}\""))),
    }
}

/// `p`'s `subpaths` param: a non-empty list of distinct indices addressing `path`.
fn subpath_indices(cmd: &str, p: &Value, path: &Path) -> Result<Vec<usize>> {
    let list = p.get("subpaths").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or_else(|| bad(cmd, "`subpaths` must list subpath indices"))?;
    let mut out: Vec<usize> = Vec::with_capacity(list.len());
    for v in list {
        let i = v.as_u64().and_then(|u| usize::try_from(u).ok()).ok_or_else(|| bad(cmd, "each `subpaths` entry is a subpath index"))?;
        if path.subpaths.get(i).is_none() {
            return Err(bad(cmd, format!("no subpath {i}")));
        }
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// The clipboard's new content: the addressed subpaths in the given order.
fn picked(path: &Path, indices: &[usize]) -> Path {
    Path { subpaths: indices.iter().filter_map(|&i| path.subpaths.get(i).cloned()).collect(), fill_rule: path.fill_rule, inverted: false }
}

/// Edit › Copy for a path component: the addressed subpaths become the path clipboard, like
/// Photoshop's; the document is untouched and the pixel clipboard is replaced.
fn path_copy(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.copy";
    let path = target_path(s, CMD, p)?;
    let indices = subpath_indices(CMD, p, &path)?;
    let subpaths = picked(&path, &indices);
    if subpaths.subpaths.is_empty() {
        return Err(bad(CMD, "no subpaths to copy"));
    }
    s.path_clipboard = Some(subpaths);
    s.clipboard = None;
    Ok(json!({ "copied": indices.len() }))
}

/// Edit › Cut: like [`path_copy`], and the subpaths come out of the target path in one
/// undoable step (a shape layer re-renders, a live shape stops being live). The clipboard
/// only changes when the cut succeeded: a refused edit leaves the previous clip alone.
fn path_cut(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.cut";
    let path = target_path(s, CMD, p)?;
    let indices = subpath_indices(CMD, p, &path)?;
    let subpaths = picked(&path, &indices);
    if subpaths.subpaths.is_empty() {
        return Err(bad(CMD, "no subpaths to cut"));
    }
    crate::path_edit_cmds::edit_path(s, CMD, p, "Cut", |path| {
        let mut rm = indices.clone();
        rm.sort_unstable();
        rm.dedup();
        for &i in rm.iter().rev() {
            if i < path.subpaths.len() {
                path.subpaths.remove(i);
            }
        }
        Ok(())
    })?;
    // One clipboard: a path clip replaces the pixel clip (as a pixel clip replaces this).
    s.path_clipboard = Some(subpaths);
    s.clipboard = None;
    Ok(json!({ "cut": indices.len() }))
}

/// The paste centre: the `center` param, else the canvas centre. Magnitude-bounded like
/// `path.transform`'s coordinates.
fn paste_center(cmd: &str, s: &Session, p: &Value) -> Result<Point> {
    let at = |q: Point| q.x.is_finite() && q.y.is_finite() && q.x.abs() <= 1_000_000.0 && q.y.abs() <= 1_000_000.0;
    if let Some(v) = p.get("center") {
        return pt(v).filter(|q| at(*q)).ok_or_else(|| bad(cmd, "`center` must be [x, y] with finite numbers of magnitude at most 1000000"));
    }
    let b = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    Ok(Point::new(f64::from(b.x0 + b.x1) / 2.0, f64::from(b.y0 + b.y1) / 2.0))
}

/// Edit › Paste: with `"name"` (and `"layer"`), the path clipboard's components append to the
/// addressed path — the work path, a saved path, a shape layer's path or its vector mask — in
/// one undoable step, as Photoshop pastes into the path selected in the Paths panel; without
/// a name the clipboard becomes a new saved path "Pasted Path" (one replaces an earlier one,
/// as `path.set` replaces by name). Either way the path is translated so its bounds centre
/// lands on the paste centre: Photoshop centres the pasted path on the document window; the
/// UI passes the view centre, the default is the canvas centre.
///
/// The position and scale rule (review on #10482): path components are resolution-independent
/// vectors, so pasting into a document with a different resolution — or any other document —
/// keeps their scale exactly (their print size follows the target's dpi, their geometry is
/// untouched); position follows the paste centre like Photoshop's Paste, and only a future
/// Paste In Place would keep the original coordinates.
fn path_paste(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.paste";
    let clip = s.path_clipboard.clone().ok_or_else(|| bad(CMD, "the clipboard is empty"))?;
    let c = paste_center(CMD, s, p)?;
    let (x0, y0, x1, y1) = clip.control_bounds().ok_or_else(|| bad(CMD, "the clipboard path has no points"))?;
    let (dx, dy) = (c.x - (x0 + x1) / 2.0, c.y - (y0 + y1) / 2.0);
    if !dx.is_finite() || !dy.is_finite() || dx.abs() > 1_000_000.0 || dy.abs() > 1_000_000.0 {
        return Err(bad(CMD, "the paste centre is too far from the path"));
    }
    let mut moved = clip.transform(&Affine::translate(dx, dy));
    if p.get("name").is_some() {
        // A targeted paste: the components append to the addressed path, with `op` overriding
        // their shape mode (like `path.set`'s append).
        if let Some(o) = p.get("op") {
            let op = o.as_str().and_then(op_from).ok_or_else(|| bad(CMD, "unknown `op`"))?;
            for sp in &mut moved.subpaths {
                sp.op = op;
            }
        }
        return crate::path_edit_cmds::edit_path(s, CMD, p, "Paste", |path| {
            path.subpaths.extend(moved.subpaths.iter().cloned());
            Ok(())
        });
    }
    s.edit("Paste", |doc, _| {
        doc.paths.retain(|q| q.name != PASTED);
        doc.paths.push(NamedPath { name: PASTED.into(), path: moved.clone(), psd_raw: None });
        Ok(())
    })?;
    Ok(json!({ "name": PASTED, "path": path_json(&moved) }))
}

const TARGET: &str = r#""name":"work"|"layer"="work","layer":id? (with "layer": a shape layer's path or a vector mask),"subpaths":[i,…]"#;

/// The shared shape of the three clipboard commands' specs: id, label, params doc, whether the
/// command journals a history step, and its engine function.
fn spec(id: &'static str, label: &'static str, params: &'static str, journal: bool, run: fn(&mut Session, &Value) -> Result<Value>) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: has_doc, run, journal }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec(
            "path.copy",
            "Copy Path Components",
            crate::vector_cmds::leak(format!("{{{TARGET}}} → {{copied}} (the addressed subpaths become the path clipboard, replacing a pixel clipboard)")),
            // Journalled like edit.copy: an action recording replays the copy that a later
            // path.paste in the same recording relies on.
            true,
            path_copy,
        ),
        spec(
            "path.cut",
            "Cut Path Components",
            crate::vector_cmds::leak(format!("{{{TARGET}}} → {{cut}} (like path.copy, and the subpaths leave the target path in one undoable step)")),
            true,
            path_cut,
        ),
        spec(
            "path.paste",
            "Paste Path",
            r#"{"center":[x,y]? (document coordinates the pasted path's bounds centre lands on; default: the canvas centre),"name":"work"|"layer"|saved-name?,"layer":id? (with "layer": the layer's shape path or vector mask),"op":"combine"|"subtract"|"intersect"|"exclude"|"join"? (the appended components' shape mode)} → {name,path} (with a name: the components append to the addressed path in one step; without: the clipboard becomes a saved path "Pasted Path", replacing an earlier one)"#,
            true,
            path_paste,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use photocraft_doc::LayerContent;
    use serde_json::json;

    use crate::Session;

    /// A session with a fresh 200×100 white document.
    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
        s
    }

    /// A shape layer with two disjoint rectangles (like the Path Selection tests draw).
    fn two_rects(s: &mut Session) -> u64 {
        let id = s.execute("shape.create", json!({"kind": "rect", "rect": [20, 20, 40, 40], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
        let k = |x: f64, y: f64| json!({"anchor": [x, y]});
        let two = json!({"subpaths": [
            {"closed": true, "knots": [k(20.0, 20.0), k(60.0, 20.0), k(60.0, 60.0), k(20.0, 60.0)]},
            {"closed": true, "knots": [k(100.0, 20.0), k(140.0, 20.0), k(140.0, 60.0), k(100.0, 60.0)]},
        ]});
        s.execute("shape.edit", json!({"layer": id, "path": two})).unwrap();
        id
    }

    /// The addressed shape layer's path.
    fn shape_path(s: &Session, id: u64) -> photocraft_doc::Path {
        let l = s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap();
        let LayerContent::Shape(sh) = &l.content else { panic!("not a shape") };
        sh.path.clone()
    }

    #[test]
    fn copy_takes_the_addressed_subpaths_and_replaces_the_pixel_clipboard() {
        let mut s = session();
        let id = two_rects(&mut s);
        let before = shape_path(&s, id);
        s.execute("edit.copy", json!({})).unwrap();
        assert!(s.clipboard.is_some());
        let r = s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [1]})).unwrap();
        assert_eq!(r["copied"], json!(1));
        assert_eq!(shape_path(&s, id), before, "copy leaves the document alone");
        assert!(s.clipboard.is_none(), "a path clip replaces the pixel clip");
        let clip = s.path_clipboard.as_ref().unwrap();
        assert_eq!(clip.subpaths.len(), 1);
        assert_eq!(clip.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(100.0, 20.0));
    }

    #[test]
    fn cut_removes_the_subpaths_in_one_undoable_step() {
        let mut s = session();
        let id = two_rects(&mut s);
        let r = s.execute("path.cut", json!({"name": "layer", "layer": id, "subpaths": [1]})).unwrap();
        assert_eq!(r["cut"], json!(1));
        let path = shape_path(&s, id);
        assert_eq!(path.subpaths.len(), 1);
        assert_eq!(path.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(20.0, 20.0), "subpath 0 stays");
        let clip = s.path_clipboard.as_ref().unwrap();
        assert_eq!(clip.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(100.0, 20.0));
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Cut"));
        assert!(s.undo());
        assert_eq!(shape_path(&s, id).subpaths.len(), 2, "undo restores the cut component");
    }

    #[test]
    fn paste_creates_pasted_path_centered_on_the_canvas_or_the_given_center() {
        let mut s = session();
        let id = two_rects(&mut s);
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [1]})).unwrap();
        // Canvas centre of the 200×100 document: (100, 50); the copied bounds centre is (120, 40).
        let r = s.execute("path.paste", json!({})).unwrap();
        assert_eq!(r["name"], json!("Pasted Path"));
        let doc = &s.active().unwrap().doc;
        let pasted = doc.paths.iter().find(|q| q.name == "Pasted Path").expect("pasted path exists");
        assert_eq!(pasted.path.subpaths.len(), 1);
        assert_eq!(pasted.path.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(80.0, 30.0), "moved by (-20, +10)");
        // An explicit centre lands there exactly; a second paste replaces the first.
        s.execute("path.paste", json!({"center": [150.0, 70.0]})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.paths.iter().filter(|q| q.name == "Pasted Path").count(), 1);
        let pasted = doc.paths.iter().find(|q| q.name == "Pasted Path").unwrap();
        assert_eq!(pasted.path.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(130.0, 50.0), "moved by (+30, +10)");
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Paste"));
    }

    #[test]
    fn paste_into_the_work_path_appends_instead_of_creating_a_saved_path() {
        let mut s = session();
        let id = two_rects(&mut s);
        s.execute("path.set", json!({"path": {"subpaths": [{"closed": true, "knots": [[150, 20], [190, 20], [190, 60], [150, 60]]}]}})).unwrap();
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [1]})).unwrap();
        let r = s.execute("path.paste", json!({"name": "work", "center": [100, 50]})).unwrap();
        assert_eq!(r["name"], json!("work"));
        let doc = &s.active().unwrap().doc;
        assert!(doc.paths.is_empty(), "a targeted paste creates no saved path");
        let work = doc.work_path.as_ref().expect("the work path");
        assert_eq!(work.subpaths.len(), 2, "the component appended to the work path");
        // The copied bounds centre (120, 40) lands on (100, 50); anchor (100, 20) follows.
        assert_eq!(work.subpaths[1].knots[0].anchor, photocraft_geom::Point::new(80.0, 30.0));
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Paste"));
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.work_path.as_ref().unwrap().subpaths.len(), 1, "undo takes the pasted component out again");
    }

    #[test]
    fn paste_into_a_saved_path_appends_and_op_overrides_the_components_mode() {
        let mut s = session();
        let id = two_rects(&mut s);
        s.execute("path.set", json!({"name": "Sister", "path": {"subpaths": [{"closed": true, "knots": [[150, 20], [190, 20], [190, 60], [150, 60]]}]}}))
            .unwrap();
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [1]})).unwrap();
        let r = s.execute("path.paste", json!({"name": "Sister", "center": [100, 50]})).unwrap();
        assert_eq!(r["name"], json!("Sister"));
        let saved = s.active().unwrap().doc.paths.iter().find(|q| q.name == "Sister").unwrap();
        assert_eq!(saved.path.subpaths.len(), 2, "the component appended to the saved path");
        assert_eq!(saved.path.subpaths[1].knots[0].anchor, photocraft_geom::Point::new(80.0, 30.0));
        assert_eq!(saved.path.subpaths[1].op, photocraft_doc::vector::PathOp::Combine, "the copied component keeps its mode");
        // `op` overrides the appended components' mode; one step undoes the paste.
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [0]})).unwrap();
        s.execute("path.paste", json!({"name": "Sister", "center": [100, 50], "op": "subtract"})).unwrap();
        let saved = s.active().unwrap().doc.paths.iter().find(|q| q.name == "Sister").unwrap();
        assert_eq!(saved.path.subpaths.len(), 3);
        assert_eq!(saved.path.subpaths[2].op, photocraft_doc::vector::PathOp::Subtract, "`op` sets the pasted components' mode");
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.paths.iter().find(|q| q.name == "Sister").unwrap().path.subpaths.len(), 2);
    }

    #[test]
    fn paste_into_a_shape_layer_appends_and_stops_it_being_live() {
        let mut s = session();
        // A fresh shape is a live shape.
        let id = s.execute("shape.create", json!({"kind": "rect", "rect": [20, 20, 40, 40], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
        let live = {
            let l = s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap();
            matches!(&l.content, LayerContent::Shape(sh) if sh.live.is_some())
        };
        assert!(live, "shape.create produces a live shape");
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [0]})).unwrap();
        let r = s.execute("path.paste", json!({"name": "layer", "layer": id, "center": [150, 70]})).unwrap();
        assert_eq!(r["name"], json!("layer"));
        let l = s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap();
        let LayerContent::Shape(sh) = &l.content else { panic!("not a shape") };
        assert_eq!(sh.path.subpaths.len(), 2, "the component appended to the shape's path");
        // The copied bounds centre (40, 40) lands on (150, 70); anchor (20, 20) follows.
        assert_eq!(sh.path.subpaths[1].knots[0].anchor, photocraft_geom::Point::new(130.0, 50.0));
        assert!(sh.live.is_none(), "appending stops the shape being a live shape");
    }

    #[test]
    fn paste_into_a_target_rejects_missing_paths_and_bad_ops() {
        let mut s = session();
        let id = two_rects(&mut s);
        let bg = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.select", json!({"layer": bg})).unwrap();
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [0]})).unwrap();
        s.execute("path.set", json!({"name": "Sister", "path": {"subpaths": [{"closed": true, "knots": [[150, 20], [190, 20], [190, 60], [150, 60]]}]}}))
            .unwrap();
        let before = s.active().unwrap().doc.paths.iter().find(|q| q.name == "Sister").unwrap().path.clone();
        assert!(s.execute("path.paste", json!({"name": "nope"})).is_err(), "no saved path named nope");
        assert!(s.execute("path.paste", json!({"name": "layer"})).is_err(), "the active pixel layer has no path to paste into");
        assert!(s.execute("path.paste", json!({"name": "work"})).is_err(), "no work path");
        assert!(s.execute("path.paste", json!({"name": "Sister", "op": "bogus"})).is_err(), "unknown op");
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.paths.iter().find(|q| q.name == "Sister").unwrap().path, before, "a refused paste touches no path");
        assert!(doc.work_path.is_none());
        assert!(s.path_clipboard.is_some(), "the clipboard survives the refusal");
    }

    #[test]
    fn bad_params_fail_without_touching_anything() {
        let mut s = session();
        let id = two_rects(&mut s);
        let before = shape_path(&s, id);
        for params in [
            json!({"name": "layer", "layer": id}),
            json!({"name": "layer", "layer": id, "subpaths": []}),
            json!({"name": "layer", "layer": id, "subpaths": [9]}),
            json!({"name": "layer", "layer": id, "subpaths": [-1]}),
            json!({"name": "layer", "layer": id, "subpaths": ["1"]}),
            json!({"name": "nope", "subpaths": [0]}),
            json!({"subpaths": [0]}),
        ] {
            assert!(s.execute("path.copy", params.clone()).is_err(), "{params}");
            assert!(s.execute("path.cut", params).is_err());
        }
        assert!(s.path_clipboard.is_none(), "a refused copy leaves the clipboard alone");
        assert_eq!(shape_path(&s, id), before, "a refused cut leaves the path alone");
        assert!(s.execute("path.paste", json!({})).is_err(), "nothing on the path clipboard");
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [0]})).unwrap();
        assert!(s.execute("path.paste", json!({"center": [1e300, 0.0]})).is_err(), "centre out of range");
        assert!(s.execute("path.paste", json!({"center": "here"})).is_err());
    }

    #[test]
    fn copy_is_journaled_for_action_recordings() {
        let mut s = session();
        let id = two_rects(&mut s);
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [0]})).unwrap();
        assert_eq!(s.journal.last().map(|(id, _)| id.as_str()), Some("path.copy"), "an action recording replays the copy a later paste relies on");
    }

    /// A recorded path.copy replays and populates the clipboard a later paste relies on: the
    /// journal flag protects the recording, this protects the playback.
    #[test]
    fn a_recorded_copy_replays_and_pastes() {
        let rect = json!({"subpaths": [{"closed": true, "knots": [[100, 20], [140, 20], [140, 60], [100, 60]]}]});
        let mut s = session();
        s.execute("path.set", json!({"name": "work", "path": rect})).unwrap();
        s.execute("actions.record", json!({"name": "Copy component"})).unwrap();
        s.execute("path.copy", json!({"subpaths": [0]})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        let action = s.actions.list[0].clone();
        // A fresh session replays the recording (name-based target)…
        let mut fresh = session();
        fresh.execute("path.set", json!({"name": "work", "path": rect})).unwrap();
        fresh.actions.list.push(action);
        let played = fresh.execute("actions.play", json!({"action": "Copy component"})).unwrap();
        assert!(played.get("failed").is_none(), "{played}");
        let clip = fresh.path_clipboard.as_ref().expect("the replayed copy populated the path clipboard");
        assert_eq!(clip.subpaths.len(), 1);
        assert_eq!(clip.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(100.0, 20.0));
        // …and the clip it left pastes.
        let r = fresh.execute("path.paste", json!({"center": [100, 50]})).unwrap();
        assert_eq!(r["name"], json!("Pasted Path"));
    }

    /// Paste into a document at a different resolution keeps the components' scale: the session
    /// clipboard is shared across documents, and path components are resolution-independent
    /// vectors — translated to the paste centre, never scaled.
    #[test]
    fn paste_into_another_document_keeps_the_components_scale() {
        let mut s = session();
        let lo = s.active().unwrap().doc.resolution_dpi;
        let id = two_rects(&mut s);
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [1]})).unwrap();
        // Another document at a different resolution: the session clipboard is shared, and the
        // components keep their scale (paths are resolution-independent vectors) while their
        // bounds centre (120, 40) lands on the paste centre, unscaled.
        s.execute("file.new", json!({"width": 400, "height": 300, "resolution": 300})).unwrap();
        let hi = s.active().unwrap().doc.resolution_dpi;
        assert_ne!(lo, hi, "the two documents differ in resolution");
        let r = s.execute("path.paste", json!({"center": [200, 150]})).unwrap();
        assert_eq!(r["name"], json!("Pasted Path"));
        let doc = &s.active().unwrap().doc;
        let pasted = doc.paths.iter().find(|q| q.name == "Pasted Path").expect("pasted into the active document");
        let anchors: Vec<_> = pasted.path.subpaths[0].knots.iter().map(|k| k.anchor).collect();
        assert_eq!(
            anchors,
            vec![
                photocraft_geom::Point::new(180.0, 130.0),
                photocraft_geom::Point::new(220.0, 130.0),
                photocraft_geom::Point::new(220.0, 170.0),
                photocraft_geom::Point::new(180.0, 170.0),
            ],
            "every knot translated by (+80, +110), not scaled"
        );
    }

    #[test]
    fn a_channel_copy_replaces_the_path_clipboard() {
        let mut s = session();
        let id = two_rects(&mut s);
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [0]})).unwrap();
        assert!(s.path_clipboard.is_some());
        // Copying a colour channel produces a pixel clip on the one clipboard.
        s.execute("channel.target", json!({"channel": "red"})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        assert!(s.path_clipboard.is_none(), "a channel copy replaces the path clip");
        assert!(s.clipboard.is_some());
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    }

    #[test]
    fn a_refused_cut_keeps_the_previous_clipboard() {
        let mut s = session();
        let id = two_rects(&mut s);
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [0]})).unwrap();
        let before = s.path_clipboard.clone();
        assert!(before.is_some() && s.clipboard.is_none());
        // No such subpath: the cut fails (as does a locked target) and must leave both
        // clipboards untouched — the clipboard only changes when the cut succeeded.
        assert!(s.execute("path.cut", json!({"name": "layer", "layer": id, "subpaths": [9]})).is_err());
        assert_eq!(s.path_clipboard, before, "the refused cut kept the path clipboard");
        assert!(s.clipboard.is_none());
        assert_eq!(shape_path(&s, id).subpaths.len(), 2, "the refused cut kept the path");
    }

    #[test]
    fn a_pixel_copy_replaces_the_path_clipboard() {
        let mut s = session();
        let id = two_rects(&mut s);
        s.execute("path.copy", json!({"name": "layer", "layer": id, "subpaths": [0]})).unwrap();
        assert!(s.path_clipboard.is_some());
        s.execute("edit.copy", json!({})).unwrap();
        assert!(s.path_clipboard.is_none(), "a pixel clip replaces the path clip");
        assert!(s.clipboard.is_some());
    }
}
