use super::*;

/// A smart object with three smart filters, the middle one blended at half opacity.
fn filtered(depth: u64) -> Session {
    let mut s = session(depth);
    paint(&mut s);
    s.execute("filter.convertForSmartFilters", json!({})).unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    s.execute("filter.noise.addNoise", json!({"amount": 20, "distribution": "gaussian", "seed": 5})).unwrap();
    s.execute("filter.blur.motionBlur", json!({"angle": 30, "distance": 6})).unwrap();
    s.execute("layer.smartFilter.blendingOptions", json!({"index": 1, "blend": "multiply", "opacity": 0.5})).unwrap();
    s
}

/// The active smart object rendered without the stage cache.
fn uncached(s: &Session) -> Surface {
    let doc = &s.active().unwrap().doc;
    let sm = active_smart(s);
    let (name, bytes) = source_bytes(&doc.metadata, &sm.source).unwrap();
    apply_smart_filters(&place(&name, &bytes, doc, &sm).unwrap(), &sm, doc.bounds()).unwrap()
}

#[test]
fn renders_from_cached_stages_match_full_renders_at_every_depth() {
    for depth in DEPTHS {
        let mut s = filtered(depth);
        let edits = [
            ("layer.smartFilter.setVisible", json!({"index": 2, "visible": false})),
            ("layer.smartFilter.setVisible", json!({"index": 2, "visible": true})),
            ("layer.smartFilter.setVisible", json!({"index": 0, "visible": false})),
            ("layer.smartFilter.setVisible", json!({"index": 0, "visible": true})),
            ("layer.smartFilter.setParams", json!({"index": 2, "params": {"distance": 9}})),
            ("layer.smartFilter.blendingOptions", json!({"index": 1, "opacity": 0.8})),
            ("layer.smartFilter.move", json!({"index": 0, "to": 2})),
            ("layer.smartFilter.disableSmartFilters", json!({"enabled": false})),
            ("layer.smartFilter.disableSmartFilters", json!({"enabled": true})),
            ("layer.smartFilter.delete", json!({"index": 1})),
        ];
        for (id, params) in edits {
            s.execute(id, params.clone()).unwrap();
            assert_eq!(active_smart(&s).cache.unwrap(), uncached(&s), "{id} {params} at {depth}-bit");
        }
    }
}

#[test]
fn a_cancelled_render_stops_before_an_uncached_filter() {
    let s = filtered(8);
    let mut sm = active_smart(&s);
    // A radius no other render uses, so the stack below it is cached but this stage isn't.
    sm.smart_filters[0].params["radius"] = json!(2.375);
    let ctx = JobCtx::new();
    ctx.cancel();
    assert!(matches!(render_in(&s.active().unwrap().doc, &sm, &ctx), Err(EngineError::Cancelled)));
}

#[test]
fn bad_smart_filter_edits_fail_at_once_with_start() {
    let mut s = filtered(8);
    let before = active_smart(&s);
    let r = s.start("layer.smartFilter.setVisible", json!({"index": 9}));
    assert!(matches!(r, Err(EngineError::BadParams { .. })), "{r:?}");
    assert_eq!(active_smart(&s), before);
}

/// A `Sz`-only difference (linked instances, or undo of a size-only edit) can't share a placed
/// stage: `placed_size` is part of the key along with the transform, warp and perspective.
#[test]
fn a_size_only_difference_gets_its_own_placed_stage_key() {
    let key = |placed_size: Option<(f64, f64)>| {
        let mut sm = SmartObject::new(SmartSource::Embedded { file_name: "a.psd".into(), bytes: Arc::new(vec![1, 2, 3]) }, Affine::IDENTITY, None);
        sm.placed_size = placed_size;
        crate::cmd::smart_cmds::stack_cache::placed_key(&[1, 2, 3], PixelFormat::RGBA8, &sm, Rect::new(0, 0, 64, 48))
    };
    assert_ne!(key(None), key(Some((32.0, 24.0))));
    assert_ne!(key(Some((32.0, 24.0))), key(Some((16.0, 24.0))));
}
