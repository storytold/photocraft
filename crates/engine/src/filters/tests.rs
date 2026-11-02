use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 32})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#3366cc"})).ok();
    s
}

fn active_pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    l.surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 48, 32))
}

fn mask_session(reveal_selection: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "#ff0000"})).unwrap();
    if reveal_selection {
        s.execute("select.all", json!({})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    } else {
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    }
    s
}

fn mask_pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    let mask = d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap();
    mask.surface.read_region(d.doc.bounds())
}

fn canvas_pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().read_region(d.doc.bounds())
}

fn paint_pattern(s: &mut Session) {
    s.edit("pattern", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        let surf = l.surface_mut().unwrap();
        for y in 0..32 {
            for x in 0..48 {
                let v = ((x * 5 + y * 3) % 17) as f32 / 16.0;
                surf.write_pixel(x, y, &[v, 1.0 - v, (x % 3) as f32 / 2.0, 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
}

const ALL: [&str; 24] = [
    "filter.blur.gaussianBlur",
    "filter.blur.boxBlur",
    "filter.blur.motionBlur",
    "filter.blur.radialBlur",
    "filter.blur.surfaceBlur",
    "filter.sharpen.unsharpMask",
    "filter.sharpen.smartSharpen",
    "filter.noise.addNoise",
    "filter.noise.median",
    "filter.noise.dustAndScratches",
    "filter.pixelate.mosaic",
    "filter.stylize.emboss",
    "filter.stylize.findEdges",
    "filter.stylize.solarize",
    "filter.distort.twirl",
    "filter.distort.pinch",
    "filter.distort.spherize",
    "filter.distort.wave",
    "filter.distort.ripple",
    "filter.distort.polarCoordinates",
    "filter.other.highPass",
    "filter.other.minimum",
    "filter.other.maximum",
    "filter.other.offset",
];

#[test]
fn every_filter_command_runs_changes_pixels_and_undoes() {
    for id in ALL {
        let mut s = session();
        paint_pattern(&mut s);
        let before = active_pixels(&s);
        let params = match id {
            "filter.other.offset" => json!({"horizontal": 5, "vertical": 2}),
            "filter.noise.dustAndScratches" => json!({"radius": 2, "threshold": 0}),
            "filter.noise.median" => json!({"radius": 2}),
            _ => json!({}),
        };
        let r = s.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(r.get("filter").is_some(), "{id}");
        assert_ne!(active_pixels(&s), before, "{id} changed nothing");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(active_pixels(&s), before, "{id} undo");
    }
}

#[test]
fn preset_filters_record_their_own_name() {
    // #528: one-step presets run a generic algorithm but name the step after themselves;
    // the generic commands keep the algorithm's name.
    let specs = specs();
    for (id, name) in [
        ("filter.blur.blur", "Blur"),
        ("filter.blur.blurMore", "Blur More"),
        ("filter.sharpen.sharpen", "Sharpen"),
        ("filter.sharpen.sharpenMore", "Sharpen More"),
        ("filter.sharpen.sharpenEdges", "Sharpen Edges"),
        ("filter.noise.despeckle", "Despeckle"),
        ("filter.blur.gaussianBlur", "Gaussian Blur"),
        ("filter.sharpen.unsharpMask", "Unsharp Mask"),
        ("filter.blur.surfaceBlur", "Surface Blur"),
    ] {
        let mut s = session();
        let steps = s.active().unwrap().history.past_len();
        s.execute(id, json!({})).unwrap();
        let h = &s.active().unwrap().history;
        assert_eq!((h.past_len(), h.undo_label()), (steps + 1, Some(name)), "{id}");
        let label = specs.iter().find(|c| c.id == id).unwrap().label;
        assert_eq!(label.trim_end_matches('…'), name, "{id}: the step is named like the command");
    }
}

#[test]
fn specs_are_complete_and_documented() {
    let specs = specs();
    for id in ALL {
        let spec = specs.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("missing {id}"));
        assert!(spec.params.starts_with('{'), "{id}");
        assert!(!spec.menu.is_empty(), "{id}");
    }
}

#[test]
fn disabled_without_pixel_layer() {
    let mut s = Session::new();
    assert!(!s.is_enabled("filter.blur.gaussianBlur"));
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    assert!(matches!(s.execute("filter.blur.gaussianBlur", json!({})), Err(EngineError::Disabled(..))));
}

#[test]
fn selection_limits_the_filter() {
    let mut s = session();
    paint_pattern(&mut s);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let has_sel = s.active().unwrap().doc.selection.is_some();
    assert!(has_sel);
    let before = active_pixels(&s);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
    let after = active_pixels(&s);
    if has_sel {
        // A pixel far from the selection is unchanged.
        let i = (30 * 48 + 40) * 4;
        assert_eq!(after[i..i + 4], before[i..i + 4]);
        let j = (5 * 48 + 5) * 4;
        assert_ne!(after[j..j + 4], before[j..j + 4]);
    }
}

#[test]
fn solarize_and_add_noise_process_untouched_reveal_all_masks() {
    for (id, params) in [
        ("filter.stylize.solarize", json!({"target": "mask"})),
        ("filter.noise.addNoise", json!({"amount": 100, "seed": 1, "monochromatic": true, "target": "mask"})),
    ] {
        let mut reveal_all = mask_session(false);
        let mut reveal_selection = mask_session(true);
        let all_mask = reveal_all.active().unwrap().doc.layer(reveal_all.active().unwrap().active_layer.unwrap()).unwrap().mask.as_ref().unwrap();
        assert!(all_mask.surface.content_bounds().is_empty(), "{id}: Reveal All stores its white canvas in the default pixel");
        let initial_all = mask_pixels(&reveal_all);
        let initial_selection = mask_pixels(&reveal_selection);
        assert_eq!(initial_all, initial_selection, "{id}: masks start with equal white values");
        assert_eq!(initial_all, vec![1.0; 16 * 16], "{id}: the full canvas starts revealed");
        let pixels_before = canvas_pixels(&reveal_all);

        reveal_all.execute(id, params.clone()).unwrap();
        reveal_selection.execute(id, params).unwrap();

        let actual = mask_pixels(&reveal_all);
        let expected = mask_pixels(&reveal_selection);
        assert!(
            actual == expected,
            "{id}: Reveal All differs from Reveal Selection: first={:?}/{:?}, changed pixels={}/{}",
            actual[0],
            expected[0],
            actual.iter().filter(|&&v| v != 1.0).count(),
            expected.iter().filter(|&&v| v != 1.0).count()
        );
        assert_eq!(canvas_pixels(&reveal_all), pixels_before, "{id}: the mask target must leave layer pixels unchanged");
        if id == "filter.stylize.solarize" {
            assert!(actual.iter().all(|&v| v == 0.0), "Solarize maps the white mask to black");
        } else {
            assert!(actual.iter().any(|&v| v != 1.0), "seeded Add Noise changes the Reveal All mask");
        }
    }
}

#[test]
fn minimum_signed_offset_matches_its_wrapped_distance() {
    for depth in [8, 16, 32] {
        let make_session = || {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 48, "height": 32, "depth": depth})).unwrap();
            s.execute("layer.new.layer", json!({})).unwrap();
            paint_pattern(&mut s);
            s
        };
        let mut actual = make_session();
        let before = active_pixels(&actual);
        actual.execute("filter.other.offset", json!({"horizontal": i32::MIN, "vertical": i32::MIN})).unwrap();
        let mut expected = make_session();
        expected.execute("filter.other.offset", json!({"horizontal": i32::MIN.rem_euclid(48), "vertical": i32::MIN.rem_euclid(32)})).unwrap();
        let after = active_pixels(&actual);
        assert_eq!(after, active_pixels(&expected), "depth {depth}");
        assert_ne!(after, before);
        actual.execute("edit.undo", json!({})).unwrap();
        assert_eq!(active_pixels(&actual), before);
        actual.execute("edit.redo", json!({})).unwrap();
        assert_eq!(active_pixels(&actual), after);
    }
}
#[test]
fn huge_offsets_clamp_instead_of_wrapping() {
    // 3e9 wrapped to a negative offset through `as i32`; it saturates now, so it
    // equals the maximal in-range offset instead of shifting the other way.
    let mut a = session();
    a.execute("filter.other.offset", json!({"horizontal": 3_000_000_000_i64, "vertical": 0})).unwrap();
    let mut b = session();
    b.execute("filter.other.offset", json!({"horizontal": 2_147_483_647_i64, "vertical": 0})).unwrap();
    assert_eq!(active_pixels(&a), active_pixels(&b));
}

#[test]
fn last_filter_repeats_with_same_params() {
    let mut s = session();
    paint_pattern(&mut s);
    assert!(!s.is_enabled("filter.lastFilter"));
    s.execute("filter.other.offset", json!({"horizontal": 3})).unwrap();
    let once = active_pixels(&s);
    s.execute("filter.lastFilter", json!({})).unwrap();
    let twice = active_pixels(&s);
    assert_ne!(once, twice);
    // Offsetting 3 twice equals offsetting 6 once.
    let mut s2 = session();
    paint_pattern(&mut s2);
    s2.execute("filter.other.offset", json!({"horizontal": 6})).unwrap();
    assert_eq!(active_pixels(&s2), twice);
}

#[test]
fn recorded_last_filter_replays_the_filter_it_repeated() {
    // #3089: the action holds the filter with its settings, not a "repeat whatever ran
    // last" step that fails in a new session.
    let mut s = session();
    paint_pattern(&mut s);
    s.execute("filter.other.offset", json!({"horizontal": 3})).unwrap();
    s.execute("actions.record", json!({"name": "Again"})).unwrap();
    s.execute("filter.lastFilter", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Again"})).unwrap()["steps"].clone();
    assert_eq!(steps, json!([["filter.other.offset", {"horizontal": 3}]]));
    let action = s.actions.list[0].clone();

    let mut fresh = session();
    paint_pattern(&mut fresh);
    fresh.actions.list.push(action);
    let played = fresh.execute("actions.play", json!({"action": "Again"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let mut direct = session();
    paint_pattern(&mut direct);
    direct.execute("filter.other.offset", json!({"horizontal": 3})).unwrap();
    assert_eq!(active_pixels(&fresh), active_pixels(&direct));
}

#[test]
fn last_filter_applies_to_the_active_layer() {
    let mut s = session();
    paint_pattern(&mut s);
    let first = s.active().unwrap().active_layer.unwrap();
    // Dialogs (Filter Gallery, Camera Raw…) record the layer they ran on.
    s.execute("filter.other.offset", json!({"horizontal": 3, "layer": first.0})).unwrap();
    let first_once = active_pixels(&s);
    s.execute("layer.new.layer", json!({})).unwrap();
    paint_pattern(&mut s);
    let before = active_pixels(&s);
    s.execute("filter.lastFilter", json!({})).unwrap();
    assert_ne!(active_pixels(&s), before, "the active layer is filtered");
    let d = s.active().unwrap();
    let l = d.doc.layer(first).unwrap();
    assert_eq!(l.surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 48, 32)), first_once, "the first layer is left alone");
}

#[test]
fn params_map_to_algorithm_units() {
    assert_eq!(params_for("filter.blur.gaussianBlur", &json!({"radius": 4.5})), Some(FilterParams::GaussianBlur { radius: 4.5 }));
    assert_eq!(
        params_for("filter.blur.radialBlur", &json!({"quality": "best"})),
        Some(FilterParams::RadialBlur { amount: 10.0, method: RadialMethod::Spin, quality: RadialQuality::Best, center_x: 0.5, center_y: 0.5 })
    );
    assert_eq!(
        params_for("filter.noise.addNoise", &json!({"amount": 10, "distribution": "gaussian", "monochromatic": true})),
        Some(FilterParams::AddNoise { amount: 10.0, distribution: Distribution::Gaussian, monochromatic: true, seed: 0 })
    );
    assert_eq!(params_for("filter.blur.gaussianBlur", &json!({"radius": -3})), Some(FilterParams::GaussianBlur { radius: 0.1 }));
    assert!(params_for("filter.nope", &json!({})).is_none());
}

#[test]
fn minimum_maximum_preserve_roundness() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 48, "height": 32, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        // A white dot on black.
        s.edit("dot", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(photocraft_geom::Rect::new(0, 0, 48, 32), &[0.0, 0.0, 0.0, 1.0]);
            surf.write_pixel(24, 16, &[1.0, 1.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let before = active_pixels(&s);
        let lit = |s: &Session, x: usize, y: usize| active_pixels(s)[(y * 48 + x) * 4] > 0.5;
        s.execute("filter.other.maximum", json!({"radius": 4, "preserve": "roundness"})).unwrap();
        // Grown into a disc of radius 4: (4, 0) and (2, 3) are in, the square's corner isn't.
        assert!(lit(&s, 28, 16) && lit(&s, 26, 19), "depth {depth}");
        assert!(!lit(&s, 28, 20) && !lit(&s, 27, 19), "depth {depth}");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(active_pixels(&s), before);
        // The default is Squareness: the corner is taken.
        s.execute("filter.other.maximum", json!({"radius": 4})).unwrap();
        assert!(lit(&s, 28, 20), "depth {depth}");
        // Minimum with roundness: only the centre of that 9×9 square holds a whole disc.
        s.execute("filter.other.minimum", json!({"radius": 4, "preserve": "roundness"})).unwrap();
        assert!(!lit(&s, 20, 12) && lit(&s, 24, 16), "depth {depth}");
    }
    assert_eq!(
        params_for("filter.other.minimum", &json!({"radius": 2, "preserve": "roundness"})),
        Some(FilterParams::Minimum { radius: 2.0, preserve: Preserve::Roundness })
    );
    assert_eq!(params_for("filter.other.maximum", &json!({})), Some(FilterParams::Maximum { radius: 1.0, preserve: Preserve::Squareness }));
}

#[test]
fn smart_object_records_smart_filter() {
    use photocraft_doc::{Layer, SmartObject, SmartSource};
    let mut s = session();
    s.edit("smart", |doc, active| {
        let mut cache = photocraft_raster::Surface::new(doc.pixel_format());
        cache.fill_rect(photocraft_geom::Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
        let l = Layer::new(
            "so",
            LayerContent::Smart(SmartObject {
                contents_id: photocraft_doc::SmartContentsId::fresh(),
                source: SmartSource::Linked { path: String::new() },
                transform: photocraft_geom::Affine::IDENTITY,
                smart_filters: vec![],
                cache: Some(cache),
                psd_raw: None,
                vector_preview: None,
                filters_enabled: true,
                filter_mask: None,
                warp: None,
                stack_mode: None,
                perspective: None,
            }),
        );
        *active = Some(doc.insert_above(*active, l));
        Ok(())
    })
    .unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    let d = s.active().unwrap();
    let LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!() };
    assert_eq!(sm.smart_filters.len(), 1);
    assert_eq!(sm.smart_filters[0].command, "filter.blur.gaussianBlur");
    assert_eq!(sm.smart_filters[0].params["radius"], json!(2));
    assert!(sm.cache.as_ref().unwrap().pixel(10, 5)[3] > 0.0, "cache was blurred");
}

/// The serial `keep_alpha` before #6205, kept as the oracle for the parallel one.
fn keep_alpha_serial(old: &photocraft_raster::Surface, new: &mut photocraft_raster::Surface) {
    if old.format() != new.format() || !new.format().alpha {
        return;
    }
    let mut coords: Vec<_> = new.tiles().filter(|(c, t)| old.tile(**c).is_none_or(|o| !std::sync::Arc::ptr_eq(o, t))).map(|(c, _)| *c).collect();
    coords.extend(old.tiles().filter(|(c, _)| new.tile(**c).is_none()).map(|(c, _)| *c));
    let n = new.channels();
    for c in coords {
        let r = c.rect();
        let o = old.read_region(r);
        let mut v = new.read_region(r);
        for (pn, po) in v.chunks_exact_mut(n).zip(o.chunks_exact(n)) {
            if let (Some(a), Some(b)) = (pn.last_mut(), po.last()) {
                *a = *b;
            }
        }
        new.write_region(r, &v);
    }
    new.prune();
}

/// A layer before and after a filter: tiles the filter changed, one it left shared, one it
/// emptied (only in `old`) and one it added (only in `new`).
fn alpha_pair(sample: photocraft_color::SampleType) -> (photocraft_raster::Surface, photocraft_raster::Surface) {
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;
    let fmt = photocraft_color::PixelFormat { mode: photocraft_color::ColorMode::Rgb, sample, alpha: true };
    let pattern = |r: Rect, k: f32| -> Vec<f32> {
        (0..r.width() as usize * r.height() as usize * 4).map(|i| ((i as f32 * 0.37 + k).sin() * 0.5 + 0.5).clamp(0.0, 1.0)).collect()
    };
    let mut old = Surface::new(fmt);
    let r = Rect::new(-40, -20, 700, 300);
    old.write_region(r, &pattern(r, 0.0));
    let mut new = old.clone();
    let changed = Rect::new(-40, -20, 400, 300);
    new.write_region(changed, &pattern(changed, 1.3));
    let emptied = photocraft_geom::TileCoord::containing(600, 100).rect();
    drop(new.take_tiles(emptied));
    let added = Rect::new(800, 0, 900, 50);
    new.write_region(added, &pattern(added, 2.1));
    (old, new)
}

#[test]
fn keep_alpha_matches_the_serial_pass_at_every_depth() {
    use photocraft_color::SampleType;
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let (old, new) = alpha_pair(sample);
        let mut want = new.clone();
        keep_alpha_serial(&old, &mut want);
        let mut got = new.clone();
        keep_alpha(&old, &mut got, &|| false).unwrap();
        assert!(got == want, "{sample:?}: parallel keep_alpha differs from the serial pass");
        assert_ne!(got, new, "{sample:?}: alpha was put back");
    }
}

#[test]
fn keep_alpha_stops_when_cancelled_and_keeps_the_tiles() {
    let (old, new) = alpha_pair(photocraft_color::SampleType::U8);
    let mut got = new.clone();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2;
    assert!(matches!(keep_alpha(&old, &mut got, &cancel), Err(EngineError::Cancelled)));
    // The cancel was seen inside the pass (after some tiles), and no tile went missing.
    assert!(calls.load(std::sync::atomic::Ordering::SeqCst) > 2);
    assert!(new.tiles().all(|(c, _)| got.tile(*c).is_some()));
}

/// #7423: Displace on a Smart Object keeps its map (read from an open document) in the smart
/// filter, as Photoshop embeds it, so the filter displaces and keeps displacing when the
/// object re-renders. A map chosen again replaces it.
#[test]
fn displace_on_a_smart_object_embeds_its_map() {
    let mut s = Session::new();
    // Documents 0 and 1: a white and a black displacement map.
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "black"})).unwrap();
    s.execute("file.new", json!({"width": 48, "height": 32, "background": "transparent"})).unwrap();
    s.edit("square", |doc, active| {
        let fmt = doc.pixel_format();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(photocraft_geom::Rect::new(10, 10, 20, 12), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        Ok(())
    })
    .unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let original = {
        let d = s.active().unwrap();
        let LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("not a smart object") };
        sm.cache.as_ref().unwrap().read_region(d.doc.bounds())
    };
    let smart = |s: &Session| {
        let d = s.active().unwrap();
        let LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("not a smart object") };
        (sm.smart_filters[0].params.clone(), sm.cache.as_ref().unwrap().read_region(d.doc.bounds()))
    };
    let first_value = |params: &Value| photocraft_io::smart_map::displace_embedded_map(params).expect("an embedded map").data[0];

    s.execute("filter.distort.displace", json!({"horizontal": 10, "vertical": 0, "mapDocument": 0})).unwrap();
    let (params, displaced) = smart(&s);
    assert!(params.get("mapDocument").is_none(), "{params}");
    assert_eq!(first_value(&params), 1.0, "the white map");
    assert_ne!(displaced, original, "the smart object is displaced");
    // Re-rendered from its params alone (the filter hidden and shown again): still displaced.
    s.execute("layer.smartFilter.setVisible", json!({"index": 0, "visible": false})).unwrap();
    assert_eq!(smart(&s).1, original);
    s.execute("layer.smartFilter.setVisible", json!({"index": 0, "visible": true})).unwrap();
    assert_eq!(smart(&s).1, displaced);

    // Another map chosen for the filter replaces the embedded one.
    s.execute("layer.smartFilter.setParams", json!({"index": 0, "params": {"mapDocument": 1}})).unwrap();
    let (params, again) = smart(&s);
    assert!(params.get("mapDocument").is_none(), "{params}");
    assert_eq!(first_value(&params), 0.0, "the black map");
    assert_ne!(again, displaced, "displaced the other way");

    // The stored params passed back whole (the old embedded map included) with another map
    // named: the named map wins, for the filter's own edit and for a new Displace alike.
    let mut back = params.as_object().unwrap().clone();
    back.insert("mapDocument".into(), json!(0));
    s.execute("layer.smartFilter.setParams", json!({"index": 0, "params": back.clone()})).unwrap();
    assert_eq!(first_value(&smart(&s).0), 1.0, "the white map, not the embedded black one");
    back.insert("mapDocument".into(), json!(1));
    s.execute("filter.distort.displace", Value::Object(back)).unwrap();
    let d = s.active().unwrap();
    let LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("not a smart object") };
    assert_eq!(first_value(&sm.smart_filters[1].params), 0.0, "the black map, not the passed-back white one");
}

#[test]
fn review_palette_map_command_preserves_display_p3_selection_holes_at_all_depths() {
    use photocraft_cms::Builtin;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;

    let area = Rect::new(0, 0, 3, 1);
    let palette = json!({
        "paletteSnapshot": {"name": "black", "colors": [[0.0, 0.0, 0.0]]},
        "mapping": {"method": "none"},
    });
    for (depth, sample) in [(8, SampleType::U8), (16, SampleType::U16), (32, SampleType::F32)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 3, "height": 1, "depth": depth, "background": "transparent"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("Display P3 selection-hole fixture", |doc, active| {
            doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            // The middle pixel is a saturated Display P3 red (outside sRGB); only its two
            // neighbours are selected, so the selection's bounds still include this hole.
            surface.write_pixel(0, 0, &[0.0, 1.0, 0.0, 0.73]);
            surface.write_pixel(1, 0, &[1.0, 0.0, 0.0, 0.61]);
            surface.write_pixel(2, 0, &[0.0, 0.0, 1.0, 0.89]);
            let mut selection = Surface::new(PixelFormat::GRAY8);
            selection.write_pixel(0, 0, &[1.0]);
            selection.write_pixel(2, 0, &[1.0]);
            doc.selection = Some(selection);
            Ok(())
        })
        .unwrap();

        let before_profile = s.active().unwrap().doc.icc_profile.clone();
        let before = {
            let d = s.active().unwrap();
            let surface = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
            assert_eq!(surface.format(), PixelFormat::new(ColorMode::Rgb, sample, true), "depth {depth}");
            surface.try_to_interleaved(area).unwrap()
        };
        s.execute(crate::palette_cmds::FILTER_ID, palette.clone()).unwrap();

        let d = s.active().unwrap();
        assert_eq!(d.doc.icc_profile, before_profile, "depth {depth}: the document profile changed");
        let surface = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
        assert_eq!(surface.format(), PixelFormat::new(ColorMode::Rgb, sample, true), "depth {depth}: the native format changed");
        let after = surface.try_to_interleaved(area).unwrap();
        let bytes_per_pixel = surface.format().bytes_per_pixel();
        assert_eq!(
            &after[bytes_per_pixel..2 * bytes_per_pixel],
            &before[bytes_per_pixel..2 * bytes_per_pixel],
            "depth {depth}: zero-coverage interior hole was remapped"
        );
        assert_ne!(&after[..bytes_per_pixel], &before[..bytes_per_pixel], "depth {depth}: selected endpoint did not change");
        assert_ne!(
            &after[2 * bytes_per_pixel..3 * bytes_per_pixel],
            &before[2 * bytes_per_pixel..3 * bytes_per_pixel],
            "depth {depth}: selected endpoint did not change"
        );
    }
}

#[test]
fn review_palette_map_render_surface_preserves_cmyk_selection_holes() {
    use photocraft_algo::palette_map::{Method, Palette, Settings};
    use photocraft_cms::Builtin;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;

    let area = Rect::new(0, 0, 3, 1);
    let format = PixelFormat::new(ColorMode::Cmyk, SampleType::U16, true);
    let mut surface = Surface::new(format);
    surface.write_pixel(0, 0, &[0.03, 0.91, 0.07, 0.11, 0.62]);
    surface.write_pixel(1, 0, &[0.17, 0.83, 0.29, 0.13, 0.64]);
    surface.write_pixel(2, 0, &[0.19, 0.07, 0.95, 0.09, 0.66]);
    let before = surface.try_to_interleaved(area).unwrap();
    let mut selection = Surface::new(PixelFormat::GRAY8);
    selection.write_pixel(0, 0, &[1.0]);
    selection.write_pixel(2, 0, &[1.0]);

    let palette = Palette { name: "black".into(), colors: vec![[0.0, 0.0, 0.0]] };
    let after = crate::palette_cmds::render_surface(
        &surface,
        area,
        &palette,
        &Settings { method: Method::None, ..Settings::default() },
        Some(Builtin::CoatedCmyk.profile()),
        Some(&selection),
        &|| false,
    )
    .unwrap();
    assert_eq!(after.format(), format, "the native CMYK depth and alpha layout changed");
    let after_bytes = after.try_to_interleaved(area).unwrap();
    let bytes_per_pixel = format.bytes_per_pixel();
    assert_eq!(
        &after_bytes[bytes_per_pixel..2 * bytes_per_pixel],
        &before[bytes_per_pixel..2 * bytes_per_pixel],
        "zero-coverage interior CMYK hole was remapped"
    );
    assert_ne!(&after_bytes[..bytes_per_pixel], &before[..bytes_per_pixel], "selected endpoint did not change");
    assert_ne!(&after_bytes[2 * bytes_per_pixel..3 * bytes_per_pixel], &before[2 * bytes_per_pixel..3 * bytes_per_pixel], "selected endpoint did not change");
}

#[test]
fn palette_map_filter_uses_one_undo_step_and_last_filter_replay() {
    let mut s = session();
    paint_pattern(&mut s);
    let before = active_pixels(&s);
    let past = s.active().unwrap().history.past_len();
    let params = json!({
        "paletteSnapshot": {"name": "binary", "colors": [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]]},
        "mapping": {"method": "none"},
    });
    s.execute(crate::palette_cmds::FILTER_ID, params).unwrap();
    let first = active_pixels(&s);
    assert_ne!(first, before);
    assert_eq!(s.active().unwrap().history.past_len(), past + 1);
    s.execute("filter.lastFilter", json!({})).unwrap();
    assert_eq!(active_pixels(&s), first);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(active_pixels(&s), first);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(active_pixels(&s), before);
}

#[test]
fn palette_map_invalid_snapshot_has_no_mutation() {
    let mut s = session();
    paint_pattern(&mut s);
    let before = active_pixels(&s);
    let past = s.active().unwrap().history.past_len();
    let error = s.execute(crate::palette_cmds::FILTER_ID, json!({"paletteSnapshot": {"name": "bad", "colors": [[2.0, 0.0, 0.0]]}})).unwrap_err();
    assert!(error.to_string().contains("paletteSnapshot"));
    assert_eq!(active_pixels(&s), before);
    assert_eq!(s.active().unwrap().history.past_len(), past);
}

#[test]
fn palette_map_smart_filter_snapshot_roundtrips_through_pcraft() {
    let mut s = session();
    paint_pattern(&mut s);
    let base = {
        let active = s.active().unwrap();
        active.doc.layer(active.active_layer.unwrap()).unwrap().surface().unwrap().clone()
    };
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute(
        crate::palette_cmds::FILTER_ID,
        json!({
            "paletteId": "gimp:does-not-exist",
            "paletteSnapshot": {"name": "binary", "colors": [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]]},
            "mapping": {"method": "none"},
        }),
    )
    .unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    let original = s.active().unwrap().doc.clone();
    let expected = original.layer(layer).unwrap().surface().unwrap().read_region(original.bounds());
    let bytes = photocraft_format::save_to_bytes(&original, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
    let LayerContent::Smart(sm) = &reopened.layer(layer).unwrap().content else {
        panic!("reopened palette map document is not a smart object");
    };
    // Re-render from the unfiltered source, not the saved cache. The catalog ID is absent,
    // so only the persisted palette snapshot can reproduce these pixels.
    let replayed = crate::smart_cmds::apply_smart_filters(&base, sm, reopened.bounds()).unwrap();
    assert_eq!(replayed.read_region(reopened.bounds()), expected);

    let mut stack = sm.clone();
    stack.smart_filters.push(photocraft_doc::SmartFilter {
        command: crate::palette_cmds::FILTER_ID.into(),
        params: json!({"paletteSnapshot": {"name": "Red", "colors": [[1.0, 0.0, 0.0]]}, "mapping": {"method": "none"}}),
        blend: photocraft_color::BlendMode::Normal,
        opacity: 0.5,
        visible: true,
    });
    let stacked = crate::smart_cmds::apply_smart_filters(&base, &stack, reopened.bounds()).unwrap();
    for (before, after) in replayed.read_region(reopened.bounds()).chunks_exact(4).zip(stacked.read_region(reopened.bounds()).chunks_exact(4)) {
        for channel in 0..3 {
            let red = if channel == 0 { 1.0 } else { 0.0 };
            assert!((after[channel] - (before[channel] + red) * 0.5).abs() <= 1.0 / 255.0);
        }
        assert_eq!(after[3], before[3]);
    }
}

#[test]
fn palette_map_rgb_profile_roundtrips_through_srgb_kernel() {
    use photocraft_cms::{Builtin, Intent, Transform};
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1, "height": 1})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let source = [0.6, 0.4, 0.3];
    s.edit("profile", |doc, active| {
        doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().write_pixel(0, 0, &[source[0], source[1], source[2], 1.0]);
        Ok(())
    })
    .unwrap();
    let mut srgb = [0.0; 16];
    Transform::new(Builtin::DisplayP3.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).unwrap().eval(&source, &mut srgb);
    s.execute(
        crate::palette_cmds::FILTER_ID,
        json!({"paletteSnapshot": {"name": "exact", "colors": [srgb[..3].to_vec(), [0.0, 0.0, 0.0]]}, "mapping": {"method": "none"}}),
    )
    .unwrap();
    let d = s.active().unwrap();
    let got = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().read_region(d.doc.bounds());
    for (actual, expected) in got[..3].iter().zip(source) {
        assert!((actual - expected).abs() < 0.025, "CMS roundtrip changed {actual} vs {expected}");
    }
}
