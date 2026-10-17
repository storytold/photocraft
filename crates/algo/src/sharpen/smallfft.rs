//! A small in-place mixed-radix FFT for the browser build.
//!
//! `rustfft` brings its whole planner (every radix, Bluestein, Rader) into the binary, and the
//! web build has a hard size limit, so the wasm build transforms with this instead. It handles
//! the lengths the deconvolution asks for (factors 2, 3 and 5 only) with radix-4, 2, 3 and 5
//! butterflies. Native builds use `rustfft` (SIMD); this module is also compiled there so tests
//! can check it against `rustfft` and run the whole deconvolution on it.

use rustfft::num_complex::Complex;

type C = Complex<f32>;

/// One pass of butterflies: `radix` sub-transforms of length `m` become one of `radix × m`.
struct Stage {
    radix: usize,
    m: usize,
    /// `exp(-2πi q k / (radix × m))` for `k < m`, `q` in `1..radix`, grouped by `k`.
    twiddle: Vec<C>,
}

pub(super) struct SmallFft {
    n: usize,
    /// Swaps that put the input in the order the first stage reads it.
    swaps: Vec<(u32, u32)>,
    stages: Vec<Stage>,
}

impl SmallFft {
    /// A transform of length `n`, or `None` when `n` has a prime factor other than 2, 3 and 5.
    pub(super) fn new(n: usize) -> Option<Self> {
        let n = n.max(1);
        // Radices from the first pass to the last: fours, a leftover two, threes, fives.
        let mut radices = Vec::new();
        let mut rest = n;
        while rest.is_multiple_of(4) {
            radices.push(4);
            rest /= 4;
        }
        for p in [2, 3, 5] {
            while rest.is_multiple_of(p) {
                radices.push(p);
                rest /= p;
            }
        }
        if rest != 1 || u32::try_from(n).is_err() {
            return None;
        }
        // The last pass splits the input by `index mod radix`, the one before splits each of
        // those the same way, and so on down: `order[pos]` is the input index that lands at `pos`.
        let mut groups = vec![(0usize, 1usize)];
        for &p in radices.iter().rev() {
            groups = groups.iter().flat_map(|&(start, stride)| (0..p).map(move |q| (start + q * stride, stride * p))).collect();
        }
        let order: Vec<usize> = groups.iter().map(|g| g.0).collect();
        let mut swaps = Vec::new();
        for (pos, &from) in order.iter().enumerate() {
            let mut j = from;
            while j < pos {
                j = order.get(j).copied().unwrap_or(pos);
            }
            if j != pos {
                swaps.push((pos as u32, j as u32));
            }
        }
        let mut stages = Vec::new();
        let mut m = 1;
        for &radix in &radices {
            let len = (radix * m) as f64;
            let twiddle = (0..m)
                .flat_map(|k| {
                    (1..radix).map(move |q| {
                        let a = -2.0 * std::f64::consts::PI * (q * k) as f64 / len;
                        C::new(a.cos() as f32, a.sin() as f32)
                    })
                })
                .collect();
            stages.push(Stage { radix, m, twiddle });
            m *= radix;
        }
        Some(SmallFft { n, swaps, stages })
    }

    /// Transforms `row` (length `n`) in place, unnormalised. `inverse` uses the conjugate kernel.
    pub(super) fn process(&self, row: &mut [C], inverse: bool) {
        if inverse { self.run::<true>(row) } else { self.run::<false>(row) }
    }

    fn run<const INV: bool>(&self, row: &mut [C]) {
        if row.len() != self.n {
            return;
        }
        for &(i, j) in &self.swaps {
            row.swap(i as usize, j as usize);
        }
        for stage in &self.stages {
            match (stage.radix, stage.m) {
                (4, 1) => first4::<INV>(row),
                (4, m) => radix4::<INV>(row, m, &stage.twiddle),
                (2, m) => radix2::<INV>(row, m, &stage.twiddle),
                (3, m) => radix3::<INV>(row, m, &stage.twiddle),
                (_, m) => radix5::<INV>(row, m, &stage.twiddle),
            }
        }
    }
}

#[inline(always)]
fn turn<const INV: bool>(x: C, w: C) -> C {
    if INV { C::new(x.re * w.re + x.im * w.im, x.im * w.re - x.re * w.im) } else { C::new(x.re * w.re - x.im * w.im, x.im * w.re + x.re * w.im) }
}

/// `d` times the transform's fourth root of unity (`-i` forward, `i` inverse).
#[inline(always)]
fn quarter<const INV: bool>(d: C) -> C {
    if INV { C::new(-d.im, d.re) } else { C::new(d.im, -d.re) }
}

/// The first radix-4 pass, where every twiddle is 1.
fn first4<const INV: bool>(row: &mut [C]) {
    for x in row.as_chunks_mut::<4>().0 {
        let (a, b, c, d) = (x[0] + x[2], x[0] - x[2], x[1] + x[3], quarter::<INV>(x[1] - x[3]));
        *x = [a + c, b + d, a - c, b - d];
    }
}

fn radix4<const INV: bool>(row: &mut [C], m: usize, twiddle: &[C]) {
    let twiddle = twiddle.as_chunks::<3>().0;
    for block in row.chunks_exact_mut(4 * m) {
        let (x0, rest) = block.split_at_mut(m);
        let (x1, rest) = rest.split_at_mut(m);
        let (x2, x3) = rest.split_at_mut(m);
        for ((((x0, x1), x2), x3), w) in x0.iter_mut().zip(x1).zip(x2).zip(x3).zip(twiddle) {
            let (t1, t2, t3) = (turn::<INV>(*x1, w[0]), turn::<INV>(*x2, w[1]), turn::<INV>(*x3, w[2]));
            let (a, b, c, d) = (*x0 + t2, *x0 - t2, t1 + t3, quarter::<INV>(t1 - t3));
            (*x0, *x1, *x2, *x3) = (a + c, b + d, a - c, b - d);
        }
    }
}

fn radix2<const INV: bool>(row: &mut [C], m: usize, twiddle: &[C]) {
    for block in row.chunks_exact_mut(2 * m) {
        let (x0, x1) = block.split_at_mut(m);
        for ((x0, x1), w) in x0.iter_mut().zip(x1).zip(twiddle) {
            let t = turn::<INV>(*x1, *w);
            (*x0, *x1) = (*x0 + t, *x0 - t);
        }
    }
}

fn radix3<const INV: bool>(row: &mut [C], m: usize, twiddle: &[C]) {
    const SIN: f32 = 0.866_025_4;
    let twiddle = twiddle.as_chunks::<2>().0;
    for block in row.chunks_exact_mut(3 * m) {
        let (x0, rest) = block.split_at_mut(m);
        let (x1, x2) = rest.split_at_mut(m);
        for (((x0, x1), x2), w) in x0.iter_mut().zip(x1).zip(x2).zip(twiddle) {
            let (t1, t2) = (turn::<INV>(*x1, w[0]), turn::<INV>(*x2, w[1]));
            let (s, d) = (t1 + t2, quarter::<INV>((t1 - t2) * SIN));
            let a = *x0 - s * 0.5;
            (*x0, *x1, *x2) = (*x0 + s, a + d, a - d);
        }
    }
}

fn radix5<const INV: bool>(row: &mut [C], m: usize, twiddle: &[C]) {
    // cos and sin of 2π/5 and 4π/5.
    const C1: f32 = 0.309_017;
    const C2: f32 = -0.809_017;
    const S1: f32 = 0.951_056_5;
    const S2: f32 = 0.587_785_25;
    let twiddle = twiddle.as_chunks::<4>().0;
    for block in row.chunks_exact_mut(5 * m) {
        let (x0, rest) = block.split_at_mut(m);
        let (x1, rest) = rest.split_at_mut(m);
        let (x2, rest) = rest.split_at_mut(m);
        let (x3, x4) = rest.split_at_mut(m);
        for (((((x0, x1), x2), x3), x4), w) in x0.iter_mut().zip(x1).zip(x2).zip(x3).zip(x4).zip(twiddle) {
            let (t1, t2, t3, t4) = (turn::<INV>(*x1, w[0]), turn::<INV>(*x2, w[1]), turn::<INV>(*x3, w[2]), turn::<INV>(*x4, w[3]));
            let (a1, a2, b1, b2) = (t1 + t4, t2 + t3, t1 - t4, t2 - t3);
            let (p, q) = (*x0 + a1 * C1 + a2 * C2, *x0 + a1 * C2 + a2 * C1);
            let (u, v) = (quarter::<INV>(b1 * S1 + b2 * S2), quarter::<INV>(b1 * S2 - b2 * S1));
            (*x0, *x1, *x2, *x3, *x4) = (*x0 + a1 + a2, p + u, q + v, q - v, p - u);
        }
    }
}
