use photocraft_engine::{Session, camera_raw_auto_cmds::AUTO};
use serde_json::json;

#[test]
fn auto_returns_only_slider_settings_without_editing_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":64,"height":16,"depth":depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("fixture", |doc, id| {
            let surf = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
            let fmt = surf.format();
            for y in 0..16 {
                for x in 0..64 {
                    let v = 0.07 + x as f32 / 64.0 * 0.2;
                    surf.write_pixel(x, y, &photocraft_raster::from_rgba(&fmt, [v, v * 0.9, v * 0.85, 1.0]));
                }
            }
            Ok(())
        })
        .unwrap();
        let original = s.active().unwrap().doc.clone();
        let steps = s.active().unwrap().history.past_len();
        let result = s.execute(AUTO, json!({"temperature":17,"tint":-9})).unwrap();
        assert!(result["settings"]["exposure"].as_f64().unwrap() > 0.0);
        assert!(result["settings"].get("temperature").is_none());
        assert!(result["settings"].get("tint").is_none());
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        assert!(std::sync::Arc::ptr_eq(&original, &s.active().unwrap().doc));
        assert_eq!(s.execute(AUTO, json!({"temperature":17,"tint":-9})).unwrap(), result);
    }
}

#[test]
fn bounded_sample_queries_work_without_a_document_and_reject_bad_inputs() {
    let mut s = Session::new();
    let result = s.execute(AUTO, json!({"samples":[[0.1,0.09,0.08,1],[0.3,0.27,0.24,1]]})).unwrap();
    assert!(result["settings"]["exposure"].as_f64().unwrap() > 0.0);
    for params in [
        json!(null),
        json!({}),
        json!({"samples":[]}),
        json!({"samples":[[1,2,3]]}),
        json!({"samples":[[0,0,0,0]]}),
        json!({"samples":[["x",0,0,1]]}),
        json!({"samples":[[1e100,0,0,1]]}),
        json!({"temperature":101}),
        json!({"layer":"wrong"}),
        json!({"samples":[[0,0,0,1]],"layer":0}),
        json!({"unknown":true}),
        json!({"samples":vec![[0.5;4];16_385]}),
    ] {
        assert!(s.execute(AUTO, params.clone()).is_err(), "accepted {params}");
    }
}
