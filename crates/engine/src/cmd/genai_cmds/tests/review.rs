//! Generative Fill after review (#10903): what a feathered edge sends, colour matching made
//! visible, the framing check and the diagnostic capture.

use super::*;

/// The decoded source PNG of the first request, as RGBA, with the area's top-left corner.
fn sent_source(f: &Fake, r: &Value) -> (Image, i32, i32) {
    let sent = f.sent.lock().unwrap();
    let img = codecs::decode(&sent[0].source_png).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    (img, r["bounds"][0].as_i64().unwrap() as i32, r["bounds"][1].as_i64().unwrap() as i32)
}

#[test]
fn a_feathered_edge_sends_none_of_the_replaced_colour() {
    // A red square on white, selected with a wide feather: even where the selection only just
    // touches a pixel, the picture sent carries no red, and the mask keeps the feather.
    let mut s = session(160, 160);
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60, "feather": 8})).unwrap();
    let f = install(&mut s, fake([0, 0, 255]));
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    let (img, x0, y0) = sent_source(&f, &r);
    let mask = codecs::decode(&f.sent.lock().unwrap()[0].mask_png).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    let mut partial = 0;
    for (i, (px, m)) in img.data().as_chunks::<4>().0.iter().zip(mask.data().as_chunks::<4>().0).enumerate() {
        let (x, y) = (x0 + (i % img.width() as usize) as i32, y0 + (i / img.width() as usize) as i32);
        let [r, g, b, _] = *px;
        assert!(i16::from(r) - i16::from(g) <= 2 && i16::from(r) - i16::from(b) <= 2, "({x},{y}) {px:?}: red was sent");
        if (1..255).contains(&m[3]) {
            partial += 1;
        }
    }
    assert!(partial > 0, "the mask keeps the feather");
}

#[test]
fn whether_colour_matching_ran_is_kept_for_each_variation() {
    // A solid colour where white was: too different around the selection to match.
    let (s, _, r) = filled(Some(1));
    assert_eq!(r["matching"][0]["colour"]["state"], "tooDifferent", "{r}");
    assert_eq!(r["matching"][0]["framing"]["state"], "same");
    let id = photocraft_doc::LayerId(r["variations"][0].as_u64().unwrap());
    let note = s.generative_variation_note(id).unwrap();
    assert!(note.colour.skipped());
    assert!(matches!(note.colour, ColourMatch::TooDifferent { limit, .. } if limit == 0.25));
    // A relit copy of the picture is matched.
    let mut s = session(160, 160);
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    install(&mut s, Fake { echo: Some(-40), ..fake([0, 0, 0]) });
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    assert_eq!(r["matching"][0]["colour"]["state"], "matched", "{r}");
    let id = photocraft_doc::LayerId(r["variations"][0].as_u64().unwrap());
    assert!(!s.generative_variation_note(id).unwrap().colour.skipped());
}

/// A 160² canvas, white with a black band down the left and a red one down the right, the
/// middle selected: the area sent is the whole canvas, square.
fn banded() -> Session {
    let mut s = session(160, 160);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 160})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("select.rect", json!({"x": 124, "y": 0, "width": 36, "height": 160})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    s
}

#[test]
fn a_result_with_added_sides_is_cut_to_its_centre_not_squeezed() {
    let mut s = banded();
    install(&mut s, Fake { echo: Some(0), reframe: Some(Reframe::Pad(40)), ..fake([0, 0, 0]) });
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    assert_eq!(r["matching"][0]["framing"], json!({"state": "centre", "returned": [240, 160], "sent": [160, 160]}), "{r}");
    // Lined up as sent, the picture around the selection matches, so its colours were matched.
    assert_eq!(r["matching"][0]["colour"]["state"], "matched", "{r}");
}

#[test]
fn a_stretched_result_is_scaled_back_when_the_whole_of_it_lines_up() {
    let mut s = banded();
    install(&mut s, Fake { echo: Some(0), reframe: Some(Reframe::Stretch(240, 160)), ..fake([0, 0, 0]) });
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    assert_eq!(r["matching"][0]["framing"]["state"], "scaled", "{r}");
}

#[test]
fn a_result_framed_differently_is_refused_not_stretched() {
    let mut s = banded();
    let layers = doc(&s).layers.len();
    install(&mut s, Fake { size: Some((1536, 1024)), ..fake([0, 0, 255]) });
    let e = s.execute(FILL, json!({"variations": 2})).unwrap_err().to_string();
    assert!(e.contains("1536×1024") && e.contains("160×160") && e.contains("not placed"), "{e}");
    assert_eq!(doc(&s).layers.len(), layers, "nothing landed");
    // One of the same shape is placed and the other dropped, like a failed variation.
    let mut s = banded();
    install(&mut s, Fake { size: Some((1024, 1024)), ..fake([0, 0, 255]) });
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    assert_eq!(r["matching"][0]["framing"]["state"], "same");
}
