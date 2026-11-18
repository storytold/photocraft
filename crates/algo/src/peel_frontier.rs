use photocraft_raster::{Cancelled, Interrupt};

#[derive(Clone, Copy)]
pub(crate) enum Connectivity {
    Four,
    Eight,
}

impl Connectivity {
    fn offsets(self) -> &'static [(isize, isize)] {
        match self {
            Self::Four => &[(-1, 0), (1, 0), (0, -1), (0, 1)],
            Self::Eight => &[(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)],
        }
    }

    fn neighbours(self, (w, h): (usize, usize), i: usize) -> impl Iterator<Item = usize> {
        let (x, y) = (i % w, i / w);
        self.offsets().iter().filter_map(move |&(dx, dy)| {
            let (nx, ny) = (x.checked_add_signed(dx)?, y.checked_add_signed(dy)?);
            (nx < w && ny < h).then(|| ny * w + nx)
        })
    }
}

/// The hole cells next to a known pixel, in the same row-major order as a full grid scan.
/// Only newly filled cells can expose more of the hole. A bit per cell avoids enqueuing the
/// same neighbour twice, including when a boundary cell was already queued but couldn't fill.
pub(crate) struct PeelFrontier {
    size: (usize, usize),
    connectivity: Connectivity,
    seen: Vec<u64>,
    cells: Vec<usize>,
    next: Vec<usize>,
}

impl PeelFrontier {
    /// `holes` gives the initial hole cells in row-major order.
    pub(crate) fn new(
        size: (usize, usize),
        connectivity: Connectivity,
        known: &[bool],
        holes: impl Iterator<Item = usize>,
        ctl: &Interrupt,
    ) -> Result<Self, Cancelled> {
        ctl.check()?;
        let mut front = Self { size, connectivity, seen: vec![0; known.len().div_ceil(64)], cells: Vec::new(), next: Vec::new() };
        for (count, i) in holes.enumerate() {
            if count.is_multiple_of(4096) {
                ctl.check()?;
            }
            if connectivity.neighbours(size, i).any(|j| known.get(j) == Some(&true)) && mark(&mut front.seen, i) {
                front.cells.push(i);
            }
        }
        Ok(front)
    }

    pub(crate) fn cells(&self) -> &[usize] {
        &self.cells
    }

    /// Call after writing a whole ring. Retain unresolved cells because a different match
    /// may reach them after the neighbouring cells fill.
    pub(crate) fn advance(&mut self, known: &[bool], is_hole: impl Fn(usize) -> bool, ctl: &Interrupt) -> Result<(), Cancelled> {
        self.next.clear();
        for (count, &i) in self.cells.iter().enumerate() {
            if count.is_multiple_of(4096) {
                ctl.check()?;
            }
            if known.get(i) != Some(&true) {
                self.next.push(i);
                continue;
            }
            for j in self.connectivity.neighbours(self.size, i) {
                if is_hole(j) && mark(&mut self.seen, j) {
                    self.next.push(j);
                }
            }
        }
        self.next.sort_unstable();
        std::mem::swap(&mut self.cells, &mut self.next);
        Ok(())
    }
}

/// Sets the bit and reports whether it was clear.
fn mark(seen: &mut [u64], i: usize) -> bool {
    let Some(word) = seen.get_mut(i / 64) else { return false };
    let mask = 1 << (i % 64);
    let first = *word & mask == 0;
    *word |= mask;
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan((w, h): (usize, usize), diagonals: bool, known: &[bool], holes: &[bool]) -> Vec<usize> {
        (0..holes.len())
            .filter(|&i| {
                holes[i]
                    && !known[i]
                    && (0..known.len()).any(|j| {
                        let dx = (i % w).abs_diff(j % w);
                        let dy = (i / w).abs_diff(j / w);
                        j / w < h && known[j] && if diagonals { dx.max(dy) == 1 } else { dx + dy == 1 }
                    })
            })
            .collect()
    }

    #[test]
    fn peel_frontier_matches_the_scan_for_both_neighbourhoods() {
        for grid in [
            ["..###...", ".#####..", "######..", "#####...", "###....#", ".#....##"],
            ["#X######", "X.######", "XX######", "XXXX####", "X.X##..#", "####..##"],
            ["########", "########", "########", "########", "########", "########"],
        ] {
            let size = (grid[0].len(), grid.len());
            let bytes: Vec<_> = grid.iter().flat_map(|row| row.bytes()).collect();
            let holes: Vec<_> = bytes.iter().map(|&c| c == b'#').collect();
            for (connectivity, diagonals) in [(Connectivity::Four, false), (Connectivity::Eight, true)] {
                let mut known: Vec<_> = bytes.iter().map(|&c| c == b'.').collect();
                let mut frontier = PeelFrontier::new(size, connectivity, &known, (0..holes.len()).filter(|&i| holes[i]), &Interrupt::NONE).unwrap();
                for ring in 0..=holes.len() {
                    let expected = scan(size, diagonals, &known, &holes);
                    assert_eq!(frontier.cells(), expected, "{grid:?}, diagonals={diagonals}, ring={ring}");
                    if expected.is_empty() {
                        break;
                    }
                    for &i in &expected {
                        known[i] = true;
                    }
                    frontier.advance(&known, |i| holes[i], &Interrupt::NONE).unwrap();
                }
                assert!(frontier.cells().is_empty());
            }
        }
    }

    #[test]
    fn peel_frontier_retries_boundary_cells_after_a_partial_fill() {
        let mut known = [true, false, false, false, false, true];
        let mut frontier = PeelFrontier::new((6, 1), Connectivity::Four, &known, 1..5, &Interrupt::NONE).unwrap();
        assert_eq!(frontier.cells(), [1, 4]);
        known[4] = true;
        frontier.advance(&known, |i| (1..5).contains(&i), &Interrupt::NONE).unwrap();
        assert_eq!(frontier.cells(), [1, 3]);
        assert_eq!(frontier.cells(), scan((6, 1), false, &known, &[false, true, true, true, true, false]));
    }

    #[test]
    fn peel_frontier_observes_cancellation() {
        let known = [true, false];
        let cancel = Interrupt::cancel_only(&|| true);
        assert!(PeelFrontier::new((2, 1), Connectivity::Four, &known, 1..2, &cancel).is_err());
        let mut frontier = PeelFrontier::new((2, 1), Connectivity::Four, &known, 1..2, &Interrupt::NONE).unwrap();
        assert!(frontier.advance(&known, |i| i == 1, &cancel).is_err());
    }
}
