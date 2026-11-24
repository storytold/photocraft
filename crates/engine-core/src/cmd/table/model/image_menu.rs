use crate::cmd::{CommandSpec, Result, Session, Value, has_doc};

/// Image › Image Rotation by a right angle or a flip (pixels, vectors, guides, … all move).
fn turn(s: &mut Session, label: &str, t: crate::canvas_geom::Turn) -> Result<Value> {
    if let Some(d) = s.active() {
        let a = t.affine(f64::from(d.doc.size.width), f64::from(d.doc.size.height));
        // Sampler points are pixel indices, not pixel edges: map the pixel's centre, so the point
        // lands on the same pixel `Turn::pixel_map` moves the pixels to (`w - 1 - x`).
        let half = photocraft_geom::Affine::translate(0.5, 0.5);
        s.set_canvas_map(a, half.then(&a).then(&photocraft_geom::Affine::translate(-0.5, -0.5)));
    }
    s.edit(label, |doc, _| {
        crate::canvas_geom::keeping_count_points(doc, |doc| crate::canvas_geom::turn_canvas(doc, t));
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Image menu: canvas rotations and flips.
pub fn specs() -> Vec<CommandSpec> {
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
    ]
}
