use super::tests::{selection, session, surface};
use super::*;
use photocraft_color::SampleType;

#[test]
fn shifted_components_survive_beyond_sparse_source_edges_at_native_depth() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for colors in [vec![1], vec![0, 2]] {
                for (dx, dy) in [(9, 2), (-263, -5)] {
                    let mut s = session(mode, depth);
                    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
                    let source = Rect::new(254, 2, 259, 5);
                    s.edit("sparse native fixture", |doc, id| {
                        let dst = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
                        *dst = Surface::new(dst.format());
                        for y in source.y0..source.y1 {
                            for x in source.x0..source.x1 {
                                let mut pixel: Vec<_> = (0..dst.channels()).map(|c| (x - 253 + y + c as i32) as f32 / 12.0).collect();
                                if depth == SampleType::F32 {
                                    pixel[0] = if x % 2 == 0 { -0.0 } else { 1.75 };
                                    pixel[1] = -0.25;
                                }
                                dst.write_pixel(x, y, &pixel);
                            }
                        }
                        Ok(())
                    })
                    .unwrap();
                    let original = surface(&s).clone();
                    let frame = source.union(&source.translate(dx, dy));
                    let fmt = original.format();
                    let background = from_rgba(&fmt, s.tools.background);
                    let mut expected = original.clone();
                    for y in frame.y0..frame.y1 {
                        for x in frame.x0..frame.x1 {
                            let mut pixel = original.pixel(x, y);
                            for &c in &colors {
                                if source.contains(x - dx, y - dy) {
                                    pixel[c] = original.sample_channel(x - dx, y - dy, c);
                                } else if source.contains(x, y) {
                                    pixel[c] = background[c];
                                }
                            }
                            expected.write_pixel(x, y, &pixel);
                        }
                    }
                    s.execute("layer.translate", json!({"target":{"colors":colors},"dx":dx,"dy":dy})).unwrap();
                    assert_eq!(
                        surface(&s).try_to_interleaved(frame).unwrap(),
                        expected.try_to_interleaved(frame).unwrap(),
                        "{mode:?}/{depth:?}/{colors:?}/{dx}/{dy}"
                    );
                    s.undo();
                    assert_eq!(surface(&s), &original);
                    s.redo();
                    assert_eq!(surface(&s).try_to_interleaved(frame).unwrap(), expected.try_to_interleaved(frame).unwrap());
                }
            }
        }
    }
}

#[test]
fn selected_component_and_set_moves_carry_the_outline_once_regardless_of_carry_flag() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for colors in [vec![0], vec![0, 2]] {
            for carry in [None, Some(false), Some(true)] {
                let mut s = session(ColorMode::Rgb, depth);
                s.execute("layer.new.layerFromBackground", json!({})).unwrap();
                selection(&mut s, 3);
                let before = s.active().unwrap().doc.clone();
                let old = before.selection.as_ref().unwrap();
                let expected = old.translated(2, -1, old.content_bounds());
                let mut params = json!({"target":{"colors":colors},"dx":2,"dy":-1});
                if let Some(carry) = carry {
                    params["selection"] = json!(carry);
                }
                let history = s.active().unwrap().history.past_len();
                s.execute("layer.translate", params).unwrap();
                assert_eq!(s.active().unwrap().doc.selection.as_ref(), Some(&expected));
                assert_eq!(s.active().unwrap().history.past_len(), history + 1);
                let moved = surface(&s).clone();
                s.undo();
                assert_eq!(s.active().unwrap().doc.selection, before.selection);
                assert_eq!(surface(&s), before.layers[0].surface().unwrap());
                s.redo();
                assert_eq!(s.active().unwrap().doc.selection.as_ref(), Some(&expected));
                assert_eq!(surface(&s), &moved);
            }
        }
    }
}

#[test]
fn unreachable_or_oversized_channel_moves_fail_without_pixel_or_history_changes() {
    let mut s = session(ColorMode::Rgb, SampleType::F32);
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    for (dx, dy) in [(i32::MIN, 0), (i32::MAX, 0), (i32::MIN, i32::MAX), (100_000_000, 0)] {
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        assert!(s.execute("layer.translate", json!({"target":{"colors":[0,2]},"dx":dx,"dy":dy})).is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn preview_limits_use_the_same_native_bounds_and_budget_as_commit() {
    use photocraft_algo::transform::{Homography, WARP_REACH};
    let mut s = session(ColorMode::Rgb, SampleType::F32);
    let doc = &s.active().unwrap().doc;
    let id = s.active().unwrap().active_layer;
    let params = json!({"target":{"color":1}});
    let mut limits = AffinePreviewBounds::new(doc.clone(), id, &params, s.tools.background).unwrap();
    for h in
        [Homography([1.0, 0.0, f64::from(WARP_REACH) + 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]), Homography([1000.0, 0.0, 0.0, 0.0, 1000.0, 0.0, 0.0, 0.0, 1.0])]
    {
        assert!(limits.check(&h).is_err());
    }
    s.edit("default-valued channel except one sample", |doc, id| {
        let surface = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
        surface.fill_rect(Rect::new(0, 0, 12, 8), &[1.0, 1.0, 1.0, 1.0]);
        surface.write_pixel(0, 0, &[1.0, 0.25, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let doc = &s.active().unwrap().doc;
    let h = Homography([200_000.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    assert!(
        crate::transform_cmds::check_channel_geometry(
            Rect::new(0, 0, 12, 8),
            PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false),
            &h,
            Some(Rect::new(0, 0, 12, 8))
        )
        .is_err()
    );
    let mut limits = AffinePreviewBounds::new(doc.clone(), id, &params, s.tools.background).unwrap();
    assert!(limits.check(&h).is_ok(), "a conservative frame must not refuse a valid sparse channel");
}

#[test]
fn native_writeback_union_is_priced_before_affine_and_warp_previews_or_commits() {
    use photocraft_color::Color;
    use photocraft_doc::{Document, Size};
    use photocraft_geom::warp::{BezierMesh, Warp};
    let mut s = Session::new();
    let mut doc = Document::with_background("union limits", Size::new(3000, 1000), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let rect = doc.bounds();
    doc.layers[0].surface_mut().unwrap().fill_rect(rect, &[0.25, 0.5, 0.75, 1.0]);
    s.add_document(doc, None);
    let st = s.active().unwrap();
    let before = st.doc.clone();
    let layer = st.active_layer;
    let params = json!({"target":{"colors":[0,2]}});
    let mut limits = AffinePreviewBounds::new(before.clone(), layer, &params, s.tools.background).unwrap();
    let h = photocraft_algo::transform::Homography([1.0, 0.0, 100_000.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    assert!(limits.check(&h).is_err());
    assert!(s.execute("edit.transform", json!({"target":{"colors":[0,2]},"dx":100_000})).is_err());
    let rect = [0.0, 0.0, 3000.0, 1000.0];
    let warp = Warp::custom(BezierMesh::identity(rect, 1, 1).map_points(|[x, y]| [x + 100_000.0, y]), rect);
    assert!(crate::warp_cmds::check_channel_warp_geometry(&before, layer, &params, rect, &warp).is_err());
    assert!(s.execute("edit.transform.warp", json!({"target":{"colors":[0,2]},"warp":warp})).is_err());
    assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
}

#[test]
fn pattern_channel_moves_wrap_native_samples_and_preserve_other_encoded_components() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for colors in [vec![0], vec![0, 2]] {
                for (dx, dy) in [(2_i32, 1_i32), (-14, -9), (12, 8)] {
                    let mut s = session(mode, depth);
                    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
                    s.edit("native wrap sentinels", |doc, active| {
                        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                        surface.write_pixel(-1, 3, &vec![0.7; surface.channels()]);
                        if depth == SampleType::F32 {
                            let mut pixel = surface.pixel(5, 3);
                            pixel[0] = -0.0;
                            pixel[1] = f32::from_bits(0x7fc0_1234);
                            pixel[2] = 1.75;
                            surface.write_pixel(5, 3, &pixel);
                        }
                        Ok(())
                    })
                    .unwrap();
                    let before = surface(&s).clone();
                    let fmt = before.format();
                    let bounds = s.active().unwrap().doc.bounds();
                    let original = before.try_to_interleaved(bounds).unwrap();
                    let history = s.active().unwrap().history.past_len();
                    s.execute("layer.translate", json!({"target":{"colors":colors},"dx":dx,"dy":dy,"wrap":true})).unwrap();
                    let actual = surface(&s).try_to_interleaved(bounds).unwrap();
                    let outside = Rect::new(-2, 0, 0, 8);
                    assert_eq!(surface(&s).try_to_interleaved(outside).unwrap(), before.try_to_interleaved(outside).unwrap());
                    let bytes = depth.bytes();
                    let bpp = fmt.bytes_per_pixel();
                    for y in 0..8 {
                        for x in 0..12 {
                            let dest = (y * 12 + x) as usize * bpp;
                            let source = (((y - dy).rem_euclid(8) * 12) + (x - dx).rem_euclid(12)) as usize * bpp;
                            for c in 0..fmt.channels() {
                                let from = if colors.contains(&c) { source } else { dest };
                                assert_eq!(
                                    &actual[dest + c * bytes..dest + (c + 1) * bytes],
                                    &original[from + c * bytes..from + (c + 1) * bytes],
                                    "{mode:?}/{depth:?}/{colors:?}/{dx}/{dy}/{x}/{y}/{c}"
                                );
                            }
                        }
                    }
                    assert_eq!(s.active().unwrap().history.past_len(), history + usize::from((dx.rem_euclid(12), dy.rem_euclid(8)) != (0, 0)));
                    if surface(&s) != &before {
                        let moved = surface(&s).clone();
                        assert!(s.undo());
                        assert_eq!(surface(&s), &before);
                        assert!(s.redo());
                        assert_eq!(surface(&s), &moved);
                    }
                }
            }
        }
    }
}

#[test]
fn single_selected_move_changes_only_selected_channel_samples_and_moves_outline_once() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for kind in [1, 2, 3] {
                let mut s = session(mode, depth);
                selection(&mut s, kind);
                s.execute("channel.target", json!({"channel":{"color":0}})).unwrap();
                let before = s.active().unwrap().doc.clone();
                let original = surface(&s).clone();
                let mut oracle = Session::new();
                oracle.tools.background = s.tools.background;
                oracle.add_document((*before).clone(), None);
                oracle.execute("edit.transform", json!({"target":{"color":0},"dx":3,"dy":1,"interpolation":"nearest"})).unwrap();
                s.execute("layer.translate", json!({"dx":3,"dy":1,"selection":true})).unwrap();
                let bounds = before.bounds().union(&surface(&s).content_bounds());
                assert_eq!(surface(&s).try_to_interleaved(bounds).unwrap(), surface(&oracle).try_to_interleaved(bounds).unwrap(), "{mode:?}/{depth:?}/{kind}");
                assert_eq!(s.active().unwrap().doc.selection, oracle.active().unwrap().doc.selection);
                assert!(s.active().unwrap().doc.is_background(s.active().unwrap().active_layer.unwrap()));
                assert!(s.undo());
                assert_eq!(surface(&s), &original);
                assert_eq!(s.active().unwrap().doc.selection, before.selection);
            }
        }
    }
}

#[test]
fn selected_wrapped_components_match_coverage_math_and_leave_off_canvas_samples_untouched() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for colors in [vec![0], vec![0, 2]] {
                for coverage in [1.0, 0.5] {
                    let mut s = session(mode, depth);
                    s.tools.background = [0.2, 0.4, 0.6, 1.0];
                    s.edit("wrap selection", |doc, active| {
                        let source = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                        source.write_pixel(-1, 3, &vec![0.7; source.channels()]);
                        let mut sel = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
                        sel.fill_rect(Rect::new(10, 2, 12, 5), &[coverage]);
                        sel.write_pixel(-1, 3, &[coverage]);
                        doc.selection = Some(sel);
                        Ok(())
                    })
                    .unwrap();
                    let before = s.active().unwrap().doc.clone();
                    let original = surface(&s).clone();
                    let sel = before.selection.as_ref().unwrap();
                    let space = photocraft_compose::cmyk_space(&before);
                    let fill = photocraft_color::convert::with_cmyk_space(space.as_ref(), || from_rgba(&original.format(), s.tools.background));
                    let fill = Surface::with_default(original.format(), &fill).pixel(0, 0);
                    // Quantize the cleared samples at native depth before blending the lifted
                    // coverage, independently of the transform/Move implementations.
                    let mut rest = original.clone();
                    for y in 0..8 {
                        for x in 0..12 {
                            let a = sel.sample_channel(x, y, 0);
                            let mut pixel = original.pixel(x, y);
                            for &c in &colors {
                                pixel[c] = pixel[c] * (1.0 - a) + fill[c] * a;
                            }
                            rest.write_pixel(x, y, &pixel);
                        }
                    }
                    let mut expected = original.clone();
                    for y in 0..8 {
                        for x in 0..12 {
                            let (sx, sy) = ((x - 3_i32).rem_euclid(12), (y - 1_i32).rem_euclid(8));
                            let a = sel.sample_channel(sx, sy, 0);
                            let mut pixel = original.pixel(x, y);
                            for &c in &colors {
                                pixel[c] = original.sample_channel(sx, sy, c) * a + rest.sample_channel(x, y, c) * (1.0 - a);
                            }
                            expected.write_pixel(x, y, &pixel);
                        }
                    }
                    let history = s.active().unwrap().history.past_len();
                    s.execute("layer.translate", json!({"target":{"colors":colors},"dx":-9,"dy":-7,"selection":true,"wrap":true})).unwrap();
                    let area = Rect::new(-2, -1, 15, 10);
                    assert_eq!(
                        surface(&s).try_to_interleaved(area).unwrap(),
                        expected.try_to_interleaved(area).unwrap(),
                        "{mode:?}/{depth:?}/{colors:?}/{coverage}"
                    );
                    let moved_selection = s.active().unwrap().doc.selection.as_ref().unwrap();
                    for y in 0..8 {
                        for x in 0..12 {
                            assert_eq!(moved_selection.sample_channel(x, y, 0), sel.sample_channel((x - 3_i32).rem_euclid(12), (y - 1_i32).rem_euclid(8), 0));
                        }
                    }
                    assert_eq!(moved_selection.sample_channel(-1, 3, 0), sel.sample_channel(-1, 3, 0));
                    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
                    assert!(s.undo());
                    assert_eq!(surface(&s), &original);
                    assert_eq!(s.active().unwrap().doc.selection, before.selection);
                    assert!(s.redo());
                    assert_eq!(surface(&s).try_to_interleaved(area).unwrap(), expected.try_to_interleaved(area).unwrap());
                }
            }
        }
    }
}

#[test]
fn floating_a_single_colour_is_refused_for_automation_before_it_can_move_rgba() {
    let mut s = session(ColorMode::Rgb, SampleType::U16);
    selection(&mut s, 2);
    s.execute("channel.target", json!({"channel":"red"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    assert!(!s.is_enabled("select.float"));
    assert!(s.execute("select.float", json!({"dx":2})).is_err());
    assert!(s.execute("select.float", json!({"target":{"color":0},"dx":2})).is_err());
    assert!(crate::float_cmds::floating(s.active().unwrap()).is_none());
    assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert_eq!(s.active().unwrap().history.past_len(), history);
    // An explicit pixel target deliberately keeps the existing full-pixel operation.
    s.execute("select.float", json!({"target":"pixels","dx":2})).unwrap();
    assert!(crate::float_cmds::floating(s.active().unwrap()).is_some());
}
