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
fn explicit_outline_carry_works_for_single_components_and_sets_move_it_once() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for colors in [vec![0], vec![0, 2]] {
            for carry in [None, Some(false), Some(true)] {
                let mut s = session(ColorMode::Rgb, depth);
                s.execute("layer.new.layerFromBackground", json!({})).unwrap();
                selection(&mut s, 3);
                let before = s.active().unwrap().doc.clone();
                let old = before.selection.as_ref().unwrap();
                let expected = if colors.len() > 1 || carry == Some(true) { old.translated(2, -1, old.content_bounds()) } else { old.clone() };
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
