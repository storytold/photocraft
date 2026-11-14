use super::*;

/// A fixed 700×300 scene, with detail distributed across both 100 px overlaps.
pub(super) fn scene() -> Vec<[f32; 3]> {
    let (w, h) = (700, 300);
    let mut state = 5u64;
    let mut random = |n: usize| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((state >> 32) as usize) % n
    };
    let mut pixels = vec![[128.0 / 255.0; 3]; w * h];
    for _ in 0..160 {
        let (x, y) = (random(w), random(h));
        let (right, bottom) = ((x + 12 + random(70)).min(w), (y + 12 + random(55)).min(h));
        let colour = std::array::from_fn(|_| (30 + random(195)) as f32 / 255.0);
        for row in pixels.chunks_exact_mut(w).take(bottom).skip(y) {
            row[x..right].fill(colour);
        }
    }
    for pixel in &mut pixels {
        let noise = (random(21) as f32 - 10.0) / 255.0;
        for channel in pixel {
            *channel += noise;
        }
    }
    pixels
}

pub(super) fn tile(pixels: &[[f32; 3]], x: usize, mode: ColorMode, sample: SampleType) -> Document {
    let mut doc = Document::new(format!("tile-{x}"), Size::new(300, 300), mode, sample);
    let mut surf = Surface::new(doc.pixel_format());
    let mut data = Vec::new();
    for row in pixels.as_chunks::<700>().0 {
        for rgb in &row[x..x + 300] {
            if mode == ColorMode::Grayscale {
                data.push(photocraft_color::convert::rgb_to_gray(*rgb));
            } else {
                data.extend(rgb);
            }
            data.push(1.0);
        }
    }
    surf.write_region(doc.bounds(), &data);
    doc.layers.push(Layer::new("Source", LayerContent::Raster(surf)));
    doc
}

#[test]
fn overlapping_whole_pixel_tiles_keep_exact_bounds_and_coverage() {
    let pixels = scene();
    for layout in ["auto", "perspective", "reposition", "collage"] {
        for blend in [false, true] {
            let mut session = Session::new();
            for x in [0, 200, 400] {
                session.add_document(tile(&pixels, x, ColorMode::Rgb, SampleType::U8), None);
            }
            let result = session.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": layout, "blend": blend})).unwrap();
            let doc = &session.active().unwrap().doc;
            assert_eq!(doc.size, Size::new(700, 300), "{layout} blend={blend}: {result}");
            for x in [0, 200, 400] {
                let layer = doc.layers.iter().find(|layer| layer.name == format!("tile-{x}")).unwrap();
                let surface = layer.surface().unwrap();
                assert_eq!(surface.content_bounds(), Rect::new(x, 0, x + 300, 300), "{layout} blend={blend}");
                if !blend {
                    let src = session.documents().iter().find(|d| d.doc.name == layer.name).unwrap();
                    assert_eq!(surface.read_region(Rect::new(x, 0, x + 300, 300)), src.doc.layers[0].surface().unwrap().read_region(src.doc.bounds()));
                }
            }
            let composite = photocraft_compose::flatten(doc);
            assert!(composite.px.iter().all(|p| p[3] > 0.99), "{layout} blend={blend}");
        }
    }
}

#[test]
fn only_arithmetic_noise_is_treated_as_a_whole_pixel_shift() {
    let placement = |dx, xy| Placement {
        projection: panorama::Projection::Plane,
        focal: 300.0,
        center: [0.0, 0.0],
        k1: 0.0,
        h: Homography([1.0, xy, dx, 0.0, 1.0, -1e-7, 0.0, 0.0, 1.0]),
    };
    assert_eq!(whole_pixel_shift(&placement(200.0 + 1e-7, 1e-10), 300, 300), Some((200, 0)));
    assert_eq!(whole_pixel_shift(&placement(200.01, 0.0), 300, 300), None);
    assert_eq!(whole_pixel_shift(&placement(200.0, 0.001), 300, 300), None);
}

#[test]
fn tone_shifted_panorama_stays_at_whole_pixel_positions() {
    let pixels = scene();
    let bright: Vec<_> = pixels.iter().map(|rgb| rgb.map(|value| (value * 2f32.sqrt()).min(1.0))).collect();
    for (middle, mode) in [(&bright, ColorMode::Rgb), (&pixels, ColorMode::Grayscale)] {
        let mut session = Session::new();
        for (source, x, mode) in [(&pixels, 0, ColorMode::Rgb), (middle, 200, mode), (&pixels, 400, ColorMode::Rgb)] {
            session.add_document(tile(source, x, mode, SampleType::U8), None);
        }
        let result = session.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "auto"})).unwrap();
        let doc = &session.active().unwrap().doc;
        assert_eq!(doc.size, Size::new(700, 300), "{mode:?}: {result}");
        for x in [0, 200, 400] {
            let layer = doc.layers.iter().find(|layer| layer.name == format!("tile-{x}")).unwrap();
            assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(x, 0, x + 300, 300), "{mode:?}: {result}");
        }
        if mode == ColorMode::Grayscale {
            let composite = photocraft_compose::flatten(doc);
            for (x, y) in [(30, 90), (640, 150), (690, 290)] {
                let i = y * 700 + x;
                for (actual, expected) in composite.px[i].iter().zip(pixels[i]) {
                    assert!((actual - expected).abs() <= 10.0 / 255.0, "{x},{y}: {actual} vs {expected}");
                }
            }
        }
    }
}
