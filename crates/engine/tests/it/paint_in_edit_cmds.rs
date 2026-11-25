//! Tests that run paint commands, moved from the engine-core `edit_cmds` tests: 11 test(s).

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

fn active_bounds(s: &Session) -> Rect {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
}

/// #368: the clipboard as a document of its own, from New from Clipboard or a Paste with
/// nothing open, at every depth.
#[test]
fn new_from_clipboard_and_paste_with_no_document() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        assert!(!s.is_enabled("file.newFromClipboard") && !s.is_enabled("edit.paste"), "nothing copied yet");
        assert!(s.execute("file.newFromClipboard", json!({})).is_err());
        s.execute("file.new", json!({"width": 100, "height": 100, "depth": depth})).unwrap();
        assert!(!s.active().unwrap().is_dirty(), "an untouched blank document is clean");
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 20, "width": 30, "height": 15})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        // With a document open: a second document, sized to the copy, its pixels at the origin.
        let r = s.execute("file.newFromClipboard", json!({})).unwrap();
        assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(30), Some(15)), "{r}");
        assert_eq!(s.documents().len(), 2);
        assert!(s.active().unwrap().is_dirty(), "a document made from clipboard pixels needs saving (#4665)");
        let d = &s.active().unwrap().doc;
        assert_eq!((d.size.width, d.size.height, d.layers.len()), (30, 15, 1));
        assert_eq!(d.pixel_format().sample, s.documents()[0].doc.pixel_format().sample, "{depth}-bit kept");
        assert_eq!(active_bounds(&s), Rect::new(0, 0, 30, 15));
        let px = d.layers[0].surface().unwrap().rgba(0, 0);
        assert!(px[0] > 0.99 && px[1] < 0.01 && px[3] > 0.99, "{px:?}");
        // With nothing open, Paste makes the document too.
        while s.active().is_some() {
            s.execute("file.close", json!({})).unwrap();
        }
        assert!(s.is_enabled("edit.paste") && !s.is_enabled("edit.pasteSpecial.pasteInPlace"));
        s.execute("edit.paste", json!({})).unwrap();
        assert_eq!(s.documents().len(), 1);
        assert!(s.active().unwrap().is_dirty(), "Paste with no tab also needs a save prompt");
        assert_eq!(active_bounds(&s), Rect::new(0, 0, 30, 15));
        assert!(s.execute("edit.pasteSpecial.pasteInPlace", json!({})).is_ok(), "into the new document");
    }
}

/// #2901 (P10/P11): Paste, Paste in Place and Paste Into recomposite only where the pasted
/// layer draws, not the whole document; nothing outside that changes, and undo brings back
/// exactly the old pixels.
#[test]
fn paste_damages_only_the_pasted_layer() {
    let render = |s: &Session| photocraft_compose::render(&s.active().unwrap().doc, s.active().unwrap().doc.bounds());
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 60, "height": 50, "depth": depth, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        // Half transparent, so an opaque copy pasted in place over it shows.
        s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
        for (id, p) in [
            ("edit.paste", json!({"center": [40, 30]})),
            ("edit.pasteSpecial.pasteInPlace", json!({})),
            ("edit.pasteSpecial.pasteInto", json!({"center": [40, 30]})),
        ] {
            s.execute("select.rect", json!({"x": 36, "y": 27, "width": 6, "height": 4})).unwrap();
            let shown = render(&s);
            s.execute(id, p).unwrap();
            let st = s.active().unwrap();
            let area = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
            let damage = st.last_damage.unwrap_or_else(|| panic!("{depth}-bit {id}: bounded damage"));
            assert!(!damage.is_empty() && damage.intersect(&area) == damage, "{depth}-bit {id}: {damage:?} within {area:?}");
            let now = render(&s);
            assert_ne!(now, shown, "{depth}-bit {id}: the paste shows");
            let w = shown.rect.width() as usize;
            for (i, (a, b)) in shown.px.iter().zip(&now.px).enumerate() {
                if !damage.contains((i % w) as i32, (i / w) as i32) {
                    assert_eq!(a, b, "{depth}-bit {id}: pixel {i} outside the damage changed");
                }
            }
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(render(&s), shown, "{depth}-bit {id}: undo");
        }
    }
}

/// #1035: a `depth`-bit document with a blue-grey square copied from layer `src` (at
/// (5,5)-(15,15)) and an active layer `m` above it with a hide-all mask.
fn mask_target_session(depth: u32) -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth})).unwrap();
    let src = LayerId(s.execute("layer.new.layer", json!({"name": "src"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let m = LayerId(s.execute("layer.new.layer", json!({"name": "m"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    (s, src, m)
}

/// The luminosity of `src`'s pixel at (`x`, `y`): what a paste into a mask writes there.
fn luminosity(s: &Session, src: LayerId, x: i32, y: i32) -> f32 {
    let p = s.active().unwrap().doc.layer(src).unwrap().surface().unwrap().rgba(x, y);
    photocraft_color::convert::rgb_to_gray([p[0], p[1], p[2]])
}

fn mask_at(s: &Session, id: LayerId, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().value(x, y)
}

#[test]
fn paste_into_a_targeted_mask_writes_the_luminosity_in_one_step() {
    for depth in [8, 16, 32] {
        let (mut s, src, m) = mask_target_session(depth);
        let lum = luminosity(&s, src, 10, 10);
        assert!(lum > 0.05 && lum < 0.95, "{depth}-bit: a mid grey ({lum})");
        let (layers, steps) = (s.active().unwrap().doc.layer_count(), s.active().unwrap().history.past_len());
        s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": "mask"})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer_count(), layers, "{depth}-bit: no new layer");
        assert_eq!(st.history.past_len(), steps + 1, "{depth}-bit: one history step");
        assert_eq!(st.active_layer, Some(m));
        assert!((mask_at(&s, m, 10, 10) - lum).abs() < 2.0 / 255.0, "{depth}-bit: {} vs {lum}", mask_at(&s, m, 10, 10));
        assert_eq!(mask_at(&s, m, 20, 20), 0.0, "{depth}-bit: the mask outside the paste is unchanged");
        assert!(st.doc.layer(m).unwrap().surface().unwrap().content_bounds().is_empty(), "{depth}-bit: the pixels are untouched");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(mask_at(&s, m, 10, 10), 0.0, "{depth}-bit: undo restores the mask");
    }
}

#[test]
fn copy_with_the_mask_targeted_takes_the_mask_as_grayscale() {
    for depth in [8, 16, 32] {
        let (mut s, _, m) = mask_target_session(depth);
        s.edit("paint mask", |doc, _| {
            doc.layer_mut(m).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(6, 6, 16, 12), &[0.75]);
            Ok(())
        })
        .unwrap();
        let steps = s.active().unwrap().history.past_len();
        let r = s.execute("edit.copy", json!({"target": "mask"})).unwrap();
        assert_eq!(r["bounds"], json!([0, 0, 40, 40]), "{depth}-bit: a mask copies whole, as in Photoshop");
        assert_eq!(s.active().unwrap().history.past_len(), steps, "{depth}-bit: copy adds no history step");
        let clip = s.clipboard.as_ref().cloned().unwrap();
        let px = clip.surface.rgba(10, 10);
        assert!((px[0] - 0.75).abs() < 2.0 / 255.0, "{depth}-bit: {} vs 0.75", px[0]);
        assert!(clip.surface.rgba(30, 30)[0] < 0.01, "{depth}-bit: the hidden area copies as black");
        assert!(px[3] > 0.99, "{depth}-bit: the copy is opaque");
        // Paste it into another layer's reveal-all mask: it replaces that mask (the request
        // in #2928): the square, and the black around it, land.
        let m2 = LayerId(s.execute("layer.new.layer", json!({"name": "m2"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": "mask"})).unwrap();
        assert!((mask_at(&s, m2, 10, 10) - 0.75).abs() < 2.0 / 255.0, "{depth}-bit: the copied mask lands");
        assert!(mask_at(&s, m2, 30, 30) < 0.01, "{depth}-bit: the copied black replaces the reveal-all");
    }
}

#[test]
fn an_untouched_mask_copies_the_whole_canvas_at_its_default() {
    let (mut s, _, _) = mask_target_session(8);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let r = s.execute("edit.copy", json!({"target": "mask"})).unwrap();
    assert_eq!(r["bounds"], json!([0, 0, 40, 40]), "a plain Reveal All mask still transfers");
    assert!(s.clipboard.as_ref().cloned().unwrap().surface.rgba(20, 20)[0] > 0.99);
}

#[test]
fn mask_copy_takes_only_the_selection() {
    let (mut s, _, m) = mask_target_session(8);
    s.edit("paint mask", |doc, _| {
        doc.layer_mut(m).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(6, 6, 16, 12), &[0.75]);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
    let r = s.execute("edit.copy", json!({"target": "mask"})).unwrap();
    assert_eq!(r["bounds"], json!([8, 8, 4, 4]));
    let clip = s.clipboard.as_ref().cloned().unwrap();
    assert!((clip.surface.rgba(9, 9)[0] - 0.75).abs() < 2.0 / 255.0);
    assert!(clip.surface.rgba(5, 5)[3] < 0.01, "nothing outside the selection");
}

#[test]
fn a_copied_mask_travels_to_another_document() {
    let (mut s, _, m) = mask_target_session(8);
    s.edit("paint mask", |doc, _| {
        doc.layer_mut(m).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(6, 6, 16, 12), &[0.75]);
        Ok(())
    })
    .unwrap();
    s.execute("edit.copy", json!({"target": "mask"})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
    let n = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": "mask"})).unwrap();
    assert!((mask_at(&s, n, 10, 10) - 0.75).abs() < 2.0 / 255.0, "the mask arrived in the second document");
    assert!(mask_at(&s, n, 30, 30) < 0.01, "it replaced the reveal-all mask there");
}

#[test]
fn paste_follows_the_mask_view_and_is_placed_like_a_paste() {
    let (mut s, src, m) = mask_target_session(8);
    let lum = luminosity(&s, src, 10, 10);
    // ⌥-click the mask thumbnail: the canvas shows the mask, and pasting targets it.
    s.execute(photocraft_engine::mask_view_cmds::ID, json!({"layer": m.0, "mode": "gray"})).unwrap();
    let r = s.execute("edit.paste", json!({"center": [30, 30]})).unwrap();
    assert_eq!(r["offset"], json!([20, 20]));
    assert!((mask_at(&s, m, 30, 30) - lum).abs() < 2.0 / 255.0);
    assert_eq!(mask_at(&s, m, 10, 10), 0.0);
    assert_eq!(s.active().unwrap().doc.layers.len(), 3, "Background, src, m");
}

#[test]
fn paste_into_and_outside_a_targeted_mask_keep_to_the_selection() {
    let (mut s, src, m) = mask_target_session(16);
    let lum = luminosity(&s, src, 10, 10);
    // The paste is centred on the selection, so lands in place: only the selection takes it.
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
    s.execute("edit.pasteSpecial.pasteInto", json!({"target": "mask"})).unwrap();
    assert!((mask_at(&s, m, 9, 9) - lum).abs() < 2.0 / 255.0);
    assert_eq!(mask_at(&s, m, 7, 7), 0.0, "pasted, but outside the selection");
    assert!(s.active().unwrap().doc.selection.is_none());
    assert_eq!(s.active().unwrap().doc.layers.len(), 3, "no new layer");
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
    s.execute("edit.pasteSpecial.pasteOutside", json!({"target": "mask"})).unwrap();
    assert_eq!(mask_at(&s, m, 9, 9), 0.0, "Paste Outside leaves the selection alone");
    assert!((mask_at(&s, m, 7, 7) - lum).abs() < 2.0 / 255.0);
}

#[test]
fn paste_follows_a_targeted_alpha_channel() {
    let (mut s, src, _) = mask_target_session(8);
    let lum = luminosity(&s, src, 10, 10);
    s.execute("channel.new", json!({})).unwrap();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": {"channel": 0}})).unwrap();
    let c = &s.active().unwrap().doc.channels[0].surface;
    assert!((c.sample_channel(10, 10, 0) - lum).abs() < 2.0 / 255.0);
    assert_eq!(s.active().unwrap().doc.layers.len(), 3, "no new layer");
}

#[test]
fn paste_into_a_missing_mask_or_channel_is_an_error() {
    let (mut s, _, m) = mask_target_session(8);
    s.edit("drop mask", |doc, _| {
        doc.layer_mut(m).unwrap().mask = None;
        Ok(())
    })
    .unwrap();
    let layers = s.active().unwrap().doc.layer_count();
    for target in [json!("mask"), json!({"channel": 7}), json!("quickMask")] {
        for id in ["edit.paste", "edit.pasteSpecial.pasteInPlace"] {
            assert!(s.execute(id, json!({ "target": target })).is_err(), "{id} {target}");
        }
    }
    assert_eq!(s.active().unwrap().doc.layer_count(), layers, "nothing pasted as a layer");
}
