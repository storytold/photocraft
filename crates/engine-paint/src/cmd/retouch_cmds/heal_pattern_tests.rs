//! #12104: the Healing Brush's Pattern source. `paint.healingBrush {"sourceMode":"pattern",
//! "pattern":<id or name>}` takes its texture from the pattern tile instead of an Alt-sampled
//! point: the tile, laid through the heal coverage math like the sampled source is, and - with
//! `mode:"replace"` (#12101) - laid as it stands, byte for byte equal to the tile's source pixels
//! read from our own document (the pattern's pixels through the same conversion chain the Pattern
//! Stamp lays). A stroke needs no source point; the phase follows the Pattern Stamp's aligned
//! rule (document origin with `aligned`). An unknown pattern, an unknown `sourceMode` string or a
//! supplied `sourceMode` that is not a string is an `Err` that paints nothing and adds no History
//! step; `sourceMode:"sampled"` (or no `sourceMode`) keeps the sampled path byte for byte, and one
//! pattern stroke is one "Healing Brush" step.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_compose::pattern::{Placement, Tile, render};
use photocraft_doc::{Pattern, Size};
use photocraft_raster::{Surface, from_rgba};

use super::*;

/// The pattern's side: a 6 x 6 tile of four flat colours (3 x 3 quadrants: light grey top-left,
/// warm top-right, cool bottom-left, pale grey bottom-right). The colours and the dark green ground
/// never coincide, so a painted pixel is always recognisably the tile's or the document's.
const TILE: u32 = 6;
const QUADRANTS: [[f32; 3]; 4] = [
    [250.0 / 255.0, 250.0 / 255.0, 250.0 / 255.0],
    [240.0 / 255.0, 120.0 / 255.0, 40.0 / 255.0],
    [60.0 / 255.0, 120.0 / 255.0, 245.0 / 255.0],
    [195.0 / 255.0, 200.0 / 255.0, 205.0 / 255.0],
];

/// The pattern's source pixel at its own (`u`, `v`) in the document's native levels: the tile
/// rendered and converted exactly as a stamping path renders it (`Tile` + `Placement` +
/// `render` + the native write, in unit scale and angle). This is the pixel a Replace stroke or a
/// Pattern Stamp lays at full coverage, and the byte pins below compare against it - not against
/// the stored sample, because that round-trips one level off for some values.
fn source_levels(s: &Session, u: i32, v: i32) -> Vec<i32> {
    let st = s.active().unwrap();
    let pat = &st.doc.patterns[0];
    let fmt = st.doc.pixel_format();
    let tile = Tile::new(pat).expect("the test tile");
    let place = Placement::new(Rect::EMPTY, false, (0.0, 0.0), 1.0, 0.0);
    let one = Rect::new(u, v, u + 1, v + 1);
    let rgba = render(&tile, &place, one)[0];
    let native = from_rgba(&fmt, rgba);
    let q = |val: f32| (val * 255.0).round() as i32;
    let mut out: Vec<i32> = native.iter().take(fmt.mode.color_channels()).map(|&val| q(val)).collect();
    if fmt.alpha {
        out.push(q(*native.last().unwrap_or(&1.0)));
    }
    out
}

/// The test document: green 40,160,60 throughout, holding one pattern ("Heal tile") with the
/// quadrants above; no clone source is ever set here.
fn document() -> Session {
    let mut s = crate::session();
    s.add_document(
        Document::with_background("t", Size::new(48, 48), ColorMode::Rgb, SampleType::U8, Color::rgb(40.0 / 255.0, 160.0 / 255.0, 60.0 / 255.0)),
        None,
    );
    let fmt = s.active().unwrap().doc.pixel_format();
    let mut surf = Surface::new(fmt);
    for (i, (dx, dy)) in [(0, 0), (3, 0), (0, 3), (3, 3)].into_iter().enumerate() {
        surf.fill_rect(Rect::new(dx, dy, dx + 3, dy + 3), &[QUADRANTS[i][0], QUADRANTS[i][1], QUADRANTS[i][2], 1.0]);
    }
    s.edit("Test pattern", |doc, _| {
        doc.patterns.push(Pattern::new("Heal tile", surf, TILE, TILE));
        Ok(())
    })
    .unwrap();
    s
}

fn pattern_id(s: &Session) -> String {
    s.active().unwrap().doc.patterns[0].id.clone()
}

/// A pattern stroke: one click at 30,24 with a 20 px hard round tip, by name or id, no source
/// point and no clone source set.
fn stroke(pattern: &str, extra: Value) -> Value {
    let mut p = json!({"points": [[30, 24]], "pattern": pattern, "sourceMode": "pattern", "size": 20, "hardness": 100});
    for (key, value) in extra.as_object().unwrap() {
        p[key] = value.clone();
    }
    p
}

fn pixel(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let st = s.active().unwrap();
    st.doc.layers[0].surface().unwrap().pixel(x, y)
}

fn state(s: &Session) -> (Vec<Vec<f32>>, usize) {
    let st = s.active().unwrap();
    let pts = (0..48).flat_map(|y| (0..48).map(move |x| (x, y))).map(|(x, y)| pixel(s, x, y)).collect();
    (pts, st.history.past_len())
}

/// The tile's source pixel the aligned placement puts at document (`x`, `y`), in native levels.
fn want(s: &Session, x: i32, y: i32) -> Vec<i32> {
    let (u, v) = (x.rem_euclid(i32::try_from(TILE).unwrap()), y.rem_euclid(i32::try_from(TILE).unwrap()));
    source_levels(s, u, v)
}

/// A pixel in native 8-bit levels.
fn levels(s: &Session, x: i32, y: i32) -> Vec<i32> {
    let q = |v: f32| (v * 255.0).round() as i32;
    pixel(s, x, y).iter().map(|&v| q(v)).collect()
}

/// The sampled fixture: the same document with a 20 x 20 checker at 6,6 (5 px cells alternating
/// 60,60,60 / 200,200,200), so the source point 16,16 and the ground it heals into are different
/// pixels. On the flat green fixture a sampled stroke heals green into green and a path that
/// painted nothing would still pass.
fn sampled_document() -> Session {
    let mut s = document();
    for cx in 0..4 {
        for cy in 0..4 {
            let color = if (cx + cy) % 2 == 0 { "#3c3c3c" } else { "#c8c8c8" };
            s.execute("select.rect", json!({"x": 6 + 5 * cx, "y": 6 + 5 * cy, "width": 5, "height": 5})).unwrap();
            s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        }
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// One sampled click at 30,24 with source 16,16 on a fresh `sampled_document`, plus the `extra`
/// keys under test. The run checks its own stroke first - the frame moved, the click's own pixel
/// moved off the ground, and it is one "Healing Brush" step - and returns the whole frame with the
/// History length, which is what the comparisons of two runs then agree on.
fn sampled_run(extra: Value) -> (Vec<Vec<f32>>, usize) {
    let mut s = sampled_document();
    let before = state(&s);
    let mut p = json!({"points": [[30, 24]], "source": [16, 16], "size": 20, "hardness": 100});
    for (key, value) in extra.as_object().unwrap() {
        p[key] = value.clone();
    }
    s.execute("paint.healingBrush", p).unwrap();
    let after = state(&s);
    let at = usize::try_from(24 * 48 + 30).unwrap();
    assert_ne!(after.0, before.0, "the sampled stroke painted");
    assert_ne!(after.0[at], before.0[at], "the click's own pixel moved off the ground: {:?} -> {:?}", before.0[at], after.0[at]);
    assert_eq!(after.1, before.1 + 1, "one stroke, one step");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Healing Brush"), "the stroke's History name");
    after
}

#[test]
fn a_pattern_stroke_needs_no_sampled_source_and_heals_from_the_tile() {
    let mut s = document();
    let before = state(&s);
    let r = s.execute("paint.healingBrush", stroke("Heal tile", json!({}))).unwrap();
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Healing Brush"), "the stroke's History name");
    assert_eq!(s.active().unwrap().history.past_len(), before.1 + 1, "one stroke, one step");
    assert_ne!(state(&s).0, before.0, "the dab painted");
    // The heal solve blends the tile's light colours towards the dark ground: inside the dab the
    // pixels move off the ground towards the tile (this document has no sampled source at all, so
    // nothing else can explain them), and outside the tip the ground is untouched.
    let ground = [40, 160, 60];
    for (x, y) in [(30, 24), (32, 26), (28, 22), (26, 24)] {
        let p = levels(&s, x, y);
        let t = want(&s, x, y);
        for c in 0..3 {
            assert!(
                p[c] != ground[c] && (p[c] - t[c]).abs() < (ground[c] - t[c]).abs(),
                "({x}, {y}) channel {c}: {p:?} moved off the ground towards the tile {t:?}"
            );
        }
    }
    assert_eq!(&levels(&s, 43, 24)[..3], &ground[..], "right of the dab");
    assert_eq!(&levels(&s, 30, 37)[..3], &ground[..], "below the dab");
    // The healed result is not a straight copy of the tile either: that is Replace, below.
    assert_ne!(levels(&s, 30, 24), want(&s, 30, 24), "Normal still solves, it does not copy the tile");
    assert_eq!(r["damage"].as_array().map(Vec::len), Some(4), "the damage rect as the sampled stroke reports it");
}

#[test]
fn pattern_replace_lays_the_tile_pixels_bit_for_bit() {
    // Pattern x Replace was not measured in #12101 or #12104; the merged Replace branch is
    // source-agnostic, so the tile goes straight onto the layer. This pins exactly that, against
    // the tile's source pixels read from our own document (through the same render and write a
    // stamped pixel goes through), like the merged Replace tests read the sampled source.
    let mut s = document();
    let steps = s.active().unwrap().history.past_len();
    s.execute("paint.healingBrush", stroke("Heal tile", json!({"mode": "replace"}))).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one step");
    let ground = [40, 160, 60];
    let (mut exact, mut faded) = (0, 0);
    for dy in -9i32..=9 {
        for dx in -9i32..=9 {
            // Measured here, as the merged Replace test measured its own disc: of the 249 pixels
            // inside this engine's 20 px hard tip, 238 are the tile pixel byte for byte, and the
            // 11 where the antialiased rim's coverage fades stay between ground and tile without
            // returning to the ground. Pixels past the tip stay on the ground, checked below.
            if dx * dx + dy * dy >= 81 {
                continue;
            }
            let (x, y) = (30 + dx, 24 + dy);
            let (got, t) = (levels(&s, x, y), want(&s, x, y));
            if got == t {
                exact += 1;
                continue;
            }
            for c in 0..3 {
                assert!(
                    got[c] >= ground[c].min(t[c]) && got[c] <= ground[c].max(t[c]),
                    "({x}, {y}) channel {c}: {got:?} lies outside ground {ground:?} and tile {t:?}"
                );
            }
            assert_ne!(&got[..3], ground.as_slice(), "({x}, {y}): a rim pixel never goes back to the ground");
            faded += 1;
        }
    }
    assert_eq!((exact, faded), (238, 11), "the loop's own shape");
    // The four flat quadrants all land inside the dab, so the tile - not one flat colour - went in.
    let flat = |x: i32, y: i32| want(&s, x, y)[..3].to_vec();
    assert_eq!(
        [levels(&s, 26, 22), levels(&s, 34, 22), levels(&s, 26, 28), levels(&s, 34, 28)].map(|p| p[..3].to_vec()),
        [flat(26, 22), flat(34, 22), flat(26, 28), flat(34, 28)],
        "all four tile quadrants land in the dab"
    );
    assert_eq!(levels(&s, 43, 24), vec![40, 160, 60, 255], "outside the tip the ground stays");
}

#[test]
fn pattern_replace_rim_lays_the_tile_through_the_coverage() {
    // The same rim rule the merged Replace tests pin for a sampled source: hardness 50 lays the
    // tile strictly between the ground and the tile colour where its coverage is partial.
    let mut s = document();
    let mut p = stroke("Heal tile", json!({"mode": "replace"}));
    p["hardness"] = json!(50);
    s.execute("paint.healingBrush", p).unwrap();
    let ground = [40, 160, 60];
    // The tip's centre still holds the full copy.
    assert_eq!(levels(&s, 30, 24), want(&s, 30, 24), "centre: the tile straight in at full coverage");
    // 8 px out on a hardness-50 tip the coverage is partial.
    let rim = levels(&s, 38, 24);
    let t = want(&s, 38, 24);
    assert_ne!(t[..3], ground, "fixture check: the rim's tile pixel differs from the ground");
    for c in 0..3 {
        assert!(rim[c] > ground[c].min(t[c]) && rim[c] < ground[c].max(t[c]), "rim {rim:?} channel {c}: between ground {ground:?} and tile {t:?}, not either");
    }
    assert_ne!(rim, t, "the rim is not a hard copy to the tip edge");
}

#[test]
fn an_unknown_pattern_is_an_error_and_paints_nothing() {
    let mut s = document();
    let before = state(&s);
    let err = s.execute("paint.healingBrush", stroke("Not There", json!({}))).unwrap_err().to_string();
    assert!(err.contains("no pattern \"Not There\""), "{err}");
    assert_eq!(state(&s), before, "nothing painted, no step");
}

#[test]
fn an_unknown_source_mode_is_an_error_and_paints_nothing() {
    let mut s = document();
    let before = state(&s);
    for mode in ["brush", "Pattern", ""] {
        let mut p = stroke("Heal tile", json!({}));
        p["sourceMode"] = json!(mode);
        let err = s.execute("paint.healingBrush", p).unwrap_err().to_string();
        assert!(err.contains("unknown sourceMode"), "sourceMode {mode:?}: {err}");
        assert_eq!(state(&s), before, "sourceMode {mode:?}: nothing painted, no step");
    }
}

#[test]
fn a_non_string_source_mode_is_an_error_and_paints_nothing() {
    // A supplied `sourceMode` has to be a string: with a sampled source set, `"sourceMode": true`
    // may not quietly take the sampled branch, the way an unknown string already errors. A null one
    // stays the absent key.
    for mode in [json!(true), json!(3), json!(["pattern"]), json!({"mode": "pattern"})] {
        let mut s = sampled_document();
        let before = state(&s);
        let p = json!({"points": [[30, 24]], "source": [16, 16], "size": 20, "hardness": 100, "sourceMode": mode.clone()});
        let err = s.execute("paint.healingBrush", p).unwrap_err().to_string();
        assert!(err.contains("`sourceMode` must be a string"), "{mode}: {err}");
        assert_eq!(state(&s), before, "{mode}: nothing painted, no step");
    }
    let plain = sampled_run(json!({}));
    assert_eq!(sampled_run(json!({"sourceMode": Value::Null})), plain, "a null sourceMode is the absent key");
}

#[test]
fn sampled_source_mode_keeps_the_sampled_stroke_byte_for_byte() {
    // The guard, at the command: with `sourceMode` absent or "sampled" the pattern key changes
    // nothing - the sampled stroke heals the same whole frame as before the option existed. The
    // fixture's source differs from the ground it heals, so each run also has to move pixels: a
    // sampled path that painted nothing fails inside `sampled_run`, not only in the comparisons.
    let plain = sampled_run(json!({}));
    assert_eq!(sampled_run(json!({"sourceMode": "sampled"})), plain, "sourceMode:\"sampled\" heals exactly like today's stroke");
    assert_eq!(sampled_run(json!({"sourceMode": "sampled", "pattern": "Heal tile"})), plain, "a pattern key on an explicit sampled stroke changes nothing");
    // A name that does not exist is not even resolved on the sampled path.
    assert_eq!(sampled_run(json!({"pattern": "Not There"})), plain, "the absent sourceMode is the sampled stroke, unknown pattern and all");
}

#[test]
fn a_pattern_stroke_is_not_drawn_as_a_live_clone_preview() {
    // The live retouching stroke paints its clone source; with a pattern source that texture is
    // not the tile, so the pattern stroke refuses the live preview and commits through the
    // command instead - and still paints.
    let s = document();
    let p = stroke("Heal tile", json!({}));
    let err = crate::retouch_cmds::LiveRetouch::begin(&s, "paint.healingBrush", &p).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("not drawn as live strokes"), "{err}");
    let mut r = document();
    let steps = r.active().unwrap().history.past_len();
    r.execute("paint.healingBrush", p).unwrap();
    assert_eq!(r.active().unwrap().history.past_len(), steps + 1, "the refused live preview does not break the stroke");
}

#[test]
fn pattern_by_id_and_the_document_tile_work_and_the_clone_source_stays_unset() {
    let mut s = document();
    let id = pattern_id(&s);
    // The id resolves like the name; the aligned phase follows the Pattern Stamp (document origin).
    s.execute("paint.healingBrush", stroke(&id, json!({"mode": "replace"}))).unwrap();
    assert_eq!(levels(&s, 30, 24), want(&s, 30, 24), "the id names the same tile");
    // A pattern stroke leaves the Clone Source slot alone: a following sampled stroke still
    // refuses, as it did before the pattern stroke ran.
    let err = s.execute("paint.healingBrush", json!({"points": [[10, 10]], "size": 8, "hardness": 100})).unwrap_err().to_string();
    assert!(err.contains("missing `source`"), "the slot stayed empty: {err}");
}

#[test]
fn a_pattern_stroke_under_pattern_preview_repeats_like_the_sampled_one() {
    // View > Pattern Preview (`wrap`) with a pattern source was not measured; this pins what the
    // code gives: the tile patch spans the stroke's padded bounds, the shared periodic composite
    // crops its seam copies out of it, and the stroke repeats across the canvas as it does for a
    // sampled source. Measured on this branch, hence the exact damage rect.
    let mut s = document();
    let before = state(&s);
    let mut p = stroke("Heal tile", json!({"wrap": true}));
    p["points"] = json!([[46, 24]]);
    let r = s.execute("paint.healingBrush", p).unwrap();
    assert_eq!(r["damage"], json!([0, 11, 48, 27]), "the stroke and its seam copies");
    assert_eq!(s.active().unwrap().history.past_len(), before.1 + 1, "one step");
    // The copy wrapped to the left edge painted there too: off the ground, like the dab itself.
    let left = levels(&s, 1, 24);
    assert_ne!(left[..3], [40, 160, 60], "the seam copy reached the left edge");
    assert_ne!(state(&s).0, before.0, "the stroke painted");
}
