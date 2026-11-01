//! The dependency's 4:2:2 chroma mode correction, checked against an independent decoder.
#[test]
fn ten_bit_422_matches_the_reference_sample_hash() {
    let stream = include_bytes!("fixtures/intra-422-10.hevc");
    let starts: Vec<_> = stream.windows(3).enumerate().filter_map(|(i, b)| (b == [0, 0, 1]).then_some(i)).collect();
    let mut sets = Vec::new();
    let mut slices = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(stream.len());
        let mut unit = &stream[start + 3..end];
        while unit.last() == Some(&0) {
            unit = &unit[..unit.len() - 1];
        }
        match unit.first().map(|b| (b >> 1) & 63) {
            Some(32..=34) => sets.push(unit),
            Some(0..=31) => slices.push(unit),
            _ => {}
        }
    }
    let frame = heic_rs::hevc::decode_still(&sets, &slices).unwrap();
    assert_eq!((frame.width, frame.height, frame.bit_depth), (128, 128, 10));
    assert_eq!(frame.chroma, heic_rs::hevc::ChromaFormat::Yuv422);
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for (plane, width, stride) in [(&frame.y, 128, frame.y_stride), (&frame.cb, 64, frame.c_stride), (&frame.cr, 64, frame.c_stride)] {
        for row in plane.chunks(stride as usize).take(128) {
            for sample in &row[..width] {
                for byte in sample.to_le_bytes() {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
    }
    assert_eq!(hash, 0xc932_4cae_793f_082f, "all coded samples must match the FFmpeg oracle");
}
