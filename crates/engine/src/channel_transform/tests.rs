use super::*;
use photocraft_color::{Color, SampleType};
use photocraft_doc::{LayerMask, Size};
use photocraft_geom::warp::{BezierMesh, Warp};
use std::sync::Arc;

fn session(mode: ColorMode, depth: SampleType) -> Session {
    let mut s = Session::new();
    let mut doc = Document::with_background("channels", Size::new(12, 8), mode, depth, Color::WHITE);
    let fmt = doc.pixel_format();
    let mut surface = Surface::new(fmt);
    let n = fmt.channels();
    for y in 0..8 {
        for x in 0..12 {
            let mut pixel: Vec<f32> = (0..n).map(|k| (1 + x + y * 12 + k as i32) as f32 / 120.0).collect();
            if let Some(alpha) = pixel.last_mut() {
                *alpha = 0.75;
            }
            surface.write_pixel(x, y, &pixel);
        }
    }
    doc.layers[0].content = LayerContent::Raster(surface);
    s.add_document(doc, None);
    s
}

fn surface(s: &Session) -> &Surface {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap()
}

fn selection(s: &mut Session, kind: u8) {
    match kind {
        1 => {
            s.execute("select.all", json!({})).unwrap();
        }
        2 | 3 => {
            s.execute("select.rect", json!({"x":2,"y":2,"width":4,"height":3})).unwrap();
            if kind == 3 {
                s.execute("select.modify.feather", json!({"radius":1})).unwrap();
            }
        }
        _ => {}
    }
}

#[test]
fn colour_transforms_preserve_every_other_encoded_component_and_background() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for k in 0..mode.color_channels() {
                for kind in 0..4 {
                    let mut s = session(mode, depth);
                    selection(&mut s, kind);
                    s.execute("channel.target", json!({"channel":{"color":k}})).unwrap();
                    let before = s.active().unwrap().doc.clone();
                    let original = surface(&s).try_to_interleaved(before.bounds()).unwrap();
                    let history = s.active().unwrap().history.past_len();
                    s.execute("edit.transform", json!({"matrix":[1,0,0,1,2,0],"interpolation":"nearest"})).unwrap();
                    let fmt = surface(&s).format();
                    let after = surface(&s).try_to_interleaved(before.bounds()).unwrap();
                    let bytes = depth.bytes();
                    for (a, b) in after.chunks_exact(fmt.bytes_per_pixel()).zip(original.chunks_exact(fmt.bytes_per_pixel())) {
                        for c in 0..fmt.channels() {
                            if c != k {
                                assert_eq!(&a[c * bytes..(c + 1) * bytes], &b[c * bytes..(c + 1) * bytes], "{mode:?}/{depth:?}/{k}/{kind}/{c}");
                            }
                        }
                    }
                    assert_ne!(after, original, "the targeted component must move");
                    let l = s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap();
                    let old = before.layer(l.id).unwrap();
                    assert_eq!(l.name, old.name);
                    assert_eq!(l.locks, old.locks, "Background structural identity is preserved");
                    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
                    s.undo();
                    assert_eq!(surface(&s).try_to_interleaved(before.bounds()).unwrap(), original);
                    s.redo();
                    assert_eq!(surface(&s).try_to_interleaved(before.bounds()).unwrap(), after);
                }
            }
        }
    }
}

#[test]
fn partial_selection_drag_moves_only_its_channel_values_and_fills_the_vacated_area() {
    let mut s = session(ColorMode::Rgb, SampleType::F32);
    selection(&mut s, 2);
    s.tools.background = [0.25, 0.5, 0.75, 1.0];
    let before = surface(&s).clone();
    s.execute("channel.target", json!({"channel":"red"})).unwrap();
    s.execute("edit.transform", json!({"matrix":[1,0,0,1,5,0],"interpolation":"nearest"})).unwrap();
    assert_eq!(surface(&s).sample_channel(2, 2, 0), 0.25);
    assert_eq!(surface(&s).sample_channel(7, 2, 0), before.sample_channel(2, 2, 0));
    assert_eq!(surface(&s).pixel(0, 0), before.pixel(0, 0));
    assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), Rect::new(7, 2, 11, 5));
}

#[test]
fn rotate_flip_perspective_and_warp_use_the_same_colour_target() {
    for command in [
        "edit.transform.flipHorizontal",
        "edit.transform.flipVertical",
        "edit.transform.rotate90Cw",
        "edit.transform.rotate180",
        "edit.transform",
        "edit.transform.warp",
    ] {
        let mut s = session(ColorMode::Cmyk, SampleType::F32);
        selection(&mut s, 2);
        s.execute("channel.target", json!({"channel":{"color":3}})).unwrap();
        let before = surface(&s).clone();
        let p = match command {
            "edit.transform" => json!({"rect":[2,2,6,5],"quad":[[3,2],[7,3],[6,6],[2,5]]}),
            "edit.transform.warp" => {
                let mesh = BezierMesh::identity([2.0, 2.0, 6.0, 5.0], 1, 1).map_points(|p| [p[0] + 3.0, p[1]]);
                json!({"warp":Warp::custom(mesh,[2.0,2.0,6.0,5.0]),"interpolation":"nearest"})
            }
            _ => json!({}),
        };
        s.execute(command, p).unwrap();
        let after = surface(&s);
        for y in 0..8 {
            for x in 0..12 {
                for c in [0, 1, 2, 4] {
                    assert_eq!(before.sample_channel(x, y, c), after.sample_channel(x, y, c), "{command}/{c}");
                }
            }
        }
        assert_ne!(after.to_interleaved(Rect::new(0, 0, 12, 8)), before.to_interleaved(Rect::new(0, 0, 12, 8)), "{command}");
    }
}

#[test]
fn explicit_target_overrides_the_panel_and_an_explicit_layer_moves_whole() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    s.execute("channel.target", json!({"channel":"red"})).unwrap();
    assert_eq!(params(&s, &json!({})).unwrap()["target"], json!({"color":0}));
    let id = s.active().unwrap().active_layer.unwrap();
    assert_eq!(params(&s, &json!({"layer":id.0})).unwrap()["target"], "pixels");
    assert_eq!(params(&s, &json!({"layers":[id.0]})).unwrap()["target"], "pixels");
    assert_eq!(params(&s, &json!({"layers":null})).unwrap()["target"], json!({"color":0}));
    assert_eq!(params(&s, &json!({"layer":id.0,"target":{"color":2}})).unwrap()["target"], json!({"color":2}));
}

#[test]
fn targeted_linked_and_unlinked_masks_transform_alone_including_warp() {
    for linked in [false, true] {
        for warp in [false, true] {
            let mut s = session(ColorMode::Rgb, SampleType::U16);
            s.edit("mask", |doc, active| {
                let mut mask = LayerMask::reveal_all();
                mask.linked = linked;
                mask.surface.fill_rect(Rect::new(2, 2, 5, 5), &[0.0]);
                doc.layer_mut(active.unwrap()).unwrap().mask = Some(mask);
                Ok(())
            })
            .unwrap();
            let before = surface(&s).to_interleaved(Rect::new(0, 0, 12, 8));
            s.execute("channel.target", json!({"channel":"mask"})).unwrap();
            let (cmd, p) = if warp {
                let mesh = BezierMesh::identity([2.0, 2.0, 5.0, 5.0], 1, 1).map_points(|p| [p[0] + 4.0, p[1]]);
                ("edit.transform.warp", json!({"warp":Warp::custom(mesh,[2.0,2.0,5.0,5.0]),"interpolation":"nearest"}))
            } else {
                ("edit.transform", json!({"matrix":[1,0,0,1,4,0],"interpolation":"nearest"}))
            };
            s.execute(cmd, p).unwrap();
            assert_eq!(surface(&s).to_interleaved(Rect::new(0, 0, 12, 8)), before);
            let st = s.active().unwrap();
            let mask = st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap();
            assert_eq!(mask.linked, linked);
            assert_eq!(mask.surface.sample_channel(3, 3, 0), 1.0);
            assert_eq!(mask.surface.sample_channel(7, 3, 0), 0.0);
        }
    }
}

#[test]
fn invalid_targets_allocation_failure_and_locks_leave_pixels_unchanged() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    let before = s.active().unwrap().doc.clone();
    for target in [json!({"color":99}), json!({"color":-1}), json!({"color":"red"}), json!({"channel":999}), json!("unknown")] {
        assert!(s.execute("edit.transform", json!({"target":target,"matrix":[1,0,0,1,2,0]})).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
    assert!(s.execute("edit.transform", json!({"layer":"invalid","matrix":[1,0,0,1,2,0]})).is_err());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    crate::allocation::fail_next_for_test();
    assert!(s.execute("edit.transform", json!({"target":{"color":0},"matrix":[1,0,0,1,2,0]})).is_err());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    let steps = s.active().unwrap().history.past_len();
    assert!(s.execute("edit.transform", json!({"target":{"color":0},"matrix":[1e8,0,0,1e8,0,0]})).is_err());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("lock", |doc, _| {
        doc.layer_mut(id).unwrap().locks.all = true;
        Ok(())
    })
    .unwrap();
    // A non-Background locked raster must refuse rather than silently unlocking it.
    s.edit("name", |doc, _| {
        doc.layer_mut(id).unwrap().locks.transparency = false;
        Ok(())
    })
    .unwrap();
    let locked = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    assert!(s.execute("edit.transform", json!({"target":{"color":0},"matrix":[1,0,0,1,2,0]})).is_err());
    assert!(Arc::ptr_eq(&locked, &s.active().unwrap().doc));
    assert_eq!(s.active().unwrap().history.past_len(), steps);
}

#[test]
fn interactive_guard_blocks_retargeting_but_all_eyes_remain_available() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    s.execute("channel.target", json!({"channel":"red"})).unwrap();
    let st = s.active_mut().unwrap();
    st.channel_transform = Some(Guard { layer: st.active_layer, target: json!({"color":0}) });
    let before = st.doc.clone();
    let history = st.history.past_len();
    let view = st.channel_view.clone();
    for (cmd, p) in [
        ("channel.target", json!({"channel":"blue"})),
        ("channel.target.slot4", json!({})),
        ("channel.new", json!({})),
        ("edit.fill", json!({"color":"#000000"})),
        ("timeline.create", json!({"duration":3})),
        ("timeline.setFrame", json!({"frame":1})),
    ] {
        assert!(s.execute(cmd, p).unwrap_err().to_string().contains(LOCKED));
        let st = s.active().unwrap();
        assert!(Arc::ptr_eq(&before, &st.doc), "{cmd}");
        assert_eq!(st.history.past_len(), history, "{cmd}");
        assert_eq!(st.channel_view, view, "{cmd}");
    }
    assert!(s.edit::<()>("direct edit", |_, _| panic!("locked edit closure must never run")).unwrap_err().to_string().contains(LOCKED));
    s.execute("channel.setVisible", json!({"channel":"composite","visible":false})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.visible_colors(3), 0);
    assert!(!visible(&s, &json!({"target":{"color":0}})).unwrap());
    s.execute("channel.setVisible", json!({"channel":"green","visible":true})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Color(0));
    s.clear_channel_transforms();
    s.execute("channel.target", json!({"channel":"blue"})).unwrap();
}

#[test]
fn history_edit_paths_preserve_a_captured_channel_transform() {
    for undoable_visibility in [false, true] {
        for non_linear in [false, true] {
            let mut s = session(ColorMode::Rgb, SampleType::U8);
            s.execute("history.options", json!({"makeLayerVisibilityChangesUndoable":undoable_visibility,"allowNonLinearHistory":non_linear})).unwrap();
            s.edit("before capture", |doc, _| {
                doc.name = "captured document".into();
                Ok(())
            })
            .unwrap();
            s.execute("channel.target", json!({"channel":"red"})).unwrap();
            let st = s.active_mut().unwrap();
            st.channel_transform = Some(Guard { layer: st.active_layer, target: json!({"color":0}) });
            let before = st.doc.clone();
            let history = st.history.entries();
            let view = st.channel_view.clone();
            let active = st.active_layer;
            assert!(s.edit_visibility::<()>("visibility", |_, _| panic!("locked visibility edit must never run")).unwrap_err().to_string().contains(LOCKED));
            for record in [false, true] {
                assert!(s.edit_with::<()>("direct edit", record, |_, _| panic!("locked edit must never run")).unwrap_err().to_string().contains(LOCKED));
            }
            assert!(!s.rollback(), "rollback must not replace the captured document");
            let st = s.active().unwrap();
            assert!(Arc::ptr_eq(&before, &st.doc));
            assert_eq!(st.history.entries(), history);
            assert_eq!(st.channel_view, view);
            assert_eq!(st.active_layer, active);
            assert!(locked(&s));
            s.clear_channel_transforms();
            assert!(s.rollback(), "rollback works after the capture is released");
        }
    }
}

#[test]
fn prechecks_cannot_bypass_the_captured_channel_target() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = session(ColorMode::Rgb, depth);
        s.execute("channel.target", json!({"channel":"red"})).unwrap();
        let st = s.active_mut().unwrap();
        let layer = st.active_layer.unwrap();
        st.channel_transform = Some(Guard { layer: Some(layer), target: json!({"color":0}) });
        let before = st.doc.clone();
        let history = st.history.past_len();
        for id in crate::PRECHECKED.iter().copied().chain(["layer.renameLayer"]) {
            let p = if id == "layer.renameLayer" { json!({"layer":layer.0,"name":"must not rename"}) } else { json!({}) };
            assert!(s.precheck(id, &p).unwrap_err().to_string().contains(LOCKED), "{id}");
            assert!(s.execute(id, p).unwrap_err().to_string().contains(LOCKED), "{id}");
            assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().history.past_len(), history);
            assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Color(0));
            assert!(locked(&s));
        }
        let eye = json!({"channel":"red","visible":false});
        s.precheck("channel.setVisible", &eye).unwrap();
        s.execute("channel.setVisible", eye).unwrap();
        assert!(locked(&s));
        assert!(!visible(&s, &json!({"target":{"color":0}})).unwrap());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn wait_for_job(s: &mut Session, job: crate::jobs::JobId) -> crate::jobs::JobEvent {
    let started = std::time::Instant::now();
    loop {
        if let Some(event) = s.poll_jobs().into_iter().find(|event| event.id == job) {
            return event;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn panel_channel_background_jobs_hold_the_target_until_commit_or_cancel() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for cancel in [false, true] {
            check_background_job(depth, cancel);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn check_background_job(depth: SampleType, cancel: bool) {
    let mut s = session(ColorMode::Rgb, depth);
    s.execute("channel.target", json!({"channel":"green"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    // No explicit target: the running job must retain the resolved panel target.
    let crate::jobs::Started::Job(job) = s.start("edit.transform", json!({"matrix":[1,0,0,1,2,0],"interpolation":"nearest"})).unwrap() else {
        panic!("native transforms run as background jobs");
    };
    assert!(locked(&s));
    assert!(s.execute("channel.target", json!({"channel":"blue"})).is_err());
    assert!(s.edit::<()>("direct edit", |_, _| panic!("locked work must never run")).is_err());
    s.execute("channel.setVisible", json!({"channel":"green","visible":false})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.visible_colors(3), 0, "hiding the last eye must not restore RGB during commit");
    assert!(locked(&s));
    s.execute("channel.setVisible", json!({"channel":"green","visible":true})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.visible_colors(3), 1);
    s.execute("channel.setVisible", json!({"channel":"composite","visible":false})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.visible_colors(3), 0, "the composite eye can hide every colour during commit");
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Color(1));
    let view = s.active().unwrap().channel_view.clone();
    if cancel {
        assert!(s.cancel_job(job));
    }
    let event = wait_for_job(&mut s, job);
    assert!(!locked(&s));
    let st = s.active().unwrap();
    assert_eq!(st.channel_view, view);
    assert_eq!(st.history.past_len(), steps + usize::from(!cancel));
    if cancel {
        assert_eq!(event.outcome, crate::jobs::JobOutcome::Cancelled);
        assert!(Arc::ptr_eq(&before, &st.doc));
    } else {
        assert!(matches!(event.outcome, crate::jobs::JobOutcome::Done(_)), "{:?}", event.outcome);
        let original = before.layers[0].surface().unwrap().try_to_interleaved(before.bounds()).unwrap();
        let moved = surface(&s).try_to_interleaved(before.bounds()).unwrap();
        assert_ne!(moved, original);
        let fmt = surface(&s).format();
        let size = depth.bytes();
        for (a, b) in moved.chunks_exact(fmt.bytes_per_pixel()).zip(original.chunks_exact(fmt.bytes_per_pixel())) {
            for c in [0, 2, 3] {
                assert_eq!(&a[c * size..(c + 1) * size], &b[c * size..(c + 1) * size]);
            }
        }
        assert!(s.undo());
        assert_eq!(surface(&s).try_to_interleaved(before.bounds()).unwrap(), original);
        assert_eq!(s.active().unwrap().channel_view, view);
        assert!(s.redo());
        assert_eq!(surface(&s).try_to_interleaved(before.bounds()).unwrap(), moved);
        assert_eq!(s.active().unwrap().channel_view, view);
    }
    s.execute("channel.target", json!({"channel":"blue"})).unwrap();
}

#[test]
fn interactive_guard_preserves_document_and_history_for_every_undo_entry_point() {
    for redo in [false, true] {
        let mut s = session(ColorMode::Rgb, SampleType::U8);
        s.execute("edit.fill", json!({"color":"#123456"})).unwrap();
        if redo {
            assert!(s.undo());
        }
        let st = s.active_mut().unwrap();
        st.channel_transform = Some(Guard { layer: st.active_layer, target: json!({"color":1}) });
        let before = st.doc.clone();
        let history = st.history.entries();
        let future: Vec<_> = st.history.redo_labels().map(str::to_owned).collect();
        let revision = st.revision;
        for id in if redo { ["edit.redo", "history.redo"] } else { ["edit.undo", "history.undo"] } {
            assert!(!s.is_enabled(id), "{id}");
            assert!(s.execute(id, json!({})).unwrap_err().to_string().contains(LOCKED));
        }
        assert!(!s.is_enabled("history.selectState"));
        assert!(s.execute("history.selectState", json!({"state":0})).unwrap_err().to_string().contains(LOCKED));
        assert!(!if redo { s.redo() } else { s.undo() });
        let st = s.active().unwrap();
        assert!(Arc::ptr_eq(&before, &st.doc));
        assert_eq!(st.history.entries(), history);
        assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), future);
        assert_eq!(st.revision, revision);
        assert!(st.channel_transform.is_some());
        s.clear_channel_transforms();
        assert!(if redo { s.redo() } else { s.undo() });
    }
}

#[test]
fn interactive_guard_allows_inline_mesh_edits_but_refuses_stored_warp_edits() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let st = s.active_mut().unwrap();
    st.channel_transform = Some(Guard { layer: st.active_layer, target: json!("mask") });
    let before = st.doc.clone();
    let history = st.history.entries();
    let rect = [0.0, 0.0, 12.0, 8.0];
    let w = Warp::custom(BezierMesh::identity(rect, 1, 1), rect);
    for id in ["edit.transform.splitWarpCrosswise", "edit.transform.splitWarpHorizontally", "edit.transform.splitWarpVertically", "edit.transform.warpGrid"] {
        let p = json!({"warp":w,"size":3});
        assert!(s.is_enabled_with(id, &p), "{id}");
        let out = s.execute(id, p).unwrap();
        let edited: Warp = serde_json::from_value(out["warp"].clone()).unwrap();
        assert!(edited.mesh.unwrap().points.len() > 16, "{id}");
        assert!(s.execute(id, json!({"size":3})).unwrap_err().to_string().contains(LOCKED));
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().history.entries(), history);
    }
    let split = s.execute("edit.transform.splitWarpCrosswise", json!({"warp":w})).unwrap();
    s.execute("edit.transform.removeWarpSplit", json!({"warp":split["warp"],"at":[6,4]})).unwrap();
    for invalid in [Value::Null, json!({"mesh":{"points":[]}})] {
        assert!(s.execute("edit.transform.warpGrid", json!({"warp":invalid,"size":3})).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
}

#[test]
fn vacated_cmyk_components_use_the_embedded_profile_for_affine_and_warp() {
    let profile = photocraft_cms::synth::cmyk_profile(&photocraft_cms::synth::CmykParams {
        description: "Channel Test Uncoated".into(),
        tvi: [0.26, 0.26, 0.26, 0.3],
        grid_a2b: 5,
        grid_b2a: 9,
        ..Default::default()
    })
    .to_bytes();
    let space = photocraft_color::convert::CmykSpace::for_profile(Some(&profile)).unwrap();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for warp in [false, true] {
            for k in 0..4 {
                let mut s = session(ColorMode::Cmyk, depth);
                s.edit("ICC", |doc, _| {
                    doc.icc_profile = Some(profile.clone());
                    Ok(())
                })
                .unwrap();
                s.tools.background = [0.8, 0.2, 0.4, 1.0];
                let fmt = surface(&s).format();
                let expected = photocraft_color::convert::with_cmyk_space(Some(&space), || from_rgba(&fmt, s.tools.background));
                let mut p = if warp {
                    let rect = [0.0, 0.0, 12.0, 8.0];
                    json!({"warp":Warp::custom(BezierMesh::identity(rect,1,1).map_points(|p| [p[0]+2.0,p[1]]),rect)})
                } else {
                    json!({"matrix":[1,0,0,1,2,0]})
                };
                p["target"] = json!({"color":k});
                p["interpolation"] = json!("nearest");
                s.execute(if warp { "edit.transform.warp" } else { "edit.transform" }, p).unwrap();
                assert!((surface(&s).sample_channel(0, 0, k) - expected[k]).abs() < 0.005, "{depth:?}/{warp}/{k}");
                assert!(photocraft_color::convert::active_cmyk_space().is_none(), "ICC scope must be restored");
            }
        }
    }
}

#[test]
fn isolated_warp_preserves_samples_outside_an_explicit_frame() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for target in [json!({"color":1}), json!("mask"), json!({"channel":0}), json!("quickMask")] {
            for selected in [false, true] {
                let mut s = session(ColorMode::Rgb, depth);
                s.execute("channel.new", json!({})).unwrap();
                s.execute("channel.target.composite", json!({})).unwrap();
                s.execute("select.editInQuickMaskMode", json!({"on":true})).unwrap();
                s.edit("scalar patterns", |doc, active| {
                    let mut pattern = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
                    for y in 0..8 {
                        for x in 0..12 {
                            pattern.write_pixel(x, y, &[(x + y + 1) as f32 / 24.0]);
                        }
                    }
                    doc.layer_mut(active.unwrap()).unwrap().mask = Some(LayerMask { surface: pattern.clone(), ..LayerMask::reveal_all() });
                    doc.channels[0].surface = pattern.clone();
                    doc.quick_mask.as_mut().unwrap().surface = pattern;
                    Ok(())
                })
                .unwrap();
                if selected {
                    s.execute("select.all", json!({})).unwrap();
                }
                let before = s.active().unwrap().doc.clone();
                let layer = s.active().unwrap().active_layer;
                let source = plane(&before, layer, &json!({"target":target}), s.tools.background).unwrap().unwrap();
                let rect = [4.0, 2.0, 8.0, 6.0];
                let mesh = BezierMesh::identity(rect, 1, 1).map_points(|p| [p[0] + 1.0, p[1]]);
                s.execute("edit.transform.warp", json!({"target":target,"rect":rect,"warp":Warp::custom(mesh,rect),"interpolation":"nearest"})).unwrap();
                let after = plane(&s.active().unwrap().doc, layer, &json!({"target":target}), s.tools.background).unwrap().unwrap();
                for y in 0..8 {
                    for x in 0..12 {
                        if !(4..9).contains(&x) || !(2..6).contains(&y) {
                            assert_eq!(after.pixel(x, y), source.pixel(x, y), "{depth:?}/{target}/{selected}/({x},{y})");
                        }
                    }
                }
                assert_eq!(after.pixel(6, 3), source.pixel(5, 3), "{depth:?}/{target}/{selected}: inside frame moves");
                let result = s.active().unwrap().doc.clone();
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc.layers, before.layers);
                assert_eq!(s.active().unwrap().doc.channels, before.channels);
                assert!(s.redo());
                assert_eq!(s.active().unwrap().doc.layers, result.layers);
            }
        }
    }
}

#[test]
fn whole_layer_warp_carries_linked_mask_content_outside_its_frame() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for smart in [false, true] {
            let mut s = session(ColorMode::Rgb, depth);
            s.edit("small pixels", |doc, active| {
                let l = doc.layer_mut(active.unwrap()).unwrap();
                let mut pixels = Surface::new(l.surface().unwrap().format());
                pixels.fill_rect(Rect::new(2, 2, 5, 5), &[0.2, 0.4, 0.6, 1.0]);
                l.content = LayerContent::Raster(pixels);
                Ok(())
            })
            .unwrap();
            if smart {
                s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
                // Whole smart objects carry the entire linked mask even with a selection.
                selection(&mut s, 2);
            }
            s.edit("outlying linked mask", |doc, active| {
                let mut mask = LayerMask::reveal_all();
                mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, depth, false), &[1.0]);
                mask.surface.fill_rect(Rect::new(8, 2, 11, 5), &[0.0]);
                doc.layer_mut(active.unwrap()).unwrap().mask = Some(mask);
                Ok(())
            })
            .unwrap();
            let before = s.active().unwrap().doc.clone();
            let rect = [2.0, 2.0, 5.0, 5.0];
            // The explicit pixel frame is smaller than the map's domain. Linked masks are
            // transformed in their entirety, rather than clipped to that pixel frame.
            let bounds = [0.0, 0.0, 12.0, 8.0];
            let w = Warp::custom(BezierMesh::identity(bounds, 1, 1).map_points(|p| [p[0], p[1] + 3.0]), bounds);
            s.execute("edit.transform.warp", json!({"target":"pixels","rect":rect,"warp":w,"interpolation":"nearest"})).unwrap();
            let after = s.active().unwrap().doc.clone();
            let mask = &after.layers[0].mask.as_ref().unwrap().surface;
            assert_eq!(mask.sample_channel(8, 2, 0), 1.0, "{depth:?}/{smart}: old mask clears");
            assert_eq!(mask.sample_channel(8, 5, 0), 0.0, "{depth:?}/{smart}: outlying mask follows");
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc.layers, before.layers);
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc.layers, after.layers);
        }
    }
}

#[test]
fn whole_smart_object_warp_moves_all_of_its_linked_mask_with_a_selection() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.edit("mask", |doc, active| {
        let mut mask = LayerMask::reveal_all();
        mask.surface.fill_rect(Rect::new(2, 2, 4, 4), &[0.0]);
        mask.surface.fill_rect(Rect::new(9, 2, 11, 4), &[0.0]);
        doc.layer_mut(active.unwrap()).unwrap().mask = Some(mask);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x":0,"y":0,"width":6,"height":6})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let rect = [0.0, 0.0, 12.0, 8.0];
    let mesh = BezierMesh::identity(rect, 1, 1).map_points(|p| [p[0], p[1] + 3.0]);
    s.execute("edit.transform.warp", json!({"target":"pixels","rect":rect,"warp":Warp::custom(mesh,rect),"interpolation":"nearest"})).unwrap();
    let st = s.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert!(matches!(layer.content, LayerContent::Smart(_)));
    let mask = &layer.mask.as_ref().unwrap().surface;
    for x in [2, 9] {
        assert_eq!(mask.sample_channel(x, 2, 0), 1.0, "old mask region clears at {x}");
        assert_eq!(mask.sample_channel(x, 5, 0), 0.0, "linked mask follows whole object at {x}");
    }
    assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(0, 3, 12, 11));
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
}

#[test]
fn selected_default_mask_samples_move_with_the_warp_and_the_outline_moves_once() {
    let mut s = session(ColorMode::Rgb, SampleType::F32);
    s.edit("mask defaults", |doc, active| {
        let mut mask = LayerMask::reveal_all();
        mask.surface.fill_rect(Rect::new(3, 3, 4, 4), &[0.0]);
        mask.surface.fill_rect(Rect::new(8, 2, 12, 5), &[0.0]);
        doc.layer_mut(active.unwrap()).unwrap().mask = Some(mask);
        Ok(())
    })
    .unwrap();
    selection(&mut s, 2);
    let st = s.active().unwrap();
    assert_eq!(source_bounds(&st.doc, st.active_layer, &json!({"target":"mask"})).unwrap(), Some(Rect::new(2, 2, 6, 5)));
    let rect = [2.0, 2.0, 6.0, 5.0];
    s.execute(
        "edit.transform.warp",
        json!({"target":"mask","warp":Warp::custom(BezierMesh::identity(rect,1,1).map_points(|p| [p[0]+6.0,p[1]]),rect),"interpolation":"nearest"}),
    )
    .unwrap();
    let st = s.active().unwrap();
    let mask = &st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface;
    assert_eq!(mask.sample_channel(3, 3, 0), 1.0);
    assert_eq!(mask.sample_channel(8, 2, 0), 1.0, "selected default values overwrite the destination");
    assert_eq!(mask.sample_channel(9, 3, 0), 0.0);
    assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(8, 2, 12, 5));
}

#[test]
fn hdr_colour_samples_survive_all_resampling_paths() {
    for kind in 0..4 {
        for warp in [false, true] {
            let mut s = session(ColorMode::Rgb, SampleType::F32);
            s.edit("HDR", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 12, 8), &[-2.0, 4.0, 1.5, 0.75]);
                Ok(())
            })
            .unwrap();
            selection(&mut s, kind);
            let (cmd, mut p) = if warp {
                let mesh = BezierMesh::identity([0.0, 0.0, 12.0, 8.0], 1, 1).map_points(|p| [p[0] + 1.0, p[1]]);
                ("edit.transform.warp", json!({"warp":Warp::custom(mesh,[0.0,0.0,12.0,8.0])}))
            } else {
                ("edit.transform", json!({"matrix":[1,0,0,1,1,0]}))
            };
            p["target"] = json!({"color":1});
            p["interpolation"] = json!("nearest");
            s.execute(cmd, p).unwrap();
            assert!(surface(&s).sample_channel(4, 3, 1) > 1.0, "{kind}/{warp}: HDR must not clip");
            assert_eq!(surface(&s).pixel(4, 3)[0], -2.0);
        }
    }
}

#[test]
fn saved_alpha_spot_and_quick_mask_transform_without_touching_layer_pixels() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for target in [json!({"channel":0}), json!("quickMask")] {
            for kind in 0..4 {
                for warp in [false, true] {
                    let mut s = session(ColorMode::Rgb, depth);
                    s.execute("channel.newSpot", json!({"name":"Ink"})).unwrap();
                    if target == "quickMask" {
                        s.execute("channel.target.composite", json!({})).unwrap();
                        s.execute("select.editInQuickMaskMode", json!({"on":true})).unwrap();
                    }
                    s.edit("channel pattern", |doc, active| {
                        let (ch, _) = crate::channel_cmds::target_surface(doc, *active, &json!({"target":target}))?;
                        *ch = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
                        ch.fill_rect(Rect::new(2, 2, 6, 5), &[0.7]);
                        Ok(())
                    })
                    .unwrap();
                    selection(&mut s, kind);
                    let original = surface(&s).try_to_interleaved(Rect::new(0, 0, 12, 8)).unwrap();
                    let before = s.active().unwrap().doc.clone();
                    let (cmd, mut p) = if warp {
                        let mesh = BezierMesh::identity([2.0, 2.0, 6.0, 5.0], 1, 1).map_points(|p| [p[0] + 3.0, p[1]]);
                        ("edit.transform.warp", json!({"warp":Warp::custom(mesh,[2.0,2.0,6.0,5.0])}))
                    } else {
                        ("edit.transform", json!({"matrix":[1,0,0,1,3,0]}))
                    };
                    p["target"] = target.clone();
                    p["interpolation"] = json!("nearest");
                    s.execute(cmd, p).unwrap();
                    assert_eq!(surface(&s).try_to_interleaved(Rect::new(0, 0, 12, 8)).unwrap(), original);
                    let st = s.active().unwrap();
                    let ch = crate::channel_cmds::read_target_surface(&st.doc, st.active_layer, &json!({"target":target})).unwrap();
                    assert!(ch.sample_channel(7, 3, 0) > if kind == 3 { 0.3 } else { 0.65 }, "{depth:?}/{target}/{kind}/{warp}");
                    let original_ch = crate::channel_cmds::read_target_surface(&before, st.active_layer, &json!({"target":target})).unwrap().clone();
                    s.undo();
                    let st = s.active().unwrap();
                    assert_eq!(crate::channel_cmds::read_target_surface(&st.doc, st.active_layer, &json!({"target":target})).unwrap(), &original_ch);
                }
            }
        }
    }
}

#[test]
fn whole_layer_warp_moves_linked_mask_only_inside_selection() {
    // #4175 / #4176: a whole-layer warp carries only the selected part of a linked raster mask.
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    s.edit("mask", |doc, active| {
        let mut mask = LayerMask::reveal_all();
        mask.surface.fill_rect(Rect::new(2, 2, 4, 5), &[0.0]);
        mask.surface.fill_rect(Rect::new(9, 2, 11, 5), &[0.0]);
        doc.layer_mut(active.unwrap()).unwrap().mask = Some(mask);
        Ok(())
    })
    .unwrap();
    selection(&mut s, 2);
    let mesh = BezierMesh::identity([2.0, 2.0, 6.0, 5.0], 1, 1).map_points(|p| [p[0], p[1] + 3.0]);
    s.execute("edit.transform.warp", json!({"target":"pixels","warp":Warp::custom(mesh,[2.0,2.0,6.0,5.0]),"interpolation":"nearest"})).unwrap();
    let st = s.active().unwrap();
    let mask = &st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface;
    assert_eq!(mask.sample_channel(2, 3, 0), 1.0);
    assert_eq!(mask.sample_channel(2, 6, 0), 0.0);
    assert_eq!(mask.sample_channel(9, 3, 0), 0.0, "unselected mask stays put");
    assert_eq!(mask.sample_channel(9, 6, 0), 1.0);
    assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(2, 5, 6, 8), "outline moves once");
}

#[test]
fn vacated_colour_uses_native_background_and_explicit_layer_bypasses_panel() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for k in 0..mode.color_channels() {
                let mut s = session(mode, depth);
                s.tools.background = [0.3, 0.5, 0.7, 1.0];
                let fill = from_rgba(&surface(&s).format(), s.tools.background)[k];
                s.execute("edit.transform", json!({"target":{"color":k},"matrix":[1,0,0,1,2,0],"interpolation":"nearest"})).unwrap();
                assert!((surface(&s).sample_channel(0, 0, k) - fill).abs() < 0.005, "{mode:?}/{depth:?}/{k}");
            }
        }
    }
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    s.execute("channel.target", json!({"channel":"red"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("edit.transform", json!({"layer":id.0,"matrix":[1,0,0,1,2,0]})).unwrap();
    assert_eq!(surface(&s).sample_channel(0, 0, 3), 0.0, "explicit layer moves transparency too");
}

#[test]
fn named_layer_transforms_keep_upstream_relative_params_and_explicit_targets() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for command in ["edit.transform", "edit.transform.flipHorizontal", "edit.transform.warp"] {
            for isolated in [false, true] {
                let mut named = session(ColorMode::Rgb, depth);
                let mut numeric = session(ColorMode::Rgb, depth);
                for s in [&mut named, &mut numeric] {
                    s.execute("channel.target", json!({"channel":"green"})).unwrap();
                }
                let st = named.active().unwrap();
                let id = st.active_layer.unwrap();
                let name = st.doc.layer(id).unwrap().name.clone();
                let before = surface(&named).try_to_interleaved(st.doc.bounds()).unwrap();
                let mut p = match command {
                    "edit.transform" => json!({"dx":2,"interpolation":"nearest"}),
                    "edit.transform.warp" => {
                        let frame = [0.0, 0.0, 12.0, 8.0];
                        let mesh = BezierMesh::identity(frame, 1, 1).map_points(|p| [p[0] + 2.0, p[1]]);
                        json!({"warp":Warp::custom(mesh,frame),"interpolation":"nearest"})
                    }
                    _ => json!({}),
                };
                if isolated {
                    p["target"] = json!({"color":0});
                }
                p["layer"] = json!(name);
                named.execute(command, p.clone()).unwrap();
                p["layer"] = json!(numeric.active().unwrap().active_layer.unwrap().0);
                numeric.execute(command, p).unwrap();
                let bounds = named.active().unwrap().doc.bounds();
                let got = surface(&named).try_to_interleaved(bounds).unwrap();
                assert_eq!(got, surface(&numeric).try_to_interleaved(bounds).unwrap(), "{command}/{depth:?}/isolated={isolated}");
                assert_ne!(got, before, "the named layer must be transformed");
                assert_eq!(named.active().unwrap().channel_view.target, ChannelTarget::Color(1));
            }
        }
    }
}

#[test]
fn saved_channel_explicit_targets_need_no_active_layer() {
    for command in ["edit.transform", "edit.transform.flipHorizontal", "edit.transform.warp"] {
        let mut s = session(ColorMode::Rgb, SampleType::U8);
        s.execute("channel.new", json!({})).unwrap();
        s.edit("paint", |doc, _| {
            doc.channels[0].surface.fill_rect(Rect::new(2, 2, 6, 5), &[0.5]);
            Ok(())
        })
        .unwrap();
        s.active_mut().unwrap().active_layer = None;
        let mut p = match command {
            "edit.transform" => json!({"matrix":[1,0,0,1,2,0]}),
            "edit.transform.warp" => json!({"style":"arc","bend":20}),
            _ => json!({}),
        };
        p["target"] = json!({"channel":0});
        s.execute(command, p).unwrap();
        assert!(s.active().unwrap().active_layer.is_none());
    }
}

#[test]
fn native_raster_masks_honour_selections_at_every_depth_with_or_without_linking() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for linked in [false, true] {
            for kind in 0..4 {
                for warp in [false, true] {
                    let mut s = session(ColorMode::Rgb, depth);
                    s.edit("native mask", |doc, active| {
                        let mut mask = LayerMask::reveal_all();
                        mask.linked = linked;
                        mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, depth, false), &[1.0]);
                        mask.surface.fill_rect(Rect::new(2, 2, 5, 5), &[0.0]);
                        doc.layer_mut(active.unwrap()).unwrap().mask = Some(mask);
                        Ok(())
                    })
                    .unwrap();
                    selection(&mut s, kind);
                    let before = s.active().unwrap().doc.clone();
                    let (cmd, mut p) = if warp {
                        let mesh = BezierMesh::identity([2.0, 2.0, 6.0, 5.0], 1, 1).map_points(|p| [p[0] + 4.0, p[1]]);
                        ("edit.transform.warp", json!({"warp":Warp::custom(mesh,[2.0,2.0,6.0,5.0])}))
                    } else {
                        ("edit.transform", json!({"matrix":[1,0,0,1,4,0]}))
                    };
                    p["target"] = json!("mask");
                    p["interpolation"] = json!("nearest");
                    s.execute(cmd, p).unwrap();
                    let st = s.active().unwrap();
                    let id = st.active_layer.unwrap();
                    let layer = st.doc.layer(id).unwrap();
                    assert_eq!(layer.surface(), before.layer(id).unwrap().surface());
                    let mask = layer.mask.as_ref().unwrap();
                    assert_eq!(mask.linked, linked);
                    assert_eq!(mask.surface.format().sample, depth);
                    assert!(mask.surface.sample_channel(7, 3, 0) < 0.6, "{depth:?}/{linked}/{kind}/{warp}");
                    assert!(mask.surface.sample_channel(3, 3, 0) > 0.4);
                    s.undo();
                    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().mask, before.layer(id).unwrap().mask);
                }
            }
        }
    }
}

/// Merging #6967 must keep its selection-centred presets for explicit whole layers while
/// the same commands still isolate an explicitly targeted colour component.
#[test]
fn preset_selection_pivots_preserve_explicit_layer_and_colour_target_precedence() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for all in [false, true] {
            let destinations = if all { [(9, 2), (2, 5), (9, 5), (7, 0), (4, 7)] } else { [(7, 2), (2, 5), (7, 5), (6, 1), (3, 6)] };
            for (command, (x, y)) in [
                "edit.transform.flipHorizontal",
                "edit.transform.flipVertical",
                "edit.transform.rotate180",
                "edit.transform.rotate90Cw",
                "edit.transform.rotate90Ccw",
            ]
            .into_iter()
            .zip(destinations)
            {
                for isolated in [false, true] {
                    let mut s = session(ColorMode::Rgb, depth);
                    s.execute("layer.new.layer", json!({})).unwrap();
                    s.edit("asymmetric marker", |doc, active| {
                        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().write_pixel(2, 2, &[0.8, 0.6, 0.4, 0.75]);
                        Ok(())
                    })
                    .unwrap();
                    s.execute("channel.target", json!({"channel":"green"})).unwrap();
                    s.execute("channel.setVisible", json!({"channel":"composite","visible":true})).unwrap();
                    if all {
                        s.execute("select.all", json!({})).unwrap();
                    } else {
                        s.execute("select.rect", json!({"x":0,"y":0,"width":10,"height":8})).unwrap();
                    }
                    let st = s.active().unwrap();
                    let id = st.active_layer.unwrap();
                    let before = st.doc.clone();
                    let view = st.channel_view.clone();
                    let history = st.history.past_len();
                    let original = before.layer(id).unwrap().surface().unwrap();
                    let mut p = json!({"layer":id.0});
                    if isolated {
                        p["target"] = json!({"color":0});
                    }
                    s.execute(command, p).unwrap();
                    let after = surface(&s);
                    assert_eq!(after.sample_channel(x, y, 0), original.sample_channel(2, 2, 0), "{command}/{depth:?}/{all}/{isolated}");
                    if isolated {
                        for y in 0..8 {
                            for x in 0..12 {
                                for c in [1, 2, 3] {
                                    assert_eq!(after.sample_channel(x, y, c).to_bits(), original.sample_channel(x, y, c).to_bits());
                                }
                            }
                        }
                    } else {
                        assert_eq!(after.pixel(x, y), original.pixel(2, 2));
                        assert_eq!(after.sample_channel(2, 2, 3), 0.0, "an explicit layer moves all its selected samples");
                    }
                    assert_eq!(s.active().unwrap().channel_view, view);
                    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
                    s.undo();
                    assert_eq!(s.active().unwrap().doc, before);
                }
            }
        }
    }
}
