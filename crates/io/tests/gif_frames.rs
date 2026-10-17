//! `import_gif_frames`: each GIF frame as a single-layer document, in order, with its delay.

use photocraft_codecs::web::{GifFrame, encode_gif_animated};
use photocraft_io::{IoError, import_gif_frames};

fn gif(colours: &[([u8; 3], u16)]) -> Vec<u8> {
    let frames: Vec<GifFrame> = colours.iter().map(|&(c, delay_cs)| GifFrame { indices: vec![0; 6], palette: vec![c], transparent: None, delay_cs }).collect();
    encode_gif_animated(3, 2, &frames, true).unwrap()
}

#[test]
fn frames_arrive_in_order_as_documents_with_their_delays() {
    let bytes = gif(&[([255, 0, 0], 3), ([0, 0, 255], 7)]);
    let mut seen = Vec::new();
    let n = import_gif_frames("a.gif", &bytes, &mut |doc, delay_ms| {
        seen.push((doc.layers.len(), doc.bounds().width(), doc.bounds().height(), delay_ms));
        Ok(())
    })
    .unwrap();
    assert_eq!(n, 2);
    assert_eq!(seen, [(1, 3, 2, 30), (1, 3, 2, 70)]);
}

#[test]
fn an_error_from_the_callback_stops_the_import() {
    let bytes = gif(&[([255, 0, 0], 3), ([0, 0, 255], 7), ([0, 255, 0], 7)]);
    let mut calls = 0;
    let r = import_gif_frames("a.gif", &bytes, &mut |_, _| {
        calls += 1;
        Err(IoError::Unsupported("stop".into()))
    });
    assert!(matches!(r, Err(IoError::Unsupported(_))));
    assert_eq!(calls, 1);
}

#[test]
fn garbage_is_an_error() {
    assert!(import_gif_frames("a.gif", b"not a gif", &mut |_, _| Ok(())).is_err());
}

/// #3001: opening an animated GIF keeps each frame's own delay and shows one frame's layer at a time.
#[test]
fn opening_an_animated_gif_keeps_every_delay() {
    let bytes = gif(&[([255, 0, 0], 3), ([0, 0, 255], 30), ([0, 255, 0], 7)]);
    let r = photocraft_io::import("a.gif", &bytes).unwrap();
    let a = r.document.frame_animation.as_ref().unwrap();
    assert_eq!(a.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), [30, 300, 70]);
    assert_eq!(r.document.layers.iter().map(|l| l.visible).collect::<Vec<_>>(), [true, false, false]);
}

/// The delays of an animated GIF's frames, in milliseconds, read back through the importer.
fn delays_of(bytes: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    import_gif_frames("x.gif", bytes, &mut |_, ms| {
        out.push(ms);
        Ok(())
    })
    .unwrap();
    out
}

/// Open an animated GIF, save it, and open the result: every frame and its delay survive.
#[test]
fn saving_an_opened_animated_gif_round_trips_frames_and_delays() {
    let bytes = gif(&[([255, 0, 0], 3), ([0, 0, 255], 30), ([0, 255, 0], 7)]);
    let opened = photocraft_io::import("a.gif", &bytes).unwrap().document;
    let saved = photocraft_io::export(&opened, "a.gif", &Default::default()).unwrap().bytes;
    assert_eq!(delays_of(&saved), [30, 300, 70]);
    let again = photocraft_io::import("a.gif", &saved).unwrap().document;
    assert_eq!(again.layers.len(), 3);
    // Each frame keeps its own colour: red, blue, green.
    let colours: Vec<[u8; 3]> = (0..3)
        .map(|i| {
            let mut d = again.clone();
            d.frame_animation.as_ref().unwrap().apply(i, &mut d.layers);
            let px = photocraft_compose::thumbnail(&d, 8).pixels;
            [px[0], px[1], px[2]]
        })
        .collect();
    assert_eq!(colours, [[255, 0, 0], [0, 0, 255], [0, 255, 0]]);
}

/// Editing the animation changes what is saved: a changed delay and a reordered frame.
#[test]
fn edits_to_the_animation_reach_the_saved_gif() {
    let bytes = gif(&[([255, 0, 0], 3), ([0, 0, 255], 30)]);
    let mut d = photocraft_io::import("a.gif", &bytes).unwrap().document;
    let a = d.frame_animation.as_mut().unwrap();
    a.frames[0].delay_ms = 500;
    a.frames.reverse();
    let saved = photocraft_io::export(&d, "a.gif", &Default::default()).unwrap().bytes;
    assert_eq!(delays_of(&saved), [300, 500]);
}

/// The loop count is written as the NETSCAPE extension: none for once, 0 for forever, repeats otherwise.
#[test]
fn the_loop_count_is_written_to_the_file() {
    use photocraft_doc::LoopCount;
    let bytes = gif(&[([255, 0, 0], 3), ([0, 0, 255], 30)]);
    let ext = |loops: LoopCount| {
        let mut d = photocraft_io::import("a.gif", &bytes).unwrap().document;
        d.frame_animation.as_mut().unwrap().loops = loops;
        let saved = photocraft_io::export(&d, "a.gif", &Default::default()).unwrap().bytes;
        saved.windows(16).position(|w| w.starts_with(b"NETSCAPE2.0")).map(|at| (saved[at + 13], saved[at + 14]))
    };
    assert_eq!(ext(LoopCount::Once), None);
    assert_eq!(ext(LoopCount::Forever), Some((0, 0)));
    assert_eq!(ext(LoopCount::Times(3)), Some((2, 0)));
}

/// A still document saves a GIF as before, with no animation.
#[test]
fn a_document_without_an_animation_saves_a_still_gif() {
    let bytes = gif(&[([255, 0, 0], 3), ([0, 0, 255], 30)]);
    let mut d = photocraft_io::import("a.gif", &bytes).unwrap().document;
    d.frame_animation = None;
    let saved = photocraft_io::export(&d, "a.gif", &Default::default()).unwrap().bytes;
    assert_eq!(delays_of(&saved).len(), 1);
}
