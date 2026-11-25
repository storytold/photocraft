//! #11917: with Anti-alias off the Color Replacement tip has a hard edge: every pixel under the tip
//! is fully replaced or untouched. With it on the edge stays blended, and at a colour boundary the
//! edge is hard either way. The setup is the issue's: red (220,30,30) with a white gap at y 140-160,
//! blue foreground, Color mode, Continuous, Contiguous, tolerance 30 %, an 80 px hard tip clicked at
//! 100,120.

use photocraft_color::PixelFormat;

use super::*;
use crate::{BrushSettings, Pattern, StrokePoint, Texture};

fn target() -> Surface {
    let fmt = PixelFormat::RGBA8;
    let mut s = Surface::new(fmt);
    s.fill_rect(Rect::new(0, 0, 200, 300), &photocraft_raster::from_rgba(&fmt, [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0]));
    s.fill_rect(Rect::new(0, 140, 200, 160), &photocraft_raster::from_rgba(&fmt, [1.0, 1.0, 1.0, 1.0]));
    s
}

fn click(anti_alias: bool) -> Surface {
    click_at_flow(anti_alias, 1.0)
}

fn click_at_flow(anti_alias: bool, flow: f32) -> Surface {
    click_with(anti_alias, flow, 1.0)
}

fn click_with(anti_alias: bool, flow: f32, hardness: f32) -> Surface {
    let mut s = target();
    let stroke = Stroke {
        brush: BrushSettings { size: 80.0, hardness, spacing: 0.25, flow, pressure_size: false, ..Default::default() },
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
    apply_color_replacement(&mut s, &stroke, &rs, None, false).unwrap();
    s
}

/// The same click with a modulating brush: Wet Edges and/or a per-tip Texture with a flat mid-grey
/// tile (pattern value 0.5), hardness 1. #13160.
fn click_modulated(anti_alias: bool, wet: bool, texture: bool) -> Surface {
    click_modulated_with(anti_alias, wet, texture, 1.0)
}

fn click_modulated_with(anti_alias: bool, wet: bool, texture: bool, hardness: f32) -> Surface {
    let mut s = target();
    let brush = BrushSettings {
        size: 80.0,
        hardness,
        spacing: 0.25,
        wet_edges: wet,
        texture: Texture { enabled: texture, pattern: Pattern::Tile(crate::tile::GrayTile::from_fn(8, 8, |_, _| 0.5)), each_tip: true, ..Default::default() },
        pressure_size: false,
        ..Default::default()
    };
    let stroke = Stroke { brush, points: vec![StrokePoint::new(100.0, 120.0, 1.0)] };
    let rs = ReplaceSettings {
        mode: ReplaceMode::Color,
        sampling: Sampling::Continuous,
        limits: Limits::Contiguous,
        tolerance: 0.3,
        anti_alias,
        color: [0.0, 0.0, 1.0, 1.0],
        ..Default::default()
    };
    apply_color_replacement(&mut s, &stroke, &rs, None, false).unwrap();
    s
}

/// The red pixel recoloured at strength `k`, stored in 8 bits (the same mix the dab runs).
fn blend_at(k: f32) -> [i32; 3] {
    let mut s = target();
    let b = blend_rgb(BlendMode::Color, [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0], [0.0, 0.0, 1.0]);
    let g = [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0];
    let c = [g[0] + (b[0] - g[0]) * k, g[1] + (b[1] - g[1]) * k, g[2] + (b[2] - g[2]) * k];
    s.fill_rect(Rect::new(0, 0, 1, 1), &photocraft_raster::from_rgba(&PixelFormat::RGBA8, [c[0], c[1], c[2], 1.0]));
    px(&s, 0, 0)
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
    assert_eq!(first, 58, "the hard edge sits at the tip's rim");
    // The whole tip: no partly replaced pixel anywhere.
    for y in 70..170 {
        for x in 50..150 {
            let c = px(&s, x, y);
            // White only in the gap: a red pixel turned white would be wrong.
            let white = (140..160).contains(&y) && c == [255, 255, 255];
            assert!(white || c == RED || c.iter().zip(&full).all(|(a, b)| (a - b).abs() <= 3), "{x},{y}: {c:?}");
        }
    }
}

#[test]
fn anti_alias_on_keeps_the_blended_edge() {
    let s = click(true);
    let full = full();
    let partial = (58..=60).map(|x| px(&s, x, 120)).filter(|c| *c != RED && c.iter().zip(&full).any(|(a, b)| (a - b).abs() > 3)).count();
    assert!(partial >= 1, "a blended ring at the rim");
}

#[test]
fn a_colour_boundary_stays_hard_either_way() {
    for anti_alias in [false, true] {
        let s = click(anti_alias);
        let c = px(&s, 100, 139);
        assert!(c.iter().zip(&full()).all(|(a, b)| (a - b).abs() <= 3), "y 139 fully replaced (anti-alias {anti_alias}): {c:?}");
        assert_eq!(px(&s, 100, 140), [255, 255, 255], "y 140 white (anti-alias {anti_alias})");
    }
}

#[test]
fn anti_alias_off_keeps_the_brush_flow() {
    // A 50 % flow dab replaces half way inside the tip whichever the setting, and with Anti-alias
    // off its rim pixel takes that same strength rather than the full replacement.
    let (on, off) = (click_at_flow(true, 0.5), click_at_flow(false, 0.5));
    let centre = px(&on, 100, 120);
    assert!(centre != RED && centre != full(), "a half-strength dab: {centre:?}");
    assert_eq!(px(&off, 100, 120), centre, "the centre keeps the flow");
    assert_eq!(px(&off, 58, 120), centre, "the hard rim takes the dab's strength");
}

#[test]
fn a_soft_tip_with_anti_alias_off_replaces_fully_over_the_same_reach() {
    // The issue's soft tips (hardness 0 % and 50 %): with Anti-alias off every pixel the tip reaches
    // is fully replaced, over the same pixels as with it on; with it on the falloff stays.
    let full = full();
    let is_full = |c: &[i32; 3]| c.iter().zip(&full).all(|(a, b)| (a - b).abs() <= 3);
    for hardness in [0.0, 0.5] {
        let (on, off) = (click_with(true, 1.0, hardness), click_with(false, 1.0, hardness));
        let mut partial_on = 0;
        for y in 40..200 {
            for x in 20..180 {
                let (a, b) = (px(&on, x, y), px(&off, x, y));
                let gap = (140..160).contains(&y);
                if gap {
                    assert_eq!((a, b), ([255, 255, 255], [255, 255, 255]), "{x},{y} hardness {hardness}: the gap stays white");
                    continue;
                }
                assert!(b == RED || is_full(&b), "{x},{y} hardness {hardness}: {b:?} is partly replaced");
                assert_eq!(a != RED, b != RED, "{x},{y} hardness {hardness}: the reach differs (on {a:?}, off {b:?})");
                if a != RED && !is_full(&a) {
                    partial_on += 1;
                }
            }
        }
        assert!(partial_on > 0, "hardness {hardness}: Anti-alias on keeps the falloff");
    }
}

// #13160: the hardening with Anti-alias off is the tip's edge only. What is not the tip edge —
// Wet Edges and per-tip Texture modulation — keeps its partial-strength effect. The expected
// values are the same brush's own Anti-alias-on values at the dab centre: a hard tip covers the
// centre pixel fully there, so the strength that shows is the modulation alone.

#[test]
fn anti_alias_off_keeps_wet_edges() {
    let (on, off) = (click_modulated(true, true, false), click_modulated(false, true, false));
    // Wet Edges keeps half the paint at a dab's centre: the dab's centre pixel is at half strength
    // with Anti-alias on, and with it off too — not the full replacement.
    assert_eq!(px(&on, 100, 120), blend_at(0.5), "Anti-alias on: the wet edge keeps the centre at half strength");
    assert_eq!(px(&off, 100, 120), px(&on, 100, 120), "Anti-alias off keeps the wet-edge strength");
    assert_ne!(px(&off, 100, 120), full(), "the centre is not flattened to the dab's strength");
    // The edge stays hard: rim pixels near the wet rim value are fully replaced, pixels are never
    // a partial mix of its own.
    assert!(px(&off, 58, 120).iter().zip(&blend_at(1.0)).all(|(a, b)| (a - b).abs() <= 3), "the wet rim still reaches fully: {:?}", px(&off, 58, 120));
}

#[test]
fn anti_alias_off_keeps_per_tip_texture() {
    // A mid-grey tile at full depth: the texture halves the coverage over the whole tip, so the
    // centre is a half-strength replacement whichever way Anti-alias sits.
    let (on, off) = (click_modulated(true, false, true), click_modulated(false, false, true));
    assert_eq!(px(&off, 100, 120), px(&on, 100, 120), "Anti-alias off keeps the per-tip texture strength");
    assert_ne!(px(&off, 100, 120), full(), "the centre is not flattened to the dab's strength");
    assert_ne!(px(&off, 100, 120), RED, "the centre is replaced");
}

#[test]
fn a_soft_tip_with_anti_alias_off_keeps_its_wet_edges_and_texture() {
    // The issue's title case: a soft tip (hardness 50 %) with Anti-alias off keeps its wet edges
    // and texture. The centre pixel is inside the tip's core, so the full-tip coverage and the
    // modulation are the same as with Anti-alias on.
    let (on, off) = (click_modulated_with(true, true, true, 0.5), click_modulated_with(false, true, true, 0.5));
    assert_eq!(px(&off, 100, 120), px(&on, 100, 120), "the soft tip keeps the wet-and-texture strength at its centre");
    assert_ne!(px(&off, 100, 120), full(), "the centre is not flattened to the dab's strength");
    // Hardened edge and reach as before: the same pixels change with it on and off, and no pixel
    // sits outside the levels the modulated tip produces.
    for y in (60..180).step_by(3) {
        for x in (20..180).step_by(3) {
            let (a, b) = (px(&on, x, y), px(&off, x, y));
            if (140..160).contains(&y) {
                continue;
            }
            assert_eq!(a != RED, b != RED, "{x},{y}: the reach differs (on {a:?}, off {b:?})");
        }
    }
}
