use super::*;

#[test]
fn direct_rle_calls_reject_overflow_and_impossible_tile_layouts_without_panicking() {
    for (bpp, pixels) in [(2, usize::MAX), (usize::MAX, 2), (0, 1), (1, 0), (33, 1), (1, 4097), (1, usize::MAX)] {
        assert!(rle_decode(&[], bpp, pixels).is_err(), "bpp={bpp}, pixels={pixels}");
    }
}

#[test]
fn rle_requires_complete_nonzero_runs_in_each_plane() {
    for input in [&[][..], &[0], &[127], &[127, 0], &[127, 0, 0, 17], &[128, 0, 0], &[128, 0, 2, 17], &[1, 17]] {
        assert!(rle_decode(input, 1, 1).is_err(), "{input:?}");
    }
    assert!(rle_decode(&[0, 17], 2, 1).is_err(), "a complete first plane cannot hide a missing second one");
}

#[test]
fn rle_literal_and_repeated_planes_preserve_interleaving_through_the_largest_tile() {
    assert_eq!(rle_decode(&[253, 1, 2, 3, 2, 9], 2, 3).unwrap(), [1, 9, 2, 9, 3, 9]);
    assert_eq!(rle_decode(&[128, 0, 3, 1, 2, 3, 127, 0, 3, 9], 2, 3).unwrap(), [1, 9, 2, 9, 3, 9]);
    let encoded: Vec<u8> = (0..32u8).flat_map(|plane| [127, 16, 0, plane]).collect();
    let decoded = rle_decode(&encoded, 32, 4096).unwrap();
    assert_eq!(decoded.len(), 4096 * 32);
    let expected: Vec<u8> = (0..32).collect();
    for pixel in decoded.as_chunks::<32>().0 {
        assert_eq!(pixel, expected.as_slice());
    }
}
