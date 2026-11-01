use super::*;
use photocraft_doc::Layer;
use photocraft_geom::Rect;

/// A 400 × 300 document at 72 ppi with a 200 × 140 pixel layer at (100, 80) named `name`.
fn session(name: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("layer.new.layer", json!({"name": name})).unwrap();
    s.edit("paint", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(100, 80, 300, 220), &[0.16, 0.35, 0.78, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// Photoshop 2025 copies this for a 200 × 140 pixel layer at (100, 80), second from the bottom.
#[test]
fn a_pixel_layer_copies_as_photoshop_writes_it() {
    let mut s = session("Box");
    let r = s.execute(ID, json!({})).unwrap();
    assert_eq!(
        r["css"],
        json!(
            ".Box {\n  background-image: url(\"Box.png\");\n  position: absolute;\n  left: 100px;\n  top: 80px;\n  width: 200px;\n  height: 140px;\n  z-index: 2;\n}\n"
        )
    );
    // Photoshop: a name with spaces keeps them in the image name and turns them to `_` in the
    // class; an opacity under 100 % follows the background.
    let mut s = session("My Box 2");
    s.edit("opacity", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().opacity = 128.0 / 255.0;
        Ok(())
    })
    .unwrap();
    let css = s.execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.starts_with(".My_Box_2 {\n  background-image: url(\"My Box 2.png\");\n  opacity: 0.502;\n  position: absolute;\n"), "{css}");
}

#[test]
fn fill_and_type_layers_describe_their_colour_and_font() {
    let doc = session("Box").active().unwrap().doc.clone();
    // Photoshop: a solid colour gives `background-color` (a fill layer without a shape covers the
    // canvas).
    let fill = Layer::new("Card", LayerContent::Fill(Fill::Solid(photocraft_color::Color::rgb(1.0, 120.0 / 255.0, 30.0 / 255.0))));
    assert_eq!(
        rule(&doc, 4, &fill).unwrap(),
        ".Card {\n  background-color: rgb(255, 120, 30);\n  position: absolute;\n  left: 0px;\n  top: 0px;\n  width: 400px;\n  height: 300px;\n  z-index: 4;\n}\n"
    );
    // A type layer: size, family, colour, auto leading and alignment, then only its position
    // (Photoshop writes no size for point text). Its position is where its pixels are.
    let mut cache = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    cache.fill_rect(Rect::new(49, 28, 120, 52), &[0.0, 0.0, 0.0, 1.0]);
    let text = photocraft_doc::TextLayer {
        font_family: "Inter".into(),
        size_pt: 24.0,
        color: photocraft_color::Color::rgb(0.0, 0.0, 0.0),
        cache: Some(cache),
        ..Default::default()
    };
    let title = Layer::new("Title", LayerContent::Text(text));
    let css = rule(&doc, 3, &title).unwrap_or_default();
    assert_eq!(
        css,
        ".Title {\n  font-size: 24px;\n  font-family: \"Inter\";\n  color: rgb(0, 0, 0);\n  line-height: 1.2;\n  text-align: left;\n  position: absolute;\n  left: 49px;\n  top: 28px;\n  z-index: 3;\n}\n"
    );
    // Groups and adjustments have no CSS.
    let group = Layer::group("Group 1", Vec::new());
    assert!(rule(&doc, 5, &group).is_none());
}

/// Photoshop 2025, paragraph text in a 200 × 120 box at (60, 40) that wraps to two lines whose
/// pixels span 156 × 51: `left: 59.937px; top: 36.948px; width: 156px; height: 51px;`. Unlike point
/// text it has a size, that of its text (not the box), and it's placed at its box, not where its
/// first pixel is; an explicit leading moves it down by that much (36 px: `top: 72.948px`).
#[test]
fn paragraph_text_is_placed_at_its_box_and_sized_by_its_text() {
    let doc = session("Box").active().unwrap().doc.clone();
    let mut cache = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    cache.fill_rect(Rect::new(62, 44, 218, 95), &[0.0, 0.0, 0.0, 1.0]);
    let mut text = photocraft_doc::TextLayer {
        text: "Hello world this wraps around".into(),
        font_family: "Inter".into(),
        size_pt: 24.0,
        cache: Some(cache),
        shape: photocraft_doc::text::TextShape::Box { x: 0.0, y: 0.0, width: 200.0, height: 120.0 },
        transform: photocraft_geom::Affine::translate(60.0, 40.0),
        ..Default::default()
    };
    let css = rule(&doc, 3, &Layer::new("Para", LayerContent::Text(text.clone()))).unwrap_or_default();
    assert_eq!(
        css,
        ".Para {\n  font-size: 24px;\n  font-family: \"Inter\";\n  color: rgb(0, 0, 0);\n  line-height: 1.2;\n  text-align: left;\n  position: absolute;\n  left: 60px;\n  top: 40px;\n  width: 156px;\n  height: 51px;\n  z-index: 3;\n}\n"
    );
    let style = photocraft_doc::text::CharStyle { font_family: "Inter".into(), size_pt: 24.0, leading_pt: Some(36.0), ..Default::default() };
    text.runs = vec![photocraft_doc::text::TextRun { len: text.text.len(), style }];
    let css = rule(&doc, 3, &Layer::new("Para", LayerContent::Text(text))).unwrap_or_default();
    assert!(
        css.contains("  line-height: 1.5;\n  text-align: left;\n  position: absolute;\n  left: 60px;\n  top: 76px;\n  width: 156px;\n  height: 51px;\n"),
        "{css}"
    );
}

#[test]
fn numbers_and_names_are_written_as_photoshop_does() {
    assert_eq!(number(48.9374), "48.937");
    assert_eq!(number(27.6), "27.6");
    assert_eq!(number(100.0), "100");
    assert_eq!(number(-0.0001), "0");
    assert_eq!(number(f64::NAN), "0");
    assert_eq!(class_name("My Box 2"), "My_Box_2");
    assert_eq!(class_name(""), "layer");
}

/// Photoshop 2025: a class can't start with a digit, so a layer named "2nd Box" gets a `_` in
/// front; a hyphen becomes `_` like the other punctuation ("-2x" is `._2x`, "my-box" `.my_box`).
#[test]
fn class_names_never_start_with_a_digit() {
    for (name, class) in [("2nd Box", "._2nd_Box {\n"), ("-2x", "._2x {\n"), ("my-box", ".my_box {\n")] {
        let css = session(name).execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
        assert!(css.starts_with(class), "{name}: {css}");
    }
}

/// Names go into quoted CSS strings: a `"`, a backslash or a line break in one is escaped, so the
/// string doesn't end early and keeps the name (Photoshop writes them unescaped, which breaks
/// the rule). Ordinary names are written as before.
#[test]
fn names_in_quoted_values_are_escaped() {
    let css = session("Say \"hi\"").execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains("  background-image: url(\"Say \\\"hi\\\".png\");\n"), "{css}");
    let css = session("a\\b\nc").execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains("  background-image: url(\"a\\\\b\\a c.png\");\n"), "{css}");
    let doc = session("Box").active().unwrap().doc.clone();
    let mut cache = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    cache.fill_rect(Rect::new(49, 28, 120, 52), &[0.0, 0.0, 0.0, 1.0]);
    let text = photocraft_doc::TextLayer { font_family: "My \"Font\"".into(), size_pt: 24.0, cache: Some(cache), ..Default::default() };
    let css = rule(&doc, 3, &Layer::new("Title", LayerContent::Text(text))).unwrap_or_default();
    assert!(css.contains("  font-family: \"My \\\"Font\\\"\";\n"), "{css}");
}

#[test]
fn bad_params_and_layers_without_css_are_errors() {
    let mut s = session("Box");
    assert!(s.execute(ID, json!({"layer": "x"})).is_err());
    assert!(s.execute(ID, json!({"layer": 999_999})).is_err());
    assert!(s.execute(ID, json!([])).is_err());
    s.execute("layer.new.group", json!({})).unwrap();
    assert!(s.execute(ID, json!({})).is_err(), "a group alone has no CSS");
    let mut empty = Session::new();
    assert!(empty.execute(ID, json!({})).is_err(), "no document");
}

/// A live rounded rectangle gives its corners as Photoshop writes them, before the fill: one
/// value when all four are equal, else top-left, top-right, bottom-right, bottom-left; a sharp
/// one writes none.
#[test]
fn a_rounded_rectangle_writes_its_border_radius() {
    let mut s = session("Box");
    let id = s.execute("shape.create", json!({"kind": "roundedRect", "rect": [40, 30, 120, 80], "radius": 12, "fill": "#ff0000"})).unwrap()["layer"]
        .as_u64()
        .unwrap();
    let css = s.execute(ID, json!({"layer": id})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains(" {\n  border-radius: 12px;\n  background-color: rgb(255, 0, 0);\n  position: absolute;\n"), "{css}");
    s.execute("shape.edit", json!({"layer": id, "radii": [20, 0, 10.5, 5]})).unwrap();
    let css = s.execute(ID, json!({"layer": id})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains("  border-radius: 20px 0px 10.5px 5px;\n  background-color: rgb(255, 0, 0);\n"), "{css}");
    s.execute("shape.edit", json!({"layer": id, "radii": [0, 0, 0, 0]})).unwrap();
    let css = s.execute(ID, json!({"layer": id})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(!css.contains("border-radius") && css.contains("  background-color: rgb(255, 0, 0);\n"), "{css}");
}
