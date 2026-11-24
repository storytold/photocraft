use super::*;

#[test]
fn options_serde_defaults_keep_old_tool_settings_and_full_stroke() {
    assert_eq!(serde_json::from_value::<StrokeOptions>(json!({})).unwrap(), StrokeOptions::default());
    let style = StrokeOptions { cap: photocraft_doc::LineCap::Round, dashes: vec![0.0, 2.0, 4.0, 1.0], dash_offset: -0.5, ..StrokeOptions::default() };
    let stroke = style.stroke(5.0, photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK));
    assert_eq!(StrokeOptions::from(&stroke), style);
    assert_eq!(serde_json::from_value::<StrokeOptions>(serde_json::to_value(&style).unwrap()).unwrap(), style);
}
