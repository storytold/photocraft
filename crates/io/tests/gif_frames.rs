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
    a.frame_mut(0).unwrap().delay_ms = 500;
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

#[test]
fn a_gif_with_too_many_frames_opens_the_first_ones_with_a_warning() {
    use photocraft_io::MAX_GIF_LAYER_FRAMES;
    let frames: Vec<GifFrame> =
        (0..MAX_GIF_LAYER_FRAMES + 5).map(|_| GifFrame { indices: vec![0], palette: vec![[9, 9, 9]], transparent: None, delay_cs: 2 }).collect();
    let bytes = encode_gif_animated(1, 1, &frames, true).unwrap();
    let r = photocraft_io::import("many.gif", &bytes).unwrap();
    assert_eq!(r.document.layers.len(), MAX_GIF_LAYER_FRAMES);
    assert_eq!(r.document.frame_animation.as_ref().unwrap().frames.len(), MAX_GIF_LAYER_FRAMES);
    assert!(r.warnings.iter().any(|w| w.contains(&format!("first {MAX_GIF_LAYER_FRAMES} frames of the GIF"))), "{:?}", r.warnings);
}

#[test]
fn an_animated_export_can_be_cancelled_between_frames_and_is_bounded() {
    use photocraft_doc::{AnimFrame, ColorMode, Document, FrameAnimation, LoopCount, SampleType};
    use photocraft_io::gif_anim::{MAX_STAGED_BYTES, export_stepped};
    let animated = |side: u32, n: usize| {
        let mut d = Document::new("a", photocraft_geom::Size { width: side, height: side }, ColorMode::Rgb, SampleType::U8);
        d.frame_animation = Some(FrameAnimation { frames: vec![AnimFrame::default().into(); n], current: 0, loops: LoopCount::Forever });
        d
    };
    // Stopping at the second step stops the export with `Cancelled`, having rendered one frame.
    let doc = animated(4, 5);
    let mut steps = Vec::new();
    let r = export_stepped(&doc, &mut |done, total| {
        steps.push((done, total));
        done < 1
    });
    assert!(matches!(r, Err(IoError::Cancelled)));
    assert_eq!(steps, [(0, 5), (1, 5)]);
    // A frame count that can't be held at once is refused before anything is rendered.
    let side = 8192u32;
    let frames = (MAX_STAGED_BYTES / (u64::from(side) * u64::from(side))) as usize + 1;
    let r = export_stepped(&animated(side, frames), &mut |_, _| panic!("must be refused before rendering"));
    assert!(matches!(r, Err(IoError::Unsupported(_))));
}

#[test]
fn unnormalised_loop_counts_export_as_they_are_defined() {
    use photocraft_doc::{AnimFrame, ColorMode, Document, FrameAnimation, LoopCount, SampleType};
    use photocraft_io::gif_anim::export_stepped;
    let bytes = |loops: LoopCount| {
        let mut d = Document::new("a", photocraft_geom::Size { width: 2, height: 2 }, ColorMode::Rgb, SampleType::U8);
        d.frame_animation = Some(FrameAnimation { frames: vec![AnimFrame::default().into(); 2], current: 0, loops });
        export_stepped(&d, &mut |_, _| true).unwrap().unwrap()
    };
    let repeats = |g: &[u8]| g.windows(11).position(|w| w == b"NETSCAPE2.0").map(|i| u16::from_le_bytes([g[i + 13], g[i + 14]]));
    assert_eq!(repeats(&bytes(LoopCount::Times(1))), None, "Times(1) is once: no loop extension");
    assert_eq!(repeats(&bytes(LoopCount::Times(0))), Some(0), "Times(0) is forever");
    assert_eq!(repeats(&bytes(LoopCount::Times(3))), Some(2), "three plays are two repeats");
}

#[test]
fn an_opened_gif_keeps_its_own_loop_count_and_saves_it_back() {
    use photocraft_codecs::web::{GifLoops, encode_gif_animated_loops};
    use photocraft_doc::LoopCount;
    let frames: Vec<GifFrame> =
        [([255, 0, 0], 3), ([0, 0, 255], 5)].iter().map(|&(c, d)| GifFrame { indices: vec![0; 6], palette: vec![c], transparent: None, delay_cs: d }).collect();
    let loops_of = |l: GifLoops| {
        let bytes = encode_gif_animated_loops(3, 2, &frames, l).unwrap();
        let r = photocraft_io::import("a.gif", &bytes).unwrap();
        // Open and save again without touching anything.
        let again = photocraft_io::export(&r.document, "a.gif", &Default::default()).unwrap().bytes;
        let back = photocraft_io::import("a.gif", &again).unwrap();
        (r.document.frame_animation.unwrap().loops, back.document.frame_animation.unwrap().loops)
    };
    assert_eq!(loops_of(GifLoops::Forever), (LoopCount::Forever, LoopCount::Forever));
    assert_eq!(loops_of(GifLoops::Once), (LoopCount::Once, LoopCount::Once), "a GIF with no loop extension plays once");
    assert_eq!(loops_of(GifLoops::Times(4)), (LoopCount::Times(4), LoopCount::Times(4)));
}

#[test]
fn a_truncated_gif_keeps_the_loop_count_it_declared_or_plays_once() {
    use photocraft_codecs::web::{GifLoops, encode_gif_animated_loops};
    use photocraft_doc::LoopCount;
    let frames: Vec<GifFrame> = [([255, 0, 0], 3), ([0, 0, 255], 5), ([0, 255, 0], 7)]
        .iter()
        .map(|&(c, d)| GifFrame { indices: vec![0; 6], palette: vec![c], transparent: None, delay_cs: d })
        .collect();
    let loops_of = |l: GifLoops| {
        let mut bytes = encode_gif_animated_loops(3, 2, &frames, l).unwrap();
        bytes.pop(); // lose the trailer: the blocks now end early
        photocraft_io::import("a.gif", &bytes).unwrap().document.frame_animation.unwrap().loops
    };
    assert_eq!(loops_of(GifLoops::Forever), LoopCount::Forever, "the extension came first, so it is still read");
    assert_eq!(loops_of(GifLoops::Times(4)), LoopCount::Times(4));
    assert_eq!(loops_of(GifLoops::Once), LoopCount::Once, "no extension: plays once, as an untruncated one does");
}

/// A 2×2 document of one colour, tagged with `icc`, with a two-frame animation.
fn tagged(mode: photocraft_doc::ColorMode, icc: Vec<u8>, colour: &[f32]) -> photocraft_doc::Document {
    use photocraft_doc::{AnimFrame, Document, FrameAnimation, Layer, LoopCount, SampleType};
    let mut d = Document::new("t", photocraft_geom::Size::new(2, 2), mode, SampleType::U8);
    let mut l = Layer::raster("A", d.pixel_format());
    l.surface_mut().unwrap().fill_rect(d.bounds(), colour);
    d.layers = vec![l];
    d.icc_profile = Some(std::sync::Arc::new(icc));
    d.frame_animation = Some(FrameAnimation { frames: vec![AnimFrame::default().into(); 2], current: 0, loops: LoopCount::Forever });
    d
}

/// The first pixel of the first frame a GIF holds, as the sRGB bytes it opens with.
fn first_pixel(gif: &[u8]) -> [u8; 3] {
    let mut out = None;
    import_gif_frames("a.gif", gif, &mut |d, _| {
        out.get_or_insert_with(|| {
            let px = photocraft_compose::thumbnail(&d, 2).pixels;
            [px[0], px[1], px[2]]
        });
        Ok(())
    })
    .unwrap();
    out.unwrap()
}

fn assert_close(a: [u8; 3], b: [u8; 3], what: &str) {
    assert!(a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 2), "{what}: {a:?} vs {b:?}");
}

/// An animated export converts each frame to sRGB like a still one: a tagged document keeps its colours.
#[test]
fn an_animated_export_converts_a_tagged_document_to_srgb() {
    use photocraft_cms::Builtin;
    use photocraft_doc::ColorMode;
    let d = tagged(ColorMode::Rgb, Builtin::ProPhotoCompat.profile().to_bytes().to_vec(), &[0.4, 0.6, 0.2, 1.0]);
    let animated = photocraft_io::export(&d, "a.gif", &Default::default()).unwrap().bytes;
    let mut still = d.clone();
    still.frame_animation = None;
    let still = photocraft_io::export(&still, "a.gif", &Default::default()).unwrap().bytes;
    let got = first_pixel(&animated);
    assert_close(got, first_pixel(&still), "against the still export");
    // The working-space values [0.4, 0.6, 0.2] are about [0.32, 0.70, 0.02] in sRGB.
    assert_close(got, [82, 178, 5], "in sRGB");
}

#[test]
fn an_animated_export_converts_cmyk_through_the_documents_profile() {
    use photocraft_cms::synth::{CmykParams, cmyk_profile};
    use photocraft_doc::ColorMode;
    let profile = cmyk_profile(&CmykParams { tvi: [0.26, 0.26, 0.26, 0.3], grid_a2b: 5, grid_b2a: 9, ..Default::default() }).to_bytes().to_vec();
    let d = tagged(ColorMode::Cmyk, profile, &[0.6, 0.2, 0.3, 0.1, 1.0]);
    let animated = photocraft_io::export(&d, "a.gif", &Default::default()).unwrap().bytes;
    let mut still = d.clone();
    still.frame_animation = None;
    let still = photocraft_io::export(&still, "a.gif", &Default::default()).unwrap().bytes;
    assert_close(first_pixel(&animated), first_pixel(&still), "the document's CMYK profile, not the built-in one");
}

/// The export's memory is bounded by what it holds at once, frames kept and the one being rendered,
/// and the compression at the end can be stopped as well.
#[test]
fn an_export_that_cannot_be_held_at_once_is_refused_and_compression_can_be_cancelled() {
    use photocraft_doc::{AnimFrame, ColorMode, Document, FrameAnimation, LoopCount, SampleType};
    use photocraft_io::gif_anim::{MAX_STAGED_BYTES, WORKING_BYTES_PER_PIXEL, export_stepped};
    let animated = |side: u32, n: usize| {
        let mut d = Document::new("a", photocraft_geom::Size { width: side, height: side }, ColorMode::Rgb, SampleType::U8);
        d.frame_animation = Some(FrameAnimation { frames: vec![AnimFrame::default().into(); n], current: 0, loops: LoopCount::Forever });
        d
    };
    // One frame of 4500 × 4500 is within the old frame limit, but rendering it takes more than the budget.
    const { assert!(4500u64 * 4500 * WORKING_BYTES_PER_PIXEL > MAX_STAGED_BYTES) };
    let r = export_stepped(&animated(4500, 1), &mut |_, _| panic!("must be refused before rendering"));
    assert!(matches!(r, Err(IoError::Unsupported(_))));
    // Stopping once every frame is rendered, while they compress, cancels too.
    let mut calls = 0;
    let r = export_stepped(&animated(4, 3), &mut |done, total| {
        calls += 1;
        !(calls > 4 && done == total)
    });
    assert!(matches!(r, Err(IoError::Cancelled)));
}
