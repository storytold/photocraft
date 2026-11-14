use super::*;

fn assert_tip_defaults(brush: &Value, spacing: f64) {
    assert!((brush["spacing"].as_f64().unwrap() - spacing).abs() < 1e-6, "{brush}");
    assert_eq!(brush["angle"], 0.0);
    assert_eq!(brush["roundness"], 1.0);
    assert_eq!(brush["flipX"], false);
    assert_eq!(brush["flipY"], false);
}

fn check_defined_spacing(command: &str) {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 6, "depth": depth, "background": "#000000"})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        // A newly defined tip gets its own defaults, rather than the previous brush's pose.
        s.execute("tools.setBrush", json!({"spacing": 0.75, "angle": 45, "roundness": 0.5, "flipX": true, "flipY": true})).unwrap();
        s.execute(command, json!({"name": "Defined spacing"})).unwrap();

        let list = s.execute("brush.presets.list", json!({"full": true})).unwrap();
        let preset = list["presets"].as_array().unwrap().iter().find(|p| p["name"] == "Defined spacing").unwrap();
        assert_tip_defaults(&preset["brush"], 0.10);
        assert_tip_defaults(&s.execute("brush.get", json!({})).unwrap(), 0.10);
        assert!(matches!(&s.tools.brush.tip, TipShape::Sampled(tip) if tip.width == 8 && tip.height == 6));
    }
}

#[test]
fn edit_define_brush_preset_starts_with_ten_percent_spacing() {
    check_defined_spacing("edit.defineBrushPreset");
}

#[test]
fn define_from_selection_starts_with_ten_percent_spacing() {
    check_defined_spacing("brush.defineFromSelection");
}

#[test]
fn imported_image_keeps_twenty_five_percent_spacing() {
    let image = photocraft_codecs::Image::from_u16(2, 2, photocraft_codecs::ChannelLayout::Rgba, &[0, 0, 0, u16::MAX].repeat(4)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let mut s = Session::new();
    let data = photocraft_paint::tile::b64_encode(&png);
    s.execute("brush.presets.importImage", json!({"data": data, "fileName": "Imported spacing.png"})).unwrap();

    let list = s.execute("brush.presets.list", json!({"full": true})).unwrap();
    let preset = list["presets"].as_array().unwrap().iter().find(|p| p["name"] == "Imported spacing").unwrap();
    assert_tip_defaults(&preset["brush"], 0.25);
    assert_tip_defaults(&s.execute("brush.get", json!({})).unwrap(), 0.25);
}
