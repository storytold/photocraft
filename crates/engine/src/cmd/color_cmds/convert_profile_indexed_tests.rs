//! #14262: Convert to Profile from an Indexed Color document to an RGB profile keeps it Indexed,
//! converts its colour table and maps every pixel to its entry's converted colour.

use super::*;

const RED: [u8; 3] = [200, 30, 30];
const BLUE: [u8; 3] = [30, 90, 200];
const GREEN: [u8; 3] = [40, 160, 70];

fn color_of(x: i32, y: i32) -> [u8; 3] {
    match (x < 4, y < 2) {
        (true, _) => RED,
        (false, true) => BLUE,
        (false, false) => GREEN,
    }
}

/// An 8×4 sRGB-tagged Indexed document of `depth` bits holding the three colours above.
fn indexed(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 4, "depth": depth, "background": "#c81e1e"})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let [r, g, b] = color_of(x, y).map(|v| f32::from(v) / 255.0);
                data.extend(photocraft_raster::from_rgba(&fmt, [r, g, b, 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false})).unwrap();
    assert_eq!(doc(&s).mode, ColorMode::Indexed);
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn table(s: &Session) -> Vec<[u8; 3]> {
    doc(s).color_table.as_ref().unwrap().colors.clone()
}

/// Every pixel as 8-bit RGB, row by row.
fn pixels(s: &Session) -> Vec<[u8; 3]> {
    let d = doc(s);
    let surf = d.layers[0].surface().unwrap();
    let b = d.bounds();
    (b.y0..b.y1)
        .flat_map(|y| (b.x0..b.x1).map(move |x| (x, y)))
        .map(|(x, y)| surf.rgba(x, y))
        .map(|c| [c[0], c[1], c[2]].map(|v| (v * 255.0).round() as u8))
        .collect()
}

fn to_adobe(s: &mut Session, extra: Value) -> Value {
    let mut p = json!({"profile": "adobe-rgb-compat", "intent": "relative", "bpc": true});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("edit.convertToProfile", p).unwrap()
}

#[test]
fn an_indexed_document_converted_to_an_rgb_profile_stays_indexed() {
    for depth in [8, 16] {
        let mut s = indexed(depth);
        let before = table(&s);
        let red = before.iter().position(|&c| c == RED).unwrap();
        let r = to_adobe(&mut s, json!({}));
        assert_eq!(r["mode"], "Indexed", "{depth}-bit: {r}");
        assert_eq!(doc(&s).mode, ColorMode::Indexed);
        let after = table(&s);
        assert_eq!(after.len(), before.len(), "{depth}-bit: the table keeps its length");
        for (k, want) in [171, 35, 35].into_iter().enumerate() {
            assert!(after[red][k].abs_diff(want) <= 1, "{depth}-bit: red entry {:?}", after[red]);
        }
        // Order kept: each entry is its own old colour converted.
        let t = Transform::new(Builtin::Srgb.profile(), Builtin::AdobeRgbCompat.profile(), Intent::RelativeColorimetric, true).unwrap();
        let old: Vec<u8> = before.iter().flatten().copied().collect();
        let mut want = vec![0u8; old.len()];
        t.convert_u8(&old, 3, &mut want, 3, false).unwrap();
        assert_eq!(after.iter().flatten().copied().collect::<Vec<_>>(), want, "{depth}-bit");
        // Each pixel is the converted entry of its original colour.
        let b = doc(&s).bounds();
        let expected: Vec<[u8; 3]> = (b.y0..b.y1)
            .flat_map(|y| (b.x0..b.x1).map(move |x| (x, y)))
            .map(|(x, y)| after[before.iter().position(|&c| c == color_of(x, y)).unwrap()])
            .collect();
        assert_eq!(pixels(&s), expected, "{depth}-bit: the image uses only table colours");
        assert_eq!(doc(&s).icc_profile, Some(Builtin::AdobeRgbCompat.profile().to_bytes()));
    }
}

#[test]
fn the_conversion_is_one_history_state_that_undo_reverts() {
    let mut s = indexed(8);
    let (entries, before_table, before_pixels, before_icc) = (s.active().unwrap().history.entries().len(), table(&s), pixels(&s), doc(&s).icc_profile.clone());
    to_adobe(&mut s, json!({}));
    let history = s.active().unwrap().history.entries();
    assert_eq!(history.len(), entries + 1);
    assert_eq!(history.last().map(String::as_str), Some("Convert to Profile"));
    s.undo();
    assert_eq!(doc(&s).mode, ColorMode::Indexed);
    assert_eq!(table(&s), before_table);
    assert_eq!(pixels(&s), before_pixels);
    assert_eq!(doc(&s).icc_profile, before_icc);
}

#[test]
fn dither_does_not_change_an_indexed_result() {
    let mut plain = indexed(8);
    to_adobe(&mut plain, json!({"dither": false}));
    let mut dithered = indexed(8);
    to_adobe(&mut dithered, json!({"dither": true}));
    assert_eq!(doc(&dithered).mode, ColorMode::Indexed);
    assert_eq!(table(&dithered), table(&plain));
    assert_eq!(pixels(&dithered), pixels(&plain));
}

#[test]
fn the_pixel_color_indices_still_hold_after_the_conversion() {
    let mut s = indexed(8);
    let layer = doc(&s).layers[0].id;
    assert!(doc(&s).color_indices.as_ref().is_some_and(|k| k.holds(layer, doc(&s).layers[0].surface().unwrap())), "recorded by Indexed Color");
    to_adobe(&mut s, json!({}));
    let d = doc(&s);
    assert!(d.color_indices.as_ref().is_some_and(|k| k.holds(layer, d.layers[0].surface().unwrap())));
}

#[test]
fn cmyk_gray_and_lab_profiles_keep_todays_result() {
    for (profile, mode) in [("coated-cmyk", ColorMode::Cmyk), ("sgray", ColorMode::Grayscale), ("lab-d50", ColorMode::Lab)] {
        let mut s = indexed(8);
        s.execute("edit.convertToProfile", json!({"profile": profile})).unwrap();
        assert_eq!(doc(&s).mode, mode, "{profile}");
        assert!(doc(&s).color_table.is_none(), "{profile}: the table no longer applies");
    }
}

#[test]
fn an_indexed_document_without_a_table_converts_without_a_panic() {
    for table in [None, Some(photocraft_doc::ColorTable::default())] {
        let mut s = indexed(8);
        s.edit("drop the table", |doc, _| {
            doc.color_table = table.clone();
            Ok(())
        })
        .unwrap();
        let r = s.execute("edit.convertToProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
        assert_eq!(r["mode"], "Rgb", "{r}");
    }
}

#[test]
fn image_mode_rgb_still_leaves_indexed() {
    let mut s = indexed(8);
    s.execute("image.mode.rgb", json!({"profile": "adobe-rgb-compat"})).unwrap();
    assert_eq!(doc(&s).mode, ColorMode::Rgb);
    assert!(doc(&s).color_table.is_none());
}

/// CodeRabbit on #16016: coordinates that share one tile still share one converted tile, so a large
/// filled document converts that tile once.
#[test]
fn shared_tiles_stay_shared() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2048, "height": 1024, "background": "#c81e1e"})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false})).unwrap();
    s.edit("share one tile", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let first = surf.tiles().next().map(|(_, t)| t.clone()).unwrap();
        let coords: Vec<_> = surf.tiles().map(|(c, _)| *c).collect();
        surf.put_tiles(coords.into_iter().map(|c| (c, first.clone())));
        Ok(())
    })
    .unwrap();
    let distinct = |s: &Session| {
        let surf = doc(s).layers[0].surface().unwrap();
        let mut seen: Vec<_> = surf.tiles().map(|(_, t)| std::sync::Arc::as_ptr(t)).collect();
        seen.sort();
        seen.dedup();
        (seen.len(), surf.tile_count())
    };
    let (before, count) = distinct(&s);
    assert!(count > 1 && before == 1, "{before} distinct of {count}");
    to_adobe(&mut s, json!({}));
    assert_eq!(distinct(&s), (1, count));
    assert_eq!(pixels(&s).first(), table(&s).first(), "the converted red");
}

/// CodeRabbit on #16016: a layer whose indices were recorded keeps them inside a group too.
#[test]
fn a_grouped_layer_keeps_its_color_indices() {
    let mut s = indexed(8);
    let layer = doc(&s).layers[0].id;
    s.execute("layer.groupLayers", json!({"layer": layer})).unwrap();
    assert!(doc(&s).layers.iter().all(|l| l.id != layer), "the layer is inside a group");
    assert!(doc(&s).color_indices.as_ref().is_some_and(|k| k.holds(layer, doc(&s).layer(layer).unwrap().surface().unwrap())));
    to_adobe(&mut s, json!({}));
    let d = doc(&s);
    assert!(d.color_indices.as_ref().is_some_and(|k| k.holds(layer, d.layer(layer).unwrap().surface().unwrap())));
}
