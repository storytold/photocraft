//! Synthetic animated WebP containers; the oracle is image-webp's independent frame reader.
use std::io::Cursor;

use photocraft_codecs::{
    AnimationFrame, ChannelLayout, DecodeOptions, EncodeOptions, Format, Image, Limits, SampleType, decode, decode_webp_frames, encode, encode_webp_animation,
    webp_animation_info,
};

fn frame(rgba: [u8; 4], ms: u32) -> AnimationFrame {
    AnimationFrame { image: Image::from_raw(3, 2, ChannelLayout::Rgba, SampleType::U8, rgba.repeat(6)).unwrap(), delay_ms: ms }
}

#[test]
fn replacement_frames_preserve_alpha_timing_and_loops_for_an_independent_reader() {
    let frames = vec![frame([255, 0, 0, 255], 0), frame([0, 255, 0, 128], 75), frame([0, 0, 0, 0], 0xffffff)];
    for loops in [0, 7, u16::MAX] {
        let bytes = encode_webp_animation(&frames, loops, &EncodeOptions::default()).unwrap();
        let info = webp_animation_info(&bytes).unwrap().unwrap();
        assert_eq!((info.width, info.height, info.frames, info.loop_count), (3, 2, 3, loops));
        let mut oracle = image_webp::WebPDecoder::new(Cursor::new(&bytes)).unwrap();
        assert!(oracle.is_animated());
        assert_eq!(oracle.num_frames(), 3);
        let actual_loops = match oracle.loop_count() {
            image_webp::LoopCount::Forever => 0,
            image_webp::LoopCount::Times(n) => n.get(),
        };
        assert_eq!(actual_loops, loops);
        let mut buf = vec![0; oracle.output_buffer_size().unwrap()];
        for f in &frames {
            assert_eq!(oracle.read_frame(&mut buf).unwrap(), f.delay_ms);
            assert_eq!(buf, f.image.to_rgba8());
        }
        let decoded = decode_webp_frames(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(decoded.loop_count, Some(loops));
        assert_eq!(decoded.frames, frames);
        assert!(!decode(&bytes).unwrap().warnings.is_empty(), "flat decode still warns about omitted frames");
    }
}

#[test]
fn still_and_one_frame_animation_are_distinct() {
    let frames = vec![frame([10, 20, 30, 255], 123)];
    let still = encode(&frames[0].image, Format::WebP, &EncodeOptions::default()).unwrap();
    let decoded = decode_webp_frames(&still, &DecodeOptions::default()).unwrap();
    assert_eq!(decoded.loop_count, None);
    assert_eq!(decoded.frames[0].delay_ms, 0);
    let animated = encode_webp_animation(&frames, 3, &EncodeOptions::default()).unwrap();
    assert_eq!(decode_webp_frames(&animated, &DecodeOptions::default()).unwrap().frames[0].delay_ms, 123);
}

#[test]
fn metadata_and_lossy_frame_payloads_survive_muxing() {
    let mut frames = vec![frame([120, 90, 30, 128], 51), frame([20, 60, 100, 255], 93)];
    frames[0].image.icc = Some(b"synthetic ICC payload".to_vec());
    frames[0].image.meta.xmp = Some("<x:xmpmeta>synthetic</x:xmpmeta>".into());
    for lossless in [true, false] {
        let opts = EncodeOptions { webp_lossless: lossless, webp_quality: 95, ..Default::default() };
        let bytes = encode_webp_animation(&frames, 0, &opts).unwrap();
        let decoded = decode_webp_frames(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(decoded.frames[0].image.icc, frames[0].image.icc);
        assert_eq!(decoded.frames[1].image.meta.xmp, frames[0].image.meta.xmp);
        for (a, b) in decoded.frames.iter().zip(&frames) {
            assert_eq!(a.delay_ms, b.delay_ms);
            let actual = a.image.to_rgba8();
            let want = b.image.to_rgba8();
            for (actual, want) in actual.as_chunks::<4>().0.iter().zip(want.as_chunks::<4>().0) {
                assert_eq!(actual[3], want[3]);
                assert!(actual[..3].iter().zip(&want[..3]).all(|(a, b)| a.abs_diff(*b) <= 12));
            }
        }
        let no_meta = encode_webp_animation(&frames, 0, &EncodeOptions { embed_icc: false, embed_metadata: false, ..opts }).unwrap();
        let first = &decode_webp_frames(&no_meta, &DecodeOptions::default()).unwrap().frames[0].image;
        assert!(first.icc.is_none() && first.meta.xmp.is_none());
    }
}

#[test]
fn limits_and_bad_inputs_fail_without_partial_frames_or_panics() {
    let frames = vec![frame([200, 0, 0, 255], 10), frame([0, 0, 200, 255], 20)];
    let bytes = encode_webp_animation(&frames, 0, &EncodeOptions::default()).unwrap();
    for n in 0..bytes.len() {
        assert!(decode_webp_frames(&bytes[..n], &DecodeOptions::default()).is_err(), "truncation {n}");
    }
    for limits in
        [Limits { max_width: 2, ..Default::default() }, Limits { max_pixels: 5, ..Default::default() }, Limits { max_alloc: 119, ..Default::default() }]
    {
        assert!(decode_webp_frames(&bytes, &DecodeOptions { limits, ..Default::default() }).is_err());
    }
    assert!(encode_webp_animation(&[], 0, &EncodeOptions::default()).is_err());
    assert!(encode_webp_animation(&[frame([0; 4], 0x1000000)], 0, &EncodeOptions::default()).is_err());
    let mut wrong_size = frames.clone();
    wrong_size[1].image = Image::from_raw(1, 1, ChannelLayout::Rgba, SampleType::U8, vec![0; 4]).unwrap();
    assert!(encode_webp_animation(&wrong_size, 0, &EncodeOptions::default()).is_err());
    // Header-only mutations include ANMF/bitstream mismatches that the underlying ALPH
    // compositor assumes its caller checked. Every byte mutation must be an error or valid.
    for i in 0..bytes.len() {
        let mut bad = bytes.clone();
        bad[i] ^= 0xff;
        let _ = decode_webp_frames(&bad, &DecodeOptions { limits: Limits { max_alloc: 1 << 20, ..Default::default() }, ..Default::default() });
    }
}

#[test]
fn composited_offsets_blending_and_disposal_are_applied() {
    // Build a 4x2 animation from two 2x2 stills with ANMF rectangles and disposal. This
    // fixture exercises the decoder independently of our full-canvas replacement writer.
    fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
        out.extend(tag);
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        if !data.len().is_multiple_of(2) {
            out.push(0);
        }
    }
    let mut body = b"WEBP".to_vec();
    chunk(&mut body, b"VP8X", &[0x12, 0, 0, 0, 3, 0, 0, 1, 0, 0]);
    chunk(&mut body, b"ANIM", &[0; 6]);
    for (x, color, flags) in [(0, [255, 0, 0, 255], 3), (1, [0, 255, 0, 128], 0), (1, [0, 0, 255, 128], 0)] {
        let image = Image::from_raw(2, 2, ChannelLayout::Rgba, SampleType::U8, color.repeat(4)).unwrap();
        let still = encode(&image, Format::WebP, &EncodeOptions::default()).unwrap();
        let at = still.windows(4).position(|w| w == b"VP8L").unwrap();
        let mut data = vec![x, 0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 50, 0, 0, flags];
        data.extend(&still[at..]);
        chunk(&mut body, b"ANMF", &data);
    }
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((body.len() as u32).to_le_bytes());
    bytes.extend(body);
    let animation = decode_webp_frames(&bytes, &DecodeOptions::default()).unwrap();
    assert_eq!(&animation.frames[0].image.to_rgba8()[..4], &[255, 0, 0, 255]);
    let second = animation.frames[1].image.to_rgba8();
    assert_eq!(&second[..4], &[0, 0, 0, 0], "previous rectangle disposed to transparency");
    assert_eq!(&second[8..12], &[0, 255, 0, 128]);
    let third = animation.frames[2].image.to_rgba8();
    assert!((190..=193).contains(&third[11]), "overlapping semi-transparent frames blend");
    assert!(third[9] > 0 && third[10] > 0);
}

#[test]
fn metadata_orientation_is_applied_to_every_frame_and_can_be_kept() {
    let mut bytes = encode_webp_animation(&[frame([200, 40, 80, 255], 30), frame([20, 60, 90, 255], 60)], 0, &EncodeOptions::default()).unwrap();
    // An independently constructed little-endian TIFF EXIF orientation entry. Inject it
    // after encoding, because our encoder correctly normalizes orientation on output.
    let exif = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0];
    bytes[20] |= 8;
    bytes.extend(b"EXIF");
    bytes.extend((exif.len() as u32).to_le_bytes());
    bytes.extend(exif);
    let size = bytes.len() as u32 - 8;
    bytes[4..8].copy_from_slice(&size.to_le_bytes());
    let upright = decode_webp_frames(&bytes, &DecodeOptions::default()).unwrap();
    assert!(upright.frames.iter().all(|f| f.image.dimensions() == (2, 3)));
    assert!(upright.frames.iter().all(|f| photocraft_codecs::exif_orientation(f.image.meta.exif.as_deref().unwrap()) == 1));
    let kept = decode_webp_frames(&bytes, &DecodeOptions { keep_orientation: true, ..Default::default() }).unwrap();
    assert!(kept.frames.iter().all(|f| f.image.dimensions() == (3, 2)));
}

#[test]
fn frame_count_limit_is_checked_before_decoding_pixels() {
    let bytes = encode_webp_animation(&[frame([0, 0, 0, 255], 20)], 0, &EncodeOptions::default()).unwrap();
    let start = bytes.windows(4).position(|b| b == b"ANMF").unwrap();
    let mut huge = bytes[..start].to_vec();
    for _ in 0..=photocraft_codecs::MAX_WEBP_FRAMES {
        huge.extend(&bytes[start..]);
    }
    let size = huge.len() as u32 - 8;
    huge[4..8].copy_from_slice(&size.to_le_bytes());
    assert!(matches!(webp_animation_info(&huge), Err(photocraft_codecs::CodecError::LimitExceeded(_))));
}
