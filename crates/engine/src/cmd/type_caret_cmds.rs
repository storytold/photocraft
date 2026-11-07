//! Headless caret geometry for type layers: `type.hitTest`, `type.caret`, `type.navigate`.
//!
//! Queries (`journal: false`, no history step). Positions are document pixels, so an agent never
//! touches text space; offsets are character indices, the same unit as `type.edit` and
//! `type.setStyle`. The Type tool calls the same word, line and hit-test helpers.

use photocraft_doc::{Affine, Document, LayerContent, LayerId};
use photocraft_geom::{Point, Rect, TILE_SIZE};
use photocraft_text::TextLayout;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn req_f64(p: &Value, key: &str, cmd: &str) -> Result<f64> {
    match p.get(key) {
        Some(v) => v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` must be a finite number"))),
        None => Err(bad(cmd, format!("missing `{key}`"))),
    }
}

/// `x` when present. Absent is fine; a present non-number is not.
fn opt_f64(p: &Value, key: &str, cmd: &str) -> Result<Option<f64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => req_f64(p, key, cmd).map(Some),
    }
}

fn req_index(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    let Some(v) = p.get(key) else { return Err(bad(cmd, format!("missing `{key}`"))) };
    let n = v.as_u64().ok_or_else(|| bad(cmd, format!("`{key}` must be a character index")))?;
    usize::try_from(n).map_err(|_| bad(cmd, format!("`{key}` is out of range")))
}

fn check_index(text: &str, index: usize, cmd: &str) -> Result<()> {
    let n = text.chars().count();
    if index > n { Err(bad(cmd, format!("index {index} is past the end of the text ({n} characters)"))) } else { Ok(()) }
}

fn resolve_layer(s: &Session, p: &Value, cmd: &str) -> Result<LayerId> {
    match p.get("layer") {
        Some(v) if !v.is_null() => {
            let id = v.as_u64().ok_or_else(|| bad(cmd, "`layer` must be a layer id"))?;
            Ok(LayerId(id))
        }
        _ => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into())),
    }
}

/// A type layer laid out once, plus the bits of it the queries need after the borrow ends.
struct Frame {
    id: LayerId,
    text: String,
    transform: Affine,
    layout: TextLayout,
}

/// Lay out the named type layer at the document's resolution for a read-only caret query.
/// Missing layers and non-type content return errors; `cmd` identifies the query in parameter
/// errors. The frame retains model geometry, while ink checks read the document's current cache.
fn frame(doc: &Document, id: LayerId, cmd: &str) -> Result<Frame> {
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Text(t) = &layer.content else {
        return Err(bad(cmd, format!("layer {} is {} {} layer, not a type layer", id.0, layer.content.article(), layer.content.kind_name())));
    };
    let text = t.text.clone();
    let transform = t.transform;
    let dpi = doc.resolution_dpi;
    let layout = {
        let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
        eng.layout(t, dpi)
    };
    Ok(Frame { id, text, transform, layout })
}

fn to_text(aff: Affine, x: f64, y: f64) -> (f32, f32) {
    let p = aff.inverse().unwrap_or(Affine::IDENTITY).apply(Point::new(x, y));
    (p.x as f32, p.y as f32)
}

fn doc_point(aff: Affine, x: f32, y: f32, cmd: &str) -> Result<[f64; 2]> {
    let p = aff.apply(Point::new(f64::from(x), f64::from(y)));
    if p.x.is_finite() && p.y.is_finite() { Ok([p.x, p.y]) } else { Err(bad(cmd, "the caret is not a finite point")) }
}

/// Whether a type layer has cached ink at a document point. The cache is already in document
/// space, including transforms and group moves; applying the text transform again is incorrect.
/// Its bounding rectangle is not a hit: rotated text and Photoshop caches can enclose large
/// transparent areas, including nonzero colour channels with zero alpha (#10266).
pub fn cached_ink_contains(doc: &Document, id: LayerId, x: f64, y: f64) -> bool {
    let in_range = |v: f64| v.is_finite() && v >= f64::from(i32::MIN) && v < f64::from(i32::MAX) + 1.0;
    if !in_range(x) || !in_range(y) {
        return false;
    }
    let Some(LayerContent::Text(t)) = doc.layer(id).map(|l| &l.content) else { return false };
    t.cache.as_ref().is_some_and(|c| c.rgba(x.floor() as i32, y.floor() as i32)[3] > 0.0)
}

/// Whether cached type ink contributes anywhere in a view's source-pixel sampling footprint.
/// Reads only alpha in intersecting tiles, without allocating or scanning the cache's bounds.
/// Sparse, very large footprints visit stored tiles rather than walking an unbounded empty grid.
pub fn cached_ink_in_rect(doc: &Document, id: LayerId, area: Rect) -> bool {
    if area.is_empty() {
        return false;
    }
    let Some(LayerContent::Text(t)) = doc.layer(id).map(|l| &l.content) else { return false };
    let Some(cache) = &t.cache else { return false };
    if !cache.format().alpha {
        return true;
    }
    let Some(alpha) = cache.channels().checked_sub(1) else { return false };
    let mut row = [0.0; TILE_SIZE as usize];
    let mut has_ink = |r: Rect| {
        let Some(samples) = row.get_mut(..r.width() as usize) else { return false };
        for y in r.y0..r.y1 {
            cache.sample_row_strided(y, r.x0, 1, alpha, samples);
            if samples.iter().any(|a| *a > 0.0) {
                return true;
            }
        }
        false
    };
    let tile_count = area.tile_count();
    if tile_count <= cache.tile_count() as u64 {
        for tc in area.tiles() {
            let r = area.intersect(&tc.rect());
            if cache.tile(tc).is_some() {
                if has_ink(r) {
                    return true;
                }
            } else if cache.sample_channel(r.x0, r.y0, alpha) > 0.0 {
                return true;
            }
        }
    } else {
        let mut present = 0;
        for (tc, _) in cache.tiles() {
            let r = area.intersect(&tc.rect());
            if !r.is_empty() {
                present += 1;
                if has_ink(r) {
                    return true;
                }
            }
        }
        // Missing tiles share one default alpha. Find one without scanning the empty grid:
        // a missing tile must occur within the first `present + 1` queried tiles.
        if present < tile_count {
            for tc in area.tiles() {
                if cache.tile(tc).is_none() {
                    let r = area.intersect(&tc.rect());
                    return cache.sample_channel(r.x0, r.y0, alpha) > 0.0;
                }
            }
        }
    }
    false
}

impl Frame {
    /// Headless document-coordinate hit: laid-out line boxes or the exact cached pixel,
    /// without the Type tool's view-dependent filtering footprint or layout slop.
    fn contains(&self, doc: &Document, x: f64, y: f64) -> bool {
        let (tx, ty) = to_text(self.transform, x, y);
        photocraft_text::text_point_inside(&self.layout, tx, ty, 0.0) || cached_ink_contains(doc, self.id, x, y)
    }
}

/// Topmost visible type layer under the point. `walk` is bottom-up; the Type tool hits the top first.
fn topmost(doc: &Document, x: f64, y: f64) -> Option<Frame> {
    let mut ids: Vec<LayerId> = doc
        .walk()
        .into_iter()
        .filter(|(path, _, layer)| matches!(layer.content, LayerContent::Text(_)) && doc.shown_at(path))
        .map(|(_, _, layer)| layer.id)
        .collect();
    ids.reverse();
    ids.into_iter().find_map(|id| {
        let f = frame(doc, id, "type.hitTest").ok()?;
        f.contains(doc, x, y).then_some(f)
    })
}

/// Line-space column for a document x. Translate and scale keep that x as the same column on
/// every line; when a rotation makes document x independent of the column, the caret's own
/// column is kept (there is nothing to solve for).
fn column(layout: &TextLayout, aff: Affine, text: &str, index: usize, doc_x: Option<f64>) -> f32 {
    let (x, top, bottom) = layout.caret(photocraft_text::byte_index(text, index));
    let Some(doc_x) = doc_x else { return x };
    let y = f64::from(top + bottom) * 0.5;
    let [a, _, c, _, e, _] = aff.m;
    // Horizontal: text = (line_x, line_y). Vertical: text = (-line_y, line_x).
    let (coef, along) = if layout.vertical { (c, doc_x - e + a * y) } else { (a, doc_x - e - c * y) };
    if coef.abs() < 1e-8 {
        return x;
    }
    let solved = along / coef;
    if solved.is_finite() { solved as f32 } else { x }
}

/// Query the nearest caret at a finite document point without changing selection or history.
/// An explicit layer may be hidden or outside the point; otherwise pick the topmost shown hit.
/// `inside` distinguishes actual layout/cached-ink containment from a nearest caret outside it.
fn hit_test(s: &Session, p: &Value) -> Result<Value> {
    const CMD: &str = "type.hitTest";
    let x = req_f64(p, "x", CMD)?;
    let y = req_f64(p, "y", CMD)?;
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let frame = match p.get("layer") {
        Some(v) if !v.is_null() => {
            let id = v.as_u64().ok_or_else(|| bad(CMD, "`layer` must be a layer id"))?;
            frame(&doc, LayerId(id), CMD)?
        }
        _ => topmost(&doc, x, y).ok_or_else(|| bad(CMD, "no type layer under the point"))?,
    };
    let (tx, ty) = to_text(frame.transform, x, y);
    let (index, line) = photocraft_text::hit_char(&frame.layout, &frame.text, tx, ty);
    Ok(json!({ "layer": frame.id.0, "index": index, "line": line, "inside": frame.contains(&doc, x, y) }))
}

fn caret(s: &Session, p: &Value) -> Result<Value> {
    const CMD: &str = "type.caret";
    let index = req_index(p, "index", CMD)?;
    let id = resolve_layer(s, p, CMD)?;
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let frame = frame(&doc, id, CMD)?;
    check_index(&frame.text, index, CMD)?;
    let byte = photocraft_text::byte_index(&frame.text, index);
    let seg = frame.layout.caret_segment(byte);
    let p0 = doc_point(frame.transform, seg[0].0, seg[0].1, CMD)?;
    let p1 = doc_point(frame.transform, seg[1].0, seg[1].1, CMD)?;
    let line = photocraft_text::line_index(&frame.layout, byte);
    Ok(json!({ "index": index, "line": line, "segment": [p0, p1] }))
}

fn navigate(s: &Session, p: &Value) -> Result<Value> {
    const CMD: &str = "type.navigate";
    let index = req_index(p, "index", CMD)?;
    let mov = p.get("move").and_then(Value::as_str).ok_or_else(|| bad(CMD, "missing `move`"))?;
    let doc_x = opt_f64(p, "x", CMD)?;
    let id = resolve_layer(s, p, CMD)?;
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let frame = frame(&doc, id, CMD)?;
    check_index(&frame.text, index, CMD)?;
    let next = match mov {
        "wordPrev" => photocraft_text::word_boundary(&frame.text, index, false),
        "wordNext" => photocraft_text::word_boundary(&frame.text, index, true),
        "linePrev" | "lineNext" => {
            let dir = if mov == "linePrev" { -1 } else { 1 };
            let x = column(&frame.layout, frame.transform, &frame.text, index, doc_x);
            photocraft_text::line_step(&frame.layout, &frame.text, index, x, dir)
        }
        "lineStart" => photocraft_text::line_edge(&frame.layout, &frame.text, index, false),
        "lineEnd" => photocraft_text::line_edge(&frame.layout, &frame.text, index, true),
        "start" => 0,
        "end" => frame.text.chars().count(),
        other => {
            return Err(bad(CMD, format!("move {other:?} must be wordPrev, wordNext, linePrev, lineNext, lineStart, lineEnd, start or end")));
        }
    };
    Ok(json!({ "index": next }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "type.hitTest",
            label: "Hit-Test Type",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?, "x":px, "y":px} → {"layer","index":char,"line","inside":bool}. No layer: the topmost visible type layer whose laid-out text or rendered pixels contain the point (same order as the Type tool); a miss is an error. With a layer, index is the nearest caret even when inside is false"##,
            enabled: has_doc,
            journal: false,
            run: |s, p| hit_test(s, p),
        },
        CommandSpec {
            id: "type.caret",
            label: "Type Caret",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?, "index":char (0..=length; empty text is 0)} → {"index","line","segment":[[x,y],[x,y]]} caret segment in document pixels, for rotated and vertical type too"##,
            enabled: has_doc,
            journal: false,
            run: |s, p| caret(s, p),
        },
        CommandSpec {
            id: "type.navigate",
            label: "Navigate Type",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?, "index":char, "move":"wordPrev|wordNext|linePrev|lineNext|lineStart|lineEnd|start|end", "x":px?} → {"index"}. Words are alphanumeric runs. x is the document x to keep across linePrev/lineNext (a caret segment's x); omitted uses the caret's own column. Empty text returns index 0"##,
            enabled: has_doc,
            journal: false,
            run: |s, p| navigate(s, p),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
        s
    }

    fn create(s: &mut Session, p: Value) -> u64 {
        s.execute("type.create", p).unwrap()["layer"].as_u64().unwrap()
    }

    fn u(v: &Value, k: &str) -> u64 {
        v[k].as_u64().unwrap_or_else(|| panic!("missing {k} in {v}"))
    }

    /// Caret midpoint hit-tests back to the same character. `inside` is asserted for axis-aligned
    /// type; a rotation can land an edge caret a fraction of a pixel outside the line box.
    fn round_trip(s: &mut Session, id: u64, check_inside: bool) {
        let info = s.execute("type.info", json!({"layer": id})).unwrap();
        let text = info["text"].as_str().unwrap();
        let lines = info["lines"].as_array().unwrap();
        let n = text.chars().count();
        for i in 0..=n {
            let c = s.execute("type.caret", json!({"layer": id, "index": i})).unwrap();
            assert_eq!(u(&c, "index"), i as u64, "{text:?} caret {i}: {c}");
            let seg = c["segment"].as_array().unwrap();
            let x = (seg[0][0].as_f64().unwrap() + seg[1][0].as_f64().unwrap()) / 2.0;
            let y = (seg[0][1].as_f64().unwrap() + seg[1][1].as_f64().unwrap()) / 2.0;
            assert!(x.is_finite() && y.is_finite(), "{c}");
            let h = s.execute("type.hitTest", json!({"layer": id, "x": x, "y": y})).unwrap();
            assert_eq!(u(&h, "index"), i as u64, "{text:?} index {i} at ({x}, {y}) -> {h}");
            assert_eq!(h["line"], c["line"], "{text:?} index {i}");
            assert_eq!(u(&h, "layer"), id);
            // A shared soft-wrap boundary belongs to the following line, where the caret is drawn (#3068).
            let on_line = lines.iter().rposition(|ln| {
                let a = ln["start"].as_u64().unwrap();
                let b = ln["end"].as_u64().unwrap();
                (i as u64) >= a && (i as u64) <= b
            });
            let expected = on_line.unwrap_or(lines.len().saturating_sub(1)) as u64;
            assert_eq!(u(&c, "line"), expected, "{text:?} index {i} lines {lines:?}");
            if check_inside {
                assert_eq!(h["inside"], true, "{text:?} index {i} at ({x}, {y})");
            }
        }
    }

    #[test]
    fn commands_are_registered_queries() {
        for id in ["type.hitTest", "type.caret", "type.navigate"] {
            let spec = crate::command_specs().iter().find(|c| c.id == id).unwrap_or_else(|| panic!("missing {id}"));
            assert!(!spec.journal, "{id}");
            assert!(spec.menu.is_empty(), "{id}");
        }
    }

    #[test]
    fn caret_midpoint_round_trips_for_point_paragraph_transformed_and_vertical() {
        let mut s = session();
        let point = create(&mut s, json!({"x": 40, "y": 70, "text": "AäB", "size": 28}));
        round_trip(&mut s, point, true);

        let para = create(&mut s, json!({"box": [15, 20, 64, 180], "text": "one two three four six seven eight nine", "size": 16}));
        let nlines = s.execute("type.info", json!({"layer": para})).unwrap()["lines"].as_array().unwrap().len();
        assert!(nlines >= 2, "paragraph should wrap, got {nlines} lines");
        round_trip(&mut s, para, true);

        let turned = create(&mut s, json!({"x": 30, "y": 40, "text": "Rotate me", "size": 22}));
        let ang = 0.55_f64;
        let scale = 1.35;
        let (sn, cs) = ang.sin_cos();
        s.execute("type.edit", json!({"layer": turned, "transform": [cs * scale, sn * scale, -sn * scale, cs * scale, 130.0, 80.0]})).unwrap();
        round_trip(&mut s, turned, false);

        let vert = create(&mut s, json!({"x": 200, "y": 30, "text": "Abc\nDef", "size": 20}));
        s.execute("type.orientation.vertical", json!({"layer": vert})).unwrap();
        round_trip(&mut s, vert, false);
    }

    fn go(s: &mut Session, id: u64, index: usize, mov: &str, x: Option<f64>) -> u64 {
        let mut p = json!({"layer": id, "index": index, "move": mov});
        if let Some(x) = x {
            p["x"] = json!(x);
        }
        u(&s.execute("type.navigate", p).unwrap(), "index")
    }

    #[test]
    fn navigate_words_lines_edges_and_keeps_a_column() {
        check_word_moves();
        check_newline_moves();
        check_vertical_column_moves();
        check_navigation_queries_are_read_only();
    }

    fn check_word_moves() {
        let mut s = session();
        let id = create(&mut s, json!({"x": 20, "y": 60, "text": "hello big world", "size": 18}));
        assert_eq!(go(&mut s, id, 0, "wordNext", None), 5);
        assert_eq!(go(&mut s, id, 5, "wordNext", None), 9);
        assert_eq!(go(&mut s, id, 9, "wordNext", None), 15);
        assert_eq!(go(&mut s, id, 15, "wordNext", None), 15);
        assert_eq!(go(&mut s, id, 15, "wordPrev", None), 10);
        assert_eq!(go(&mut s, id, 7, "wordPrev", None), 6);
        assert_eq!(go(&mut s, id, 6, "wordPrev", None), 0);
        assert_eq!(go(&mut s, id, 0, "wordPrev", None), 0);
        assert_eq!(go(&mut s, id, 4, "start", None), 0);
        assert_eq!(go(&mut s, id, 4, "end", None), 15);
        assert_eq!(go(&mut s, id, 4, "lineStart", None), 0);
        assert_eq!(go(&mut s, id, 4, "lineEnd", None), 15);
    }

    fn check_newline_moves() {
        let mut s = session();
        // A newline is not part of either word; line edges agree with type.info.
        let id = create(&mut s, json!({"x": 20, "y": 120, "text": "hello\nworld", "size": 18}));
        assert_eq!(go(&mut s, id, 0, "wordNext", None), 5);
        assert_eq!(go(&mut s, id, 5, "wordNext", None), 11);
        assert_eq!(go(&mut s, id, 6, "wordPrev", None), 0);
        let info = s.execute("type.info", json!({"layer": id})).unwrap();
        let lines = info["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 2, "{lines:?}");
        let (l0s, l0e) = (lines[0]["start"].as_u64().unwrap(), lines[0]["end"].as_u64().unwrap());
        let (l1s, l1e) = (lines[1]["start"].as_u64().unwrap(), lines[1]["end"].as_u64().unwrap());
        assert_eq!(go(&mut s, id, 2, "lineEnd", None), l0e);
        assert_eq!(go(&mut s, id, 2, "lineStart", None), l0s);
        assert_eq!(go(&mut s, id, l1s as usize + 1, "lineStart", None), l1s);
        assert_eq!(go(&mut s, id, l1s as usize, "lineEnd", None), l1e);
        let next = go(&mut s, id, 1, "lineNext", None);
        assert!((l1s..=l1e).contains(&next), "lineNext {next} not on {l1s}..={l1e}");
        assert_eq!(go(&mut s, id, 0, "linePrev", None), 0);
        assert_eq!(go(&mut s, id, l1e as usize, "lineNext", None), "hello\nworld".chars().count() as u64);
    }

    fn check_vertical_column_moves() {
        let mut s = session();
        // Repeated line moves keep the document x of the original column.
        let id = create(&mut s, json!({"x": 24, "y": 200, "text": "WWWWWW\nI", "size": 28}));
        let info = s.execute("type.info", json!({"layer": id})).unwrap();
        let lines = info["lines"].as_array().unwrap();
        let end0 = lines[0]["end"].as_u64().unwrap();
        let (start1, end1) = (lines[1]["start"].as_u64().unwrap(), lines[1]["end"].as_u64().unwrap());
        assert!(end1 > start1, "{lines:?}");
        let caret0 = s.execute("type.caret", json!({"layer": id, "index": 0})).unwrap();
        let x = caret0["segment"][0][0].as_f64().unwrap();
        assert_eq!(go(&mut s, id, end0 as usize, "lineNext", Some(x)), start1);
        assert_eq!(go(&mut s, id, end0 as usize, "lineNext", None), end1);
        assert_eq!(go(&mut s, id, start1 as usize, "linePrev", Some(x)), lines[0]["start"].as_u64().unwrap());
    }

    fn check_navigation_queries_are_read_only() {
        let mut s = session();
        let id = create(&mut s, json!({"x": 24, "y": 200, "text": "WWWWWW\nI", "size": 28}));
        let caret0 = s.execute("type.caret", json!({"layer": id, "index": 0})).unwrap();
        let x = caret0["segment"][0][0].as_f64().unwrap();
        // Queries do not record history or a journal entry.
        let steps = s.active().unwrap().history.entries().len();
        let rev = s.active().unwrap().revision;
        let journal = s.journal.len();
        s.execute("type.hitTest", json!({"layer": id, "x": x, "y": caret0["segment"][0][1]})).unwrap();
        s.execute("type.caret", json!({"layer": id, "index": 1})).unwrap();
        s.execute("type.navigate", json!({"layer": id, "index": 0, "move": "end"})).unwrap();
        assert_eq!(s.active().unwrap().history.entries().len(), steps);
        assert_eq!(s.active().unwrap().revision, rev);
        assert_eq!(s.journal.len(), journal);
    }

    #[test]
    fn wrapped_caret_queries_use_the_line_that_contains_the_caret() {
        let mut s = session();
        let id = create(&mut s, json!({"text": "ab cd", "box": [20, 20, 20, 70], "size": 12, "font": "Inter"}));
        let info = s.execute("type.info", json!({"layer": id})).unwrap();
        let lines = info["lines"].as_array().unwrap();
        assert!(lines.len() >= 2, "{lines:?}");
        let index = lines[1]["start"].as_u64().unwrap() as usize;
        assert_eq!(index as u64, lines[0]["end"].as_u64().unwrap(), "shared wrap boundary");
        assert_eq!(s.execute("type.caret", json!({"layer": id, "index": index})).unwrap()["line"], 1);
        assert_eq!(go(&mut s, id, index, "lineStart", None), index as u64);
        assert_eq!(go(&mut s, id, index, "lineEnd", None), lines[1]["end"].as_u64().unwrap());

        let hard = create(&mut s, json!({"text": "ab\ncd", "x": 30, "y": 100, "size": 12, "font": "Inter"}));
        assert_eq!(s.execute("type.caret", json!({"layer": hard, "index": 3})).unwrap()["line"], 1);
        assert_eq!(go(&mut s, hard, 3, "lineStart", None), 3);
        assert_eq!(go(&mut s, hard, 3, "lineEnd", None), 5);
    }

    #[test]
    fn hit_test_picks_the_topmost_visible_type_layer_and_empty_text_is_index_zero() {
        let mut s = session();
        let lower = create(&mut s, json!({"x": 50, "y": 80, "text": "AAAA", "size": 36}));
        let upper = create(&mut s, json!({"x": 50, "y": 80, "text": "BBBB", "size": 36}));
        let c = s.execute("type.caret", json!({"layer": upper, "index": 1})).unwrap();
        let x = (c["segment"][0][0].as_f64().unwrap() + c["segment"][1][0].as_f64().unwrap()) / 2.0;
        let y = (c["segment"][0][1].as_f64().unwrap() + c["segment"][1][1].as_f64().unwrap()) / 2.0;
        let hit = s.execute("type.hitTest", json!({"x": x, "y": y})).unwrap();
        assert_eq!(u(&hit, "layer"), upper);
        assert_eq!(hit["inside"], true);
        s.execute("layer.setProps", json!({"layer": upper, "visible": false})).unwrap();
        let hit = s.execute("type.hitTest", json!({"x": x, "y": y})).unwrap();
        assert_eq!(u(&hit, "layer"), lower);
        // A named hidden layer is still queried.
        let hit = s.execute("type.hitTest", json!({"layer": upper, "x": x, "y": y})).unwrap();
        assert_eq!(u(&hit, "layer"), upper);
        assert!(s.execute("type.hitTest", json!({"x": -40.0, "y": -40.0})).is_err());

        let empty = create(&mut s, json!({"x": 10, "y": 40, "text": "", "size": 18}));
        let c = s.execute("type.caret", json!({"layer": empty, "index": 0})).unwrap();
        assert_eq!(u(&c, "index"), 0);
        let x = c["segment"][0][0].as_f64().unwrap();
        let y = (c["segment"][0][1].as_f64().unwrap() + c["segment"][1][1].as_f64().unwrap()) / 2.0;
        let hit = s.execute("type.hitTest", json!({"layer": empty, "x": x, "y": y})).unwrap();
        assert_eq!(u(&hit, "index"), 0);
        for mov in ["wordNext", "wordPrev", "lineNext", "linePrev", "lineStart", "lineEnd", "start", "end"] {
            assert_eq!(go(&mut s, empty, 0, mov, None), 0, "{mov}");
        }
    }

    /// #10266: Photoshop caches can retain colour in transparent padding. Their overall bounds
    /// must not hide lower text; real cached ink still counts even away from our font layout.
    #[test]
    fn hit_test_ignores_transparent_cache_padding_but_keeps_imported_ink() {
        use photocraft_color::PixelFormat;
        use photocraft_raster::Surface;
        use std::sync::Arc;

        for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = session();
            let lower = create(&mut s, json!({"x": 50, "y": 80, "text": "Lower", "size": 36}));
            let upper = create(&mut s, json!({"x": 200, "y": 180, "text": "Upper", "size": 24}));
            let caret = s.execute("type.caret", json!({"layer": lower, "index": 1})).unwrap();
            let x = caret["segment"][0][0].as_f64().unwrap();
            let y = (caret["segment"][0][1].as_f64().unwrap() + caret["segment"][1][1].as_f64().unwrap()) / 2.0;
            let mut cache = Surface::new(format);
            cache.write_pixel(0, 0, &[1.0, 0.5, 0.25, 0.0]);
            cache.write_pixel(399, 299, &[1.0, 0.5, 0.25, 0.0]);
            cache.write_pixel(320, 250, &[0.2, 0.4, 0.8, 1.0]);
            assert!(cache.content_bounds().contains(x.floor() as i32, y.floor() as i32));
            let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
            let LayerContent::Text(t) = &mut doc.layer_mut(LayerId(upper)).unwrap().content else { panic!("text") };
            t.cache = Some(cache);
            let steps = s.active().unwrap().history.entries().len();
            assert_eq!(u(&s.execute("type.hitTest", json!({"x": x, "y": y})).unwrap(), "layer"), lower);
            assert_eq!(s.execute("type.hitTest", json!({"layer": upper, "x": x, "y": y})).unwrap()["inside"], false);
            assert_eq!(u(&s.execute("type.hitTest", json!({"x": 320.5, "y": 250.5})).unwrap(), "layer"), upper);
            assert_eq!(s.active().unwrap().history.entries().len(), steps, "picking remains read-only");
            let doc = &s.active().unwrap().doc;
            for (x, y) in [(f64::NAN, 250.0), (320.0, f64::INFINITY), (f64::MAX, 250.0), (320.0, f64::MIN)] {
                assert!(!cached_ink_contains(doc, LayerId(upper), x, y));
            }
        }
    }

    /// Filtering must find neighboring alpha at every depth, without treating coloured padding,
    /// an excluded half-open edge, or a vast sparse cache rectangle as ink.
    #[test]
    fn cached_ink_footprints_respect_alpha_edges_and_sparse_defaults() {
        use photocraft_color::PixelFormat;
        use photocraft_raster::Surface;
        use std::sync::Arc;

        for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = session();
            let id = LayerId(create(&mut s, json!({"x": 10, "y": 40, "text": "A", "size": 18})));
            let mut cache = Surface::new(format);
            cache.write_pixel(256, -1, &[1.0, 0.0, 0.0, 0.25]);
            cache.write_pixel(-256, 256, &[1.0, 0.0, 0.0, 0.0]);
            let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
            let LayerContent::Text(t) = &mut doc.layer_mut(id).unwrap().content else { panic!("text") };
            t.cache = Some(cache);
            assert!(cached_ink_in_rect(doc, id, Rect::new(254, -3, 258, 1)), "{format:?}: neighboring ink across a tile edge");
            assert!(!cached_ink_in_rect(doc, id, Rect::new(254, -3, 256, 1)), "{format:?}: excluded right edge");
            assert!(!cached_ink_in_rect(doc, id, Rect::new(-258, 254, -254, 258)), "{format:?}: transparent colour is not ink");
            assert!(!cached_ink_in_rect(doc, id, Rect::new(i32::MIN, 1, i32::MAX, i32::MAX)), "{format:?}: sparse transparent region");
            let LayerContent::Text(t) = &mut doc.layer_mut(id).unwrap().content else { panic!("text") };
            t.cache = Some(Surface::with_default(format, &[0.0, 0.0, 0.0, 0.5]));
            assert!(cached_ink_in_rect(doc, id, Rect::new(-258, 254, -254, 258)), "{format:?}: default alpha is visible");
            assert!(!cached_ink_in_rect(doc, id, Rect::EMPTY), "{format:?}: empty footprint");
        }
    }

    #[test]
    fn automatic_hit_test_skips_hidden_parent_groups() {
        let mut s = session();
        let text = create(&mut s, json!({"x": 50, "y": 80, "text": "AB", "size": 36}));
        let caret = s.execute("type.caret", json!({"layer": text, "index": 1})).unwrap();
        let x = (caret["segment"][0][0].as_f64().unwrap() + caret["segment"][1][0].as_f64().unwrap()) / 2.0;
        let y = (caret["segment"][0][1].as_f64().unwrap() + caret["segment"][1][1].as_f64().unwrap()) / 2.0;

        assert_eq!(u(&s.execute("type.hitTest", json!({"x": x, "y": y})).unwrap(), "layer"), text);
        s.execute("layer.new.groupFromLayers", json!({"name": "Inner"})).unwrap();
        s.execute("layer.new.groupFromLayers", json!({"name": "Outer"})).unwrap();
        s.execute("layer.setProps", json!({"visible": false})).unwrap();

        assert!(s.execute("type.hitTest", json!({"x": x, "y": y})).is_err(), "hidden ancestor must suppress automatic targeting");
        assert_eq!(
            u(&s.execute("type.hitTest", json!({"layer": text, "x": x, "y": y})).unwrap(), "layer"),
            text,
            "explicit targeting of hidden text is still allowed"
        );

        s.execute("layer.setProps", json!({"visible": true})).unwrap();
        assert_eq!(u(&s.execute("type.hitTest", json!({"x": x, "y": y})).unwrap(), "layer"), text);
    }

    #[test]
    fn bad_params_are_errors() {
        let mut bare = Session::new();
        assert!(matches!(bare.execute("type.hitTest", json!({"x": 0, "y": 0})), Err(EngineError::Disabled(..))));
        assert!(matches!(bare.execute("type.caret", json!({"index": 0})), Err(EngineError::Disabled(..))));
        assert!(matches!(bare.execute("type.navigate", json!({"index": 0, "move": "end"})), Err(EngineError::Disabled(..))));

        let mut s = session();
        let id = create(&mut s, json!({"x": 10, "y": 40, "text": "Hi", "size": 20}));
        let pixel = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let bad = |s: &mut Session, cmd: &str, p: Value| match s.execute(cmd, p) {
            Err(EngineError::BadParams { .. }) => {}
            other => panic!("{cmd} -> {other:?}"),
        };
        bad(&mut s, "type.hitTest", json!({}));
        bad(&mut s, "type.hitTest", json!({"x": "1", "y": 2}));
        bad(&mut s, "type.hitTest", json!({"layer": id, "x": 1}));
        bad(&mut s, "type.hitTest", json!({"layer": "nope", "x": 1, "y": 2}));
        bad(&mut s, "type.hitTest", json!({"layer": pixel, "x": 1, "y": 2}));
        bad(&mut s, "type.caret", json!({"layer": id}));
        bad(&mut s, "type.caret", json!({"layer": id, "index": -1}));
        bad(&mut s, "type.caret", json!({"layer": id, "index": 1.5}));
        bad(&mut s, "type.caret", json!({"layer": id, "index": 99}));
        bad(&mut s, "type.caret", json!({"layer": pixel, "index": 0}));
        bad(&mut s, "type.navigate", json!({"layer": id, "index": 0}));
        bad(&mut s, "type.navigate", json!({"layer": id, "index": 0, "move": "left"}));
        bad(&mut s, "type.navigate", json!({"layer": id, "index": 9, "move": "end"}));
        bad(&mut s, "type.navigate", json!({"layer": id, "index": 0, "move": "lineNext", "x": "no"}));
        bad(&mut s, "type.navigate", json!({"layer": pixel, "index": 0, "move": "start"}));
        assert!(matches!(s.execute("type.caret", json!({"layer": u64::MAX, "index": 0})), Err(EngineError::UnknownLayer { .. })));
        // A point on the named layer but outside its text still returns a caret, marked outside.
        let far = s.execute("type.hitTest", json!({"layer": id, "x": 390, "y": 290})).unwrap();
        assert_eq!(far["inside"], false);
        assert!(u(&far, "index") <= 2);
    }
}
