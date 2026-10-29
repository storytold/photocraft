use super::{Adjustment, CommandSpec, Value, destructive_adjust, has_doc, has_pixel_layer, turn};

/// Image menu commands: canvas rotations and flips, and Desaturate.
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("image.imageRotation.flipCanvasHorizontal", "Flip Canvas Horizontal", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Flip Canvas Horizontal", crate::canvas_geom::Turn::FlipHorizontal)
        }),
        cmd!("image.imageRotation.flipCanvasVertical", "Flip Canvas Vertical", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Flip Canvas Vertical", crate::canvas_geom::Turn::FlipVertical)
        }),
        cmd!("image.imageRotation.180", "180°", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Rotate Canvas", crate::canvas_geom::Turn::Rotate180)
        }),
        cmd!("image.imageRotation.90cw", "90° Clockwise", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Rotate Canvas", crate::canvas_geom::Turn::Cw90)
        }),
        cmd!("image.imageRotation.90ccw", "90° Counter Clockwise", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Rotate Canvas", crate::canvas_geom::Turn::Ccw90)
        }),
        cmd!("image.adjustments.desaturate", "Desaturate", ["Image", "Adjustments"], Some("Cmd+Shift+U"), "{}", has_pixel_layer, |s, _| {
            destructive_adjust(
                s,
                "Desaturate",
                Adjustment::HueSaturation {
                    hue: 0.0,
                    saturation: -100.0,
                    lightness: 0.0,
                    colorize: false,
                    ranges: photocraft_doc::adjust::HueRange::defaults(),
                },
                &Value::Null,
            )
        }),
    ]
}
