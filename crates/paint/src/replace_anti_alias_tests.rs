//! #11917: with Anti-alias off the Color Replacement tip has a hard edge: every pixel under the tip
//! is fully replaced or untouched. With it on the edge stays blended, and at a colour boundary the
//! edge is hard either way. The setup is the issue's: red (220,30,30) with a white gap at y 140-160,
//! blue foreground, Color mode, Continuous, Contiguous, tolerance 30 %, an 80 px hard tip clicked at
//! 100,120.

use photocraft_color::PixelFormat;

use super::*;
use crate::{BrushSettings, StrokePoint};

fn target() -> Surface {
    let fmt = PixelFormat::RGBA8;
    let mut s = Surface::new(fmt);
    s.fill_rect(Rect::new(0, 0, 200, 300), &photocraft_raster::from_rgba(&fmt, [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0]));
    s.fill_rect(Rect::new(0, 140, 200, 160), &photocraft_raster::from_rgba(&fmt, [1.0, 1.0, 1.0, 1.0]));
    s
}

fn click(anti_alias: bool) -> Surface {
    let mut s = target();
    let stroke = Stroke {
        brush: BrushSettings { size: 80.0, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
        points: vec![StrokePoint::new(100.0, 120.0, 1.0)],
    };
    let rs = ReplaceSettings {
        mode: ReplaceMode::Color,
        sampling: Sampling::Continuous,
        limits: Limits::Contiguous,
        tolerance: 0.3,
        anti_alias,
        color: [0.0, 0.0, 1.0, 1.0],
        ..Default::default()
    };
    apply_color_replacement(&mut s, &stroke, &rs, None, false);
    s
}

fn px(s: &Surface, x: i32, y: i32) -> [i32; 3] {
    let c = s.rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as i32)
}

const RED: [i32; 3] = [220, 30, 30];

/// Fully replaced: the Color blend of the red with the blue foreground.
fn full() -> [i32; 3] {
    let mut s = target();
    let b = blend_rgb(BlendMode::Color, [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0], [0.0, 0.0, 1.0]);
    s.fill_rect(Rect::new(0, 0, 1, 1), &photocraft_raster::from_rgba(&PixelFormat::RGBA8, [b[0], b[1], b[2], 1.0]));
    px(&s, 0, 0)
}

#[test]
fn anti_alias_off_has_a_hard_edge() {
    let s = click(false);
    let full = full();
    // Along the row through the centre, from outside the tip inwards: untouched, then fully
    // replaced, nothing in between.
    let row: Vec<[i32; 3]> = (50..70).map(|x| px(&s, x, 120)).collect();
    for (x, c) in (50..70).zip(&row) {
        assert!(*c == RED || c.iter().zip(&full).all(|(a, b)| (a - b).abs() <= 3), "x {x}: {c:?} is neither untouched nor fully replaced ({full:?})");
    }
    let first = (50..70).find(|x| px(&s, *x, 120) != RED).expect("the tip replaced the row");
    assert!((58..=60).contains(&first), "the hard edge sits at the tip's rim: x {first}");
    // The whole tip: no partly replaced pixel anywhere.
    for y in 70..170 {
        for x in 50..150 {
            let c = px(&s, x, y);
            let white = c == [255, 255, 255];
            assert!(white || c == RED || c.iter().zip(&full).all(|(a, b)| (a - b).abs() <= 3), "{x},{y}: {c:?}");
        }
    }
}

#[test]
fn anti_alias_on_keeps_the_blended_edge() {
    let s = click(true);
    let full = full();
    let partial = (55..65).map(|x| px(&s, x, 120)).filter(|c| *c != RED && c.iter().zip(&full).any(|(a, b)| (a - b).abs() > 3)).count();
    assert!(partial >= 1, "a blended ring at the rim");
}

#[test]
fn a_colour_boundary_stays_hard_either_way() {
    for anti_alias in [false, true] {
        let s = click(anti_alias);
        assert_ne!(px(&s, 100, 139), RED, "y 139 replaced (anti-alias {anti_alias})");
        assert_eq!(px(&s, 100, 140), [255, 255, 255], "y 140 white (anti-alias {anti_alias})");
    }
}
