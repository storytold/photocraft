//! #9329 PR A: a pixel change damages only the tiles that changed, and command rects are hints
//! checked against them.

use super::*;
use photocraft_geom::TILE_SIZE;

fn session(w: u32, h: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": w, "height": h})).unwrap();
    s
}

fn doc(s: &Session) -> std::sync::Arc<Document> {
    s.active().unwrap().doc.clone()
}

fn damage(s: &Session) -> Option<Rect> {
    s.active().unwrap().last_damage
}

/// The composite of `after` equals `before`'s outside `damage`.
fn unchanged_outside(before: &Document, after: &Document, damage: Rect) -> bool {
    let (x, y) = (photocraft_compose::render(before, before.bounds()), photocraft_compose::render(after, after.bounds()));
    let w = before.bounds().width() as i32;
    x.px.iter().zip(&y.px).enumerate().all(|(i, (p, q))| p == q || damage.contains(i as i32 % w, i as i32 / w))
}

/// The tiles `r` overlaps, as one rect.
fn tiles_of(r: Rect) -> Rect {
    let down = |v: i32| v.div_euclid(TILE_SIZE) * TILE_SIZE;
    Rect::new(down(r.x0), down(r.y0), down(r.x1 - 1) + TILE_SIZE, down(r.y1 - 1) + TILE_SIZE)
}

/// A new pixel layer with `rects` filled opaque, through one plain edit.
fn layer_with(s: &mut Session, rects: &[(Rect, [f32; 4])]) -> LayerId {
    let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("paint", |doc, _| {
        let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        for (r, px) in rects {
            surf.fill_rect(*r, px);
        }
        Ok(())
    })
    .unwrap();
    id
}

fn full_layer(s: &mut Session) -> LayerId {
    let canvas = doc(s).bounds();
    layer_with(s, &[(canvas, [0.4, 0.5, 0.6, 1.0])])
}

/// One plain edit of layer `id`'s pixels, returning the documents on either side.
fn paint(s: &mut Session, id: LayerId, r: Rect, px: [f32; 4]) -> (std::sync::Arc<Document>, std::sync::Arc<Document>) {
    let before = doc(s);
    s.edit("paint", |doc, _| {
        doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(r, &px);
        Ok(())
    })
    .unwrap();
    (before, doc(s))
}

/// The damage `note_damage` used to report after a move: both extents of the moved layer.
#[test]
fn a_move_still_damages_both_extents() {
    let mut s = session(1024, 768);
    let r = Rect::new(100, 100, 200, 200);
    let id = layer_with(&mut s, &[(r, [1.0, 0.0, 0.0, 1.0])]);
    let before = doc(&s);
    s.execute("layer.translate", json!({"layer": id.0, "dx": 300, "dy": 50})).unwrap();
    let after = doc(&s);
    let canvas = after.bounds();
    let extents = |d: &Document| photocraft_compose::change_bounds(d.layer(id).unwrap(), canvas).unwrap();
    let both = extents(&before).union(&extents(&after));
    assert_eq!(both, r.union(&r.translate(300, 50)));
    assert_eq!(damage(&s), Some(both));
    assert!(unchanged_outside(&before, &after, both));
}

#[test]
fn set_props_damage_is_unchanged_without_note_damage() {
    let r = Rect::new(100, 100, 200, 200);
    for props in [json!({"opacity": 0.5}), json!({"fill": 0.5}), json!({"blend": "multiply"}), json!({"visible": false})] {
        let mut s = session(1024, 768);
        let id = layer_with(&mut s, &[(r, [1.0, 0.0, 0.0, 0.8])]);
        let before = doc(&s);
        let mut p = props.clone();
        p["layer"] = json!(id.0);
        s.execute("layer.setProps", p).unwrap();
        assert_eq!(damage(&s), Some(r), "{props}");
        assert!(unchanged_outside(&before, &doc(&s), r), "{props}");
    }
}

#[test]
fn a_mask_edit_damages_its_tiles_grown_by_the_feather() {
    let mut s = session(1024, 768);
    let id = full_layer(&mut s);
    let feather = 10.0;
    s.edit("mask", |doc, _| {
        doc.layer_mut(id).unwrap().mask = Some(photocraft_doc::LayerMask { feather, ..photocraft_doc::LayerMask::reveal_all() });
        Ok(())
    })
    .unwrap();
    let before = doc(&s);
    let hole = Rect::new(600, 400, 610, 410);
    s.edit("mask paint", |doc, _| {
        doc.layer_mut(id).unwrap().mask.as_mut().unwrap().surface.fill_rect(hole, &[0.0]);
        Ok(())
    })
    .unwrap();
    let after = doc(&s);
    let d = step_damage(&before, &after).unwrap();
    let grown = photocraft_vector::feather::grow(tiles_of(hole), photocraft_vector::feather::feather_sigma(feather));
    assert_eq!(d, grown.intersect(&after.bounds()));
    assert!(d.contains_rect(&tiles_of(hole)) && d != after.bounds(), "{d:?}");
    assert!(unchanged_outside(&before, &after, d));
}

#[test]
fn tile_damage_grows_by_the_layer_effect_margin() {
    let mut s = session(1024, 768);
    let (a, b) = (Rect::new(50, 50, 150, 150), Rect::new(600, 400, 700, 500));
    let id = layer_with(&mut s, &[(a, [1.0, 0.0, 0.0, 1.0]), (b, [1.0, 0.0, 0.0, 1.0])]);
    s.edit("fx", |doc, _| {
        doc.layer_mut(id).unwrap().effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        Ok(())
    })
    .unwrap();
    let m = photocraft_compose::effects::margin(doc(&s).layer(id).unwrap());
    assert!(m > 0);
    let (before, after) = paint(&mut s, id, Rect::new(640, 440, 660, 460), [0.0, 0.0, 0.0, 0.0]);
    let d = step_damage(&before, &after).unwrap();
    let whole = photocraft_compose::change_bounds(after.layer(id).unwrap(), after.bounds()).unwrap();
    assert_eq!(d, tiles_of(b).inflate(m).intersect(&whole));
    assert!(d.x0 < tiles_of(b).x0 && d.y0 < tiles_of(b).y0, "the shadow reaches past the tile: {d:?}");
    assert!(unchanged_outside(&before, &after, d));
}

#[test]
fn painting_a_clip_base_under_a_styled_clipped_layer_is_covered() {
    let mut s = session(1024, 768);
    let base = layer_with(&mut s, &[(Rect::new(500, 300, 800, 600), [0.0, 1.0, 0.0, 1.0])]);
    let top = full_layer(&mut s);
    s.edit("clip", |doc, _| {
        let l = doc.layer_mut(top).unwrap();
        l.clipped = true;
        l.effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        Ok(())
    })
    .unwrap();
    let (before, after) = paint(&mut s, base, Rect::new(300, 200, 340, 240), [0.0, 0.0, 1.0, 1.0]);
    let d = damage(&s).expect("a bounded base edit");
    assert!(unchanged_outside(&before, &after, d), "the clipped layer changed outside {d:?}");
    let (before, after) = paint(&mut s, base, Rect::new(640, 440, 660, 460), [0.0, 0.0, 0.0, 0.0]);
    let d = damage(&s).expect("a bounded base edit");
    assert!(unchanged_outside(&before, &after, d), "the clipped layer changed outside {d:?}");
    // At a tile's edge too: the clipped layer's shadow is built from its own shape, and the base
    // only masks it pixel by pixel, so a base edit changes nothing past its own tiles.
    let (before, after) = paint(&mut s, base, Rect::new(3 * TILE_SIZE - 6, 440, 3 * TILE_SIZE, 460), [0.0, 0.0, 0.0, 0.0]);
    let d = damage(&s).expect("a bounded base edit");
    assert!(d.x1 <= 3 * TILE_SIZE, "{d:?}");
    assert!(unchanged_outside(&before, &after, d), "the clipped layer changed outside {d:?}");
}

/// #4053: effects laid out over the layer's frame repaint it everywhere when it grows.
#[test]
fn a_frame_spanning_effect_still_redraws_its_whole_frame() {
    use photocraft_doc::{Effect, FxCommon, Gradient};
    let mut s = session(1024, 768);
    let id = layer_with(&mut s, &[(Rect::new(100, 100, 700, 200), [1.0, 0.0, 0.0, 1.0])]);
    s.edit("fx", |doc, _| {
        doc.layer_mut(id).unwrap().effects.items.push(Effect::GradientOverlay {
            common: FxCommon::new(photocraft_doc::BlendMode::Normal, 1.0),
            gradient: Gradient::default(),
            dither: false,
            interpolation: None,
        });
        Ok(())
    })
    .unwrap();
    let (before, after) = paint(&mut s, id, Rect::new(700, 100, 800, 200), [0.0, 1.0, 0.0, 1.0]);
    let d = step_damage(&before, &after).unwrap();
    let extents = |d: &Document| photocraft_compose::change_bounds(d.layer(id).unwrap(), d.bounds()).unwrap();
    let whole = extents(&before).union(&extents(&after));
    assert!(whole.contains_rect(&Rect::new(100, 100, 800, 200)), "{whole:?}");
    assert_eq!(d, whole);
    assert!(unchanged_outside(&before, &after, d));
}

#[test]
fn refreshing_type_layers_damages_only_those_layers() {
    let mut s = session(1500, 1000);
    let one = LayerId(s.execute("type.create", json!({"x": 100, "y": 100, "text": "One", "size": 20})).unwrap()["layer"].as_u64().unwrap());
    let two = LayerId(s.execute("type.create", json!({"x": 1000, "y": 800, "text": "Two", "size": 20})).unwrap()["layer"].as_u64().unwrap());
    let before = doc(&s);
    let busy = s.refresh_type_layers(&[crate::TypeLayerRef { doc: before.id, layer: one, first_snapshot: false }]);
    assert!(busy.is_empty());
    let after = doc(&s);
    let d = damage(&s).expect("a bounded refresh");
    let bounds = |d: &Document, id: LayerId| photocraft_compose::change_bounds(d.layer(id).unwrap(), d.bounds()).unwrap();
    assert!(bounds(&before, one).union(&bounds(&after, one)).contains_rect(&d), "{d:?}");
    assert!(d.intersect(&bounds(&after, two)).is_empty(), "{d:?}");
    assert!(unchanged_outside(&before, &after, d));
}

#[test]
fn a_hint_that_misses_a_changed_tile_is_counted() {
    let mut s = session(1024, 768);
    let id = full_layer(&mut s);
    paint(&mut s, id, Rect::new(600, 400, 620, 420), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(s.damage_misses, 0);
    s.hint_damage(Rect::new(0, 0, 10, 10));
    assert_eq!(s.damage_misses, 1);
    assert_eq!(damage(&s), Some(Rect::EMPTY));
    // A hint covering the change is not a miss.
    paint(&mut s, id, Rect::new(600, 400, 620, 420), [0.0, 1.0, 0.0, 1.0]);
    s.hint_damage(Rect::new(600, 400, 620, 420));
    assert_eq!(s.damage_misses, 1);
    assert_eq!(damage(&s), Some(Rect::new(600, 400, 620, 420)));
}

// PR C (#9329): damage spans a command, not a revision.

/// The damage since the command began: measured from the revision before it, bounded, and
/// covering every pixel the command changed.
fn command_damage(s: &mut Session, cmd: &str, p: serde_json::Value) -> (std::sync::Arc<Document>, Rect) {
    let (before, rev) = (doc(s), s.active().unwrap().revision);
    s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
    let st = s.active().unwrap();
    assert!(st.revision - rev >= 2, "{cmd}: several revisions ({} → {})", rev, st.revision);
    assert_eq!(st.damage_from(), rev, "{cmd}: measured from before the command");
    let d = st.last_damage.unwrap_or_else(|| panic!("{cmd}: bounded damage"));
    assert!(unchanged_outside(&before, &st.doc, d), "{cmd}: pixels changed outside {d:?}");
    (before, d)
}

#[test]
fn paste_into_reports_damage_since_the_command_began() {
    let mut s = session(1024, 768);
    layer_with(&mut s, &[(Rect::new(0, 0, 1024, 768), [0.2, 0.4, 0.6, 1.0]), (Rect::new(40, 40, 120, 120), [0.9, 0.1, 0.1, 1.0])]);
    s.execute("select.rect", json!({"x": 40, "y": 40, "width": 100, "height": 100})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 700, "y": 500, "width": 120, "height": 90})).unwrap();
    let (_, d) = command_damage(&mut s, "edit.pasteSpecial.pasteInto", json!({}));
    assert!(!d.intersect(&Rect::new(700, 500, 820, 590)).is_empty(), "the pasted area: {d:?}");
    assert!(u64::from(d.width()) * u64::from(d.height()) * 4 < 1024 * 768, "the pasted area, not the canvas: {d:?}");
}

/// A pattern that an existing layer refers to (here a fill layer whose pattern was missing)
/// changes that layer's pixels when it is added: not bounded by the tile diff.
#[test]
fn a_new_pattern_a_layer_refers_to_is_unbounded() {
    let mut s = session(400, 300);
    s.execute("layer.newFillLayer.pattern", json!({"pattern": "Checkerboard"})).unwrap();
    // Without its pixel cache, so both sides render the fill from the library.
    let mut with = (*doc(&s)).clone();
    with.layers.iter_mut().for_each(|l| l.fill_cache = None);
    let mut without = with.clone();
    without.patterns.clear();
    assert!(!same_pixels(&without, &with), "the fill layer draws only with its pattern");
    assert_eq!(step_damage(&without, &with), None);
    // Unchanged libraries, and an unreferenced addition, stay pixel-equal.
    assert_eq!(step_damage(&with, &with), Some(Rect::EMPTY));
    let plain = doc(&session(400, 300));
    let mut more = (*plain).clone();
    more.patterns = with.patterns.clone();
    assert_eq!(step_damage(&plain, &more), Some(Rect::EMPTY));
    assert!(unchanged_outside(&plain, &more, Rect::EMPTY));
}

/// An appended pattern sharing its name with one a layer already resolves by id loses to that id
/// match (`pattern::find` tries ids first), so the layer draws as before: bounded, not a whole
/// redraw. A cross-document layer copy can bring such a pattern along.
#[test]
fn a_same_named_new_pattern_a_reference_still_resolves_past_is_bounded() {
    let mut s = session(400, 300);
    s.execute("layer.newFillLayer.pattern", json!({"pattern": "Checkerboard"})).unwrap();
    let mut with = (*doc(&s)).clone();
    with.layers.iter_mut().for_each(|l| l.fill_cache = None);
    let refers_by_id = with.layers.iter().any(|l| matches!(&l.content, LayerContent::Fill(photocraft_doc::Fill::Pattern { id, .. }) if !id.is_empty()));
    assert!(refers_by_id, "the fill layer refers to its pattern by id");
    let mut twin = with.patterns[0].clone();
    twin.id = "another-id".into();
    twin.surface.fill_rect(Rect::new(0, 0, twin.width as i32, twin.height as i32), &[1.0, 0.0, 1.0, 1.0]);
    let mut more = with.clone();
    more.patterns.push(twin);
    assert_eq!(step_damage(&with, &more), Some(Rect::EMPTY));
    assert!(unchanged_outside(&with, &more, Rect::EMPTY));
}

#[test]
fn a_failed_compound_reports_no_change() {
    let mut s = session(600, 400);
    let id = layer_with(&mut s, &[(Rect::new(0, 0, 50, 50), [1.0, 0.0, 0.0, 1.0])]);
    let (before, rev) = (doc(&s), s.active().unwrap().revision);
    let r: Result<()> = crate::analysis_cmds::compound(&mut s, "Both", |s| {
        paint(s, id, Rect::new(300, 200, 340, 240), [0.0, 1.0, 0.0, 1.0]);
        paint(s, id, Rect::new(10, 300, 40, 340), [0.0, 0.0, 1.0, 1.0]);
        Err(EngineError::Other("the second step failed".into()))
    });
    assert!(r.is_err());
    // What `Session::dispatch` does after any inline command.
    s.note_command_damage("test.compound", &before, rev);
    let st = s.active().unwrap();
    assert!(same_pixels(&before, &st.doc), "rolled back");
    assert_eq!((st.damage_from(), st.last_damage), (rev, Some(Rect::EMPTY)), "nothing changed since it began");
    assert_eq!(s.whole_redraws, 0);
}

#[test]
fn whole_document_redraws_of_small_edits_are_counted() {
    let mut s = session(1000, 1000);
    let id = layer_with(&mut s, &[(Rect::new(0, 0, 1000, 1000), [0.2, 0.4, 0.6, 1.0])]);
    // A synthetic command that paints 1 % and reports "everything".
    let rev = s.active().unwrap().revision;
    let (before, _) = paint(&mut s, id, Rect::new(10, 10, 110, 110), [1.0, 0.0, 0.0, 1.0]);
    s.active_mut().unwrap().last_damage = None;
    s.note_command_damage("test.wholeRedraw", &before, rev);
    assert_eq!(s.whole_redraws, 1);
    // A canvas-size change really does redraw everything: not counted.
    s.execute("image.canvasSize", json!({"width": 1200, "height": 900})).unwrap();
    assert_eq!(s.active().unwrap().last_damage, None);
    assert_eq!(s.whole_redraws, 1);
}
