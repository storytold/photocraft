//! #13807: Edit › Fade below 100 % in Color Burn, Color Dodge, Vivid Light, Hard Mix and
//! Difference gives the reference application's pixels: the opacity acts as a layer's Fill does
//! in these modes. Every other mode, and every mode at 100 %, keeps mixing the full-strength
//! result with the base.

use photocraft_color::blend::composite;
use serde_json::json;

use crate::Session;

/// The issue's sample points: inside the blue, green and grey blocks.
const SAMPLES: [(i32, i32); 3] = [(300, 75), (100, 225), (300, 225)];

/// The issue's test document at `depth`: 400×300 px, a white `Background` and an opaque pixel
/// layer `Paint` with red, blue, green and grey blocks and a yellow square.
fn blocks(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    for (x, y, w, h, color) in [
        (0, 0, 200, 150, "#ff0000"),
        (200, 0, 200, 150, "#0080ff"),
        (0, 150, 200, 150, "#40c840"),
        (200, 150, 200, 150, "#808080"),
        (50, 50, 100, 100, "#ffff00"),
    ] {
        s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// [`blocks`] after Image › Adjustments › Invert on `Paint`, so Fade fades the Invert.
fn inverted_blocks(depth: u32) -> Session {
    let mut s = blocks(depth);
    s.execute("image.adjustments.invert", json!({})).unwrap();
    s
}

/// The active layer's straight RGBA at `(x, y)`, in 0..1.
fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

/// The colour channels of `p` in 8-bit levels.
fn levels(p: [f32; 4]) -> [f32; 3] {
    [p[0] * 255.0, p[1] * 255.0, p[2] * 255.0]
}

/// Asserts that `got` is within one level of `want` in every channel.
fn assert_near(got: [f32; 3], want: [f32; 3], what: &str) {
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1.0), "{what}: got {got:?}, want {want:?}");
}

/// Fades the Invert of [`inverted_blocks`] at `opacity` in `mode` and returns the sample points
/// before the Invert, after it and after the Fade.
fn fade_samples(depth: u32, mode: &str, opacity: f32) -> Vec<([f32; 4], [f32; 4], [f32; 4])> {
    let base = blocks(depth);
    let before: Vec<_> = SAMPLES.iter().map(|&(x, y)| rgba(&base, x, y)).collect();
    let mut s = inverted_blocks(depth);
    let after: Vec<_> = SAMPLES.iter().map(|&(x, y)| rgba(&s, x, y)).collect();
    s.execute("edit.fade", json!({"opacity": opacity, "mode": mode})).unwrap();
    SAMPLES.iter().zip(before.into_iter().zip(after)).map(|(&(x, y), (b, a))| (b, a, rgba(&s, x, y))).collect()
}

/// The reference's measured pixels at the three sample points, Invert faded 37 %.
#[test]
fn fade_37_percent_in_the_fill_modes_gives_the_reference_pixels() {
    let measured: [(&str, [[f32; 3]; 3]); 5] = [
        ("Color Burn", [[0.0, 99.0, 255.0], [44.0, 178.0, 44.0], [99.0, 99.0, 99.0]]),
        ("Color Dodge", [[0.0, 157.0, 255.0], [88.0, 217.0, 88.0], [157.0, 157.0, 157.0]]),
        ("Vivid Light", [[0.0, 127.0, 255.0], [78.0, 186.0, 78.0], [127.0, 127.0, 127.0]]),
        ("Hard Mix", [[0.0, 128.0, 255.0], [63.0, 200.0, 63.0], [128.0, 128.0, 128.0]]),
        ("Difference", [[94.0, 81.0, 255.0], [6.0, 180.0, 6.0], [81.0, 81.0, 81.0]]),
    ];
    for (mode, want) in measured {
        for ((_, _, got), (want, at)) in fade_samples(8, mode, 37.0).into_iter().zip(want.into_iter().zip(SAMPLES)) {
            assert_near(levels(got), want, &format!("{mode} 37 % at {at:?}"));
            assert_eq!(got[3], 1.0, "{mode} at {at:?}: the layer stays opaque");
        }
    }
}

/// Every other mode below 100 %, and every mode at 100 %, still mixes the full-strength result
/// with the base: `base + opacity × (B(base, inverted) − base)`. Dissolve picks per pixel instead (#13809).
#[test]
fn other_modes_and_full_opacity_keep_mixing_the_result_with_the_base() {
    let special = ["Color Burn", "Color Dodge", "Vivid Light", "Hard Mix", "Difference"];
    for mode in photocraft_color::BlendMode::LAYER_MODES {
        let name = mode.label();
        let opacities: &[f32] = match name {
            "Dissolve" => &[],
            _ if special.contains(&name) => &[100.0],
            _ => &[37.0, 100.0],
        };
        for &opacity in opacities {
            for ((before, after, got), at) in fade_samples(8, name, opacity).into_iter().zip(SAMPLES) {
                let full = composite(mode, before, after, 1.0);
                let want: [f32; 3] = std::array::from_fn(|i| (before[i] + (full[i] - before[i]) * opacity / 100.0) * 255.0);
                assert_near(levels(got), want, &format!("{name} {opacity} % at {at:?}"));
            }
        }
    }
}

/// 16-bit documents weren't measured and keep the mix: Color Burn 37 % at the green block is
/// still 40,126,40, not the 8-bit 44,178,44.
#[test]
fn a_16_bit_layer_keeps_the_mix() {
    let (_, _, got) = fade_samples(16, "Color Burn", 37.0)[1];
    assert_near(levels(got), [40.0, 126.0, 40.0], "16-bit Color Burn 37 % at (100, 225)");
}

/// Where the layer was transparent before the step, a Fill-mode fade leaves the faded pixels as
/// the mix does: the new colour at the opacity's alpha. Where it was opaque, the blend changes.
#[test]
fn a_pixel_transparent_before_the_step_fades_to_the_new_colour_at_the_opacity() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20, "depth": 8, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "T"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"color": "#40c840"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#bf37bf"})).unwrap();
    s.execute("edit.fade", json!({"opacity": 37, "mode": "Color Burn"})).unwrap();
    let clear = rgba(&s, 30, 10);
    assert_near(levels(clear), [191.0, 55.0, 191.0], "over transparency");
    assert!((clear[3] - 0.37).abs() < 0.5 / 255.0 + 1e-4, "over transparency the alpha is the opacity: {clear:?}");
    let opaque = rgba(&s, 10, 10);
    assert_near(levels(opaque), [44.0, 178.0, 44.0], "over the opaque block");
    assert_eq!(opaque[3], 1.0);
}

/// The Fade dialog's preview shows what OK applies in these modes too.
#[test]
fn the_preview_equals_ok_in_the_fill_modes() {
    for mode in ["Color Burn", "Hard Mix"] {
        let mut s = inverted_blocks(8);
        let p = json!({"opacity": 37, "mode": mode});
        let preview = super::fade_preview(&s, &p).and_then(super::FadePreview::render).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        let shown: Vec<_> = SAMPLES.iter().map(|&(x, y)| preview.layer(id).unwrap().surface().unwrap().rgba(x, y)).collect();
        s.execute("edit.fade", p).unwrap();
        let applied: Vec<_> = SAMPLES.iter().map(|&(x, y)| rgba(&s, x, y)).collect();
        assert_eq!(shown, applied, "{mode}");
    }
}
