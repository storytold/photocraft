//! #14975: in a Grayscale document a colour pattern paints the greys of the document's grey
//! profile, as measured in the reference application (blue 69, green 221, red 129 of 255), not the
//! darker luma-formula greys (29 / 150 / 76). RGB, Lab, 16-bit and 32-bit documents keep the
//! pattern's own colours. What the issue did not measure keeps the luma greys: 32-bit Grayscale,
//! Duotone and Bitmap documents, and a Grayscale document in another grey working space.

use std::sync::Arc;

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Effect, FxCommon, Layer, Pattern};
use photocraft_geom::{Rect, Size};

use super::{PatternSpace, pattern_rgba};
use crate::render;

/// The issue's test pattern: 20 x 20 px, quadrants red (top left), green (top right), blue
/// (bottom left) and black (bottom right).
fn quadrants() -> Pattern {
    let mut s = photocraft_raster::Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
    s.fill_rect(Rect::new(10, 0, 20, 10), &[0.0, 1.0, 0.0, 1.0]);
    s.fill_rect(Rect::new(0, 10, 10, 20), &[0.0, 0.0, 1.0, 1.0]);
    s.fill_rect(Rect::new(10, 10, 20, 20), &[0.0, 0.0, 0.0, 1.0]);
    Pattern::new("quadrants", s, 20, 20)
}

/// The quadrants as a 16-bit grey pattern holding the luma formula's greys (0.299 R + 0.587 G +
/// 0.114 B): no document converts a grey pattern, so it paints what the quadrants painted in a
/// Grayscale document before #14975.
fn luma_quadrants() -> Pattern {
    let mut s = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, true));
    s.fill_rect(Rect::new(0, 0, 10, 10), &[0.299, 1.0]);
    s.fill_rect(Rect::new(10, 0, 20, 10), &[0.587, 1.0]);
    s.fill_rect(Rect::new(0, 10, 10, 20), &[0.114, 1.0]);
    s.fill_rect(Rect::new(10, 10, 20, 20), &[0.0, 1.0]);
    Pattern::new("quadrants", s, 20, 20)
}

/// The measured greys (of 255) of the red, green and blue cells.
const MEASURED: [(&str, (i32, i32), f32); 3] = [("red", (5, 5), 129.0), ("green", (15, 5), 221.0), ("blue", (5, 15), 69.0)];

/// The luma formula's greys (of 255) of the same cells: what they painted before #14975.
const LUMA: [(&str, (i32, i32), f32); 3] = [("red", (5, 5), 76.245), ("green", (15, 5), 149.685), ("blue", (5, 15), 29.07)];

/// A 20 x 20 document of `mode` and `depth` with a white Background and a layer covering it that
/// carries `pat` as a Normal 100 % Pattern Overlay.
fn overlay_doc_of(pat: Pattern, mode: ColorMode, depth: SampleType) -> Document {
    let mut d = Document::with_background("t", Size::new(20, 20), mode, depth, Color::WHITE);
    let fmt = d.pixel_format();
    let mut l = Layer::raster("overlay", fmt);
    // An opaque mid grey, encoded in the document's own channels.
    l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 20, 20), &photocraft_raster::from_rgba(&fmt, [0.5, 0.5, 0.5, 1.0]));
    l.effects.enabled = true;
    l.effects.items.push(Effect::PatternOverlay {
        common: FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
        name: pat.name.clone(),
        id: pat.id.clone(),
        scale: 1.0,
        angle: 0.0,
        link: true,
        phase: (0.0, 0.0),
    });
    d.patterns.push(pat);
    d.layers.push(l);
    d
}

/// [`overlay_doc_of`] the issue's quadrants.
fn overlay_doc(mode: ColorMode, depth: SampleType) -> Document {
    overlay_doc_of(quadrants(), mode, depth)
}

fn at(d: &Document, (x, y): (i32, i32)) -> [f32; 4] {
    render(d, Rect::from_xywh(x, y, 1, 1)).px[0]
}

/// The ICC bytes of a built-in profile, as a document embeds them.
fn icc(profile: craft_cms::Builtin) -> Option<Arc<Vec<u8>>> {
    Some(profile.profile().to_bytes())
}

#[test]
fn a_colour_pattern_converts_to_the_grey_profiles_greys() {
    let px = pattern_rgba(&quadrants(), PatternSpace::ProfileGray);
    let mut wrong = Vec::new();
    for (cell, (x, y), want) in MEASURED {
        let got = px[(y * 20 + x) as usize];
        if (got[0] * 255.0 - want).abs() > 2.0 || got[0] != got[1] || got[1] != got[2] || got[3] != 1.0 {
            wrong.push(format!("{cell}: {:?} (grey {:.1}), want {want}", got, got[0] * 255.0));
        }
    }
    // Black stays black.
    let black = px[15 * 20 + 15];
    if black[0] * 255.0 > 0.5 {
        wrong.push(format!("black: {black:?}"));
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A caller that names the Grayscale colour mode, not a document, keeps the pixel format's grey:
/// Image › Mode › Bitmap ranks a Custom Pattern's thresholds by it.
#[test]
fn a_named_colour_mode_keeps_its_pixel_format_conversion() {
    let px = pattern_rgba(&quadrants(), ColorMode::Grayscale);
    for (cell, (x, y), want) in LUMA {
        let got = px[(y * 20 + x) as usize];
        assert!((got[0] * 255.0 - want).abs() < 0.01 && got[0] == got[1] && got[1] == got[2], "{cell}: {got:?} (grey {:.2}), want {want}", got[0] * 255.0);
    }
    assert_eq!(PatternSpace::from(ColorMode::Grayscale), PatternSpace::Mode(ColorMode::Grayscale));
}

#[test]
fn a_pattern_overlay_in_a_grayscale_document_paints_the_measured_greys() {
    // A document created in Grayscale, and one whose mode was set to Grayscale (the state Image ›
    // Mode › Grayscale leaves): the overlay is drawn from the same conversion. Untagged, and
    // tagged with the default grey profile, as Assign Profile leaves it.
    let mut wrong = Vec::new();
    for depth in [SampleType::U8, SampleType::U16] {
        for tagged in [false, true] {
            let mut d = overlay_doc(ColorMode::Grayscale, depth);
            if tagged {
                d.icc_profile = icc(craft_cms::Builtin::DotGain20Compat);
            }
            assert_eq!(PatternSpace::of(&d), PatternSpace::ProfileGray, "{depth:?} tagged {tagged}");
            for (cell, xy, want) in MEASURED {
                let got = at(&d, xy);
                if (got[0] * 255.0 - want).abs() > 2.0 {
                    wrong.push(format!("{depth:?} tagged {tagged} {cell}: grey {:.1}, want {want}", got[0] * 255.0));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The cases #14975 lists as not measured paint what they painted before: the luma formula's
/// greys, which a grey pattern holding them paints in the same document.
#[test]
fn unmeasured_grayscale_documents_keep_the_luma_greys() {
    let gamma22 = |depth| {
        let mut d = overlay_doc(ColorMode::Grayscale, depth);
        d.icc_profile = icc(craft_cms::Builtin::GrayGamma22);
        d
    };
    let cases = [
        ("32-bit Grayscale", overlay_doc(ColorMode::Grayscale, SampleType::F32)),
        ("Duotone", overlay_doc(ColorMode::Duotone, SampleType::U8)),
        ("Bitmap", overlay_doc(ColorMode::Bitmap, SampleType::U8)),
        ("Grayscale tagged Gray Gamma 2.2", gamma22(SampleType::U8)),
        ("16-bit Grayscale tagged Gray Gamma 2.2", gamma22(SampleType::U16)),
    ];
    let mut wrong = Vec::new();
    for (what, d) in cases {
        // The compositor's conversion, and the one Edit › Fill builds its tile in.
        let spaces = (PatternSpace::of(&d), PatternSpace::of_pixels(&d));
        if spaces != (PatternSpace::Mode(ColorMode::Grayscale), PatternSpace::Mode(ColorMode::Rgb)) {
            wrong.push(format!("{what}: {spaces:?}"));
        }
        let mut luma = overlay_doc_of(luma_quadrants(), d.mode, d.depth);
        luma.icc_profile = d.icc_profile.clone();
        for (cell, xy, level) in LUMA {
            let (got, want) = (at(&d, xy), at(&luma, xy));
            if got.iter().zip(want).any(|(g, w)| (g - w).abs() > 1e-4) {
                wrong.push(format!("{what} {cell}: {got:?}, the luma grey paints {want:?}"));
            }
            // In the 8- and 16-bit Grayscale documents that is the luma level itself.
            if d.mode == ColorMode::Grayscale && d.depth != SampleType::F32 && (got[0] * 255.0 - level).abs() > 1.0 {
                wrong.push(format!("{what} {cell}: grey {:.1}, want {level:.1}", got[0] * 255.0));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The default grey working space is the document's own profile: untagged, or tagged with bytes
/// that are no grey profile (the document's profile is then the default grey one).
#[test]
fn the_grey_working_space_is_read_from_the_documents_profile() {
    let gray = |profile: Option<Arc<Vec<u8>>>| {
        let mut d = Document::new("t", Size::new(4, 4), ColorMode::Grayscale, SampleType::U8);
        d.icc_profile = profile;
        PatternSpace::of(&d)
    };
    assert_eq!(gray(None), PatternSpace::ProfileGray);
    assert_eq!(gray(icc(craft_cms::Builtin::DotGain20Compat)), PatternSpace::ProfileGray);
    assert_eq!(gray(icc(craft_cms::Builtin::Srgb)), PatternSpace::ProfileGray, "an RGB profile is not the document's");
    assert_eq!(gray(Some(Arc::new(b"not a profile".to_vec()))), PatternSpace::ProfileGray, "unreadable bytes are not the document's profile");
    for other in [craft_cms::Builtin::GrayGamma22, craft_cms::Builtin::SGray] {
        assert_eq!(gray(icc(other)), PatternSpace::Mode(ColorMode::Grayscale), "{other:?}");
    }
    // The same allocation asked twice (the answer is cached by allocation).
    let tagged = icc(craft_cms::Builtin::GrayGamma22);
    assert_eq!((gray(tagged.clone()), gray(tagged)), (PatternSpace::Mode(ColorMode::Grayscale), PatternSpace::Mode(ColorMode::Grayscale)));
    // Other colour modes are themselves, and written pixels keep the pattern's own colours.
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        let d = Document::new("t", Size::new(4, 4), mode, SampleType::U8);
        assert_eq!((PatternSpace::of(&d), PatternSpace::of_pixels(&d)), (PatternSpace::Mode(mode), PatternSpace::Mode(ColorMode::Rgb)), "{mode:?}");
    }
    let d = Document::new("t", Size::new(4, 4), ColorMode::Grayscale, SampleType::U16);
    assert_eq!(PatternSpace::of_pixels(&d), PatternSpace::ProfileGray);
}

#[test]
fn rgb_lab_16_and_32_bit_documents_keep_the_patterns_colours() {
    let cells = [((5, 5), [1.0, 0.0, 0.0]), ((15, 5), [0.0, 1.0, 0.0]), ((5, 15), [0.0, 0.0, 1.0]), ((15, 15), [0.0, 0.0, 0.0])];
    for mode in [ColorMode::Rgb, ColorMode::Lab] {
        assert_eq!(pattern_rgba(&quadrants(), mode), pattern_rgba(&quadrants(), ColorMode::Rgb), "{mode:?}: the pattern's own colours");
    }
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let d = overlay_doc(ColorMode::Rgb, depth);
        for (xy, rgb) in cells {
            let got = at(&d, xy);
            assert!(got.iter().zip(rgb).all(|(g, w)| (g - w).abs() < 0.01), "RGB {depth:?} at {xy:?}: {got:?}, want {rgb:?}");
        }
    }
}
