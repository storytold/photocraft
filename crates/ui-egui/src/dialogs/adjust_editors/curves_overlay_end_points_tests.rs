use super::curve_overlay_changed;

#[test]
fn changed_channel_curves_are_overlaid_on_composite() {
    assert!(curve_overlay_changed(&[[32.0, 0.0], [255.0, 255.0]]));
    assert!(!curve_overlay_changed(&[[0.0, 0.0], [255.0, 255.0]]));
    assert!(curve_overlay_changed(&[[0.0, 0.0], [128.0, 90.0], [255.0, 255.0]]));
    assert!(curve_overlay_changed(&[[0.0, 12.0], [255.0, 255.0]]));
    assert!(curve_overlay_changed(&[[0.0, 0.0], [255.0, 220.0]]));
}
