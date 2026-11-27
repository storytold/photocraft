use super::*;

const COMMANDS: [&str; 2] = ["edit.defineBrushPreset", "brush.defineFromSelection"];

/// A 6000 x 40 white document with a black `paint` rectangle on a new layer.
fn wide_document(paint: Rect) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 6000, "height": 40, "background": "#ffffff"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Tip"})).unwrap();
    s.edit("BrushSrc", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(paint, &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

fn select(s: &mut Session, width: u32) {
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": width, "height": 40})).unwrap();
}

#[test]
fn a_5000_px_strip_makes_a_5000_px_tip() {
    for command in COMMANDS {
        let mut s = wide_document(Rect::new(0, 0, 5000, 40));
        select(&mut s, 5000);
        assert!(s.is_enabled(command), "{command}");
        let result = s.execute(command, json!({"name": "Strip"})).unwrap();
        assert_eq!((result["width"].as_u64(), result["height"].as_u64()), (Some(5000), Some(40)), "{command}");
        assert_eq!(s.tools.brush.size, 5000.0);
        let TipShape::Sampled(tip) = &*s.tools.brush.tip else { panic!("expected a sampled tip") };
        assert_eq!((tip.width, tip.height), (5000, 40));
    }
}

#[test]
fn the_area_read_is_limited_to_5000_px_even_when_the_paint_is_small() {
    for command in COMMANDS {
        let mut s = wide_document(Rect::new(10, 10, 30, 30));
        assert!(!s.is_enabled(command), "{command}: no selection reads the 6000 px canvas");
        s.execute("select.all", json!({})).unwrap();
        assert!(!s.is_enabled(command), "{command}: Select All");
        assert!(s.execute(command, json!({"name": "Square"})).is_err(), "{command}");
        select(&mut s, 5001);
        assert!(!s.is_enabled(command), "{command}: 5001 px selection");
        assert!(s.execute(command, json!({"name": "Square"})).is_err(), "{command}");
        select(&mut s, 5000);
        assert!(s.is_enabled(command), "{command}: 5000 px selection");
        let result = s.execute(command, json!({"name": "Square"})).unwrap();
        assert_eq!((result["width"].as_u64(), result["height"].as_u64()), (Some(20), Some(20)), "{command}");
    }
}
