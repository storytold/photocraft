use photocraft_doc::{LayerContent, LayerId, LiveShape};
use photocraft_engine::Session;
use serde_json::json;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":64,"height":48,"depth":depth,"background":"transparent"})).unwrap();
    s
}
fn rgba(s: &Session, id: LayerId, x: i32, y: i32) -> [f32; 4] {
    let surface = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap();
    let fmt = surface.format();
    photocraft_raster::to_rgba(&fmt, &surface.read_region(photocraft_geom::Rect::new(x, y, x + 1, y + 1)))
}

#[test]
fn pixel_shapes_respect_selection_opacity_blend_and_undo_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let id = s.active().unwrap().active_layer.unwrap();
        let original = s.active().unwrap().doc.clone();
        s.execute("select.rect", json!({"x":10,"y":5,"width":12,"height":20})).unwrap();
        let history = s.active().unwrap().history.past_len();
        s.execute("shape.drawPixels", json!({"rect":[5,5,25,20],"color":"#ff0000","opacity":50})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        assert!(rgba(&s, id, 15, 10)[0] > 0.99);
        assert!((rgba(&s, id, 15, 10)[3] - 0.5).abs() < 0.005);
        assert_eq!(rgba(&s, id, 6, 10)[3], 0.0);
        let painted = s.active().unwrap().doc.clone();
        s.execute("shape.drawPixels", json!({"rect":[5,5,25,20],"color":"#00ff00","mode":"multiply"})).unwrap();
        assert!(rgba(&s, id, 15, 10)[0] < 0.01);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, painted);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(rgba(&s, id, 15, 10)[3], 0.0);
        assert_eq!(s.active().unwrap().doc.layers.len(), original.layers.len());
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, painted);
    }
}

#[test]
fn pixel_antialias_and_transparency_lock_are_observable() {
    let mut s = session(32);
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("shape.drawPixels", json!({"kind":"ellipse","rect":[5.2,5.4,20.3,20.6],"antiAlias":true})).unwrap();
    let surface = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap();
    assert!(surface.read_region(photocraft_geom::Rect::new(0, 0, 32, 32)).chunks_exact(surface.format().channels()).any(|p| p[3] > 0.0 && p[3] < 1.0));
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("shape.drawPixels", json!({"kind":"ellipse","rect":[5.2,5.4,20.3,20.6],"antiAlias":false})).unwrap();
    let surface = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap();
    assert!(surface.read_region(photocraft_geom::Rect::new(0, 0, 32, 32)).chunks_exact(surface.format().channels()).all(|p| p[3] == 0.0 || p[3] == 1.0));
    s.execute("layer.setProps", json!({"locks":{"transparency":true}})).unwrap();
    s.execute("shape.drawPixels", json!({"rect":[0,0,40,40],"color":"#ff0000"})).unwrap();
    assert_eq!(rgba(&s, id, 1, 1)[3], 0.0);
}

#[test]
fn pixel_mask_target_and_bad_params_preserve_document_on_failure() {
    let mut s = session(16);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("shape.drawPixels", json!({"rect":[3,3,20,20],"target":"mask","color":"#000000"})).unwrap();
    let layer = s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap();
    assert!(layer.mask.as_ref().unwrap().surface.sample_channel(10, 10, 0) < 0.01);
    s.execute("layer.setProps", json!({"locks":{"all":true}})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for params in [
        json!({}),
        json!({"rect":[0,0,10,10],"target":"mask"}),
        json!({"rect":[0,0,10,10],"opacity":101}),
        json!({"rect":[0,0,10,10],"mode":"unknown"}),
        json!({"rect":[0,0,10,10],"antiAlias":"true"}),
        json!({"rect":[1e100,0,10,10]}),
        json!({"kind":"path","path":null}),
    ] {
        assert!(s.execute("shape.drawPixels", params.clone()).is_err(), "{params}");
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn rounded_polygons_and_arrowheads_keep_live_properties_and_undo() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let id =
            s.execute("shape.create", json!({"kind":"star","sides":5,"rect":[5,5,40,30],"cornerRadius":2,"starRatio":0.5})).unwrap()["layer"].as_u64().unwrap();
        s.execute("shape.edit", json!({"layer":id,"sides":6})).unwrap();
        let info = s.execute("shape.info", json!({"layer":id})).unwrap();
        assert_eq!(info["live"]["star_ratio"], 0.5);
        assert_eq!(info["live"]["corner_radius"], 2.0);
        s.execute("shape.edit", json!({"layer":id,"cornerRadius":0})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.execute("shape.edit", json!({"layer":id,"cornerRadius":-1})).is_err());
        let id = s
            .execute("shape.create", json!({"kind":"line","from":[5,20],"to":[55,20],"weight":2,"arrowheads":{"start":true,"end":true,"width":10,"length":12}}))
            .unwrap()["layer"]
            .as_u64()
            .unwrap();
        let LayerContent::Shape(sh) = &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content else { panic!() };
        assert_eq!(sh.path.subpaths.len(), 1);
        assert_eq!(sh.path.subpaths[0].knots.len(), 10);
        assert!(matches!(sh.live, Some(LiveShape::Line { arrowheads: Some(_), .. })));
        s.execute("shape.edit", json!({"layer":id,"weight":4})).unwrap();
        assert!(s.execute("shape.edit", json!({"layer":id,"arrowheads":{"width":1e100}})).is_err());
    }
}

#[test]
fn pixels_draw_in_grayscale_without_format_assumptions() {
    let mut s = session(16);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("shape.drawPixels", json!({"rect":[4,4,12,12],"color":"#ff0000"})).unwrap();
    let p = rgba(&s, id, 8, 8);
    assert!((p[0] - p[1]).abs() < 1e-6 && (p[1] - p[2]).abs() < 1e-6 && p[3] > 0.99);
}

#[test]
fn pixel_shapes_follow_color_alpha_and_quick_mask_targets() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":16,"height":16,"background":"white"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("channel.target.slot3", json!({})).unwrap();
    s.execute("shape.drawPixels", json!({"rect":[2,2,8,8],"color":"#000000"})).unwrap();
    let [r, g, b, _] = rgba(&s, id, 4, 4);
    assert!(r < 0.01 && g > 0.99 && b > 0.99, "targeted red channel: {r}, {g}, {b}");

    s.execute("channel.new", json!({})).unwrap();
    let before = rgba(&s, id, 4, 4);
    s.execute("shape.drawPixels", json!({"rect":[2,2,8,8],"color":"#ffffff"})).unwrap();
    assert_eq!(rgba(&s, id, 4, 4), before, "alpha target leaves pixels unchanged");
    assert!(s.active().unwrap().doc.channels[0].surface.sample_channel(4, 4, 0) > 0.99);

    s.execute("channel.target.composite", json!({})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({"on":true})).unwrap();
    let before = rgba(&s, id, 4, 4);
    s.execute("shape.drawPixels", json!({"rect":[2,2,8,8],"color":"#000000"})).unwrap();
    assert_eq!(rgba(&s, id, 4, 4), before, "Quick Mask target leaves pixels unchanged");
    assert!(s.active().unwrap().doc.quick_mask.as_ref().unwrap().surface.sample_channel(4, 4, 0) < 0.01);
}

#[test]
fn pixel_shapes_refuse_hidden_layers_and_ancestor_groups_without_history_changes() {
    let mut s = session(8);
    let id = s.active().unwrap().active_layer.unwrap();
    let initial_doc = s.active().unwrap().doc.clone();
    s.execute("layer.setProps", json!({"visible":false})).unwrap();
    let hidden_doc = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let err = s.execute("shape.drawPixels", json!({"rect":[2,2,8,8],"color":"#ff0000"})).unwrap_err().to_string();
    assert!(err.contains("target layer is hidden"), "{err}");
    assert_eq!(s.active().unwrap().doc, hidden_doc);
    assert_eq!(s.active().unwrap().history.past_len(), history);

    s.execute("layer.setProps", json!({"visible":true})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    let group = s.active().unwrap().active_layer.unwrap();
    assert_ne!(group, id);
    s.execute("layer.setProps", json!({"layer":group.0,"visible":false})).unwrap();
    s.execute("layer.select", json!({"layer":id.0})).unwrap();
    let hidden_group_doc = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let err = s.execute("shape.drawPixels", json!({"rect":[2,2,8,8],"color":"#ff0000"})).unwrap_err().to_string();
    assert!(err.contains("target layer is hidden"), "{err}");
    assert_eq!(s.active().unwrap().doc, hidden_group_doc);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    assert_ne!(initial_doc, hidden_group_doc);
}

#[test]
fn pixel_shapes_validate_an_explicit_raster_layer_instead_of_active_shape() {
    let mut s = session(8);
    let raster = s.active().unwrap().active_layer.unwrap();
    s.execute("shape.create", json!({"kind":"rect","rect":[30,30,8,8]})).unwrap();
    let active_shape = s.active().unwrap().active_layer.unwrap();
    assert_ne!(raster, active_shape);
    s.execute("shape.drawPixels", json!({"rect":[2,2,8,8],"color":"#ff0000","layer":raster.0})).unwrap();
    assert!(rgba(&s, raster, 4, 4)[0] > 0.99);
    assert_eq!(s.active().unwrap().active_layer, Some(active_shape));
}

#[test]
fn explicit_pixel_layer_preserves_channel_routing_and_active_selection_on_undo() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":16,"height":16,"background":"white"})).unwrap();
    let raster = s.active().unwrap().active_layer.unwrap();
    s.execute("shape.create", json!({"kind":"rect","rect":[12,12,3,3]})).unwrap();
    let active_shape = s.active().unwrap().active_layer.unwrap();
    let selected = s.active().unwrap().selected_layers.clone();
    s.execute("channel.target.slot3", json!({})).unwrap();
    let before = s.active().unwrap().doc.clone();

    s.execute("shape.drawPixels", json!({"rect":[2,2,8,8],"color":"#000000","layer":raster.0})).unwrap();
    let [r, g, b, _] = rgba(&s, raster, 4, 4);
    assert!(r < 0.01 && g > 0.99 && b > 0.99, "explicit raster should remain cyan: {r}, {g}, {b}");
    assert_eq!(s.active().unwrap().active_layer, Some(active_shape));
    assert_eq!(s.active().unwrap().selected_layers, selected);

    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().active_layer, Some(active_shape));
    assert_eq!(s.active().unwrap().selected_layers, selected);

    s.execute("select.deselectLayers", json!({})).unwrap();
    assert_eq!(s.active().unwrap().active_layer, None);
    assert!(s.active().unwrap().selected_layers.is_empty());
    let before_without_active = s.active().unwrap().doc.clone();
    s.execute("shape.drawPixels", json!({"rect":[2,2,8,8],"color":"#000000","layer":raster.0})).unwrap();
    let [r, g, b, _] = rgba(&s, raster, 4, 4);
    assert!(r < 0.01 && g > 0.99 && b > 0.99, "explicit raster without active layer should remain cyan: {r}, {g}, {b}");
    assert_eq!(s.active().unwrap().active_layer, None);
    assert!(s.active().unwrap().selected_layers.is_empty());

    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, before_without_active);
    assert_eq!(s.active().unwrap().active_layer, None);
    assert!(s.active().unwrap().selected_layers.is_empty());
}

#[test]
#[ignore = "release performance measurement on a 36 MP canvas"]
fn pixel_shape_36mp() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":7200,"height":5000,"background":"transparent"})).unwrap();
    let start = std::time::Instant::now();
    s.execute("shape.drawPixels", json!({"rect":[0,0,7200,5000],"color":"#ff0000"})).unwrap();
    eprintln!("36 MP pixel shape: {:?}", start.elapsed());
    let id = s.active().unwrap().active_layer.unwrap();
    assert!(rgba(&s, id, 7199, 4999)[0] > 0.99);
}

#[test]
fn huge_pixel_shape_rejects_before_allocation_without_changing_document_or_history() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":300000,"height":300000,"depth":32,"background":"transparent"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let err = s.execute("shape.drawPixels", json!({"rect":[0,0,300000,300000]})).unwrap_err();
    assert!(err.to_string().contains("drawing memory limit"));
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    // Large sparse canvases still support small local shapes.
    s.execute("shape.drawPixels", json!({"rect":[2,2,8,8],"color":"#ff0000"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    assert!(rgba(&s, id, 4, 4)[0] > 0.99);
}
