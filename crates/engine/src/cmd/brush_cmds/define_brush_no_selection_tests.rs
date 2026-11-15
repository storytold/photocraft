use super::*;

const COMMANDS: [&str; 2] = ["edit.defineBrushPreset", "brush.defineFromSelection"];

fn source(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 150, "depth": depth, "background": "#ffffff"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Tip"})).unwrap();
    s.edit("BrushSrc", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(50, 40, 90, 70), &[0.0, 0.0, 0.0, 1.0]);
        surf.fill_rect(Rect::new(120, 60, 150, 90), &[128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

fn tip(s: &Session) -> &GrayTile {
    let TipShape::Sampled(tip) = &s.tools.brush.tip else { panic!("expected a sampled tip") };
    tip
}

#[test]
fn without_selection_the_canvas_is_trimmed_to_the_measured_content() {
    for command in COMMANDS {
        for depth in [8, 16, 32] {
            let mut s = source(depth);
            assert!(s.active().unwrap().doc.selection.is_none());
            assert!(s.is_enabled(command), "{command}, depth {depth}");
            let doc = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let presets = s.tools.presets.len();

            let result = s.execute(command, json!({"name": "Canvas tip"})).unwrap();

            assert_eq!((result["width"].as_u64(), result["height"].as_u64()), (Some(100), Some(50)));
            assert_eq!(s.tools.brush.size, 100.0);
            assert_eq!((tip(&s).width, tip(&s).height), (100, 50));
            assert_eq!(tip(&s).get(0, 0), 1.0);
            assert_eq!(tip(&s).get(60, 0), 0.0, "transparent gap contributes no paint");
            assert!((tip(&s).get(70, 20) - 127.0 / 255.0).abs() < 0.003);
            assert_eq!(s.tools.presets.len(), presets + 1);
            assert_eq!(s.tools.current_preset.as_deref(), Some("Canvas tip"));
            assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc), "source document is untouched");
            assert_eq!(s.active().unwrap().history.past_len(), history);
            assert!(s.active().unwrap().doc.selection.is_none(), "no synthetic selection is left behind");
        }
    }
}

#[test]
fn an_existing_selection_still_limits_the_tip_to_the_grey_square() {
    for command in COMMANDS {
        let mut s = source(8);
        s.execute("select.rect", json!({"x": 115, "y": 55, "width": 40, "height": 40})).unwrap();
        let result = s.execute(command, json!({"name": "Selected tip"})).unwrap();
        assert_eq!((result["width"].as_u64(), result["height"].as_u64()), (Some(30), Some(30)));
        assert_eq!(s.tools.brush.size, 30.0);
        assert!((tip(&s).get(0, 0) - 127.0 / 255.0).abs() < 0.003);
        assert!(s.active().unwrap().doc.selection.is_some());
    }
}

#[test]
fn a_partial_selection_keeps_coverage_alpha_and_canvas_clipping() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
    s.edit("Half opaque source", |doc, active| {
        let bounds = doc.bounds();
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.0, 0.0, 0.0, 0.5]);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": -3, "y": -2, "width": 8, "height": 8})).unwrap();
    s.edit("Partial coverage", |doc, _| {
        doc.selection.as_mut().unwrap().fill_rect(Rect::new(0, 0, 5, 6), &[0.25]);
        Ok(())
    })
    .unwrap();

    s.execute("brush.defineFromSelection", json!({"name": "Masked tip"})).unwrap();

    assert_eq!((tip(&s).width, tip(&s).height), (5, 6));
    assert!((tip(&s).get(0, 0) - 0.125).abs() < 0.003, "density includes source alpha and selection coverage");
    assert!((tip(&s).get(4, 5) - 0.125).abs() < 0.003);
}

#[test]
fn no_document_is_disabled_and_a_group_active_capture_uses_visible_pixels() {
    let mut s = Session::new();
    for command in COMMANDS {
        assert!(!s.is_enabled(command));
        assert!(s.execute(command, json!({"name": "No document"})).is_err());
    }
    s = source(8);
    s.execute("layer.new.group", json!({"name": "Not pixels"})).unwrap();
    for command in COMMANDS {
        assert!(s.is_enabled(command));
        s.execute(command, json!({"name": "Visible pixels"})).unwrap();
        assert_eq!((tip(&s).width, tip(&s).height), (100, 50));
    }
}

#[test]
fn a_large_canvas_is_trimmed_before_the_existing_tip_side_limit() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": MAX_TIP + 1, "height": 2, "background": "#ffffff"})).unwrap();
    s.edit("One black pixel", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(MAX_TIP as i32, 0, MAX_TIP as i32 + 1, 1), &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let made = s.execute("edit.defineBrushPreset", json!({"name": "One pixel"})).unwrap();
    assert_eq!((made["width"].as_u64(), made["height"].as_u64()), (Some(1), Some(1)));
    assert_eq!(tip(&s).get(0, 0), 1.0);

    s.edit("Too wide painted area", |doc, active| {
        let bounds = doc.bounds();
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let brush = s.tools.brush.clone();
    let presets = s.tools.presets.clone();
    let err = s.execute("edit.defineBrushPreset", json!({"name": "Too wide"})).unwrap_err();
    assert!(err.to_string().contains("too large for a brush tip"), "{err}");
    assert_eq!(s.tools.brush, brush);
    assert_eq!(s.tools.presets, presets);
}

#[test]
fn a_canvas_capture_allocation_failure_leaves_the_source_and_presets_unchanged() {
    let mut s = source(8);
    let brush = s.tools.brush.clone();
    let presets = s.tools.presets.clone();
    let current = s.tools.current_preset.clone();
    let doc = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    crate::allocation::fail_next_for_test();
    let result = s.execute("brush.defineFromSelection", json!({"name": "Allocation failure"}));
    let _ = crate::allocation::capacity::<u8>(0, "reset failpoint");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not enough memory") && err.contains("defining a brush from the selection"), "{err}");
    assert_eq!(s.tools.brush, brush);
    assert_eq!(s.tools.presets, presets);
    assert_eq!(s.tools.current_preset, current);
    assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

// Sparse source and mask fixtures keep just one or two tiles, even when their
// bounds would require gigabytes of dense sampling buffers.
fn budget_source(width: u32, height: u32, selected: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": width, "height": height, "background": "transparent"})).unwrap();
    s.edit("Sparse budget source", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(0, 0, 1, 1), &[0.0, 0.0, 0.0, 1.0]);
        assert_eq!(surf.tile_count(), 1);
        if selected {
            let mut mask = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
            mask.fill_rect(Rect::new(0, 0, 1, 1), &[1.0]);
            mask.fill_rect(Rect::new(width as i32 - 1, height as i32 - 1, width as i32, height as i32), &[1.0]);
            assert_eq!(mask.tile_count(), 2);
            assert_eq!(mask.content_bounds(), doc.bounds());
            doc.selection = Some(mask);
        }
        Ok(())
    })
    .unwrap();
    s
}

fn assert_budget_refusal(s: &mut Session, command: &str) {
    let brush = s.tools.brush.clone();
    let presets = s.tools.presets.clone();
    let current = s.tools.current_preset.clone();
    let doc = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let revision = s.active().unwrap().revision;
    // This also makes the regression safe on the unfixed code: its first dense
    // allocation fails instead of trying to fill the enormous buffer.
    crate::allocation::fail_next_for_test();
    let result = s.execute(command, json!({"name": "Over budget"}));
    let unspent = crate::allocation::capacity::<u8>(0, "consume budget test failpoint");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("working memory") && err.contains("defining a brush from the selection"), "{err}");
    assert!(unspent.is_err(), "the budget refusal must precede the first fallible allocation");
    assert_eq!(s.tools.brush, brush);
    assert_eq!(s.tools.presets, presets);
    assert_eq!(s.tools.current_preset, current);
    assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
    assert_eq!(s.active().unwrap().history.past_len(), history);
    assert_eq!(s.active().unwrap().revision, revision);
}

#[test]
fn an_over_budget_canvas_is_refused_before_sampling() {
    const WIDTH: u32 = 32768;
    // The pixel count fits usize on both targets, while its f32 density buffer
    // alone is just over that target's dense-memory budget.
    let height = (crate::allocation::MAX_DENSE_BYTES / (4 * u64::from(WIDTH))) as u32 + 1;
    for command in COMMANDS {
        let mut s = budget_source(WIDTH, height, false);
        assert!(s.active().unwrap().doc.selection.is_none());
        assert_budget_refusal(&mut s, command);
    }
}

#[test]
fn selection_capture_counts_density_and_mask_buffers_together() {
    const WIDTH: u32 = 32768;
    let height = (crate::allocation::MAX_DENSE_BYTES / (4 * u64::from(WIDTH))) as u32;
    let density_bytes = u64::from(WIDTH) * u64::from(height) * 4;
    assert_eq!(density_bytes, crate::allocation::MAX_DENSE_BYTES, "density alone would fit; the live mask must also count");
    for command in COMMANDS {
        let mut s = budget_source(WIDTH, height, true);
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().channels(), 1);
        assert_budget_refusal(&mut s, command);
    }
}

#[test]
fn a_small_selection_on_an_over_budget_canvas_still_defines_a_tip() {
    const WIDTH: u32 = 32768;
    let height = (crate::allocation::MAX_DENSE_BYTES / (4 * u64::from(WIDTH))) as u32 + 1;
    for command in COMMANDS {
        let mut s = budget_source(WIDTH, height, false);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 1, "height": 1})).unwrap();
        let result = s.execute(command, json!({"name": "Small selected tip"})).unwrap();
        assert_eq!((result["width"].as_u64(), result["height"].as_u64()), (Some(1), Some(1)));
        assert_eq!(tip(&s).get(0, 0), 1.0);
    }
}
