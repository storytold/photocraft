//! Paint geometric selection coverage; deliberately independent of image colours.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_algo::selection;
#[cfg(test)]
use photocraft_algo::selection::{Region, SelectionMode};
#[cfg(test)]
use photocraft_geom::Rect;
use serde_json::{Value, json};

/// Tag validation failures with the command id used by automation clients.
fn bad(msg: &str) -> EngineError {
    EngineError::BadParams { cmd: "select.brush".into(), msg: msg.into() }
}

// Compatibility helper for the original geometric regression tests. Production code uses
// the sparse L2 stroke directly and never materialises a full stroke bounding box.
#[cfg(test)]
/// Materialize small test masks for compatibility with geometric selection assertions.
fn region(points: &[[f64; 2]], diameter: f32, canvas: Rect) -> Result<Option<Region>> {
    use photocraft_algo::selection::brush::{Options, StrokeMask};
    let mut stroke = StrokeMask::new(Options { size: diameter, hardness: 1.0, ..Default::default() }, canvas).map_err(bad)?;
    stroke.append(points).map_err(bad)?;
    let mask = stroke.finish().map_err(bad)?;
    let bbox = mask.content_bounds();
    if bbox.is_empty() {
        return Ok(None);
    }
    Ok(Some(Region { bbox, mask: mask.read_region(bbox).iter().map(|v| (v * 255.0).round() as u8).collect() }))
}

/// Read a finite bounded brush setting, using its default only when the field is absent.
fn number(p: &Value, key: &str, default: f32, lo: f32, hi: f32) -> Result<f32> {
    let Some(v) = p.get(key) else { return Ok(default) };
    let v = v.as_f64().ok_or_else(|| bad("brush settings must be numbers"))? as f32;
    if !v.is_finite() || !(lo..=hi).contains(&v) {
        return Err(bad("brush setting is outside its supported range"));
    }
    Ok(v)
}

/// Validate and rasterize before atomically composing one undoable selection edit.
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    use photocraft_algo::selection::brush::{Options, StrokeMask};
    use photocraft_color::PixelFormat;
    use photocraft_raster::Surface;
    let raw = p.get("points").and_then(Value::as_array).ok_or_else(|| bad("points must be an array"))?;
    if raw.is_empty() || raw.len() > 65_536 {
        return Err(bad("use 1..65536 points"));
    }
    let points: Vec<[f64; 2]> = raw
        .iter()
        .map(|p| Ok([p.get(0).and_then(Value::as_f64).ok_or_else(|| bad("invalid x"))?, p.get(1).and_then(Value::as_f64).ok_or_else(|| bad("invalid y"))?]))
        .collect::<Result<_>>()?;
    let mode = match p.get("mode") {
        None => "add",
        Some(v) => v.as_str().ok_or_else(|| bad("mode must be a string"))?,
    };
    if !matches!(mode, "add" | "subtract" | "replace") {
        return Err(bad("mode must be add, subtract or replace"));
    }
    let fill_closed = match p.get("fillClosed") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| bad("fillClosed must be boolean"))?,
    };
    let options = Options {
        size: number(p, "size", 60.0, 1.0, 5000.0)?,
        hardness: number(p, "hardness", 70.0, 0.0, 100.0)? / 100.0,
        opacity: number(p, "opacity", 100.0, 0.0, 100.0)? / 100.0,
        fill_closed,
    };
    let canvas = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    let mut stroke = StrokeMask::new(options, canvas).map_err(bad)?;
    stroke.append(&points).map_err(bad)?;
    let mask = stroke.finish().map_err(bad)?;
    if mask.content_bounds().is_empty() || (mode == "subtract" && s.active().is_none_or(|st| st.doc.selection.is_none())) {
        return Ok(json!({"changed":false}));
    }
    let selected = s.edit("Selection Brush", |doc, _| {
        let empty = Surface::new(PixelFormat::GRAY8);
        let old = doc.selection.as_ref().unwrap_or(&empty);
        let result = match mode {
            "replace" => mask.clone(),
            "subtract" => selection::combine_sparse(old, &mask, canvas, |a, b| (a - b).max(0.0)),
            _ => selection::combine_sparse(old, &mask, canvas, f32::max),
        };
        doc.selection = (!result.content_bounds().is_empty()).then_some(result);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({"changed":true,"selected":selected}))
}

/// Expose the same selection operation to the native UI, CLI and automation registry.
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "select.brush",
        label: "Selection Brush",
        menu: &[],
        shortcut: None,
        params: r#"{"points":[[x,y],…],"size":px=60,"hardness":0..100=70,"opacity":0..100=100,"fillClosed":bool=true,"mode":"add|subtract|replace"="add"}; closed strokes fill their interior"#,
        enabled: |s| s.active().map(|_| ()).ok_or_else(|| "no document open".into()),
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    /// Verify no-op strength and invalid parameters cannot add history entries.
    fn selection_brush_zero_opacity_and_rejected_options_preserve_history() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":100,"height":100})).unwrap();
        s.execute("select.rect", json!({"x":5,"y":5,"width":20,"height":20})).unwrap();
        let rev = s.active().unwrap().revision;
        assert_eq!(s.execute("select.brush", json!({"points":[[50,50]],"opacity":0})).unwrap()["changed"], false);
        for p in [json!({"points":[[0,0]],"opacity":101}), json!({"points":[[0,0]],"hardness":-1}), json!({"points":[[0,0]],"mode":true})] {
            assert!(s.execute("select.brush", p).is_err());
        }
        assert_eq!(s.active().unwrap().revision, rev);
        s.execute("select.brush", json!({"points":[[50,50]],"size":10})).unwrap();
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(10, 10, 0), 1.0);
    }
    #[test]
    /// Exercise subtraction and undo/redo at all supported document depths.
    fn selection_brush_history_and_depths() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":100,"height":100,"depth":depth})).unwrap();
            s.execute("select.brush", json!({"points":[[30,30],[70,30]],"size":10})).unwrap();
            assert!(s.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(50, 30, 0) > 0.9);
            s.execute("select.brush", json!({"points":[[50,30]],"size":10,"mode":"subtract"})).unwrap();
            assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(50, 30, 0), 0.0);
            s.execute("edit.undo", json!({})).unwrap();
            assert!(s.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(50, 30, 0) > 0.9);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(50, 30, 0), 0.0);
            assert!(s.execute("select.brush", json!({"points":[["bad",30]]})).is_err());
        }
    }
    #[test]
    /// Check geometric open strokes, closed fill and subtraction composition.
    fn selection_brush_round_strokes_and_closed_interiors() {
        let canvas = Rect::new(0, 0, 100, 100);
        let open = region(&[[20., 20.], [80., 20.], [80., 80.]], 10., canvas).unwrap().unwrap();
        assert_eq!(open.at(50, 50), 0.0);
        assert!(open.at(50, 20) > 0.9);
        let closed = region(&[[20., 20.], [80., 20.], [80., 80.], [20., 80.], [20., 20.]], 10., canvas).unwrap().unwrap();
        assert_eq!(closed.at(50, 50), 1.0);
        assert_eq!(closed.at(5, 5), 0.0);
        let erased =
            selection::combine_region(selection::combine_region(None, Some(&closed), SelectionMode::Add).as_ref(), Some(&open), SelectionMode::Subtract)
                .unwrap();
        assert!(erased.sample_channel(50, 50, 0) > 0.9);
        assert_eq!(erased.sample_channel(50, 20, 0), 0.0);
    }
    #[test]
    /// Check clipped clicks, empty input and non-finite geometry.
    fn selection_brush_click_clipping_and_invalid_input() {
        let c = Rect::new(0, 0, 100, 100);
        assert!(region(&[[0., 0.]], 10., c).unwrap().unwrap().at(1, 1) > 0.9);
        assert!(region(&[[-100., -100.]], 10., c).unwrap().is_none());
        assert!(region(&[], 10., c).is_err());
        assert!(region(&[[f64::NAN, 0.]], 10., c).is_err());
        assert!(region(&[[0., 0.]], f32::INFINITY, c).is_err());
    }
}
