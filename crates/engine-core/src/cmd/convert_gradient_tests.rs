//! Converting a document keeps Photoshop's own gradient of a Gradient Fill layer and a Gradient
//! Overlay (#13557): with Smoothness and a moved midpoint, the rendered stops are baked again from
//! the converted gradient, so a save still writes it.

use super::*;
use photocraft_doc::effects::{FxCommon, Gradient, GradientInterpolation};
use photocraft_io::gradient_bake::{bake_interpolation, interpolation_matches};

fn source() -> GradientInterpolation {
    let mut g = GradientInterpolation::new("classic", 100.0, vec![(0.0, Color::rgba(0.9, 0.1, 0.1, 1.0)), (1.0, Color::rgba(0.1, 0.2, 0.9, 1.0))]);
    g.midpoints = vec![0.3];
    g
}

fn document(source: &GradientInterpolation) -> Document {
    let mut d = Document::with_background("t", photocraft_doc::Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::rgba(1.0, 1.0, 1.0, 1.0));
    d.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
    let stops = bake_interpolation(source);
    let mut fill = Fill::gradient(stops.clone(), 90.0, 1.0, Default::default(), false);
    if let Fill::Gradient { interpolation, .. } = &mut fill {
        *interpolation = Some(Box::new(source.clone()));
    }
    let mut layer = Layer::new("Gradient Fill", LayerContent::Fill(fill));
    let gradient = Gradient { stops, ..Gradient::default() };
    layer.effects.items.push(Effect::GradientOverlay {
        common: FxCommon::new(photocraft_doc::BlendMode::Normal, 1.0),
        gradient,
        dither: false,
        interpolation: Some(source.clone()),
    });
    d.layers.push(layer);
    d
}

fn kept(d: &Document) -> (bool, bool) {
    let l = d.layers.last().unwrap();
    let LayerContent::Fill(Fill::Gradient { stops, opacity_stops, interpolation: Some(src), .. }) = &l.content else { panic!("no kept gradient") };
    let Some(Effect::GradientOverlay { gradient, interpolation: Some(fx), .. }) = l.effects.items.first() else { panic!("no kept overlay") };
    (interpolation_matches(src, stops, opacity_stops), interpolation_matches(fx, &gradient.stops, &gradient.opacity_stops))
}

#[test]
fn converting_keeps_photoshops_gradient_of_a_fill_layer_and_an_overlay() {
    let mut d = document(&source());
    assert_eq!(kept(&d), (true, true));
    convert_document(&mut d, Builtin::DisplayP3.profile(), Intent::RelativeColorimetric, false).unwrap();
    assert_eq!(kept(&d), (true, true), "the rendered stops are the converted gradient baked");
    let l = d.layers.last().unwrap();
    let LayerContent::Fill(Fill::Gradient { interpolation: Some(src), .. }) = &l.content else { panic!() };
    assert_ne!(src.stops, source().stops, "the kept gradient was converted");
}

#[test]
fn converting_keeps_stops_edited_since_the_gradient_was_kept() {
    let mut d = document(&source());
    let edited = vec![(0.0, Color::rgba(0.0, 0.0, 0.0, 1.0)), (1.0, Color::rgba(1.0, 1.0, 1.0, 1.0))];
    if let LayerContent::Fill(Fill::Gradient { stops, .. }) = &mut d.layers.last_mut().unwrap().content {
        *stops = edited.clone();
    }
    convert_document(&mut d, Builtin::DisplayP3.profile(), Intent::RelativeColorimetric, false).unwrap();
    let LayerContent::Fill(Fill::Gradient { stops, .. }) = &d.layers.last().unwrap().content else { panic!() };
    assert_eq!(stops.len(), 2, "the edited stops are converted, not replaced by the kept gradient");
    assert!(stops[0].1.c[..3].iter().all(|v| *v < 0.01) && stops[1].1.c[..3].iter().all(|v| *v > 0.99), "{stops:?}");
}
