use super::*;
use photocraft_doc::Layer;

const COMMANDS: [&str; 2] = ["edit.defineBrushPreset", "brush.defineFromSelection"];

fn tip(s: &Session) -> &GrayTile {
    let TipShape::Sampled(tip) = &*s.tools.brush.tip else { panic!("expected a sampled tip") };
    tip
}

fn source(depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 200, "height": 150, "depth": depth, "background": "#ffffff"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Tip"})).unwrap();
    s.edit("Two visible squares", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(50, 40, 90, 70), &[0.0, 0.0, 0.0, 1.0]);
        surf.fill_rect(Rect::new(120, 60, 150, 90), &[128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn visible_capture_is_identical_with_background_tip_group_or_no_active_layer() {
    for command in COMMANDS {
        for depth in [8, 16, 32] {
            let mut s = source(depth);
            let background = s.active().unwrap().doc.layers[0].id;
            let painted = s.active().unwrap().active_layer.unwrap();
            s.execute("layer.new.group", json!({"name": "Empty active group"})).unwrap();
            let group = s.active().unwrap().active_layer.unwrap();
            let mut expected = None;
            for active in [Some(background), Some(painted), Some(group), None] {
                s.edit("Choose capture-independent active layer", |_, current| {
                    *current = active;
                    Ok(())
                })
                .unwrap();
                let doc = s.active().unwrap().doc.clone();
                let history = s.active().unwrap().history.past_len();
                assert!(s.is_enabled(command));
                let made = s.execute(command, json!({"name": "Merged tip"})).unwrap();
                assert_eq!((made["width"].as_u64(), made["height"].as_u64()), (Some(100), Some(50)));
                assert_eq!(s.tools.brush.size, 100.0);
                assert_eq!(tip(&s).get(0, 0), 1.0);
                assert_eq!(tip(&s).get(60, 0), 0.0);
                assert!((tip(&s).get(70, 20) - 127.0 / 255.0).abs() < 0.003);
                let pixels: Vec<_> = (0..50).flat_map(|y| (0..100).map(move |x| (x, y))).map(|(x, y)| tip(&s).get(x, y)).collect();
                if let Some(expected) = &expected {
                    assert_eq!(&pixels, expected);
                } else {
                    expected = Some(pixels);
                }
                assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
                assert_eq!(s.active().unwrap().active_layer, active);
                assert_eq!(s.active().unwrap().history.past_len(), history);
            }
        }
    }
}

#[test]
fn selection_limits_the_visible_composite_even_with_background_active() {
    for command in COMMANDS {
        let mut s = source(8);
        s.edit("Activate background", |doc, active| {
            *active = Some(doc.layers[0].id);
            Ok(())
        })
        .unwrap();
        s.execute("select.rect", json!({"x": 115, "y": 55, "width": 40, "height": 40})).unwrap();
        let mask = s.active().unwrap().doc.selection.clone();
        s.execute(command, json!({"name": "Selected composite"})).unwrap();
        assert_eq!((tip(&s).width, tip(&s).height), (30, 30));
        assert!((tip(&s).get(0, 0) - 127.0 / 255.0).abs() < 0.003);
        assert_eq!(s.active().unwrap().doc.selection, mask);
    }
}

#[test]
fn visible_group_children_and_occlusion_count_but_hidden_layers_and_groups_do_not() {
    for command in COMMANDS {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        s.edit("Visible and hidden composite contributors", |doc, active| {
            let mut black = Layer::raster("Nested visible black", doc.pixel_format());
            black.surface_mut().unwrap().fill_rect(Rect::new(1, 1, 5, 5), &[0.0, 0.0, 0.0, 1.0]);
            let visible = Layer::group("Visible group", vec![black]);
            let mut hidden_black = Layer::raster("Hidden black", doc.pixel_format());
            hidden_black.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.0, 0.0, 0.0, 1.0]);
            hidden_black.visible = false;
            let mut hidden_group = Layer::group("Hidden ancestor", vec![hidden_black.duplicate()]);
            hidden_group.children_mut().unwrap()[0].visible = true;
            hidden_group.visible = false;
            let mut white = Layer::raster("Visible white occlusion", doc.pixel_format());
            white.surface_mut().unwrap().fill_rect(Rect::new(1, 1, 3, 5), &[1.0, 1.0, 1.0, 1.0]);
            *active = Some(hidden_black.id);
            doc.layers.extend([visible, hidden_black, hidden_group, white]);
            Ok(())
        })
        .unwrap();
        s.execute(command, json!({"name": "Only visible ink"})).unwrap();
        assert_eq!((tip(&s).width, tip(&s).height), (2, 4));
        for y in 0..4 {
            for x in 0..2 {
                assert_eq!(tip(&s).get(x, y), 1.0);
            }
        }
    }
}

#[test]
fn composite_transparency_and_selection_coverage_are_applied_once() {
    for command in COMMANDS {
        for depth in [8, 16, 32] {
            let mut s = crate::session();
            s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth, "background": "transparent"})).unwrap();
            s.edit("Nested translucent source", |doc, active| {
                let mut black = Layer::raster("Half black", doc.pixel_format());
                black.surface_mut().unwrap().fill_rect(Rect::new(1, 1, 6, 7), &[0.0, 0.0, 0.0, 0.5]);
                let group = Layer::group("Active group", vec![black]);
                *active = Some(group.id);
                doc.layers.push(group);
                Ok(())
            })
            .unwrap();
            for selected in [false, true] {
                if selected {
                    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
                    s.edit("Quarter coverage", |doc, _| {
                        doc.selection.as_mut().unwrap().fill_rect(Rect::new(0, 0, 8, 8), &[0.25]);
                        Ok(())
                    })
                    .unwrap();
                }
                s.execute(command, json!({"name": "Translucent composite"})).unwrap();
                assert_eq!((tip(&s).width, tip(&s).height), (5, 6));
                let expected = if selected { 0.125 } else { 0.5 };
                for y in 0..6 {
                    for x in 0..5 {
                        assert!((tip(&s).get(x, y) - expected).abs() < 0.003);
                    }
                }
            }
        }
    }
}

#[test]
fn a_hidden_black_source_over_white_or_transparency_is_empty_and_does_not_replace_a_preset() {
    for command in COMMANDS {
        for background in ["white", "transparent"] {
            let mut s = crate::session();
            s.execute("file.new", json!({"width": 8, "height": 8, "background": background})).unwrap();
            s.execute("layer.new.layer", json!({"name": "Hidden ink"})).unwrap();
            s.edit("Hide all ink", |doc, active| {
                let bounds = doc.bounds();
                let layer = doc.layer_mut(active.unwrap()).unwrap();
                layer.surface_mut().unwrap().fill_rect(bounds, &[0.0, 0.0, 0.0, 1.0]);
                layer.visible = false;
                Ok(())
            })
            .unwrap();
            s.execute("brush.presets.save", json!({"name": "Kept"})).unwrap();
            let brush = (*s.tools.brush).clone();
            let presets = (*s.tools.presets).clone();
            let current = s.tools.current_preset.clone();
            let doc = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let err = s.execute(command, json!({"name": "Kept"})).unwrap_err();
            assert!(err.to_string().contains("contain no paint"), "{err}");
            assert_eq!(*s.tools.brush, brush);
            assert_eq!(*s.tools.presets, presets);
            assert_eq!(s.tools.current_preset, current);
            assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().history.past_len(), history);
        }
    }
}

#[test]
fn merged_rgb_retains_the_document_profile_before_gray_conversion() {
    for command in COMMANDS {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8, "depth": 32, "background": "transparent"})).unwrap();
        s.edit("Linear gray nested under active group", |doc, active| {
            doc.icc_profile = Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes());
            let mut layer = Layer::raster("Linear half gray", doc.pixel_format());
            layer.surface_mut().unwrap().fill_rect(Rect::new(1, 1, 5, 5), &[0.5, 0.5, 0.5, 1.0]);
            let group = Layer::group("Active group", vec![layer]);
            *active = Some(group.id);
            doc.layers.push(group);
            Ok(())
        })
        .unwrap();
        s.execute(command, json!({"name": "Profiled merged tip"})).unwrap();
        assert_eq!((tip(&s).width, tip(&s).height), (4, 4));
        // Encoded sGray for linear RGB 0.5 is about 0.735, not encoded RGB 0.5.
        assert!((tip(&s).get(0, 0) - 0.265).abs() < 0.01);
    }
}

fn sparse_budget_source(width: u32, height: u32, selected: bool) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": width, "height": height, "background": "transparent"})).unwrap();
    s.edit("Sparse composite budget fixture", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(0, 0, 1, 1), &[0.0, 0.0, 0.0, 1.0]);
        assert_eq!(surf.tile_count(), 1);
        if selected {
            let mut mask = Surface::new(photocraft_color::PixelFormat::GRAY8);
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

#[test]
fn composite_storage_is_counted_with_density_and_mask_before_any_allocation() {
    const WIDTH: u32 = 32768;
    for command in COMMANDS {
        for selected in [false, true] {
            // Density plus optional mask fit exactly; the simultaneously live RGBA band
            // must also count, even though the document itself is sparse.
            let bytes = if selected { 8 } else { 4 };
            let height = (crate::allocation::MAX_DENSE_BYTES / (bytes * u64::from(WIDTH))) as u32;
            let mut s = sparse_budget_source(WIDTH, height, selected);
            let doc = s.active().unwrap().doc.clone();
            let brush = (*s.tools.brush).clone();
            let presets = (*s.tools.presets).clone();
            let current = s.tools.current_preset.clone();
            let history = s.active().unwrap().history.past_len();
            crate::allocation::fail_next_for_test();
            let result = s.execute(command, json!({"name": "Composite over budget"}));
            let unspent = crate::allocation::capacity::<u8>(0, "consume composite budget failpoint");
            let err = result.unwrap_err();
            assert!(err.to_string().contains("working memory"), "{err}");
            assert!(unspent.is_err(), "budget refusal must precede the first allocation");
            assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().history.past_len(), history);
            assert_eq!(*s.tools.brush, brush);
            assert_eq!(*s.tools.presets, presets);
            assert_eq!(s.tools.current_preset, current);
        }
    }
}

#[test]
fn compositor_allocation_boundary_failure_preserves_source_and_preset_state() {
    for command in COMMANDS {
        for selected in [false, true] {
            let mut s = source(8);
            if selected {
                s.execute("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 150})).unwrap();
            }
            let doc = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let brush = (*s.tools.brush).clone();
            let presets = (*s.tools.presets).clone();
            let current = s.tools.current_preset.clone();
            crate::allocation::fail_after_for_test(if selected { 2 } else { 1 });
            let result = s.execute(command, json!({"name": "Compositor allocation failure"}));
            let _ = crate::allocation::capacity::<u8>(0, "reset compositor failpoint");
            let err = result.unwrap_err();
            assert!(err.to_string().contains("not enough memory"), "{err}");
            assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().history.past_len(), history);
            assert_eq!(*s.tools.brush, brush);
            assert_eq!(*s.tools.presets, presets);
            assert_eq!(s.tools.current_preset, current);
        }
    }
}

#[test]
fn a_banded_capture_that_fits_the_budget_is_not_charged_for_a_full_rgba_canvas() {
    const WIDTH: u32 = 32768;
    let band_rows = photocraft_compose::band_rows_for(WIDTH, 0) as u64;
    let band_bytes = u64::from(WIDTH) * band_rows * 16;
    for command in COMMANDS {
        for selected in [false, true] {
            let sample_bytes = if selected { 8 } else { 4 };
            let height = ((crate::allocation::MAX_DENSE_BYTES - band_bytes) / (sample_bytes * u64::from(WIDTH))) as u32;
            let pixels = u64::from(WIDTH) * u64::from(height);
            assert!(pixels * sample_bytes + band_bytes <= crate::allocation::MAX_DENSE_BYTES);
            assert!(pixels * (sample_bytes + 16) > crate::allocation::MAX_DENSE_BYTES, "a full-plane estimate would incorrectly reject this fixture");
            let mut s = sparse_budget_source(WIDTH, height, selected);
            let doc = s.active().unwrap().doc.clone();
            let brush = (*s.tools.brush).clone();
            let presets = (*s.tools.presets).clone();
            // Refuse the first real allocation, after verifying that the budget allowed it.
            crate::allocation::fail_next_for_test();
            let result = s.execute(command, json!({"name": "Fits with bands"}));
            let unspent = crate::allocation::capacity::<u8>(0, "consume band acceptance failpoint");
            let err = result.unwrap_err().to_string();
            assert!(err.contains("not enough memory") && !err.contains("working memory"), "{err}");
            assert!(unspent.is_ok(), "the accepted budget must reach the first allocation");
            assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
            assert_eq!(*s.tools.brush, brush);
            assert_eq!(*s.tools.presets, presets);
        }
    }
}

#[test]
fn selected_visible_capture_is_identical_with_background_tip_group() {
    for command in COMMANDS {
        for depth in [8, 16, 32] {
            let mut s = source(depth);
            let background = s.active().unwrap().doc.layers[0].id;
            let painted = s.active().unwrap().active_layer.unwrap();
            s.execute("layer.new.group", json!({"name": "Empty active group"})).unwrap();
            let group = s.active().unwrap().active_layer.unwrap();
            s.execute("select.rect", json!({"x": 50, "y": 40, "width": 100, "height": 50})).unwrap();
            let mut expected = None;
            for active in [Some(background), Some(painted), Some(group)] {
                s.edit("Choose capture-independent active layer", |_, current| {
                    *current = active;
                    Ok(())
                })
                .unwrap();
                let doc = s.active().unwrap().doc.clone();
                let history = s.active().unwrap().history.past_len();
                assert!(s.is_enabled(command));
                let made = s.execute(command, json!({"name": "Merged tip"})).unwrap();
                assert_eq!((made["width"].as_u64(), made["height"].as_u64()), (Some(100), Some(50)));
                assert_eq!(s.tools.brush.size, 100.0);
                assert_eq!(tip(&s).get(0, 0), 1.0);
                assert_eq!(tip(&s).get(60, 0), 0.0);
                assert!((tip(&s).get(70, 20) - 127.0 / 255.0).abs() < 0.003);
                let pixels: Vec<_> = (0..50).flat_map(|y| (0..100).map(move |x| (x, y))).map(|(x, y)| tip(&s).get(x, y)).collect();
                if let Some(expected) = &expected {
                    assert_eq!(&pixels, expected);
                } else {
                    expected = Some(pixels);
                }
                assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
                assert_eq!(s.active().unwrap().active_layer, active);
                assert_eq!(s.active().unwrap().history.past_len(), history);
            }
        }
    }
}

#[test]
fn visible_composite_keeps_occlusion_and_ignores_hidden_source() {
    for command in COMMANDS {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        s.edit("Visible composite with hidden source", |doc, active| {
            let mut black = Layer::raster("Visible black", doc.pixel_format());
            black.surface_mut().unwrap().fill_rect(Rect::new(1, 1, 5, 5), &[0.0, 0.0, 0.0, 1.0]);
            let mut hidden_black = Layer::raster("Hidden black", doc.pixel_format());
            hidden_black.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.0, 0.0, 0.0, 1.0]);
            hidden_black.visible = false;
            let mut white = Layer::raster("Visible white occlusion", doc.pixel_format());
            white.surface_mut().unwrap().fill_rect(Rect::new(1, 1, 3, 5), &[1.0, 1.0, 1.0, 1.0]);
            *active = Some(hidden_black.id);
            doc.layers.extend([black, hidden_black, white]);
            Ok(())
        })
        .unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
        s.execute(command, json!({"name": "Only visible ink"})).unwrap();
        assert_eq!((tip(&s).width, tip(&s).height), (2, 4));
        for y in 0..4 {
            for x in 0..2 {
                assert_eq!(tip(&s).get(x, y), 1.0);
            }
        }
    }
}

#[test]
fn selection_buffer_failure_preserves_source_and_preset_state() {
    for command in COMMANDS {
        let mut s = source(8);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 150})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        let brush = (*s.tools.brush).clone();
        let presets = (*s.tools.presets).clone();
        let current = s.tools.current_preset.clone();
        crate::allocation::fail_after_for_test(1);
        let result = s.execute(command, json!({"name": "Selection buffer failure"}));
        let _ = crate::allocation::capacity::<u8>(0, "reset selection failpoint");
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not enough memory"), "{err}");
        assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().history.past_len(), history);
        assert_eq!(*s.tools.brush, brush);
        assert_eq!(*s.tools.presets, presets);
        assert_eq!(s.tools.current_preset, current);
    }
}
