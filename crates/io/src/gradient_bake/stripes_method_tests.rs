//! #14688: the Gradient Map's Stripes method holds each stop's colour until the next stop, and only the end reaches
//! the last stop: black → white maps 0..254 to black and 255 to white; black / red at 50 % / white maps 0..127 to
//! black, 128..254 to red and 255 to white. The layer effects don't take Stripes yet.

use photocraft_doc::adjust::GradientSource;

use super::{Method, bake_source};

/// The baked stops at `t`, interpolated linearly as the compositors do.
fn sample(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else { return [0.0; 3] };
    if t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    let i = stops.windows(2).position(|w| t >= w[0].0 && t <= w[1].0).unwrap();
    let ((t0, a), (t1, b)) = (stops[i], stops[i + 1]);
    let u = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
    std::array::from_fn(|c| a[c] + (b[c] - a[c]) * u)
}

fn source(stops: &[(f32, [f32; 3])]) -> GradientSource {
    GradientSource { stops: stops.to_vec(), midpoints: vec![0.5; stops.len() - 1], smoothness: 1.0, method: "stripes".into() }
}

/// The 8-bit output of every input level.
fn levels(stops: &[(f32, [f32; 3])]) -> Vec<[u8; 3]> {
    let baked = bake_source(&source(stops));
    (0..=255).map(|x| sample(&baked, x as f32 / 255.0).map(|v| (v * 255.0).round() as u8)).collect()
}

const BLACK: [f32; 3] = [0.0; 3];
const WHITE: [f32; 3] = [1.0; 3];
const RED: [f32; 3] = [1.0, 0.0, 0.0];

#[test]
fn black_to_white_holds_black_until_the_end() {
    let out = levels(&[(0.0, BLACK), (1.0, WHITE)]);
    assert!(out[..255].iter().all(|c| *c == [0; 3]), "0..254 are black");
    assert_eq!(out[255], [255; 3]);
}

#[test]
fn each_stop_holds_until_the_next() {
    let out = levels(&[(0.0, BLACK), (0.5, RED), (1.0, WHITE)]);
    assert!(out[..128].iter().all(|c| *c == [0; 3]), "0..127 are black");
    assert!(out[128..255].iter().all(|c| *c == [255, 0, 0]), "128..254 are red");
    assert_eq!(out[255], [255; 3]);
}

#[test]
fn only_the_gradient_map_names_stripes() {
    assert_eq!(Method::for_gradient_map("stripes"), Some(Method::Stripes));
    assert_eq!(Method::for_gradient_map("Smooth"), Some(Method::Smooth));
    assert_eq!(Method::from_name("stripes"), None, "the layer effects and the Gradient tool don't take it yet");
}
