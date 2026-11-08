//! Reference pictures (#10940): what the `references` param accepts, how each picture is made
//! ready to send, and that a fill, Generate More and the disclosure carry them.

use photocraft_paint::tile::b64_encode;

use super::super::references::{fit, param, prepare};
use super::*;

/// A `w`×`h` picture of one colour, encoded as `format`.
fn picture(w: u32, h: u32, rgb: [u8; 3], format: Format) -> Vec<u8> {
    let px: Vec<u8> = (0..w * h).flat_map(|_| rgb).collect();
    let img = Image::from_u8(w, h, ChannelLayout::Rgb, px).unwrap();
    codecs::encode(&img, format, &Default::default()).unwrap()
}

/// `bytes` as a `references` entry.
fn entry(bytes: &[u8]) -> Value {
    json!({"data": b64_encode(bytes)})
}

/// A 100×100 session with a selection and `f` installed.
fn selected(f: Fake) -> (Session, Arc<Fake>) {
    let mut s = session(100, 100);
    s.execute("select.rect", json!({"x": 30, "y": 30, "width": 40, "height": 40})).unwrap();
    let f = install(&mut s, f);
    (s, f)
}

/// The message of a bad-params error, or a panic naming what came back.
fn refused<T: std::fmt::Debug>(r: Result<T>) -> String {
    match r {
        Err(EngineError::BadParams { msg, .. }) => msg,
        other => panic!("expected a bad-params error, got {other:?}"),
    }
}

#[test]
fn no_references_is_an_empty_list() {
    assert!(param(&json!({}), FILL).unwrap().is_empty());
    assert!(param(&json!({"references": null}), FILL).unwrap().is_empty());
    assert!(param(&json!({"references": []}), FILL).unwrap().is_empty());
}

#[test]
fn references_keep_their_order_and_bytes() {
    let (a, b) = (picture(4, 4, [255, 0, 0], Format::Png), picture(3, 5, [0, 255, 0], Format::Jpeg));
    let got = param(&json!({"references": [entry(&a), entry(&b)]}), FILL).unwrap();
    assert_eq!(got, [a, b]);
}

#[test]
fn bad_references_are_refused_with_the_reason() {
    let one = entry(&picture(2, 2, [1, 2, 3], Format::Png));
    let cases = [
        (json!({"references": "x"}), "must be a list"),
        (json!({"references": [{"layer": 1}]}), "reference picture 1 must be"),
        (json!({"references": [one.clone(), {"data": 5}]}), "reference picture 2 must be"),
        (json!({"references": [{"data": "%%%"}]}), "not valid base64"),
        (json!({"references": [{"data": ""}]}), "reference picture 1 is empty"),
        (json!({"references": vec![one; MAX_REFERENCES + 1]}), "at most 4 reference pictures"),
    ];
    for (p, want) in cases {
        let msg = match param(&p, FILL) {
            Err(EngineError::BadParams { msg, .. }) => msg,
            other => panic!("{p}: expected a refusal, got {other:?}"),
        };
        assert!(msg.contains(want), "{p}: {msg}");
    }
}

#[test]
fn an_oversized_reference_is_refused_before_and_after_decoding() {
    // Too long as text: refused without decoding it.
    let long = "A".repeat((MAX_REFERENCE_BYTES / 3 + 2) * 4);
    let msg = refused(param(&json!({"references": [{"data": long}]}), FILL));
    assert!(msg.contains("larger than 10 MB"), "{msg}");
    // Exactly the limit is fine, one byte over is not.
    let at = vec![7u8; MAX_REFERENCE_BYTES];
    assert_eq!(param(&json!({"references": [entry(&at)]}), FILL).unwrap()[0].len(), MAX_REFERENCE_BYTES);
    let over = vec![7u8; MAX_REFERENCE_BYTES + 1];
    assert!(refused(param(&json!({"references": [entry(&over)]}), FILL)).contains("larger than 10 MB"));
}

#[test]
fn fit_scales_the_longest_side_down_and_never_up() {
    assert_eq!(fit(100, 50, 2048), (100, 50));
    assert_eq!(fit(4096, 2048, 2048), (2048, 1024));
    assert_eq!(fit(1000, 4000, 2048), (512, 2048));
    assert_eq!(fit(2048, 2048, 2048), (2048, 2048));
    assert_eq!(fit(9000, 1, 2048), (2048, 1));
    assert_eq!(fit(0, 0, 2048), (1, 1));
}

#[test]
fn a_reference_is_sent_as_a_png_of_its_pixels_only() {
    // A PNG carrying text (as a camera or editor might write a location into it) and a JPEG:
    // both come back as PNG with the same pixels and no metadata.
    let px: Vec<u8> = (0..6 * 4).flat_map(|_| [200, 40, 10]).collect();
    let mut meta = codecs::Metadata::default();
    meta.text.push(("Comment".into(), "51.5072 N, 0.1276 W".into()));
    let tagged = codecs::encode(&Image::from_u8(6, 4, ChannelLayout::Rgb, px).unwrap().with_meta(meta), Format::Png, &Default::default()).unwrap();
    assert!(!codecs::decode(&tagged).unwrap().meta.text.is_empty(), "the fixture carries text");
    for bytes in [tagged, picture(6, 4, [200, 40, 10], Format::Jpeg)] {
        let out = prepare(&bytes, 1, 2048).unwrap();
        assert_eq!(codecs::detect(&out), Some(Format::Png));
        let img = codecs::decode(&out).unwrap();
        assert!(img.meta.text.is_empty() && img.meta.exif.is_none() && img.meta.xmp.is_none());
        assert_eq!(img.dimensions(), (6, 4));
        let rgba = img.convert(ChannelLayout::Rgba, CodecSample::U8);
        for p in rgba.data().as_chunks::<4>().0 {
            assert!(p[0].abs_diff(200) <= 3 && p[1].abs_diff(40) <= 3 && p[2].abs_diff(10) <= 3 && p[3] == 255, "{p:?}");
        }
    }
}

#[test]
fn a_large_reference_is_scaled_to_the_request_side() {
    let out = prepare(&picture(300, 120, [0, 0, 255], Format::Png), 1, 100).unwrap();
    assert_eq!(codecs::decode(&out).unwrap().dimensions(), (100, 40));
}

#[test]
fn an_unreadable_reference_names_the_picture() {
    let e = prepare(b"not a picture", 3, 2048).unwrap_err().to_string();
    assert!(e.contains("reference picture 3") && e.contains("PNG, JPEG or WebP"), "{e}");
    let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;";
    assert!(prepare(gif, 1, 2048).unwrap_err().to_string().contains("reference picture 1"));
}

#[test]
fn a_fill_sends_its_references_after_the_rest_and_discloses_each() {
    let (mut s, f) = selected(fake([0, 0, 255]));
    let (a, b) = (picture(8, 8, [255, 0, 0], Format::Png), picture(5, 9, [0, 255, 0], Format::Jpeg));
    s.execute(FILL, json!({"prompt": "a boat", "variations": 1, "references": [entry(&a), entry(&b)]})).unwrap();
    let sent = f.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    let refs = &sent[0].references;
    assert_eq!(refs.len(), 2, "both pictures, in order");
    assert_eq!(codecs::decode(&refs[0]).unwrap().dimensions(), (8, 8));
    assert_eq!(codecs::decode(&refs[1]).unwrap().dimensions(), (5, 9));
    assert!(refs.iter().all(|r| codecs::detect(r) == Some(Format::Png)));
    drop(sent);
    let fills = s.genai_state.fills.len();
    assert_eq!(fills, 0, "the fill landed");
}

#[test]
fn the_disclosure_lists_each_reference() {
    let (mut s, _f) = selected(Fake { remote: true, ..fake([0, 0, 255]) });
    let a = picture(4, 4, [255, 0, 0], Format::Png);
    let r = s.execute(FILL, json!({"variations": 1, "references": [entry(&a), entry(&a)]}));
    assert!(r.is_err(), "a remote model waits for consent");
    let id = s.generative_fills()[0].id;
    let d = s.generative_fill_disclosure(id).unwrap();
    let inputs: Vec<_> = d.items.iter().map(|i| i.input).collect();
    assert_eq!(inputs, [SentInput::Source, SentInput::Mask, SentInput::Prompt, SentInput::Reference, SentInput::Reference]);
    assert_eq!(serde_json::to_value(SentInput::Reference).unwrap(), "reference");
}

#[test]
fn generate_more_sends_the_same_references_again() {
    let (mut s, f) = selected(fake([0, 0, 255]));
    let a = picture(8, 8, [255, 0, 0], Format::Png);
    s.execute(FILL, json!({"prompt": "a boat", "variations": 1, "references": [entry(&a)]})).unwrap();
    s.execute(MORE, json!({"count": 1})).unwrap();
    let sent = f.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1], sent[0], "the same request, references included");
    assert_eq!(sent[1].references.len(), 1);
}

#[test]
fn a_reference_that_cannot_be_read_fails_the_fill_and_sends_nothing() {
    let (mut s, f) = selected(fake([0, 0, 255]));
    let e = s.execute(FILL, json!({"variations": 1, "references": [{"data": b64_encode(b"not a picture")}]})).unwrap_err();
    assert!(e.to_string().contains("reference picture 1"), "{e}");
    assert_eq!(f.plans.load(Ordering::SeqCst), 0, "nothing was planned");
    assert!(f.sent.lock().unwrap().is_empty());
}

#[test]
fn a_bad_references_param_starts_no_fill() {
    let (mut s, f) = selected(fake([0, 0, 255]));
    let msg = refused(s.execute(FILL, json!({"references": "x"})));
    assert!(msg.contains("must be a list"), "{msg}");
    assert!(s.generative_fills().is_empty(), "no fill was started");
    assert_eq!(f.plans.load(Ordering::SeqCst), 0);
}
