use super::tests::{selection, session, surface};
use super::*;
use photocraft_color::SampleType;
use photocraft_geom::warp::{BezierMesh, Warp};

fn geometry(case: usize) -> (&'static str, Value) {
    match case {
        0 => ("edit.transform", json!({"dx":2,"dy":1,"interpolation":"nearest"})),
        1 => ("edit.transform", json!({"scaleX":85,"scaleY":110,"rotate":13,"interpolation":"bilinear"})),
        2 => ("edit.transform", json!({"quad":[[0,0],[12,1],[10,8],[1,7]],"interpolation":"bicubic"})),
        3 => ("edit.transform.flipHorizontal", json!({})),
        4 => ("edit.transform.rotate90Cw", json!({})),
        5 => ("edit.transform.rotate180", json!({})),
        6 => ("edit.transform.warp", json!({"style":"arc","bend":20})),
        _ => {
            let rect = [0.0, 0.0, 12.0, 8.0];
            let mesh = BezierMesh::identity(rect, 1, 1).map_points(|p| [p[0] + 1.0, p[1] + 0.02 * p[0] * p[0]]);
            ("edit.transform.warp", json!({"warp":Warp::custom(mesh, rect)}))
        }
    }
}

#[test]
fn colour_set_geometry_matches_independent_scalar_oracles_at_native_depth() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for kind in 0..4 {
                for case in 0..8 {
                    let mut actual = session(mode, depth);
                    selection(&mut actual, kind);
                    actual.tools.background = [0.23, 0.51, 0.77, 1.0];
                    let colors = if mode == ColorMode::Cmyk { vec![0, 1, 2] } else { vec![0, 2] };
                    actual.execute("channel.target", json!({"colors":colors})).unwrap();
                    let before = actual.active().unwrap().doc.clone();
                    let history = actual.active().unwrap().history.past_len();
                    let (command, params) = geometry(case);
                    let mut oracle = Session::new();
                    oracle.tools.background = actual.tools.background;
                    oracle.add_document((*before).clone(), None);
                    let mut outline = None;
                    for &k in &colors {
                        // Each scalar reference uses the same initial selection. Samples from
                        // earlier components stay in place while this component is transformed.
                        oracle.active_mut().unwrap().doc = {
                            let mut d = (*oracle.active().unwrap().doc).clone();
                            d.selection = before.selection.clone();
                            std::sync::Arc::new(d)
                        };
                        let mut p = params.clone();
                        p["target"] = json!({"color":k});
                        oracle.execute(command, p).unwrap();
                        outline = oracle.active().unwrap().doc.selection.clone();
                    }
                    actual.execute(command, params).unwrap();
                    assert_eq!(surface(&actual), surface(&oracle), "{mode:?}/{depth:?}/selection={kind}/case={case}");
                    assert_eq!(actual.active().unwrap().doc.selection, outline, "outline moves once");
                    assert_eq!(actual.active().unwrap().history.past_len(), history + 1);
                    let got = surface(&actual).clone();
                    actual.undo();
                    assert_eq!(surface(&actual), before.layers[0].surface().unwrap());
                    assert_eq!(actual.active().unwrap().doc.selection, before.selection);
                    actual.redo();
                    assert_eq!(surface(&actual), &got);
                }
            }
        }
    }
}

#[test]
fn colour_set_move_preserves_unselected_samples_and_matches_single_moves() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for (dx, dy) in [(2, -1), (-3, 2), (i32::MIN, i32::MAX)] {
                let mut actual = session(mode, depth);
                actual.execute("layer.new.layerFromBackground", json!({})).unwrap();
                actual.tools.background = [0.23, 0.51, 0.77, 1.0];
                actual.execute("channel.target", json!({"colors":[0,2]})).unwrap();
                let before = actual.active().unwrap().doc.clone();
                let history = actual.active().unwrap().history.past_len();
                let mut oracle = Session::new();
                oracle.tools.background = actual.tools.background;
                oracle.add_document((*before).clone(), None);
                for k in [0, 2] {
                    oracle.execute("layer.translate", json!({"target":{"color":k},"dx":dx,"dy":dy})).unwrap();
                }
                actual.execute("layer.translate", json!({"dx":dx,"dy":dy})).unwrap();
                assert_eq!(surface(&actual), surface(&oracle), "{mode:?}/{depth:?}/{dx}/{dy}");
                assert_eq!(actual.active().unwrap().history.past_len(), history + 1);
                actual.undo();
                assert_eq!(surface(&actual), before.layers[0].surface().unwrap());
            }
        }
    }
}

#[test]
fn invalid_set_geometry_is_atomic_and_explicit_sets_validate_before_allocation() {
    for target in [json!({"colors":[]}), json!({"colors":[0,3]}), json!({"colors":[-1,2]}), json!({"colors":[0,2],"color":1})] {
        let mut s = session(ColorMode::Rgb, SampleType::F32);
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute("edit.transform", json!({"target":target,"dx":2})).is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
    for skip in [0, 2, 4, 8, 10] {
        let mut s = session(ColorMode::Rgb, SampleType::F32);
        s.execute("channel.target", json!({"colors":[0,2]})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        crate::allocation::fail_after_for_test(skip);
        assert!(s.execute("edit.transform", json!({"dx":2})).is_err(), "failpoint {skip}");
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn selected_colour_set_move_matches_transform_and_moves_the_outline_once() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for kind in [1, 2, 3] {
                let mut actual = session(mode, depth);
                actual.execute("layer.new.layerFromBackground", json!({})).unwrap();
                selection(&mut actual, kind);
                actual.execute("channel.target", json!({"colors":[0,2]})).unwrap();
                let before = actual.active().unwrap().doc.clone();
                let mut oracle = Session::new();
                oracle.tools.background = actual.tools.background;
                oracle.add_document((*before).clone(), None);
                oracle.execute("edit.transform", json!({"target":{"colors":[0,2]},"dx":2,"dy":1,"interpolation":"nearest"})).unwrap();
                let history = actual.active().unwrap().history.past_len();
                actual.execute("layer.translate", json!({"dx":2,"dy":1})).unwrap();
                assert_eq!(surface(&actual), surface(&oracle), "{mode:?}/{depth:?}/{kind}");
                assert_eq!(actual.active().unwrap().doc.selection, oracle.active().unwrap().doc.selection);
                assert_eq!(actual.active().unwrap().history.past_len(), history + 1);
                actual.undo();
                assert_eq!(surface(&actual), before.layers[0].surface().unwrap());
                assert_eq!(actual.active().unwrap().doc.selection, before.selection);
            }
        }
    }
}

#[test]
fn colour_set_source_budget_is_checked_before_capture_or_allocation() {
    let mut s = session(ColorMode::Rgb, SampleType::F32);
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.size = photocraft_doc::Size::new(16384, 16384);
    s.active_mut().unwrap().doc = std::sync::Arc::new(doc);
    let st = s.active().unwrap();
    let before = st.doc.clone();
    let params = json!({"target":{"colors":[0,2]},"dx":1});
    assert!(source_bounds(&st.doc, st.active_layer, &params).unwrap_err().to_string().contains("512 MiB"));
    assert!(s.execute("edit.transform", params).unwrap_err().to_string().contains("512 MiB"));
    assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
}

#[test]
fn colour_set_geometry_preserves_untargeted_nan_payloads_and_hdr_samples() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for selected in [false, true] {
            let mut s = session(mode, SampleType::F32);
            s.edit("HDR and payloads", |doc, layer| {
                let surf = doc.layer_mut(layer.unwrap()).unwrap().surface_mut().unwrap();
                let mut pixel = vec![0.3; surf.channels()];
                pixel[0] = 4.25;
                pixel[1] = f32::from_bits(0x7fc01234);
                pixel[mode.color_channels() - 1] = -1.5;
                *pixel.last_mut().unwrap() = 0.4;
                surf.write_pixel(3, 3, &pixel);
                Ok(())
            })
            .unwrap();
            if selected {
                selection(&mut s, 3);
            }
            let before = surface(&s).try_to_interleaved(s.active().unwrap().doc.bounds()).unwrap();
            s.execute("edit.transform", json!({"target":{"colors":[0,mode.color_channels()-1]},"dx":2,"interpolation":"nearest"})).unwrap();
            let after = surface(&s).try_to_interleaved(s.active().unwrap().doc.bounds()).unwrap();
            let fmt = surface(&s).format();
            for (a, b) in after.chunks_exact(fmt.bytes_per_pixel()).zip(before.chunks_exact(fmt.bytes_per_pixel())) {
                assert_eq!(&a[4..8], &b[4..8], "the untouched NaN payload is copied exactly");
                assert_eq!(&a[fmt.bytes_per_pixel() - 4..], &b[fmt.bytes_per_pixel() - 4..], "alpha is unchanged");
            }
            if !selected {
                assert_eq!(surface(&s).sample_channel(5, 3, 0), 4.25);
                assert_eq!(surface(&s).sample_channel(5, 3, mode.color_channels() - 1), -1.5);
            }
        }
    }
}

#[test]
fn colour_set_geometry_and_move_respect_position_pixel_and_all_locks() {
    for lock in ["position", "pixels", "all"] {
        for command in ["layer.translate", "edit.transform", "edit.transform.warp"] {
            let mut s = session(ColorMode::Rgb, SampleType::U8);
            s.execute("layer.new.layerFromBackground", json!({})).unwrap();
            s.execute("channel.target", json!({"colors":[0,2]})).unwrap();
            s.edit("lock", |doc, layer| {
                let locks = &mut doc.layer_mut(layer.unwrap()).unwrap().locks;
                match lock {
                    "position" => locks.position = true,
                    "pixels" => locks.pixels = true,
                    _ => locks.all = true,
                };
                Ok(())
            })
            .unwrap();
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let p = if command == "edit.transform.warp" { json!({"style":"arc","bend":10}) } else { json!({"dx":2}) };
            assert!(s.execute(command, p).is_err(), "{command}/{lock}");
            assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().history.past_len(), history);
        }
    }
}

#[test]
fn colour_set_transforms_keep_native_surfaces_without_transparency() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut s = session(mode, depth);
            s.edit("opaque native surface", |doc, layer| {
                let l = doc.layer_mut(layer.unwrap()).unwrap();
                l.content = LayerContent::Raster(l.surface().unwrap().convert(PixelFormat::new(mode, depth, false)));
                Ok(())
            })
            .unwrap();
            let before = s.active().unwrap().doc.clone();
            let id = s.active().unwrap().active_layer.unwrap();
            let old = before.layer(id).unwrap();
            let fmt = old.surface().unwrap().format();
            let original = old.surface().unwrap().try_to_interleaved(before.bounds()).unwrap();
            s.execute("edit.transform", json!({"target":{"colors":[0,mode.color_channels()-1]},"dx":2,"interpolation":"nearest"})).unwrap();
            let now = s.active().unwrap().doc.layer(id).unwrap();
            assert_eq!(now.name, old.name);
            assert_eq!(now.locks, old.locks);
            assert_eq!(now.surface().unwrap().format(), fmt);
            let after = now.surface().unwrap().try_to_interleaved(before.bounds()).unwrap();
            let b = depth.bytes();
            for (a, z) in after.chunks_exact(fmt.bytes_per_pixel()).zip(original.chunks_exact(fmt.bytes_per_pixel())) {
                assert_eq!(&a[b..2 * b], &z[b..2 * b]);
            }
        }
    }
}

#[test]
fn public_component_move_validates_against_the_actual_raster_format() {
    let mut doc = (*session(ColorMode::Rgb, SampleType::F32).active().unwrap().doc).clone();
    let layer = doc.layers[0].id;
    let mut native = Surface::new(PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true));
    native.fill_rect(Rect::new(0, 0, 8, 4), &[0.1, 0.2, 0.3, 0.4, 0.75]);
    native.write_pixel(2, 2, &[0.1, 0.2, 0.3, 0.9, 0.75]);
    doc.layers[0].content = LayerContent::Raster(native.clone());
    crate::channel_cmds::move_color_channel(&mut doc, layer, 3, 1, 0, [1.0; 4]).unwrap();
    let got = doc.layer(layer).unwrap().surface().unwrap();
    assert_eq!(got.sample_channel(3, 2, 3), 0.9);
    assert_eq!(got.pixel(3, 2)[..3], native.pixel(3, 2)[..3]);
    assert_eq!(got.sample_channel(3, 2, 4), 0.75);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn colour_set_background_commit_and_cancel_keep_hidden_eyes_and_the_captured_set() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for cancel in [false, true] {
            let mut s = session(ColorMode::Rgb, depth);
            s.execute("channel.target", json!({"colors":[0,2]})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let crate::jobs::Started::Job(job) = s.start("edit.transform", json!({"dx":2,"interpolation":"nearest"})).unwrap() else {
                panic!("a background job")
            };
            assert!(locked(&s));
            assert!(s.execute("channel.target", json!({"colors":[1,2]})).is_err());
            assert!(s.execute("layer.new.layer", json!({})).is_err());
            s.execute("channel.setVisible", json!({"channel":"composite","visible":false})).unwrap();
            let view = s.active().unwrap().channel_view.clone();
            assert_eq!(view.visible_colors(3), 0);
            if cancel {
                assert!(s.cancel_job(job));
            }
            let event = super::tests::wait_for_job(&mut s, job);
            assert!(!locked(&s));
            let st = s.active().unwrap();
            assert_eq!(st.channel_view, view);
            assert_eq!(crate::color_channels::selected(&s).unwrap().params(), json!({"colors":[0,2]}));
            assert_eq!(st.history.past_len(), history + usize::from(!cancel));
            if cancel {
                assert_eq!(event.outcome, crate::jobs::JobOutcome::Cancelled);
                assert!(std::sync::Arc::ptr_eq(&before, &st.doc));
            } else {
                assert!(matches!(event.outcome, crate::jobs::JobOutcome::Done(_)));
                let moved = surface(&s).clone();
                assert_ne!(&moved, before.layers[0].surface().unwrap());
                assert!(s.undo());
                assert_eq!(surface(&s), before.layers[0].surface().unwrap());
                assert_eq!(s.active().unwrap().channel_view, view);
                assert!(s.redo());
                assert_eq!(surface(&s), &moved);
                assert_eq!(s.active().unwrap().channel_view, view);
            }
        }
    }
}

#[test]
fn malformed_implicit_move_targets_never_fall_back_to_whole_layer_movement() {
    for (primary, selected) in [(99, vec![]), (0, vec![0, 99])] {
        let mut s = session(ColorMode::Rgb, SampleType::U8);
        s.execute("layer.new.layerFromBackground", json!({})).unwrap();
        let st = s.active_mut().unwrap();
        st.channel_view.target = crate::channel_cmds::ChannelTarget::Color(primary);
        st.channel_view.selected_colors = selected;
        let before = st.doc.clone();
        let history = st.history.past_len();
        assert!(s.execute("layer.translate", json!({"dx":2})).is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn tiled_storage_limits_reject_extreme_aspect_ratios_before_materialization() {
    // These probes deliberately stop at validation: running the old allocation would need
    // gigabytes despite a logical area of only one megapixel.
    let mut s = session(ColorMode::Rgb, SampleType::F32);
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.size = photocraft_doc::Size::new(1_000_000, 1);
    let p = json!({"target":{"colors":[0,2]}});
    assert!(source_bounds(&doc, s.active().unwrap().active_layer, &p).unwrap_err().to_string().contains("512 MiB"));
    assert!(validate_area(Rect::new(0, 0, 1_000_000, 1)).is_err());
    assert!(validate_area(Rect::new(0, 0, 10_000, 10_000)).is_ok(), "a normal 100 MP frame still fits the existing area limit");
    assert!(validate_area(Rect::new(-1_000_000, 0, -999_999, 1)).is_ok(), "a distant small frame stays sparse");
    // The intended 36 MP, three-component F32 workflow remains admitted.
    s = session(ColorMode::Cmyk, SampleType::F32);
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.size = photocraft_doc::Size::new(6000, 6000);
    assert!(source_bounds(&doc, s.active().unwrap().active_layer, &json!({"target":{"colors":[0,1,2]}})).is_ok());
}

#[test]
fn zero_offset_selected_set_move_preserves_native_pixels_and_selection_exactly() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut s = session(mode, depth);
            selection(&mut s, 3);
            let mut doc = (*s.active().unwrap().doc).clone();
            let layer = s.active().unwrap().active_layer.unwrap();
            let before = doc.clone();
            let colors = crate::color_channels::ColorChannels::from_indices([0, 2], mode.color_channels()).unwrap();
            crate::channel_cmds::move_color_channels(&mut doc, layer, colors, 0, 0, [1.0; 4]).unwrap();
            assert_eq!(doc.layer(layer).unwrap().surface(), before.layer(layer).unwrap().surface(), "{mode:?}/{depth:?}");
            assert_eq!(doc.selection, before.selection);
        }
    }
}

#[test]
fn far_scalar_presets_fail_without_overflowing_the_frame_center() {
    for preset in ["rotate180", "rotate90Cw", "rotate90Ccw", "flipHorizontal", "flipVertical"] {
        let mut s = session(ColorMode::Rgb, SampleType::U8);
        s.execute("channel.new", json!({})).unwrap();
        s.edit("distant scalar content", |doc, _| {
            let mut plane = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
            plane.fill_rect(Rect::new(1_200_000_000, 1_200_000_000, 1_200_000_001, 1_200_000_001), &[0.5]);
            doc.channels[0].surface = plane;
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        assert!(s.execute(&format!("edit.transform.{preset}"), json!({"target":{"channel":0}})).is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn repeat_quad_transform_uses_native_frame_despite_other_selected_layers() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for kind in [0, 1, 3] {
                let mut s = session(mode, depth);
                let background = s.active().unwrap().active_layer.unwrap();
                s.execute("layer.new.layer", json!({})).unwrap();
                s.execute("layer.select", json!({"layer": background.0, "mode": "add"})).unwrap();
                selection(&mut s, kind);
                s.execute("channel.target", json!({"colors": [0, 2]})).unwrap();
                let st = s.active().unwrap();
                assert_eq!(st.selected_layers().len(), 2);
                let b = st.doc.selection.as_ref().map_or(st.doc.bounds(), |sel| sel.content_bounds().intersect(&st.doc.bounds()));
                let rect = [b.x0, b.y0, b.x1, b.y1];
                let quad = [[b.x0 + 1, b.y0], [b.x1 + 1, b.y0], [b.x1 + 1, b.y1], [b.x0 + 1, b.y1]];
                // Match the rect+quad journal entry made by the interactive box, rather than
                // the matrix-only repeat path (which does not resolve a replacement frame).
                s.execute(
                    "edit.transform",
                    json!({"layer": background.0, "target": {"colors": [0, 2]}, "rect": rect, "quad": quad, "interpolation": "nearest"}),
                )
                .unwrap();
                let st = s.active().unwrap();
                let (before, view, history) = (st.doc.clone(), st.channel_view.clone(), st.history.past_len());
                let mut oracle = Session::new();
                oracle.tools.background = s.tools.background;
                oracle.add_document((*before).clone(), None);
                oracle
                    .execute(
                        "edit.transform",
                        json!({"layer": background.0, "target": {"colors": [0, 2]}, "matrix": [1, 0, 0, 1, 1, 0], "interpolation": "nearest"}),
                    )
                    .unwrap();
                assert!(s.is_enabled("edit.transform.again"));
                s.execute("edit.transform.again", json!({})).unwrap();
                let st = s.active().unwrap();
                assert!(st.doc.layers == oracle.active().unwrap().doc.layers, "{mode:?}/{depth:?}/{kind}");
                assert_eq!(st.doc.selection, oracle.active().unwrap().doc.selection);
                assert_eq!(st.channel_view, view);
                assert_eq!(st.history.past_len(), history + 1);
                assert!(st.doc.is_background(background));
                assert!(s.undo());
                assert!(s.active().unwrap().doc.layers == before.layers);
                assert_eq!(s.active().unwrap().doc.selection, before.selection);
                assert!(s.redo());
                assert!(s.active().unwrap().doc.layers == oracle.active().unwrap().doc.layers);
            }
        }
    }
}

#[test]
fn repeating_a_transform_on_a_copy_refuses_colour_sets_without_side_effects() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    s.execute("edit.transform", json!({"matrix":[1,0,0,1,1,0],"interpolation":"nearest"})).unwrap();
    s.execute("channel.target", json!({"colors":[0,2]})).unwrap();
    let st = s.active().unwrap();
    let (before, view, history) = (st.doc.clone(), st.channel_view.clone(), st.history.past_len());
    let journal = s.journal.clone();
    for params in [json!({}), json!({"target":"pixels"}), json!({"target":{"color":0}})] {
        assert!(s.execute("edit.transform.againCopy", params).is_err(), "a partial set cannot duplicate the full layer");
        let st = s.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&before, &st.doc));
        assert_eq!(st.channel_view, view);
        assert_eq!(st.history.past_len(), history);
        assert_eq!(s.journal, journal);
    }
    assert!(!s.is_enabled("edit.transform.againCopy"));
    let mut oracle = Session::new();
    oracle.tools.background = s.tools.background;
    oracle.add_document((*before).clone(), None);
    oracle.execute("edit.transform", json!({"target":{"colors":[0,2]},"matrix":[1,0,0,1,1,0],"interpolation":"nearest"})).unwrap();
    s.execute("edit.transform.again", json!({})).unwrap();
    assert_eq!(surface(&s), surface(&oracle), "ordinary Repeat still edits the current set");
}
