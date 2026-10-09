//! Public Affinity documents (`cargo xtask corpus --affinity`, pinned and sha256-verified) open
//! natively and look like Affinity's own picture of them: every document embeds a thumbnail
//! Affinity rendered when saving it. Each file's mean difference from that thumbnail (0–255 per
//! channel, both flattened on white, at the smaller of the two sizes) must stay under its ceiling,
//! set just above the difference measured when the importer landed.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use photocraft_io::{ExportOptions, export, import};

/// (file, ceiling). Measured differences when the importer landed are in the comments.
const FILES: &[(&str, f64)] = &[
    ("vector-art/mexican-guy.af", 3e+00),                  // 1.92: brush strokes as plain strokes
    ("vector-art/mexican-man.af", 4e+00),                  // 2.74
    ("vector-art/mexican-woman.af", 4e+00),                // 2.24
    ("vector-art/playing-cards.af", 4e+00),                // 3.04: layer effects
    ("vector-art/beer.afdesign", 4e+00),                   // 2.91
    ("vector-art/cactus.afdesign", 2e+00),                 // 0.57
    ("vector-art/car.afdesign", 2e+00),                    // 0.39
    ("vector-art/flowers.afdesign", 3e+00),                // 2.09
    ("vector-art/lion-track.afdesign", 6e+00),             // 4.41
    ("afdesignload/color.afdesign", 1e+00),                // 0.00
    ("afdesignload/layer_mode.afdesign", 1e+00),           // 0.00
    ("afdesignload/layer_test.afdesign", 1e+00),           // 0.00
    ("afdesignload/margins.afdesign", 1e+00),              // 0.00
    ("afdesignload/raster_test.afdesign", 2e+00),          // 0.82: pixel layer
    ("afdesignload/revision_test.afdesign", 1e+00),        // 0.01
    ("afdesignload/shape_test.afdesign", 6e+00),           // 4.60: cloud and heart shapes as ellipses
    ("afdesignload/slice_test.afdesign", 1e+00),           // 0.02
    ("afdesignload/test_path.afdesign", 2e+00),            // 1.37
    ("jac21/SimpleLogo.afdesign", 2e+00),                  // 0.78
    ("jac21/SimpleLogoBanner.afdesign", 4e+00),            // 2.72: placed images in a Display P3 document
    ("asset-store-template/AssetStore.aftemplate", 1e+00), // 0.00: artboards
];

fn corpus(rel: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/affinity").join(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}: run `cargo xtask corpus --affinity`", p.display()))
}

/// Straight RGBA8 pixels flattened on white.
fn on_white(px: &[u8]) -> Vec<f64> {
    px.chunks_exact(4)
        .flat_map(|p| {
            let a = f64::from(p[3]) / 255.0;
            [0, 1, 2].map(|i| f64::from(p[i]) * a + 255.0 * (1.0 - a))
        })
        .collect()
}

/// Resample `src` (`sw`×`sh`, 3 channels) to `dw`×`dh`: each target pixel averages the source
/// pixels it covers (at least one, so small documents scale up too).
fn shrink(src: &[f64], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f64> {
    let span = |d: usize, s: usize, n: usize| {
        let a = (d * s / n).min(s - 1);
        (a, ((d + 1) * s).div_ceil(n).clamp(a + 1, s))
    };
    let mut out = Vec::with_capacity(dw * dh * 3);
    for ty in 0..dh {
        let (y0, y1) = span(ty, sh, dh);
        for tx in 0..dw {
            let (x0, x1) = span(tx, sw, dw);
            let mut sum = [0.0f64; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += src[(y * sw + x) * 3 + c];
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as f64;
            out.extend(sum.map(|s| s / n));
        }
    }
    out
}

#[test]
fn public_affinity_documents_look_like_their_affinity_thumbnails() {
    let mut failures = Vec::new();
    for (name, ceiling) in FILES {
        let bytes = corpus(name);
        let r = import(name, &bytes).unwrap();
        assert!(!r.preview_only, "{name} opened as its preview: {:?}", r.warnings);
        let thumb = photocraft_codecs::decode(photocraft_affinity::preview(&bytes).unwrap().png).unwrap();
        let (tw, th) = thumb.dimensions();
        let png = export(&r.document, "x.png", &ExportOptions::default()).unwrap().bytes;
        let flat = photocraft_codecs::decode(&png).unwrap();
        let (fw, fh) = flat.dimensions();
        // Several pages sit side by side; the thumbnail shows the first.
        let fw_first = ((f64::from(fh) * f64::from(tw) / f64::from(th)).round() as u32).min(fw);
        let rgba = flat.to_rgba8();
        let cropped: Vec<u8> = rgba.chunks_exact(fw as usize * 4).flat_map(|row| row[..fw_first as usize * 4].to_vec()).collect();
        // Compare at the smaller of the two sizes, so neither picture is enlarged.
        let (cw, ch) = if fh < th { (fw_first, fh) } else { (tw, th) };
        let got = shrink(&on_white(&cropped), fw_first as usize, fh as usize, cw as usize, ch as usize);
        let want = shrink(&on_white(&thumb.to_rgba8()), tw as usize, th as usize, cw as usize, ch as usize);
        let diff = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).sum::<f64>() / want.len() as f64;
        eprintln!("{name}: {diff:.2} (ceiling {ceiling})");
        if diff > *ceiling {
            failures.push(format!("{name}: {diff:.2} > {ceiling}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
