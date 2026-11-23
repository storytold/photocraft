//! Tests that run paint commands, moved from the engine-core `adjust_cmds` tests: 2 test(s).

use photocraft_engine::Session;
use serde_json::json;

const DEPTHS: [u64; 3] = [8, 16, 32];

fn session(depth: u64, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 32, "depth": depth, "mode": mode})).unwrap();
    paint(&mut s, |x, y| [0.1 + x as f32 / 60.0, 0.2 + y as f32 / 50.0, 0.6 - x as f32 / 120.0, 1.0]);
    s
}

fn paint(s: &mut Session, f: impl Fn(i32, i32) -> [f32; 4]) {
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                data.extend(photocraft_raster::from_rgba(&fmt, f(x, y)));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
}

fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn mask_at(s: &Session, x: i32, y: i32) -> f32 {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0)
}

/// #2456: Edit > Fill (and the fill keys) fill the targeted mask of an adjustment layer.
#[test]
fn fill_works_on_an_adjustment_layers_mask() {
    let mut s = session(8, "rgb");
    s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let mask = json!({"target": "mask"});
    assert!(!s.is_enabled("edit.fill"), "no pixels to fill without the mask target");
    assert!(s.is_enabled_with("edit.fill", &mask));
    assert!(s.is_enabled_with("edit.fillForeground", &mask));
    s.execute("edit.fill", json!({"contents": "black", "target": "mask"})).unwrap();
    assert_eq!(mask_at(&s, 3, 3), 0.0, "the mask is filled black");
    s.execute("edit.fill", json!({"contents": "white", "target": "mask"})).unwrap();
    assert_eq!(mask_at(&s, 3, 3), 1.0);
}

/// #5335: Curves (⌘M) edits the targeted layer mask, saved channel or Quick Mask at every depth.
#[test]
fn curves_apply_to_non_colour_targets_at_every_depth() {
    let area = photocraft_geom::Rect::from_xywh(0, 0, 48, 32);
    for depth in DEPTHS {
        for target in ["mask", "alpha", "quickMask"] {
            let mut s = session(depth, "rgb");
            s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
            let p = match target {
                "mask" => {
                    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                    json!({"points": [[0, 0], [128, 200], [255, 255]], "target": "mask"})
                }
                "alpha" => {
                    s.execute("channel.new", json!({"name": "Sky sel", "fill": "white"})).unwrap();
                    json!({"points": [[0, 0], [128, 200], [255, 255]], "target": {"channel": 0}})
                }
                _ => {
                    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                    json!({"points": [[0, 0], [128, 200], [255, 255]], "target": "quickMask"})
                }
            };
            let edit_target = p.get("target").cloned().unwrap_or(json!(null));
            s.edit("levels", |doc, active| {
                let (surf, _) = photocraft_engine::channel_cmds::target_surface(doc, *active, &json!({ "target": edit_target })).unwrap();
                surf.fill_rect(area, &[0.5]);
                Ok(())
            })
            .unwrap();
            let read = |s: &Session| {
                let d = s.active().unwrap();
                let layer = d.doc.layer(d.active_layer.unwrap()).unwrap();
                let t = match target {
                    "quickMask" => d.doc.quick_mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
                    "alpha" => d.doc.channels[0].surface.sample_channel(8, 8, 0),
                    _ => layer.mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
                };
                (rgba(s, 8, 8), t)
            };
            let ((pixels, before), past) = (read(&s), s.active().unwrap().history.past_len());
            s.execute("image.adjustments.curves", p).unwrap();
            let (pixels_after, after) = read(&s);
            let case = format!("{target} at {depth}");
            assert_eq!(pixels_after, pixels, "{case}: colour pixels stay put");
            assert!(after > before + 0.1, "{case}: the curve lifted the midtone ({before} → {after})");
            assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{case}");
            s.undo();
            assert!((read(&s).1 - before).abs() < 1e-5, "{case}: undo restores the target");
        }
    }
}
