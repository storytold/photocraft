//! Tests that run paint commands, moved from the engine-core `rotate_canvas_right_angle` tests: 1 test(s).

use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

use photocraft_doc::Size;

const ARBITRARY: &str = "image.rotation.arbitrary";

const CW: &str = "image.imageRotation.90cw";

const CCW: &str = "image.imageRotation.90ccw";

const HALF: &str = "image.imageRotation.180";

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "background": "white"})).unwrap();
    s.execute("tools.setColors", json!({"background": "#ff00ff"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.edit("Fixture", |doc, active| {
        let background = doc.background_mut().unwrap().surface_mut().unwrap();
        let blue = photocraft_raster::from_rgba(&background.format(), [0.0, 0.0, 1.0, 1.0]);
        background.fill_rect(Rect::new(396, 297, 400, 300), &blue);
        let top = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let green = photocraft_raster::from_rgba(&top.format(), [0.0, 1.0, 0.0, 1.0]);
        top.fill_rect(Rect::new(175, 125, 225, 175), &green);
        let translucent = photocraft_raster::from_rgba(&top.format(), [0.15432, 0.891, 1.0, 0.63654]);
        top.fill_rect(Rect::new(175, 125, 182, 130), &translucent);
        Ok(())
    })
    .unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

#[test]
fn arbitrary_right_angles_match_the_dedicated_rotation_commands() {
    for depth in [8, 16, 32] {
        for (angle, direction, standard) in
            [(90, "cw", CW), (-90, "cw", CCW), (90, "ccw", CCW), (270, "cw", CCW), (180, "cw", HALF), (450, "cw", CW), (-450, "ccw", CW)]
        {
            let mut arbitrary = session(depth);
            let mut exact = session(depth);
            let params = json!({"angle": angle, "direction": direction});
            arbitrary.execute("view.colorSamplers.add", json!({"x":399,"y":299})).unwrap();
            exact.execute("view.colorSamplers.add", json!({"x":399,"y":299})).unwrap();
            for s in [&mut arbitrary, &mut exact] {
                s.execute("view.newGuide", json!({"orientation":"vertical","position":20.5,"color":"#ff00ff"})).unwrap();
                s.execute("view.newGuide", json!({"orientation":"horizontal","position":0,"color":"#00ff00"})).unwrap();
            }
            let steps = arbitrary.active().unwrap().history.past_len();
            let result = arbitrary.execute(ARBITRARY, params.clone()).unwrap();
            exact.execute(standard, json!({})).unwrap();

            let actual = doc(&arbitrary);
            let expected = doc(&exact);
            assert_eq!(result, json!({"width": expected.size.width, "height": expected.size.height}), "{depth} bpc {params}");
            assert_eq!(actual.size, if standard == HALF { Size::new(400, 300) } else { Size::new(300, 400) });
            assert_eq!(actual.guides, expected.guides, "guides at {depth} bpc for {params}");
            assert_eq!(
                actual.layers[1].surface().unwrap().content_bounds(),
                if standard == HALF { Rect::new(175, 125, 225, 175) } else { Rect::new(125, 175, 175, 225) },
                "{depth} bpc {params}"
            );
            assert_eq!(actual.layers.len(), expected.layers.len());
            for (got, want) in actual.layers.iter().zip(&expected.layers) {
                let got = got.surface().unwrap();
                let want = want.surface().unwrap();
                assert_eq!(got.content_bounds(), want.content_bounds(), "{depth} bpc {params}");
                assert!(got.read_region(actual.bounds()) == want.read_region(expected.bounds()), "pixel data differs at {depth} bpc for {params}");
            }
            if standard == HALF {
                assert_eq!(actual.layers[0].surface().unwrap().rgba(0, 0), [0.0, 0.0, 1.0, 1.0]);
            }
            let actual_points = arbitrary.execute("view.colorSamplers.list", json!({})).unwrap();
            let exact_points = exact.execute("view.colorSamplers.list", json!({})).unwrap();
            assert_eq!(actual_points, exact_points, "sampler at {depth} bpc for {params}");
            let history = &arbitrary.active().unwrap().history;
            assert_eq!(history.past_len(), steps + 1);
            assert_eq!(history.undo_label(), Some("Rotate Canvas"));
        }
    }
}
