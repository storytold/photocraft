//! Pixels keep their colours in a document of another profile: a placed file converts from its
//! embedded profile, and pasted pixels from the profile they were copied in, into the document's.
use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_engine::{Session, file_cmds, smart_cmds};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use serde_json::json;

const SIDE: u32 = 8;
/// A muted red in Adobe RGB, from a Rubens portrait that pasted gray.
const ADOBE: [u8; 3] = [145, 85, 75];

/// `ADOBE` in sRGB (relative colorimetric, black point compensation).
fn in_srgb() -> [u8; 3] {
    let t = Transform::new(Builtin::AdobeRgbCompat.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).unwrap();
    let mut out = [0.0f32; 3];
    t.convert_f32(&ADOBE.map(|v| f32::from(v) / 255.0), 3, &mut out, 3, false);
    out.map(|v| (v * 255.0).round() as u8)
}

fn adobe_png() -> Vec<u8> {
    let px = [ADOBE[0], ADOBE[1], ADOBE[2], 255].repeat((SIDE * SIDE) as usize);
    let mut img = Image::from_raw(SIDE, SIDE, ChannelLayout::Rgba, SampleType::U8, px).unwrap();
    img.icc = Some(Builtin::AdobeRgbCompat.profile().to_bytes().to_vec());
    photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap()
}

fn rgb8(px: [f32; 4]) -> [u8; 3] {
    [px[0], px[1], px[2]].map(|v| (v * 255.0).round() as u8)
}

fn centre(s: &Session) -> [u8; 3] {
    let buf = photocraft_compose::flatten(&s.active().unwrap().doc);
    rgb8(buf.px[(SIDE * SIDE / 2 + SIDE / 2) as usize])
}

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1)
}

fn new_doc(s: &mut Session, profile: &str) {
    s.execute("file.new", json!({"width": SIDE, "height": SIDE, "profile": profile})).unwrap();
}

#[test]
fn a_placed_file_converts_from_its_embedded_profile_and_keeps_it_when_rendered_again() {
    let want = in_srgb();
    assert!(!close(want, ADOBE), "the test colour must differ between the profiles");
    let mut s = Session::new();
    new_doc(&mut s, "srgb");
    file_cmds::place_bytes(&mut s, "rubens.png", adobe_png(), None, &json!({})).unwrap();
    assert!(close(centre(&s), want), "placed {:?}, want {want:?}", centre(&s));
    // Free Transform and Edit Contents render the smart object again from its source.
    let d = s.active().unwrap();
    let layer = d.doc.layer(d.active_layer.unwrap()).unwrap();
    let LayerContent::Smart(so) = &layer.content else { panic!("not a smart object") };
    let again = smart_cmds::render(&d.doc, so).unwrap().unwrap();
    let px = again.pixel(SIDE as i32 / 2, SIDE as i32 / 2);
    assert!(close(rgb8([px[0], px[1], px[2], 1.0]), want), "rendered again {px:?}, want {want:?}");
}

#[test]
fn pasted_pixels_convert_from_the_profile_they_were_copied_in() {
    let mut s = Session::new();
    let mut doc = Document::new("rubens", Size::new(SIDE, SIDE), ColorMode::Rgb, photocraft_color::SampleType::U8);
    doc.icc_profile = Some(Builtin::AdobeRgbCompat.profile().to_bytes());
    let r = Rect::new(0, 0, SIDE as i32, SIDE as i32);
    let px = [ADOBE[0], ADOBE[1], ADOBE[2], 255].repeat((SIDE * SIDE) as usize);
    doc.layers.push(Layer::new("art", LayerContent::Raster(Surface::from_interleaved(PixelFormat::RGBA8, r, &px))));
    s.add_document(doc, None);
    s.execute("edit.copy", json!({})).unwrap();

    new_doc(&mut s, "srgb");
    s.execute("edit.paste", json!({})).unwrap();
    assert!(close(centre(&s), in_srgb()), "pasted {:?}, want {:?}", centre(&s), in_srgb());

    // Into a targeted channel, the luminosity of the converted colours.
    new_doc(&mut s, "srgb");
    s.execute("channel.target", json!({"channel": "red"})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let gray = photocraft_color::convert::rgb_to_gray(in_srgb().map(f32::from));
    assert!((f32::from(centre(&s)[0]) - gray).abs() <= 1.0, "red channel {:?}, want {gray}", centre(&s));

    // Into a document of the same profile the values stay as they are.
    new_doc(&mut s, "adobe-rgb-compat");
    s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(centre(&s), ADOBE);

    // New from Clipboard keeps the profile the pixels are in.
    s.execute("file.newFromClipboard", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.icc_profile, Some(Builtin::AdobeRgbCompat.profile().to_bytes()));
}
