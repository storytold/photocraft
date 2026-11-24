//! Render Video's Alpha Channel option for image sequences (#14163).

use photocraft_codecs::{ChannelLayout, Image, SampleType};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use super::*;

/// A 4 x 2 document at `depth` whose left half is opaque red and right half transparent; with
/// `background` the right half is the white Background instead, so the composite is opaque.
fn half_red(depth: u32, background: bool) -> Session {
    let mut s = crate::session();
    let bg = if background { "white" } else { "transparent" };
    s.execute("file.new", json!({"width": 4, "height": 2, "depth": depth, "background": bg})).unwrap();
    s.edit("red", |doc, active| {
        let id = (*active).ok_or(EngineError::NoDocument)?;
        let surf = crate::pixels_mut(doc.layer_mut(id).ok_or(EngineError::NoDocument)?)?;
        let red = photocraft_raster::from_rgba(&surf.format(), [1.0, 0.0, 0.0, 1.0]);
        surf.fill_rect(Rect::new(0, 0, 2, 2), &red);
        Ok(())
    })
    .unwrap();
    s
}

/// The one PNG frame Render Video writes for `params`.
fn render(s: &Session, params: Value) -> Image {
    let mut out = Vec::new();
    render_video_with(
        s,
        &params,
        |_, bytes| {
            out.push(bytes.to_vec());
            Ok(())
        },
        |_, _| true,
    )
    .unwrap();
    assert_eq!(out.len(), 1);
    photocraft_codecs::decode(&out[0]).unwrap()
}

/// The pixel at `x, y` in 0..255, with alpha when the file has it.
fn px(img: &Image, x: u32, y: u32) -> Vec<u32> {
    (0..img.layout().channels()).map(|c| (img.get(x, y, c) * 255.0).round() as u32).collect()
}

#[test]
fn each_choice_writes_the_measured_transparent_pixel_and_keeps_the_opaque_one() {
    let cases: [(Value, ChannelLayout, [u32; 4]); 6] = [
        (json!({}), ChannelLayout::Rgb, [0, 0, 0, 255]),
        (json!({"alpha": "none"}), ChannelLayout::Rgb, [0, 0, 0, 255]),
        (json!({"alpha": "straight"}), ChannelLayout::Rgba, [255, 255, 255, 0]),
        (json!({"alpha": "premultipliedBlack"}), ChannelLayout::Rgba, [0, 0, 0, 0]),
        (json!({"alpha": "premultipliedWhite"}), ChannelLayout::Rgba, [255, 255, 255, 0]),
        (json!({"alpha": "premultipliedColor", "matte": "#00ff00"}), ChannelLayout::Rgba, [0, 255, 0, 0]),
    ];
    for depth in [8, 16] {
        let s = half_red(depth, false);
        for (choice, layout, transparent) in &cases {
            let mut params = choice.clone();
            params["format"] = json!("png");
            let img = render(&s, params);
            assert_eq!(img.layout(), *layout, "{choice} at {depth} bit");
            let n = layout.channels();
            assert_eq!(px(&img, 3, 1), transparent[..n], "transparent pixel, {choice} at {depth} bit");
            assert_eq!(px(&img, 0, 0), [255, 0, 0, 255][..n], "opaque pixel, {choice} at {depth} bit");
        }
    }
}

/// A level only 16 bits can hold: 1000 / 65535 is between two 8-bit steps.
const FINE: f32 = 1000.0 / 65535.0;

#[test]
fn a_16_bit_frame_keeps_its_precision_and_partly_transparent_pixels() {
    let mut s = half_red(16, false);
    s.edit("soft", |doc, active| {
        let id = (*active).ok_or(EngineError::NoDocument)?;
        crate::pixels_mut(doc.layer_mut(id).ok_or(EngineError::NoDocument)?)?.write_pixel(2, 0, &[FINE, 0.25, 0.0, 0.5]);
        Ok(())
    })
    .unwrap();
    let near = |got: f32, want: f32, what: &str| assert!((got - want).abs() < 1.0 / 65535.0, "{what}: {got} vs {want}");
    for alpha in ["straight", "premultipliedBlack", "premultipliedWhite", "premultipliedColor"] {
        let img = render(&s, json!({"format": "png", "alpha": alpha, "matte": "#00ff00"}));
        assert_eq!(img.sample_type(), SampleType::U16, "{alpha}");
        // Not measured by the issue, so a partly transparent pixel keeps today's colour and alpha.
        near(img.get(2, 0, 0), FINE, alpha);
        near(img.get(2, 0, 3), 0.5, alpha);
    }
    let img = render(&s, json!({"format": "png", "alpha": "none"}));
    assert_eq!((img.layout(), img.sample_type()), (ChannelLayout::Rgb, SampleType::U16));
    near(img.get(2, 0, 0), FINE * 0.5, "none composites over black");
}

#[test]
fn an_opaque_document_writes_rgb_for_every_choice() {
    let s = half_red(8, true);
    for alpha in ["none", "straight", "premultipliedBlack", "premultipliedWhite", "premultipliedColor"] {
        let img = render(&s, json!({"format": "png", "alpha": alpha, "matte": "#00ff00"}));
        assert_eq!(img.layout(), ChannelLayout::Rgb, "{alpha}");
        assert_eq!(px(&img, 3, 1), [255, 255, 255], "{alpha}: the white Background shows");
    }
}

/// The encoder reads ".png" and "PNG" as PNG, so the choice applies to every spelling of it.
#[test]
fn every_spelling_of_png_takes_the_choice() {
    let s = half_red(8, false);
    for format in ["png", ".png", "PNG", ".PNG"] {
        let img = render(&s, json!({"format": format, "alpha": "premultipliedColor", "matte": "#00ff00"}));
        assert_eq!(px(&img, 3, 1), [0, 255, 0, 0], "{format}: the matte fills the transparent pixel");
        let img = render(&s, json!({"format": format}));
        assert_eq!(px(&img, 3, 1), [0, 0, 0], "{format}: no alpha by default");
    }
}

#[test]
fn formats_without_the_option_keep_their_transparency() {
    let s = half_red(8, false);
    let img = render(&s, json!({"format": "tif", "alpha": "premultipliedColor", "matte": "#00ff00"}));
    assert!(img.layout().channels() == 4, "TIFF keeps today's transparency: {:?}", img.layout());
    assert_eq!(px(&img, 3, 1)[3], 0);
}

#[test]
fn a_wrong_alpha_or_matte_is_an_error() {
    let s = half_red(8, false);
    for params in [
        json!({"format": "png", "alpha": "matte"}),
        json!({"format": "png", "alpha": true}),
        json!({"format": "png", "alpha": "premultipliedColor", "matte": "green"}),
        json!({"format": "png", "alpha": "premultipliedColor", "matte": [0, 2, 0]}),
        json!({"format": "png", "alpha": "premultipliedColor", "matte": [0, 1]}),
        json!({"format": "png", "alpha": "premultipliedColor", "matte": [0, "1", 0]}),
        json!({"format": "png", "alpha": "premultipliedColor", "matte": 7}),
        json!({"format": "png", "alpha": "premultipliedColor", "matte": "00ff00"}),
        json!({"format": "png", "alpha": "premultipliedColor", "matte": "#00ff0080"}),
        json!({"format": "gif", "alpha": "matte"}),
    ] {
        let r = render_video_with(&s, &params, |_, _| Ok(()), |_, _| true);
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{params}: {r:?}");
    }
}
