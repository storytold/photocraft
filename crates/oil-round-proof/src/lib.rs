//! Exhaustive proof for the rounding implementation used by the oil filter.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

// Compile the same source that the oil filter uses, with this package's test profile.
#[cfg(test)]
#[path = "../../algo/src/oil/round.rs"]
mod round;

#[cfg(test)]
mod tests {
    use super::round::round_i32;
    // Keep the proof in the algorithm crate's dependent test graph.
    use photocraft_algo as _;
    use rayon::prelude::*;

    /// Every `f32` bit pattern in [-4096, 4096] (both signs, about 2.3e9 values, split across
    /// threads) rounds exactly like `f32::round`.
    #[test]
    fn rounding_matches_the_standard_library_for_every_f32_up_to_4096() {
        let last = 4096.0f32.to_bits();
        let bad = (0..=last >> 20)
            .into_par_iter()
            .filter_map(|hi| {
                let lo = hi << 20;
                let hi_end = (lo | 0xf_ffff).min(last);
                (lo..=hi_end).find_map(|bits| {
                    let v = f32::from_bits(bits);
                    [v, -v].into_iter().find(|v| round_i32(*v) != v.round() as i32)
                })
            })
            .min_by(|a, b| a.abs().total_cmp(&b.abs()));
        assert_eq!(bad, None, "round_i32 differs from f32::round");
    }
}
