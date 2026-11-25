//! Tests that run paint commands, moved from the engine-core `extra_cmds` tests: 3 test(s).

use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

#[test]
fn equalize_average_and_clouds_edit_the_targeted_channel() {
    // #935: these four ran through `map_pixels`, which always rewrote the active layer, so
    // with an alpha channel or the Quick Mask targeted they changed the layer and left the
    // channel alone.
    let cmds = ["image.adjustments.equalize", "filter.blur.average", "filter.render.clouds", "filter.render.differenceClouds"];
    for depth in [8, 16, 32] {
        for target in ["quickMask", "alpha", "mask"] {
            for cmd in cmds {
                let mut s = session(depth);
                s.tools.foreground = [0.0, 0.0, 0.0, 1.0];
                s.tools.background = [1.0, 1.0, 1.0, 1.0];
                s.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
                let mut p = match target {
                    "quickMask" => {
                        s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                        json!({"seed": 1})
                    }
                    "alpha" => {
                        s.execute("channel.new", json!({"fill": "white"})).unwrap();
                        s.execute("channel.target", json!({"channel": 0})).unwrap();
                        json!({"seed": 1})
                    }
                    _ => {
                        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                        json!({"seed": 1, "target": "mask"})
                    }
                };
                // Average has no seed parameter; only Clouds uses it.
                if cmd == "filter.blur.average" {
                    p.as_object_mut().unwrap().remove("seed");
                }
                // Two levels in the target, so Equalize and Average have something to change.
                let edit_target = json!({ "target": if target == "alpha" { json!({"channel": 0}) } else { json!(target) } });
                s.edit("levels", |doc, active| {
                    let (surf, _) = photocraft_engine::channel_cmds::target_surface(doc, *active, &edit_target).unwrap();
                    surf.fill_rect(Rect::new(0, 0, 20, 30), &[0.4]);
                    surf.fill_rect(Rect::new(20, 0, 40, 30), &[0.5]);
                    Ok(())
                })
                .unwrap();
                let full = Rect::new(0, 0, 40, 30);
                let read = |s: &Session| {
                    let d = doc(s);
                    let layer = d.layer(s.active().unwrap().active_layer.unwrap()).unwrap();
                    let t = match target {
                        "quickMask" => d.quick_mask.as_ref().unwrap().surface.read_region(full),
                        "alpha" => d.channels[0].surface.read_region(full),
                        _ => layer.mask.as_ref().unwrap().surface.read_region(full),
                    };
                    (layer.surface().unwrap().read_region(full), t)
                };
                let ((layer0, target0), past) = (read(&s), s.active().unwrap().history.past_len());
                s.execute(cmd, p).unwrap();
                let (layer1, target1) = read(&s);
                let case = format!("{cmd} on {target} at {depth}");
                assert_eq!(layer1, layer0, "{case}: the layer is untouched");
                assert_ne!(target1, target0, "{case}: the target changed");
                if cmd == "filter.blur.average" {
                    // The equal-sized halves contribute equally, using their stored sample values.
                    let mean = (f64::from(target0[0]) + f64::from(target0[20])) / 2.0;
                    let tolerance = match depth {
                        8 => 1.0 / 255.0,
                        16 => 1.0 / 65535.0,
                        _ => 1e-6,
                    };
                    for value in &target1 {
                        assert!((f64::from(*value) - mean).abs() <= tolerance, "{case}: {value}, expected {mean}");
                    }
                }
                assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{case}");
                s.undo();
                assert_eq!(read(&s).1, target0, "{case}: undo restores the target");
            }
        }
    }
}

fn active(s: &Session) -> &Layer {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap()
}

fn pixel(s: &Session, x: i32, y: i32) -> Vec<f32> {
    active(s).surface().unwrap().pixel(x, y)
}

fn paint_square(s: &mut Session, r: Rect, c: [f32; 4]) {
    s.edit("paint", |doc, a| {
        doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(r, &c);
        Ok(())
    })
    .unwrap();
}

#[test]
fn reveal_all_skips_erased_pixels() {
    // #9379: the Eraser only clears alpha, so erased pixels keep their colour bytes; Reveal
    // All grew the canvas to them although nothing there is visible.
    // No Background: its off-canvas pixels are opaque and would be revealed anyway.
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": "transparent"})).unwrap();
        paint_square(&mut s, Rect::new(0, 0, 40, 30), [1.0, 0.0, 0.0, 1.0]);
        let erase = json!({"points": [[32, -10], [32, 40]], "size": 28, "erase": true, "brush": {"hardness": 1.0}});
        s.execute("paint.stroke", erase).unwrap();
        assert_eq!(pixel(&s, 30, 15)[3], 0.0, "{depth}: erased");
        s.execute("image.canvasSize", json!({"width": 20, "height": 30, "anchor": "topLeft"})).unwrap();
        assert_eq!(s.execute("image.revealAll", json!({})).unwrap()["changed"], false, "{depth}");
        assert_eq!((doc(&s).size.width, doc(&s).size.height), (20, 30), "{depth}");
        // A pixel that is only partly erased is still there to reveal.
        paint_square(&mut s, Rect::new(25, 5, 26, 6), [0.0, 0.0, 1.0, 0.5]);
        assert_eq!(s.execute("image.revealAll", json!({})).unwrap()["changed"], true, "{depth}");
        assert_eq!((doc(&s).size.width, doc(&s).size.height), (26, 30), "{depth}");
    }
}

#[test]
fn rasterize_smart_object_layer() {
    let mut s = session(8);
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    let px_before = pixel(&s, 5, 5);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(matches!(active(&s).content, LayerContent::Smart(_)));
    s.execute("layer.rasterize.smartObject", json!({})).unwrap();
    assert!(matches!(active(&s).content, LayerContent::Raster(_)));
    let px_after = pixel(&s, 5, 5);
    assert_eq!(px_after, px_before);

    // Case 2: New layer above background
    let mut s2 = session(8);
    s2.execute("layer.new.layer", json!({})).unwrap();
    s2.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s2.execute("edit.fill", json!({"contents": "color", "color": "#00ff00"})).unwrap();
    s2.execute("select.deselect", json!({})).unwrap();
    let px_mid = pixel(&s2, 20, 20);
    assert_eq!(px_mid[1], 1.0); // green
    s2.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(matches!(active(&s2).content, LayerContent::Smart(_)));
    s2.execute("layer.rasterize.smartObject", json!({})).unwrap();
    assert!(matches!(active(&s2).content, LayerContent::Raster(_)));
    let px_after_rast = pixel(&s2, 20, 20);
    assert_eq!(px_after_rast, px_mid);

    // Case 3: Shape layer converted to smart object then rasterized
    let mut s3 = session(8);
    s3.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#0000ff"})).unwrap();
    let px_shape = pixel(&s3, 20, 20);
    s3.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(matches!(active(&s3).content, LayerContent::Smart(_)));
    s3.execute("layer.rasterize.layer", json!({})).unwrap();
    assert!(matches!(active(&s3).content, LayerContent::Raster(_)));
    let px_after_shape = pixel(&s3, 20, 20);
    assert_eq!(px_after_shape, px_shape);

    // Case 4: Transformed smart object
    let mut s4 = session(8);
    s4.execute("layer.new.layer", json!({})).unwrap();
    s4.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s4.execute("edit.fill", json!({"contents": "color", "color": "#ffff00"})).unwrap();
    s4.execute("select.deselect", json!({})).unwrap();
    s4.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    // Move smart object by 10, 10
    s4.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 10, 10]})).unwrap();
    s4.execute("layer.rasterize.smartObject", json!({})).unwrap();
    assert!(matches!(active(&s4).content, LayerContent::Raster(_)));
    assert_eq!(pixel(&s4, 30, 30), vec![1.0, 1.0, 0.0, 1.0]);
}
