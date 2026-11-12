use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

fn tolerance(depth: SampleType) -> f32 {
    match depth {
        SampleType::U8 => 1. / 255.,
        SampleType::U16 => 1. / 65535.,
        SampleType::F32 => 0.00001,
    }
}

fn assert_samples(actual: &[f32], expected: &[f32], depth: SampleType) {
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!((actual - expected).abs() <= tolerance(depth), "{depth:?} sample {index}: {actual} versus {expected}");
    }
}

fn command_source(source: &Surface, selection: Option<&Surface>, width: u32, height: u32, depth: u32, id: &str, params: Value) -> Surface {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":width,"height":height,"depth":depth})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    session
        .edit("owner extent fixture", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().content = LayerContent::Raster(source.clone());
            doc.selection = selection.cloned();
            Ok(())
        })
        .unwrap();
    session.execute(id, params).unwrap();
    let document = session.active().unwrap();
    document.doc.layer(document.active_layer.unwrap()).unwrap().surface().unwrap().clone()
}

#[test]
fn average_replay_intersects_requested_area_with_owner_extent_at_all_depths() {
    for depth in SampleType::ALL {
        // An opaque source makes the default black outside its owner observable:
        // it must not enter the mean merely because replay requested a larger area.
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, false));
        let owner = Rect::new(0, 0, 2, 1);
        source.write_region(owner, &[1., 0., 0., 0., 0., 1.]);
        let requested = Rect::new(-1, 0, 3, 1);
        let actual = apply_filter_to_surface("filter.blur.average", &json!({}), &source, requested, None, owner).unwrap().unwrap();
        assert_samples(&actual.read_region(owner), &[0.5, 0., 0.5, 0.5, 0., 0.5], depth);
        for outside in [Rect::new(-1, 0, 0, 1), Rect::new(2, 0, 3, 1)] {
            assert_eq!(actual.read_region(outside), source.read_region(outside), "publication remains inside the owner");
        }
    }
}

#[test]
fn average_command_keeps_canvas_publication_and_selection_weighted_mean() {
    for (depth, bits) in [(SampleType::U8, 8), (SampleType::U16, 16), (SampleType::F32, 32)] {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, false));
        let canvas = Rect::new(0, 0, 2, 1);
        source.write_region(canvas, &[1., 0., 0., 0., 0., 1.]);
        source.write_pixel(-1, 0, &[0., 1., 0.]);
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        selection.write_region(Rect::new(-1, 0, 2, 1), &[1., 0.5, 1.]);
        let actual = command_source(&source, Some(&selection), 2, 1, bits, "filter.blur.average", json!({}));
        assert_samples(&actual.read_region(canvas), &[2. / 3., 0., 1. / 3., 1. / 3., 0., 2. / 3.], depth);
        assert_eq!(actual.pixel(-1, 0), source.pixel(-1, 0), "off-canvas selection cannot enlarge Average publication");
    }
}

fn hdr_owner_extent(replay: bool) {
    for (depth, bits) in [(SampleType::U8, 8), (SampleType::U16, 16), (SampleType::F32, 32)] {
        let format = PixelFormat::new(ColorMode::Rgb, depth, true);
        let owner = Rect::new(-2, 0, 2, 2);
        let canvas = Rect::new(0, 0, 2, 2);
        let shifted_owner = owner.translate(2, 0);
        let shifted_canvas = canvas.translate(2, 0);
        let mut source = Surface::new(format);
        for y in 0..2 {
            for x in -2..2 {
                let value = if x < 0 { if depth == SampleType::F32 { 6. + y as f32 } else { 0.9 } } else { 0.1 + (x + y) as f32 * 0.1 };
                source.write_pixel(x, y, &[value, value * 0.7, value * 0.4, 0.75]);
            }
        }
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        selection.write_region(canvas, &[0.5, 1., 0., 0.75]);
        let params = json!({"radius":1,"strength":1,"detail":0,"saturation":0,"exposure":0.35});
        let options = crate::hdr_filter::params(&params).unwrap();
        // Independently move the complete owner into positive coordinates. Only
        // the requested canvas moves with it; adaptation still sees every sample.
        let mut shifted_source = Surface::new(format);
        shifted_source.write_region(shifted_owner, &source.read_region(owner));
        let mut shifted_selection = Surface::new(selection.format());
        shifted_selection.write_region(shifted_canvas, &selection.read_region(canvas));
        let reference =
            algo::tone::checked_hdr::apply_with(&shifted_source, &options, shifted_canvas, shifted_owner, Some(&shifted_selection), &Interrupt::NONE).unwrap();
        let expected = reference.read_region(shifted_canvas);
        let clipped = algo::tone::checked_hdr::apply_with(&source, &options, canvas, canvas, Some(&selection), &Interrupt::NONE).unwrap();
        assert!(
            clipped.read_region(canvas).iter().zip(&expected).any(|(a, b)| (a - b).abs() > tolerance(depth)),
            "fixture distinguishes clipped adaptation at {depth:?}"
        );
        let actual = if replay {
            apply_filter_to_surface(crate::hdr_filter::ID, &params, &source, canvas, Some(&selection), canvas).unwrap().unwrap()
        } else {
            command_source(&source, Some(&selection), 2, 2, bits, crate::hdr_filter::ID, params)
        };
        assert_samples(&actual.read_region(canvas), &expected, depth);
        assert_eq!(actual.read_region(Rect::new(-2, 0, 0, 2)), source.read_region(Rect::new(-2, 0, 0, 2)), "adaptation context is not a publication request");
        assert_eq!(actual.pixel(0, 1), source.pixel(0, 1), "zero coverage preserves every sample");
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(actual.pixel(x, y)[3], source.pixel(x, y)[3]);
            }
        }
    }
}

#[test]
fn hdr_replay_adapts_over_translated_owner_but_publishes_only_masked_canvas() {
    hdr_owner_extent(true);
}

#[test]
fn hdr_command_adapts_over_translated_owner_but_publishes_only_masked_canvas() {
    hdr_owner_extent(false);
}
