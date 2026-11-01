//! Revision markers: red boxes over content in a document (typically a manual / 说明书) that
//! still needs to be changed. A marker saves a snapshot of the framed pixels; `revision.detect`
//! re-renders the frame and turns markers green when their content changed, so nothing marked for
//! revision is missed. Markers can be bound to a layer (`layerId`): the box then follows the
//! layer when it moves with the Move tool or `layer.translate`.
//!
//! Markers are document data ([`photocraft_doc::RevisionMarker`]), saved in `.pcraft` and as a
//! private PSD image resource; each change here is one history step.

use photocraft_doc::{Document, LayerId, RevisionMarker};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_markers(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| !d.doc.revision_markers.is_empty()) {
        Ok(())
    } else {
        Err("the document has no revision markers".into())
    }
}

pub fn marker_json(i: usize, m: &RevisionMarker) -> Value {
    json!({
        "index": i,
        "name": m.name,
        "x": m.pos[0],
        "y": m.pos[1],
        "width": m.size[0],
        "height": m.size[1],
        "layerId": m.layer_id.map(|id| id.0),
        "done": m.done,
        "note": m.note,
        "hasSnapshot": !m.snapshot.is_empty(),
    })
}

fn list_json(d: &Document) -> Value {
    json!({"markers": d.revision_markers.iter().enumerate().map(|(i, m)| marker_json(i, m)).collect::<Vec<_>>()})
}

fn index_of(s: &Session, p: &Value, cmd: &str) -> Result<usize> {
    let n = s.active().ok_or(EngineError::NoDocument)?.doc.revision_markers.len();
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "pass `index`"))? as usize;
    if i < n { Ok(i) } else { Err(bad(cmd, format!("no revision marker {i} (the document has {n})"))) }
}

fn f64_of(cmd: &str, p: &Value, key: &str) -> Result<Option<f64>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => {
            let f = v.as_f64().ok_or_else(|| bad(cmd, format!("`{key}` must be a number")))?;
            if !f.is_finite() {
                return Err(bad(cmd, format!("`{key}` must be finite")));
            }
            Ok(Some(f))
        }
    }
}

/// Render the framed area of `doc` to a snapshot grid as straight RGBA8 (row-major). The frame
/// is clamped to the canvas; a frame entirely outside it has no content and is an error.
fn snapshot_at(doc: &Document, pos: [f64; 2], size: [f64; 2], grid: [u32; 2], name: &str) -> Result<Vec<u8>> {
    let canvas = doc.bounds();
    let x0 = pos[0].floor().clamp(f64::from(canvas.x0), f64::from(canvas.x1)) as i32;
    let y0 = pos[1].floor().clamp(f64::from(canvas.y0), f64::from(canvas.y1)) as i32;
    let x1 = (pos[0] + size[0]).ceil().clamp(f64::from(canvas.x0), f64::from(canvas.x1)) as i32;
    let y1 = (pos[1] + size[1]).ceil().clamp(f64::from(canvas.y0), f64::from(canvas.y1)) as i32;
    let r = Rect::new(x0, y0, x1, y1);
    if r.is_empty() {
        return Err(EngineError::Other(format!("revision marker \"{name}\" is entirely outside the canvas")));
    }
    let full = photocraft_compose::composite_to_surface(doc, r, doc.pixel_format(), None);
    let (w, h) = (r.width() as usize, r.height() as usize);
    let mut rgba = vec![[0u8; 4]; w * h];
    full.read_rgba8_into(r, &mut rgba);
    let (gw, gh) = (grid[0] as usize, grid[1] as usize);
    if gw == 0 || gh == 0 || gw > 4096 || gh > 4096 {
        return Err(EngineError::Other(format!("revision marker \"{name}\" has an invalid snapshot size")));
    }
    Ok(box_resample(&rgba, w, h, gw, gh))
}

fn snapshot_for(doc: &Document, m: &RevisionMarker) -> Result<Vec<u8>> {
    snapshot_at(doc, m.pos, m.size, m.snapshot_size, &m.name)
}

/// Downsample `src` (RGBA8, `w×h`) to a `gw×gh` grid by area average (nearest pixel when
/// upscaling, which only happens for a frame smaller than the grid).
fn box_resample(src: &[[u8; 4]], w: usize, h: usize, gw: usize, gh: usize) -> Vec<u8> {
    let mut out = vec![0u8; gw * gh * 4];
    for gy in 0..gh {
        let sy0 = gy * h / gh;
        let sy1 = ((gy + 1) * h / gh).max(sy0 + 1).min(h);
        for gx in 0..gw {
            let sx0 = gx * w / gw;
            let sx1 = ((gx + 1) * w / gw).max(sx0 + 1).min(w);
            let mut sum = [0u64; 4];
            let mut n = 0u64;
            for y in sy0..sy1 {
                for x in sx0..sx1 {
                    let p = src[y * w + x];
                    sum[0] += u64::from(p[0]);
                    sum[1] += u64::from(p[1]);
                    sum[2] += u64::from(p[2]);
                    sum[3] += u64::from(p[3]);
                    n += 1;
                }
            }
            let i = (gy * gw + gx) * 4;
            for c in 0..4 {
                out[i + c] = (sum[c] / n) as u8;
            }
        }
    }
    out
}

/// Whether two snapshot grids differ enough to count as "content changed". Tolerates the small
/// differences a re-render or compression brings (a few % of a channel); a real edit — a word
/// replaced, a number corrected — changes whole cells and trips the 0.5% pixel threshold.
fn changed(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() || a.len() % 4 != 0 || a.is_empty() {
        return false;
    }
    let mut diff = 0u64;
    for i in (0..a.len()).step_by(4) {
        let dr = a[i].abs_diff(b[i]);
        let dg = a[i + 1].abs_diff(b[i + 1]);
        let db = a[i + 2].abs_diff(b[i + 2]);
        let da = a[i + 3].abs_diff(b[i + 3]);
        if dr.max(dg).max(db) > 24 || da > 48 {
            diff += 1;
        }
    }
    // ≥ 0.5% of the sampled cells differ.
    diff * 200 >= a.len() as u64 / 4
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "revision.add";
    let x = f64_of(CMD, p, "x")?.ok_or_else(|| bad(CMD, "pass `x` and `y`"))?;
    let y = f64_of(CMD, p, "y")?.ok_or_else(|| bad(CMD, "pass `x` and `y`"))?;
    let width = f64_of(CMD, p, "width")?.ok_or_else(|| bad(CMD, "pass `width` and `height`"))?;
    let height = f64_of(CMD, p, "height")?.ok_or_else(|| bad(CMD, "pass `width` and `height`"))?;
    if width <= 0.0 || height <= 0.0 || width > 1.0e7 || height > 1.0e7 {
        return Err(bad(CMD, "marker size must be positive and reasonable"));
    }
    let name = p.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let note = p.get("note").and_then(Value::as_str).unwrap_or("").to_string();
    let layer_id = match p.get("layerId") {
        None => None,
        Some(v) => {
            let id = v.as_u64().ok_or_else(|| bad(CMD, "`layerId` must be a layer id"))?;
            Some(LayerId(id))
        }
    };
    let i = s.edit("Add Revision Marker", |d, _| {
        let mut m = RevisionMarker {
            name,
            pos: [x, y],
            size: [width, height],
            layer_id,
            note,
            ..Default::default()
        };
        m.snapshot = snapshot_for(d, &m)?;
        d.revision_markers.push(m);
        Ok(d.revision_markers.len() - 1)
    })?;
    let d = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    Ok(json!({"index": i, "marker": marker_json(i, &d.revision_markers[i])}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "revision.remove";
    let i = index_of(s, p, CMD)?;
    s.edit("Delete Revision Marker", |d, _| {
        d.revision_markers.remove(i);
        Ok(())
    })?;
    Ok(Value::Null)
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "revision.set";
    let i = index_of(s, p, CMD)?;
    let d0 = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let mut m = d0.revision_markers[i].clone();
    if let Some(name) = p.get("name").and_then(Value::as_str) {
        m.name = name.to_string();
    }
    if let Some(note) = p.get("note").and_then(Value::as_str) {
        m.note = note.to_string();
    }
    let mut reframe = false;
    for (k, set) in [("x", 0usize), ("y", 1usize)] {
        if let Some(v) = f64_of(CMD, p, k)? {
            m.pos[set] = v;
            reframe = true;
        }
    }
    for (k, set) in [("width", 0usize), ("height", 1usize)] {
        if let Some(v) = f64_of(CMD, p, k)? {
            if v <= 0.0 || v > 1.0e7 {
                return Err(bad(CMD, "marker size must be positive and reasonable"));
            }
            m.size[set] = v;
            reframe = true;
        }
    }
    if let Some(v) = p.get("layerId") {
        m.layer_id = match v {
            Value::Null => None,
            _ => Some(LayerId(v.as_u64().ok_or_else(|| bad(CMD, "`layerId` must be a layer id or null"))?)),
        };
    }
    if m == d0.revision_markers[i] {
        return Ok(json!({"index": i, "marker": marker_json(i, &m), "changed": false}));
    }
    let label = if reframe { "Resize Revision Marker" } else { "Edit Revision Marker" };
    let out = marker_json(i, &m);
    s.edit(label, |d, _| {
        // A moved or resized frame must snapshot the area it now covers, or detection would
        // compare the new frame against pixels from the old one.
        if reframe {
            m.snapshot = snapshot_for(d, &m)?;
            m.done = false;
        }
        d.revision_markers[i] = m.clone();
        Ok(())
    })?;
    Ok(json!({"index": i, "marker": out, "changed": true}))
}

fn snapshot(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "revision.snapshot";
    // The whole document, or one marker by `index`.
    let one = p.get("index").and_then(Value::as_u64).map(|i| i as usize);
    let d0 = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    if let Some(i) = one
        && i >= d0.revision_markers.len()
    {
        return Err(bad(CMD, format!("no revision marker {i}")));
    }
    s.edit("Update Revision Snapshot", |d, _| {
        let indices: Vec<usize> = (0..d.revision_markers.len()).filter(|&i| one.is_none_or(|o| o == i)).collect();
        for i in indices {
            let m = d.revision_markers[i].clone();
            let snap = snapshot_for(d, &m)?;
            let m = &mut d.revision_markers[i];
            m.snapshot = snap;
            m.done = false;
        }
        Ok(())
    })?;
    Ok(list_json(&s.active().ok_or(EngineError::NoDocument)?.doc))
}

/// Topmost visible, non-background pixel layer whose content bounds contain `pt`, if any. Groups
/// (folders) are skipped: a group's bounds cover its children, and binding must track the actual
/// layer under the frame, not the folder.
fn layer_at_point(doc: &Document, pt: [f64; 2]) -> Option<LayerId> {
    let canvas = doc.bounds();
    let (x, y) = (pt[0].round() as i32, pt[1].round() as i32);
    for (_, _, layer) in doc.walk().into_iter().rev() {
        if doc.is_background(layer.id) || !layer.visible || layer.is_group() {
            continue;
        }
        let b = photocraft_compose::layer_bounds(layer, canvas);
        if !b.is_empty() && b.contains(x, y) {
            return Some(layer.id);
        }
    }
    None
}

/// Re-bind markers whose bound layer was deleted: the frame re-tracks the topmost visible,
/// non-background layer under its centre (the replacement layer), or is unbound when nothing is
/// there. Returns whether anything changed. Runs at the start of `revision.detect`, so deleting a
/// bound layer and placing its replacement re-binds automatically.
fn repair_bindings(d: &mut Document) -> bool {
    // Collect the frames whose bound layer is gone first, so the centre lookups below do not
    // clash with the mutable borrow of the marker list.
    let stale: Vec<(usize, [f64; 2])> = d
        .revision_markers
        .iter()
        .enumerate()
        .filter(|(_, m)| m.layer_id.is_some_and(|id| d.layer(id).is_none()))
        .map(|(i, m)| (i, [m.pos[0] + m.size[0] / 2.0, m.pos[1] + m.size[1] / 2.0]))
        .collect();
    if stale.is_empty() {
        return false;
    }
    for (i, centre) in stale {
        // The bound layer is gone: bind to the layer now under the frame centre, if any.
        d.revision_markers[i].layer_id = layer_at_point(d, centre);
    }
    true
}

fn detect(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "revision.detect";
    let one = p.get("index").and_then(Value::as_u64).map(|i| i as usize);
    let d0 = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    if let Some(i) = one
        && i >= d0.revision_markers.len()
    {
        return Err(bad(CMD, format!("no revision marker {i}")));
    }
    s.edit("Check Revision Markers", |d, _| {
        // A marker whose bound layer was deleted (and replaced) re-binds to the layer now under
        // its frame before pixels are compared, so the box keeps following the content.
        repair_bindings(d);
        let indices: Vec<usize> = (0..d.revision_markers.len()).filter(|&i| one.is_none_or(|o| o == i)).collect();
        for i in indices {
            let m = d.revision_markers[i].clone();
            if m.snapshot.is_empty() {
                continue;
            }
            // Re-render the framed area (snapshot_for samples to the marker's own grid, which a
            // file may carry at another size than the current default) and compare.
            let cur = snapshot_for(d, &m)?;
            let done = changed(&cur, &m.snapshot);
            d.revision_markers[i].done = done;
        }
        Ok(())
    })?;
    Ok(list_json(&s.active().ok_or(EngineError::NoDocument)?.doc))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "revision.add",
            label: "Add Revision Marker",
            menu: &[],
            shortcut: None,
            params: r##"{"x":px,"y":px,"width":px>0,"height":px>0,"name":str="","layerId":layer?,"note":str=""} → marker (box over content to change; a snapshot of the framed area is saved)"##,
            enabled: has_doc,
            run: add,
            journal: true,
        },
        CommandSpec {
            id: "revision.set",
            label: "Edit Revision Marker",
            menu: &[],
            shortcut: None,
            params: r##"{"index":n,"name":str?,"x":px?,"y":px?,"width":px?,"height":px?,"layerId":layer|null?,"note":str?} (moving or resizing re-snapshots the frame)"##,
            enabled: has_markers,
            run: set,
            journal: true,
        },
        CommandSpec {
            id: "revision.remove",
            label: "Delete Revision Marker",
            menu: &[],
            shortcut: None,
            params: r##"{"index":n}"##,
            enabled: has_markers,
            run: remove,
            journal: true,
        },
        CommandSpec {
            id: "revision.snapshot",
            label: "Update Revision Snapshot",
            menu: &[],
            shortcut: None,
            params: r##"{}|{"index":n} → markers (re-snapshot the framed area; detected-as-changed state resets)"##,
            enabled: has_markers,
            run: snapshot,
            journal: true,
        },
        CommandSpec {
            id: "revision.detect",
            label: "Check Revision Markers",
            menu: &[],
            shortcut: None,
            params: r##"{}|{"index":n} → markers with done=true where the framed content changed since its snapshot (red→green)"##,
            enabled: has_markers,
            run: detect,
            journal: true,
        },
        CommandSpec {
            id: "revision.list",
            label: "Revision Markers",
            menu: &[],
            shortcut: None,
            params: r##"{} → markers"##,
            enabled: has_doc,
            run: list,
            journal: false,
        },
    ]
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    Ok(list_json(&d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 150})).unwrap();
        s
    }

    #[test]
    fn add_detect_unchanged() {
        let mut s = session();
        s.execute("revision.add", json!({"x": 10.0, "y": 10.0, "width": 60.0, "height": 40.0, "name": "改这里"})).unwrap();
        let v = s.execute("revision.detect", json!({})).unwrap();
        let markers = v["markers"].as_array().unwrap();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0]["done"], json!(false), "content unchanged → red");
    }

    #[test]
    fn changed_after_paint() {
        let mut s = session();
        s.execute("revision.add", json!({"x": 10.0, "y": 10.0, "width": 60.0, "height": 40.0})).unwrap();
        // Paint a solid rectangle over the framed area.
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
        s.execute("edit.fill", json!({"color": [1.0, 0.0, 0.0]})).unwrap();
        let v = s.execute("revision.detect", json!({})).unwrap();
        let markers = v["markers"].as_array().unwrap();
        assert_eq!(markers[0]["done"], json!(true), "content changed → green");
    }

    #[test]
    fn move_with_layer() {
        let mut s = session();
        s.execute("revision.add", json!({"x": 10.0, "y": 10.0, "width": 60.0, "height": 40.0})).unwrap();
        let v = s.execute("revision.list", json!({})).unwrap();
        let x0 = v["markers"][0]["x"].as_f64().unwrap();
        // Bind to a new (ordinary) layer and translate it; the marker must follow.
        s.execute("layer.new.layer", json!({})).unwrap();
        let layers = s.execute("layer.list", json!({})).unwrap();
        let id = layers["layers"].as_array().unwrap()[0]["id"].as_u64().unwrap();
        s.execute("revision.set", json!({"index": 0, "layerId": id})).unwrap();
        s.execute("layer.translate", json!({"layer": id, "dx": 12, "dy": 8})).unwrap();
        let v = s.execute("revision.list", json!({})).unwrap();
        assert_eq!(v["markers"][0]["x"].as_f64().unwrap(), x0 + 12.0);
        assert_eq!(v["markers"][0]["y"].as_f64().unwrap(), 10.0 + 8.0);
    }

    #[test]
    fn snapshot_resets_done() {
        let mut s = session();
        s.execute("revision.add", json!({"x": 10.0, "y": 10.0, "width": 60.0, "height": 40.0})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
        s.execute("edit.fill", json!({"color": [0.0, 0.0, 1.0]})).unwrap();
        s.execute("revision.detect", json!({})).unwrap();
        let v = s.execute("revision.list", json!({})).unwrap();
        assert_eq!(v["markers"][0]["done"], json!(true));
        // Re-snapshot accepts the new content as the baseline → back to red.
        s.execute("revision.snapshot", json!({})).unwrap();
        let v = s.execute("revision.list", json!({})).unwrap();
        assert_eq!(v["markers"][0]["done"], json!(false));
    }

    #[test]
    fn layer_at_point_skips_groups() {
        // A folder's bounds cover its children; the point hit must resolve to the child layer,
        // not the folder, so markers bind to the layer actually under the frame.
        let mut s = session();
        s.execute("layer.new.layer", json!({})).unwrap();
        let layers = s.execute("layer.list", json!({})).unwrap();
        let child = layers["layers"].as_array().unwrap()[0]["id"].as_u64().unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
        s.execute("edit.fill", json!({"color": [0.0, 0.0, 1.0]})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        // Wrap the layer in a group: the group now covers the same bounds, above the child.
        s.execute("layer.new.groupFromLayers", json!({"layer": child})).unwrap();
        let d = s.active().unwrap().doc.clone();
        let hit = layer_at_point(&d, [40.0, 30.0]);
        assert_eq!(hit, Some(LayerId(child)), "hit resolves to the child layer, not the folder");
        assert!(hit.is_some_and(|id| !d.layer(id).unwrap().is_group()));
    }

    #[test]
    fn rebind_after_layer_deleted() {
        // Deleting a bound layer and placing a replacement over the same area re-binds the marker
        // to the new layer on the next detect, so the box keeps following the content.
        let mut s = session();
        s.execute("revision.add", json!({"x": 10.0, "y": 10.0, "width": 60.0, "height": 40.0})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let layers = s.execute("layer.list", json!({})).unwrap();
        let old = layers["layers"].as_array().unwrap()[0]["id"].as_u64().unwrap();
        s.execute("revision.set", json!({"index": 0, "layerId": old})).unwrap();
        // Delete the bound layer, then add a replacement filled over the same area.
        s.execute("layer.delete", json!({"layer": old})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let layers = s.execute("layer.list", json!({})).unwrap();
        let new = layers["layers"].as_array().unwrap()[0]["id"].as_u64().unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
        s.execute("edit.fill", json!({"color": [0.0, 0.0, 1.0]})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        // Detect re-binds to the replacement layer now under the frame and flags it changed.
        s.execute("revision.detect", json!({})).unwrap();
        let v = s.execute("revision.list", json!({})).unwrap();
        assert_eq!(v["markers"][0]["layerId"], json!(new), "re-bound to the replacement layer");
        assert_eq!(v["markers"][0]["done"], json!(true), "content changed vs the old snapshot → green");
    }

    #[test]
    fn undo_restores_done() {
        // Undoing a reset (re-snapshot) restores the previous snapshot and the green state.
        let mut s = session();
        s.execute("revision.add", json!({"x": 10.0, "y": 10.0, "width": 60.0, "height": 40.0})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
        s.execute("edit.fill", json!({"color": [0.0, 1.0, 0.0]})).unwrap();
        s.execute("revision.detect", json!({})).unwrap();
        let v = s.execute("revision.list", json!({})).unwrap();
        assert_eq!(v["markers"][0]["done"], json!(true), "changed → green");
        // Reset to red (new baseline).
        s.execute("revision.snapshot", json!({})).unwrap();
        let v = s.execute("revision.list", json!({})).unwrap();
        assert_eq!(v["markers"][0]["done"], json!(false), "reset → red");
        // Undo the reset: the marker is green again.
        s.execute("history.undo", json!({})).unwrap();
        let v = s.execute("revision.list", json!({})).unwrap();
        assert_eq!(v["markers"][0]["done"], json!(true), "undo restores green");
    }
}
