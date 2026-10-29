#![no_main]
//! Parse arbitrary bytes as an ICC profile, then build transforms to and from sRGB in every
//! intent and push a few pixels through each, so curve, CLUT and matrix evaluation run too.

use libfuzzer_sys::fuzz_target;
use photocraft_cms::{Intent, Profile, Transform, builtin};

/// Wider than any ICC colour space (15 channels), so the stride check never trips.
const STRIDE: usize = 16;

fuzz_target!(|data: &[u8]| {
    let Ok(profile) = Profile::parse(data) else { return };
    let _ = profile.to_bytes();
    let srgb = builtin::srgb();
    let src: Vec<f32> = (0..STRIDE * 5).map(|i| (i % 7) as f32 / 6.0).collect();
    let mut dst = vec![0.0f32; STRIDE * 5];
    for intent in [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation, Intent::AbsoluteColorimetric] {
        for (from, to) in [(&profile, srgb), (srgb, &profile)] {
            if let Ok(t) = Transform::new(from, to, intent, intent == Intent::Perceptual) {
                t.convert_f32(&src, STRIDE, &mut dst, STRIDE, false);
            }
        }
    }
});
