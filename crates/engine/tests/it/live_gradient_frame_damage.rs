//! Known live-preview residual of merged #5340: committed damage already includes relayout.
//! These CPU comparisons cover the public live producer, not GPU paging or a native window.
use photocraft_compose::{Buffer, effects_frame, render, render_reduced_damage};
use photocraft_engine::{Session, brush_cmds::LiveStroke, paint::StrokePoint};
use photocraft_geom::Rect;
use serde_json::json;

#[derive(Clone, Copy)]
enum Case {
    Horizontal,
    NoOverlay,
    UnchangedFrame,
    Vertical,
}

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 2e-5, "actual {actual}, independently expected {expected}");
}

/// Check all RGB channels and opacity against an independently derived grey value.
fn opaque_grey(buffer: &Buffer, x: i32, expected: f32) {
    let pixel = buffer.get(x, 32);
    for actual in &pixel[..3] {
        near(*actual, expected);
    }
    near(pixel[3], 1.0);
}

/// Compare every retained and repainted channel with the full current CPU render.
fn same(shown: &Buffer, cold: &Buffer) {
    assert_eq!(shown.rect, cold.rect);
    let worst = shown.px.iter().zip(&cold.px).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
    assert!(worst < 2e-5, "live partial redraw must match the full current render; largest channel error {worst}");
}

/// Establish literal old/new gradient values, with unchanged-mapping and no-overlay controls.
fn reference_colours(case: Case, shown: &Buffer, cold: &Buffer) {
    if matches!(case, Case::Horizontal) {
        // Literal independent oracle: endpoint widths704 ->842. The gradient corner -0.5
        // cancels the pixel-centre +0.5, so RGB=(x-64)/width. No product gradient helper.
        opaque_grey(shown, 256, 3.0 / 11.0);
        opaque_grey(cold, 256, 96.0 / 421.0);
        opaque_grey(shown, 600, 67.0 / 88.0);
        opaque_grey(cold, 600, 268.0 / 421.0);
    } else {
        let expected = if matches!(case, Case::UnchangedFrame) { 3.0 / 11.0 } else { 0.5 };
        opaque_grey(shown, 256, expected);
        opaque_grey(cold, 256, expected);
    }
}

fn raster_fixture(edit_label: &str) -> (Session, photocraft_doc::LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":1024,"height":64,"mode":"rgb","depth":32,"background":"white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.tools.brush = Default::default();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit(edit_label, |doc, _| {
        doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::new(64, 8, 768, 56), &[0.5, 0.5, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    (s, id)
}

/// Patch the previous display using public live damage and compare with a cold CPU render.
/// The horizontal case also checks that committing the same live stroke preserves its pixels.
fn run(case: Case) {
    let (mut s, id) = raster_fixture("small opaque raster fixture");
    if !matches!(case, Case::NoOverlay) {
        s.execute(
            "layer.layerStyle.gradientOverlay",
            json!({
                "from":"#000000", "to":"#ffffff", "angle":if matches!(case, Case::Vertical) {90} else {0},
                "scale":100,"opacity":100,"blend":"Normal","style":"linear","reverse":false
            }),
        )
        .unwrap();
    }
    let (start, end) = if matches!(case, Case::UnchangedFrame) { (600.0, 704.0) } else { (744.0, 896.0) };
    let mut params = json!({
        "points":[[start,32.0,1.0]],"size":16,"hardness":1.0,"spacing":0.25,
        "opacity":1.0,"flow":1.0,"color":"#000000","seed":42,
        "brush":{"pressureSize":false,"smoothing":{"enabled":false}}
    });
    let mut live = LiveStroke::begin(&s, &params).unwrap();
    let before = live.doc.clone();
    assert_eq!(effects_frame(before.layer(id).unwrap(), before.bounds()), Rect::new(64, 8, 768, 56));
    let mut shown = render(&before, before.bounds());
    let old_source = before.layer(id).unwrap().surface().unwrap().pixel(256, 32);
    let raw = live.push(&[StrokePoint::new(end, 32.0, 1.0)]).unwrap();
    let surface = live.doc.layer(id).unwrap().surface().unwrap();
    assert_eq!(surface.pixel(256, 32), old_source, "the old source pixel itself has not changed");
    // radius8, core6.8, shoulder1.2 => reach9.92. Conservative dab slack is11;
    // actual 32f edge alpha stops at x905, not the nominal radius or tile boundary.
    let support = if matches!(case, Case::UnchangedFrame) { Rect::new(589, 21, 716, 44) } else { Rect::new(733, 21, 908, 44) };
    assert!(support.contains_rect(&live.bounds()), "stroke support exceeds the derived bound: {:?}", live.bounds());
    let frame = if matches!(case, Case::UnchangedFrame) { Rect::new(64, 8, 768, 56) } else { Rect::new(64, 8, 906, 56) };
    assert_eq!(effects_frame(live.doc.layer(id).unwrap(), live.doc.bounds()), frame);
    if !matches!(case, Case::UnchangedFrame) {
        assert!(surface.pixel(905, 32)[3] > 0.0);
        assert_eq!(surface.pixel(906, 32)[3], 0.0);
    }
    let reach = live.doc.layers.iter().filter(|l| photocraft_compose::effects::has_effects(l)).map(photocraft_compose::effects::margin).max().unwrap_or(0);
    assert_eq!(reach, if matches!(case, Case::NoOverlay) { 0 } else { 2 });
    let damage = raw.inflate(reach);
    let cold = render(&live.doc, live.doc.bounds());
    reference_colours(case, &shown, &cold);
    // Existing same-size CPU texture update: render only the reported live damage plus
    // effect reach, then retain every other pixel from the previous displayed frame.
    let patch = render_reduced_damage(&live.doc, 1024, 64, damage);
    println!("live raw damage {raw:?}, effect reach {reach}, brush support {:?}, current frame {frame:?}, CPU patch {:?}", live.bounds(), patch.rect);
    copy_patch(&mut shown, &patch, 1024);
    if matches!(case, Case::Horizontal) {
        // Release already repairs this known preview residual; do not imply export corruption.
        let old_roi = Rect::new(256, 24, 272, 40);
        let relayout = photocraft_compose::relayout_damage(&s.active().unwrap().doc, &live.doc);
        assert!(relayout.contains_rect(&old_roi));
        params["points"] = json!([[start, 32.0, 1.0], [end, 32.0, 1.0]]);
        params["seed"] = json!(live.seed);
        s.offer_live_stroke(live).unwrap();
        assert_eq!(s.execute("paint.stroke", params).unwrap()["fromLive"], true);
        let st = s.active().unwrap();
        assert!(st.last_damage.unwrap().contains_rect(&old_roi));
        same(&cold, &render(&st.doc, st.doc.bounds()));
    }
    same(&shown, &cold);
}

#[test]
fn horizontal_frame_growth_live_redraw_matches_full_current_render() {
    run(Case::Horizontal);
}

#[test]
fn no_overlay_live_redraw_matches_full_current_render() {
    run(Case::NoOverlay);
}

#[test]
fn unchanged_gradient_frame_live_redraw_matches_full_current_render() {
    run(Case::UnchangedFrame);
}

#[test]
fn vertical_gradient_with_unchanged_y_frame_allows_local_live_redraw() {
    run(Case::Vertical);
}

// Additional gradient mapping coverage.
fn mapping_session(style: &str, angle: f32, scale: f32) -> (Session, photocraft_doc::LayerId) {
    let (mut s, id) = raster_fixture("gradient damage fixture");
    s.execute(
        "layer.layerStyle.gradientOverlay",
        json!({"from":"#000000","to":"#ffffff","style":style,"angle":angle,"scale":scale,"opacity":100,"blend":"Normal"}),
    )
    .unwrap();
    (s, id)
}

fn live_params(x: f64, y: f64) -> serde_json::Value {
    json!({"points":[[x,y,1.0]],"size":16,"hardness":1.0,"spacing":0.25,"opacity":1.0,"flow":1.0,
        "color":"#000000","seed":42,"brush":{"pressureSize":false,"smoothing":{"enabled":false}}})
}

fn apply_damage(shown: &mut Buffer, doc: &photocraft_doc::Document, damage: Rect) {
    let patch = render_reduced_damage(doc, doc.size.width, doc.size.height, damage);
    copy_patch(shown, &patch, doc.size.width as i32);
}

/// Replace only the reported patch, preserving stale pixels outside it for the oracle to detect.
fn copy_patch(shown: &mut Buffer, patch: &Buffer, width: i32) {
    for y in patch.rect.y0..patch.rect.y1 {
        for x in patch.rect.x0..patch.rect.x1 {
            shown.px[(y * width + x) as usize] = patch.get(x, y);
        }
    }
}

#[test]
fn initial_dab_relayout_damage_stays_separate_from_commit_pixel_bounds() {
    let (s, id) = mapping_session("linear", 0.0, 100.0);
    let original = &s.active().unwrap().doc;
    let mut shown = render(original, original.bounds());
    let live = LiveStroke::begin(&s, &live_params(896.0, 32.0)).unwrap();
    assert_eq!(effects_frame(live.doc.layer(id).unwrap(), live.doc.bounds()), Rect::new(64, 8, 906, 56));
    let old_roi = Rect::new(256, 24, 272, 40);
    assert!(live.initial_damage().contains_rect(&old_roi));
    assert!(live.bounds().intersect(&old_roi).is_empty(), "commit pixel bounds remain local");
    opaque_grey(&shown, 256, 3.0 / 11.0);
    let cold = render(&live.doc, live.doc.bounds());
    opaque_grey(&cold, 256, 96.0 / 421.0);
    apply_damage(&mut shown, &live.doc, live.initial_damage().inflate(2));
    same(&shown, &cold);
}

/// Exercise each gradient style and gradient stroke with stored offset/alignment/reference settings.
#[test]
fn changed_gradient_mappings_cover_all_styles_offsets_align_and_reference() {
    for (style, angle, scale, stroke) in [
        ("linear", 25.0, 73.0, false),
        ("reflected", 30.0, 80.0, false),
        ("radial", 20.0, 90.0, false),
        ("angle", 45.0, 100.0, false),
        ("diamond", 40.0, 70.0, false),
        ("linear", 25.0, 73.0, true),
    ] {
        let (mut s, id) = mapping_session(style, angle, scale);
        s.edit("stored gradient geometry", |doc, _| {
            let layer = doc.layer_mut(id).unwrap();
            layer.effects.reference = Some((7.0, 11.0));
            for effect in &mut layer.effects.items {
                if let photocraft_doc::Effect::GradientOverlay { gradient, .. } = effect {
                    gradient.align = false;
                    gradient.offset = (0.125, -0.25);
                    gradient.reverse = true;
                }
            }
            if stroke {
                let overlay = layer.effects.items.remove(0);
                let photocraft_doc::Effect::GradientOverlay { common, gradient, .. } = overlay else { panic!("fixture overlay") };
                layer.effects.items.push(photocraft_doc::Effect::Stroke(photocraft_doc::StrokeFx {
                    common,
                    size: 4.0,
                    position: photocraft_doc::StrokePosition::Inside,
                    paint: photocraft_doc::FxPaint::Gradient(gradient),
                    overprint: false,
                }));
            }
            Ok(())
        })
        .unwrap();
        let mut live = LiveStroke::begin(&s, &live_params(744.0, 32.0)).unwrap();
        let mut shown = render(&live.doc, live.doc.bounds());
        let damage = live.push(&[StrokePoint::new(896.0, 32.0, 1.0)]).unwrap();
        assert!(damage.contains_rect(&Rect::new(256, 24, 272, 40)), "{style}: align/reference do not fix this frame");
        apply_damage(&mut shown, &live.doc, damage.inflate(2));
        same(&shown, &render(&live.doc, live.doc.bounds()));
    }
}

/// Require local damage for inactive gradients and frame changes that leave the mapping unchanged.
#[test]
fn disabled_and_zero_opacity_gradients_and_unchanged_horizontal_mapping_stay_local() {
    for inactive in 0..3 {
        let (mut s, id) = mapping_session("linear", 0.0, 100.0);
        s.edit("inactive gradient", |doc, _| {
            let layer = doc.layer_mut(id).unwrap();
            layer.effects.enabled = inactive != 0;
            for effect in &mut layer.effects.items {
                if let photocraft_doc::Effect::GradientOverlay { common, .. } = effect {
                    common.enabled = inactive != 1;
                    common.opacity = if inactive == 2 { 0.0 } else { 1.0 };
                }
            }
            Ok(())
        })
        .unwrap();
        let mut live = LiveStroke::begin(&s, &live_params(744.0, 32.0)).unwrap();
        let mut shown = render(&live.doc, live.doc.bounds());
        let damage = live.push(&[StrokePoint::new(896.0, 32.0, 1.0)]).unwrap();
        assert!(Rect::new(733, 21, 908, 44).contains_rect(&damage));
        apply_damage(&mut shown, &live.doc, damage.inflate(2));
        same(&shown, &render(&live.doc, live.doc.bounds()));
    }
    for (start, end, grows_y) in [((600.0, 32.0), (704.0, 32.0), false), ((300.0, 32.0), (300.0, 80.0), true)] {
        let (s, id) = mapping_session("linear", 0.0, 100.0);
        let mut live = LiveStroke::begin(&s, &live_params(start.0, start.1)).unwrap();
        let mut shown = render(&live.doc, live.doc.bounds());
        let damage = live.push(&[StrokePoint::new(end.0, end.1, 1.0)]).unwrap();
        let frame = effects_frame(live.doc.layer(id).unwrap(), live.doc.bounds());
        assert_eq!((frame.x0, frame.x1), (64, 768));
        if grows_y {
            assert!(frame.y1 > 56);
            assert!(damage.x0 >= 289 && damage.x1 <= 312, "height is not an input to this unchanged horizontal mapping");
        } else {
            assert_eq!(frame, Rect::new(64, 8, 768, 56));
            assert!(Rect::new(589, 21, 716, 44).contains_rect(&damage), "unchanged frame stays local");
        }
        apply_damage(&mut shown, &live.doc, damage.inflate(2));
        same(&shown, &render(&live.doc, live.doc.bounds()));
    }
}

#[test]
fn erasing_an_edge_shrinks_the_gradient_frame_with_initial_relayout_damage() {
    let (mut s, id) = mapping_session("linear", 0.0, 100.0);
    // Erasing preserves RGB. Use an ordinary opaque black base so fully erased pixels
    // become the all-zero default pixel, rather than leaving hidden grey RGB in the frame.
    s.edit("opaque black shrink fixture", |doc, _| {
        doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::new(64, 8, 768, 56), &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let original = &s.active().unwrap().doc;
    assert_eq!(original.layer(id).unwrap().surface().unwrap().pixel(767, 32), [0.0, 0.0, 0.0, 1.0]);
    let mut shown = render(original, original.bounds());
    let mut params = live_params(760.0, 32.0);
    params["erase"] = json!(true);
    params["size"] = json!(128);
    let live = LiveStroke::begin(&s, &params).unwrap();
    let erased = live.doc.layer(id).unwrap().surface().unwrap().pixel(767, 32);
    assert_eq!(erased, [0.0, 0.0, 0.0, 0.0], "fully erased black pixels match the surface default");
    assert!(effects_frame(live.doc.layer(id).unwrap(), live.doc.bounds()).x1 < 768);
    assert!(live.initial_damage().contains_rect(&Rect::new(256, 24, 272, 40)));
    apply_damage(&mut shown, &live.doc, live.initial_damage().inflate(2));
    same(&shown, &render(&live.doc, live.doc.bounds()));
}

#[test]
fn painting_a_child_updates_its_ancestors_gradient_mapping() {
    let (mut s, red) = mapping_session("linear", 0.0, 100.0);
    s.execute("layer.groupLayers", json!({"name":"Gradient group"})).unwrap();
    let group = s.active().unwrap().active_layer.unwrap();
    s.edit("move gradient to ancestor", |doc, _| {
        let effects = std::mem::take(&mut doc.layer_mut(red).unwrap().effects);
        doc.layer_mut(group).unwrap().effects = effects;
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer":red.0})).unwrap();
    let mut live = LiveStroke::begin(&s, &live_params(744.0, 32.0)).unwrap();
    let mut shown = render(&live.doc, live.doc.bounds());
    let damage = live.push(&[StrokePoint::new(896.0, 32.0, 1.0)]).unwrap();
    assert_eq!(effects_frame(live.doc.layer(group).unwrap(), live.doc.bounds()), Rect::new(64, 8, 906, 56));
    assert!(damage.contains_rect(&Rect::new(256, 24, 272, 40)));
    apply_damage(&mut shown, &live.doc, damage.inflate(2));
    same(&shown, &render(&live.doc, live.doc.bounds()));
}

/// Grow the frame symmetrically without changing the angle/radial mapping; damage stays local.
#[test]
fn symmetric_frame_growth_with_unchanged_angle_or_radial_mapping_stays_local() {
    for (style, angle) in [("angle", 45.0), ("radial", 90.0)] {
        let (s, id) = mapping_session(style, angle, 100.0);
        let mut live = LiveStroke::begin(&s, &live_params(744.0, 32.0)).unwrap();
        let mut shown = render(&live.doc, live.doc.bounds());
        let damage = live.push(&[StrokePoint::new(64.0, 32.0, 1.0), StrokePoint::new(768.0, 32.0, 1.0)]).unwrap();
        assert_eq!(effects_frame(live.doc.layer(id).unwrap(), live.doc.bounds()), Rect::new(54, 8, 778, 56));
        assert!(damage.y0 >= 21 && damage.y1 <= 44, "{style}: changed width leaves the mapping inputs unchanged");
        apply_damage(&mut shown, &live.doc, damage.inflate(2));
        same(&shown, &render(&live.doc, live.doc.bounds()));
    }
}

/// A raster mask can shrink the Stroke's gradient frame without changing its layer's pixels.
#[test]
fn mask_stroke_frame_change_repaints_distant_gradient_but_plain_mask_damage_stays_local() {
    for gradient_stroke in [true, false] {
        let (mut s, id) = raster_fixture("masked gradient Stroke fixture");
        if gradient_stroke {
            s.execute(
                "layer.layerStyle.gradientOverlay",
                json!({"from":"#000000","to":"#ffffff","angle":0,"scale":100,"opacity":100,"blend":"Normal","style":"linear"}),
            )
            .unwrap();
            s.edit("use the gradient as an inside Stroke", |doc, _| {
                let layer = doc.layer_mut(id).unwrap();
                let photocraft_doc::Effect::GradientOverlay { common, gradient, .. } = layer.effects.items.remove(0) else { panic!("fixture overlay") };
                layer.effects.items.push(photocraft_doc::Effect::Stroke(photocraft_doc::StrokeFx {
                    common,
                    size: 4.0,
                    position: photocraft_doc::StrokePosition::Inside,
                    paint: photocraft_doc::FxPaint::Gradient(gradient),
                    overprint: false,
                }));
                Ok(())
            })
            .unwrap();
        }
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let original = &s.active().unwrap().doc;
        let frame = effects_frame(original.layer(id).unwrap(), original.bounds());
        let mut shown = render(original, original.bounds());
        let distant = Rect::new(256, 8, 272, 12);
        let before = shown.get(256, 9);
        let source_pixel = original.layer(id).unwrap().surface().unwrap().pixel(256, 9);
        let mut params = live_params(896.0, 32.0);
        params["target"] = json!("mask");
        params["size"] = json!(96);
        let mut live = LiveStroke::begin(&s, &params).unwrap();
        // The first dab is beyond the content; the pushed dab removes its right edge through
        // the mask. Neither dab changes the raw raster extent used by effects_frame.
        same(&shown, &render(&live.doc, live.doc.bounds()));
        let damage = live.push(&[StrokePoint::new(752.0, 32.0, 1.0)]).unwrap();
        let layer = live.doc.layer(id).unwrap();
        assert_eq!(effects_frame(layer, live.doc.bounds()), frame);
        assert_eq!(layer.surface().unwrap().pixel(256, 9), source_pixel);
        assert_eq!(layer.mask.as_ref().unwrap().surface.pixel(256, 9)[0], 1.0);
        assert_eq!(layer.mask.as_ref().unwrap().surface.pixel(752, 32)[0], 0.0);
        assert!(live.bounds().intersect(&distant).is_empty(), "the actual mask brush stays distant");
        let cold = render(&live.doc, live.doc.bounds());
        if gradient_stroke {
            assert!((cold.get(256, 9)[0] - before[0]).abs() > 0.01, "the unchanged distant Stroke really changes colour");
            assert!(damage.contains_rect(&distant), "mask-dependent Stroke mapping must invalidate its distant pixels");
        } else {
            assert_eq!(cold.get(256, 9), before);
            assert!(damage.intersect(&distant).is_empty(), "a plain mask keeps local live damage");
        }
        let reach = if gradient_stroke { photocraft_compose::effects::margin(layer) } else { 0 };
        apply_damage(&mut shown, &live.doc, damage.inflate(reach));
        same(&shown, &cold);
    }
}

/// A legitimate interior edit keeps the same rendered Stroke frame and stays local.
#[test]
fn an_interior_mask_hole_keeps_gradient_stroke_damage_local() {
    let (mut s, id) = raster_fixture("interior mask Stroke control");
    s.execute(
        "layer.layerStyle.gradientOverlay",
        json!({"from":"#000000","to":"#ffffff","angle":0,"scale":100,"opacity":100,"blend":"Normal","style":"linear"}),
    )
    .unwrap();
    s.edit("inside gradient Stroke", |doc, _| {
        let layer = doc.layer_mut(id).unwrap();
        let photocraft_doc::Effect::GradientOverlay { common, gradient, .. } = layer.effects.items.remove(0) else { panic!("fixture overlay") };
        layer.effects.items.push(photocraft_doc::Effect::Stroke(photocraft_doc::StrokeFx {
            common,
            size: 4.0,
            position: photocraft_doc::StrokePosition::Inside,
            paint: photocraft_doc::FxPaint::Gradient(gradient),
            overprint: false,
        }));
        Ok(())
    })
    .unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let original = &s.active().unwrap().doc;
    let frame = effects_frame(original.layer(id).unwrap(), original.bounds());
    let mut shown = render(original, original.bounds());
    let before = shown.get(256, 9);
    let distant = Rect::new(256, 8, 272, 12);
    let mut params = live_params(384.0, 32.0);
    params["target"] = json!("mask");
    let mut live = LiveStroke::begin(&s, &params).unwrap();
    let initial = live.initial_damage();
    assert!(initial.intersect(&distant).is_empty(), "an interior initial dab keeps the unchanged gradient frame local");
    apply_damage(&mut shown, &live.doc, initial.inflate(photocraft_compose::effects::margin(live.doc.layer(id).unwrap())));
    same(&shown, &render(&live.doc, live.doc.bounds()));
    let damage = live.push(&[StrokePoint::new(416.0, 32.0, 1.0)]).unwrap();
    let layer = live.doc.layer(id).unwrap();
    assert_eq!(effects_frame(layer, live.doc.bounds()), frame);
    // Mask support is strictly inside the original shape; its four extrema stay fully opaque.
    // An inside Stroke has no outside-frame growth, so its compositor gradient bbox is unchanged.
    assert!(Rect::new(68, 12, 764, 52).contains_rect(&live.bounds()));
    for (x, y) in [(64, 8), (767, 8), (64, 55), (767, 55)] {
        assert_eq!(layer.surface().unwrap().pixel(x, y)[3], 1.0);
        assert_eq!(layer.mask.as_ref().unwrap().surface.pixel(x, y)[0], 1.0);
    }
    assert_eq!(layer.mask.as_ref().unwrap().surface.pixel(416, 32)[0], 0.0);
    let cold = render(&live.doc, live.doc.bounds());
    assert_eq!(cold.get(256, 9), before);
    assert!(damage.intersect(&distant).is_empty(), "an interior push must not invalidate a distant unchanged Stroke");
    apply_damage(&mut shown, &live.doc, damage.inflate(photocraft_compose::effects::margin(layer)));
    same(&shown, &cold);
}

/// Exercise the real mask budget decision without a huge allocation, then use actual FxMaps
/// to distinguish its old-hole/default-mask transition even though its Stroke frame is unchanged.
#[test]
fn mask_build_fallback_change_is_visible_in_cached_stroke_coverage() {
    assert!(photocraft_compose::masks::can_build_mask(Rect::new(0, 0, 64, 64)));
    assert!(!photocraft_compose::masks::can_build_mask(Rect::new(0, 0, 7200, 7200)));
    let (mut s, id) = mapping_session("linear", 0.0, 100.0);
    s.edit("inside gradient Stroke", |doc, _| {
        let layer = doc.layer_mut(id).unwrap();
        let photocraft_doc::Effect::GradientOverlay { common, gradient, .. } = layer.effects.items.remove(0) else { panic!("fixture overlay") };
        layer.effects.items.push(photocraft_doc::Effect::Stroke(photocraft_doc::StrokeFx {
            common,
            size: 4.0,
            position: photocraft_doc::StrokePosition::Inside,
            paint: photocraft_doc::FxPaint::Gradient(gradient),
            overprint: false,
        }));
        Ok(())
    })
    .unwrap();
    let doc = &s.active().unwrap().doc;
    let layer = doc.layer(id).unwrap();
    let photocraft_doc::Effect::Stroke(stroke) = &layer.effects.items[0] else { panic!("fixture Stroke") };
    let rect = Rect::new(0, 0, 64, 64);
    let tex = photocraft_compose::effects::TextureCtx { rect, patterns: &[], anchor: (0.0, 0.0) };
    let mut old = vec![1.0; 64 * 64];
    for y in 8..16 {
        for x in 8..16 {
            old[y * 64 + x] = 0.0;
        }
    }
    let before = photocraft_compose::effects::build_maps(layer, old, rect, &doc.global_light, &tex);
    // When the larger mask build is refused, the returned default1 mask removes the old hole.
    let after = photocraft_compose::effects::build_maps(layer, vec![1.0; 64 * 64], rect, &doc.global_light, &tex);
    assert_eq!(before.stroke_frame(stroke), after.stroke_frame(stroke));
    let changed = before.gradient_shape().unwrap().changed_bounds(after.gradient_shape().unwrap());
    assert!(changed.contains_rect(&Rect::new(8, 8, 16, 16)), "the distant old hole changed despite the stable outer frame");
    assert!(changed.intersect(&Rect::new(48, 48, 56, 56)).is_empty(), "coverage comparison is not unconditional full extent damage");
}

/// A real Block-mode mask stroke crosses the effect-shape threshold far beyond its brush halo.
#[test]
fn feather_rounding_stroke_frame_changes_use_the_actual_renderer_maps() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":512,"height":64,"mode":"rgb","depth":32,"background":"white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.tools.brush = Default::default();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("sparse threshold-sensitive shape and mask", |doc, _| {
        let layer = doc.layer_mut(id).unwrap();
        let surface = layer.surface_mut().unwrap();
        surface.fill_rect(Rect::new(80, 0, 96, 64), &[0.5, 0.5, 0.5, 1.0]);
        for y in [28, 30, 32, 34, 36] {
            surface.fill_rect(Rect::new(80, y, 400, y + 1), &[0.5, 0.5, 0.5, 1.0]);
        }
        let mut mask = photocraft_doc::LayerMask::hide_all();
        mask.density = 0.998_039_2;
        mask.feather = 2.0;
        mask.surface.fill_rect(Rect::new(0, 0, 1, 1), &[1.0]);
        mask.surface.fill_rect(Rect::new(511, 63, 512, 64), &[1.0]);
        layer.mask = Some(mask);
        Ok(())
    })
    .unwrap();
    s.execute(
        "layer.layerStyle.gradientOverlay",
        json!({"from":"#000000","to":"#ffffff","angle":0,"scale":100,"opacity":100,"blend":"Normal","style":"linear"}),
    )
    .unwrap();
    s.edit("inside gradient Stroke", |doc, _| {
        let layer = doc.layer_mut(id).unwrap();
        let photocraft_doc::Effect::GradientOverlay { common, gradient, .. } = layer.effects.items.remove(0) else { panic!("fixture overlay") };
        layer.effects.items.push(photocraft_doc::Effect::Stroke(photocraft_doc::StrokeFx {
            common,
            size: 4.0,
            position: photocraft_doc::StrokePosition::Inside,
            paint: photocraft_doc::FxPaint::Gradient(gradient),
            overprint: false,
        }));
        Ok(())
    })
    .unwrap();
    let original = &s.active().unwrap().doc;
    let mut shown = render(original, original.bounds());
    let before = photocraft_compose::gradient_stroke_layout(original, original.layer(id).unwrap()).unwrap();
    let mask = photocraft_compose::masks::combined_mask(original.layer(id).unwrap(), original.bounds()).unwrap();
    assert_eq!(mask.sample_channel(350, 32, 0), 129.0 / 65535.0);
    let params =
        json!({"points":[[136.0,32.0,1.0]],"target":"mask","block":true,"zoom":1.0,"color":"#ffffff","seed":42,"brush":{"smoothing":{"enabled":false}}});
    let live = LiveStroke::begin_with(&s, "paint.pencil", &params).unwrap();
    let layer = live.doc.layer(id).unwrap();
    let current = photocraft_compose::gradient_stroke_layout(&live.doc, layer).unwrap();
    let mask = photocraft_compose::masks::combined_mask(layer, live.doc.bounds()).unwrap();
    assert_eq!(mask.sample_channel(350, 32, 0), 128.0 / 65535.0);
    let distant = Rect::new(350, 28, 351, 37);
    assert!(live.bounds().intersect(&distant).is_empty());
    assert!(before.shape.changed_bounds(&current.shape).contains_rect(&distant));
    assert!(live.initial_damage().contains_rect(&distant), "actual renderer coverage changes expand the initial damage");
    let cold = render(&live.doc, live.doc.bounds());
    apply_damage(&mut shown, &live.doc, live.initial_damage().inflate(photocraft_compose::effects::margin(layer)));
    same(&shown, &cold);
}
