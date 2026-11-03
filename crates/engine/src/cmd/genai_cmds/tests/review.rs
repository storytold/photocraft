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
