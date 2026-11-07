//! A `GdFl` gradient fill with no `Angl` renders horizontally, like one written at 0°.
//!
//! Photoshop omits `Angl` when the angle is 0°: psd-tools' `gradient-styles.psd` has five
//! linear noise fills without the key whose stored pixels vary along x. That corpus set is
//! MIT-licensed, fetched by `xtask` and documented in `docs/development.md`; it is not
//! committed, and the descriptors below are synthesized, so this test ships no third-party
//! bytes.

use photocraft_color::Color;
use photocraft_doc::{Fill, GradientStyle};
use photocraft_geom::Rect;
use photocraft_io::blocks::{fill_from_desc, fill_to_desc};

const W: usize = 16;
const H: usize = 8;

fn gradient(angle: f32) -> Fill {
    Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], angle, 1.0, GradientStyle::Linear, false)
}

/// The fill's red channel at `(x, y)`, rendered over the whole test frame.
fn red(px: &[[f32; 4]], x: usize, y: usize) -> f32 {
    px[y * W + x][0]
}

fn render(f: &Fill) -> Vec<[f32; 4]> {
    let frame = Rect::new(0, 0, W as i32, H as i32);
    photocraft_compose::gradient_fill::render(f, frame, frame)
}

#[test]
fn a_gradient_fill_without_angl_renders_horizontally() {
    let f = gradient(0.0);
    let (k, mut d) = fill_to_desc(&f);
    d.items.retain(|(key, _)| !key.is("Angl"));
    let missing = fill_from_desc(&k, &d).expect("a GdFl without Angl parses");
    let explicit_zero = fill_from_desc(&k, &fill_to_desc(&f).1).expect("a GdFl at 0° parses");

    let (no_angl, zero) = (render(&missing), render(&explicit_zero));
    assert_eq!(no_angl, zero, "a missing Angl renders exactly like an explicit 0");

    let (left, right) = (red(&no_angl, 0, H / 2), red(&no_angl, W - 1, H / 2));
    assert!(left < 0.1 && right > 0.8, "the ramp runs along x: left {left}, right {right}");
    for x in [0, W / 2, W - 1] {
        assert!((red(&no_angl, x, 0) - red(&no_angl, x, H - 1)).abs() < 1e-6, "column {x} is constant");
    }
}

#[test]
fn an_explicit_angle_still_renders_vertically() {
    let f = gradient(90.0);
    let (k, d) = fill_to_desc(&f);
    let vertical = render(&fill_from_desc(&k, &d).expect("a GdFl at 90° parses"));

    let (top, bottom) = (red(&vertical, W / 2, 0), red(&vertical, W / 2, H - 1));
    assert!(top > 0.8 && bottom < 0.2, "the ramp runs along y: top {top}, bottom {bottom}");
    for y in [0, H / 2, H - 1] {
        assert!((red(&vertical, 0, y) - red(&vertical, W - 1, y)).abs() < 1e-6, "row {y} is constant");
    }
}
