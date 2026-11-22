//! The scratch read-error generation as the UI's caches key on it.
//!
//! A tile whose scratch bytes fail to read shows a zero placeholder through `&self`, and its real pixels again once
//! a reload succeeds, without moving its surface's generation or its document's revision (the `PlainData` comment on
//! `photocraft_raster::Tile`). Both transitions move `spill::read_error_generation`, so every picture or derived
//! value the UI keeps of document or preset pixels keys on [`generation`] too; otherwise a thumbnail drawn during a
//! read error keeps its zeros after the tile recovers.

/// The scratch read-error generation now: it moves when a tile fails to read and when it reads again.
pub(crate) fn generation() -> u64 {
    let real = photocraft_raster::spill::read_error_generation();
    #[cfg(test)]
    let real = real.wrapping_add(tests::BUMPS.with(std::cell::Cell::get));
    real
}

/// [`generation`] folded with `revision`: one number for a cache keyed by a single `u64` that must follow both.
pub(crate) fn with(revision: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    (revision, generation()).hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;

    thread_local! {
        /// Read-error transitions simulated on this test's thread, so a test sees one without setting up the
        /// process-wide scratch file, and tests on other threads never do.
        pub(super) static BUMPS: Cell<u64> = const { Cell::new(0) };
    }

    /// Simulates one read-error transition (a tile failing, or one recovering) as the caches see it.
    pub(crate) fn simulate_read_error_transition() {
        BUMPS.with(|bumps| bumps.set(bumps.get().wrapping_add(1)));
    }

    #[test]
    fn a_simulated_transition_moves_the_generation_and_the_folded_key() {
        let (generation, folded) = (super::generation(), super::with(7));
        assert_eq!(super::with(7), folded, "the same state folds to the same key");
        simulate_read_error_transition();
        assert_ne!(super::generation(), generation);
        assert_ne!(super::with(7), folded);
    }
}
