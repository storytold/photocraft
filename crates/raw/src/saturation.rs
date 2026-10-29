//! Sensors that clip below the stated white level.
//!
//! A DNG's `WhiteLevel` is what the writer says the samples top out at, not always where the sensor
//! really saturates: a camera app can state 4095 for 12-bit data that piles up at 3567 (85 % of the
//! range above black, storytold/photocraft#4348, as storytold/lightcraft#548). Development clips each
//! channel at 1 after white balance, so samples stuck 15 % below the white level are not clipped
//! together: red and blue (multiplied by more than green) reach 1, green stays at 0.85, and blown
//! highlights turn magenta.
//!
//! [`Sensor::lift_clipped_samples`] finds the real saturation point of each colour from the samples
//! (a pile-up of identical values near the top of the range) and raises the samples at or above it to
//! the stated white level. Unclipped samples keep their values, and a file whose data reaches its white
//! level, or has no pile-up, is left alone. Same detector as storytold/lightcraft#586.

use crate::sensor::Sensor;

/// A saturation plateau must hold at least this share of its colour's samples (and 100 of them), as
/// many must lie below it (a scene, not a flat field)...
const MIN_SHARE: f64 = 1e-4;
/// ...be at least this many times as populated as the average value just below it...
const MIN_SPIKE: f64 = 16.0;
/// ...and have next to nothing above it (a real clip point is the top of the data; a popular value in
/// the middle of sparse or curve-coded data has plenty of samples above): at most this share of the
/// plateau lies more than `ABOVE_MARGIN` of the range over it (noise spilling past the plateau).
const MAX_ABOVE: f64 = 0.02;
const ABOVE_MARGIN: f32 = 0.01;
/// Plateaus within this share of the range from the white level are the white level (no lift needed).
const NEAR_WHITE: f32 = 0.03;
/// How far below the plateau the "average value just below it" reaches.
const NEIGHBOURHOOD: usize = 64;

type Histogram = (Vec<[u32; 3]>, [u64; 3]);

// CFA lookup is periodic. Resolve its origin/phase once rather than dividing and
// taking a modulus twice for every pixel in a 24–150 MP sensor. Keep the generic
// path for arbitrary public Sensor patterns. Native jobs each own bounded scratch.
fn histogram(sensor: &Sensor, low_bin: usize) -> Histogram {
    let a = sensor.active;
    let Some(cfa) = &sensor.cfa else { return (vec![[0; 3]; 65536], [0; 3]) };
    let patterns = (cfa.width <= 16 && cfa.height <= 16 && cfa.width > 0 && cfa.height > 0).then(|| {
        (0..cfa.height)
            .map(|y| (0..cfa.width).map(|x| usize::from(cfa.color(a.x.saturating_add(x), a.y.saturating_add(y))).min(2)).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    });
    let jobs = if a.width.saturating_mul(a.height) >= 1_000_000 { crate::par::workers().min(8).min(a.height).max(1) } else { 1 };
    let rows = a.height.div_ceil(jobs);
    let parts = crate::par::map(jobs, |job| {
        let mut hist = vec![[0u32; 3]; 65536];
        let mut total = [0u64; 3];
        let start = job.saturating_mul(rows);
        for (y, row) in sensor.data.chunks_exact(sensor.width.max(1)).enumerate().skip(a.y + start).take(rows.min(a.height.saturating_sub(start))) {
            let Some(data) = row.get(a.x..a.x.saturating_add(a.width)) else { continue };
            let pattern = patterns.as_ref().and_then(|p| p.get((y - a.y) % p.len().max(1)));
            let mut add = |value: u16, color: usize| {
                // The detector only examines the upper half and its 64-bin
                // neighbourhood. Dark samples still count toward the population
                // but never need random histogram writes.
                if usize::from(value) >= low_bin
                    && let Some(bin) = hist.get_mut(usize::from(value)).and_then(|h| h.get_mut(color))
                {
                    *bin = bin.saturating_add(1);
                }
            };
            if let Some(pattern) = pattern {
                let cycles = data.len() / pattern.len();
                let remainder = data.len() % pattern.len();
                for (index, &color) in pattern.iter().enumerate() {
                    if let Some(n) = total.get_mut(color) {
                        *n = n.saturating_add((cycles + usize::from(index < remainder)) as u64);
                    }
                }
                for (&value, &color) in data.iter().zip(pattern.iter().cycle()) {
                    add(value, color);
                }
            } else {
                for (x, &value) in data.iter().enumerate() {
                    let color = usize::from(cfa.color(a.x.saturating_add(x), y)).min(2);
                    add(value, color);
                    if let Some(n) = total.get_mut(color) {
                        *n = n.saturating_add(1);
                    }
                }
            }
        }
        (hist, total)
    });
    let mut parts = parts.into_iter();
    let (mut hist, mut total) = parts.next().unwrap_or_else(|| (vec![[0; 3]; 65536], [0; 3]));
    for (next, counts) in parts {
        for (out, input) in hist.iter_mut().zip(next) {
            for (a, b) in out.iter_mut().zip(input) {
                *a = a.saturating_add(b);
            }
        }
        for (a, b) in total.iter_mut().zip(counts) {
            *a = a.saturating_add(b);
        }
    }
    (hist, total)
}

impl Sensor {
    /// The value each colour's samples saturate at, when that is clearly below the stated white level
    /// (see the module docs). Only CFA data without a linearization table is examined: with a table the
    /// stored values are not on the white level's scale.
    pub(crate) fn saturation_points(&self) -> [Option<u16>; 3] {
        let none = [None; 3];
        if self.cfa.is_none() {
            return none;
        }
        if self.samples != 1 || self.linearization.is_some() || self.width.checked_mul(self.height) != Some(self.data.len()) {
            return none;
        }
        let white = self.white[0];
        let black = if self.black.values.is_empty() { 0.0 } else { self.black.values.iter().sum::<f32>() / self.black.values.len() as f32 };
        // a hostile file can state any levels: NaN, negative or inverted ones are left alone
        if !(black.is_finite() && white.is_finite() && black >= 0.0 && white > black && white <= 65535.0) {
            return none;
        }
        let a = self.active;
        let fits = |start: usize, len: usize, max: usize| start.checked_add(len).is_some_and(|end| end <= max);
        if !fits(a.x, a.width, self.width) || !fits(a.y, a.height, self.height) {
            return none;
        }
        let lo = (black + 0.5 * (white - black)).ceil() as usize;
        let hi = white as usize;
        if lo + NEIGHBOURHOOD >= hi {
            return none;
        }
        let (hist, total) = histogram(self, lo.saturating_sub(NEIGHBOURHOOD));
        let mut out = none;
        for c in 0..3 {
            // the most populated value of the upper half of the range
            let Some((v, n)) = (lo..hi).map(|v| (v, hist[v][c])).max_by_key(|&(v, n)| (n, v)) else { continue };
            let min_n = (total[c] as f64 * MIN_SHARE).max(100.0);
            if f64::from(n) < min_n || v as f32 >= white - NEAR_WHITE * (white - black) {
                continue;
            }
            // a colour whose samples all sit on one value is a flat field, not a scene that clipped
            let darker = total[c].saturating_sub(hist[v..].iter().map(|h| u64::from(h[c])).sum::<u64>());
            if (darker as f64) < min_n {
                continue;
            }
            let below: u64 = (v.saturating_sub(NEIGHBOURHOOD)..v).map(|u| u64::from(hist[u][c])).sum();
            let from = (v + ((ABOVE_MARGIN * (white - black)) as usize).max(1) + 1).min(hist.len());
            let above: u64 = hist[from..].iter().map(|h| u64::from(h[c])).sum();
            if f64::from(n) >= MIN_SPIKE * (below as f64 / NEIGHBOURHOOD as f64).max(1.0) && above as f64 <= MAX_ABOVE * f64::from(n) {
                out[c] = u16::try_from(v).ok();
            }
        }
        out
    }

    /// Raises samples at or above their colour's real saturation point ([`Self::saturation_points`]) to
    /// the stated white level, so development clips them together. Returns how many samples changed
    /// (0: the data is untouched).
    pub(crate) fn lift_clipped_samples(&mut self) -> usize {
        let points = self.saturation_points();
        if points.iter().all(Option::is_none) {
            return 0;
        }
        let Some(cfa) = self.cfa.clone() else { return 0 };
        let (a, w) = (self.active, self.width);
        // the white level itself, rounded up: (white - black) / (white - black) is exactly 1
        let white = self.white[0].ceil() as u16;
        let mut changed = 0;
        for y in a.y..a.y + a.height {
            let row = &mut self.data[y * w + a.x..][..a.width];
            for (x, v) in (a.x..).zip(row.iter_mut()) {
                if let Some(p) = points[usize::from(cfa.color(x, y)).min(2)]
                    && *v >= p
                    && *v < white
                {
                    *v = white;
                    changed += 1;
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn parallel_periodic_histogram_preserves_every_bin_at_crop_and_band_boundaries() {
        for (width, height, phase) in [(67, 71, (3, 5)), (1153, 1091, (5, 1))] {
            let mut sensor = sensor_with(None, 4095.0);
            sensor.width = width;
            sensor.height = height;
            sensor.data = (0..width * height).map(|i| ((i * 997 + i / width * 31) & 65535) as u16).collect();
            sensor.active = Rect::new(3, 7, width - 8, height - 12);
            for period in [0, 2, 6, 17] {
                let cfa =
                    Cfa { width: period, height: period, colors: (0..period * period).map(|i| (i % 3) as u8).collect(), origin_x: phase.0, origin_y: phase.1 };
                sensor.cfa = Some(cfa.clone());
                let mut want = vec![[0u32; 3]; 65536];
                let mut totals = [0u64; 3];
                let a = sensor.active;
                for y in a.y..a.y + a.height {
                    for x in a.x..a.x + a.width {
                        let c = usize::from(cfa.color(x, y)).min(2);
                        want[usize::from(sensor.data[y * width + x])][c] += 1;
                        totals[c] += 1;
                    }
                }
                let (got, counts) = super::histogram(&sensor, 0);
                assert_eq!(got, want, "period={period}");
                assert_eq!(counts, totals);
                let (upper, counts) = super::histogram(&sensor, 20000);
                assert_eq!(counts, totals);
                assert_eq!(&upper[20000..], &want[20000..]);
                assert!(upper[..20000].iter().all(|v| *v == [0; 3]));
            }
        }
    }

    /// A 64 x 64 RGGB mosaic with a gradient, black 528, white `white`, whose samples above `clip` (when
    /// given) are stuck at `clip` like a sensor that saturates early.
    fn sensor_with(clip: Option<u16>, white: f32) -> Sensor {
        let data = (0..64 * 64)
            .map(|i| {
                let v = 528 + ((i % 64) * 60 + (i / 64) * 7) as u16 % 4000;
                clip.map_or(v, |c| v.min(c))
            })
            .collect();
        Sensor {
            format: RawFormat::Dng,
            make: None,
            model: None,
            width: 64,
            height: 64,
            samples: 1,
            data,
            cfa: Cfa::bayer("RGGB"),
            linearization: None,
            black: BlackLevels::uniform(528.0),
            white: [white; 3],
            active: Rect::new(0, 0, 64, 64),
            crop: Rect::new(0, 0, 64, 64),
            color: ColorInfo::default(),
            camera_wb: None,
            orientation: 1,
            baseline_exposure: 0.0,
            gain_maps: Vec::new(),
            tone_curve: Vec::new(),
            black_point: 0.0,
            warnings: Vec::new(),
        }
    }

    /// The gradient of `sensor_with`, wrapped so it never gets near the top of the range.
    fn without_top(s: &mut Sensor) {
        for v in s.data.iter_mut() {
            *v = 528 + (*v - 528) % 3000;
        }
    }

    #[test]
    fn a_pile_up_below_the_white_level_is_found_and_lifted() {
        let mut s = sensor_with(Some(3567), 4095.0);
        assert_eq!(s.saturation_points(), [Some(3567); 3]);
        let before = s.data.clone();
        assert!(s.lift_clipped_samples() > 100);
        for (x, y) in before.iter().zip(&s.data) {
            // plateau samples go to the white level, everything else is untouched
            assert_eq!(*y, if *x >= 3567 { 4095 } else { *x });
        }
        assert_eq!(s.lift_clipped_samples(), 0, "idempotent");
    }

    #[test]
    fn data_that_reaches_its_white_level_is_left_alone() {
        let mut s = sensor_with(Some(4095), 4095.0);
        let before = s.data.clone();
        assert_eq!(s.saturation_points(), [None; 3]);
        assert_eq!(s.lift_clipped_samples(), 0);
        assert_eq!(s.data, before);
    }

    #[test]
    fn data_without_a_pile_up_is_left_alone() {
        let mut s = sensor_with(None, 4095.0);
        without_top(&mut s);
        let before = s.data.clone();
        assert_eq!(s.saturation_points(), [None; 3]);
        assert_eq!(s.lift_clipped_samples(), 0);
        assert_eq!(s.data, before);
    }

    /// Sparse or curve-coded data has popular values with plenty of samples above them: not a clip point.
    #[test]
    fn a_popular_value_with_data_above_it_is_not_a_clip_point() {
        let mut s = sensor_with(None, 4095.0);
        without_top(&mut s);
        for (i, v) in s.data.iter_mut().enumerate() {
            // a third of the samples sit on one code in the upper half, a few hundred are brighter
            if i % 3 == 0 {
                *v = 3000;
            } else if i % 11 == 0 {
                *v = 3000 + (i % 700) as u16 + 1;
            }
        }
        assert_eq!(s.saturation_points(), [None; 3]);
        assert_eq!(s.lift_clipped_samples(), 0);
    }

    /// A uniform patch (a flat-field test frame) has nothing darker: not a scene that clipped.
    #[test]
    fn a_flat_field_is_not_a_clip_point() {
        let mut s = sensor_with(None, 4095.0);
        s.data.fill(3000);
        assert_eq!(s.saturation_points(), [None; 3]);
        assert_eq!(s.lift_clipped_samples(), 0);
    }

    #[test]
    fn a_few_stuck_samples_are_not_a_plateau() {
        let mut s = sensor_with(None, 4095.0);
        without_top(&mut s);
        for v in s.data.iter_mut().take(20) {
            *v = 3800;
        }
        assert_eq!(s.saturation_points(), [None; 3]);
    }

    /// With a linearization table the stored codes are not on the white level's scale.
    #[test]
    fn linearized_data_is_left_alone() {
        let mut s = sensor_with(Some(3567), 4095.0);
        s.linearization = Some((0..4096).collect());
        assert_eq!(s.saturation_points(), [None; 3]);
        assert_eq!(s.lift_clipped_samples(), 0);
    }

    #[test]
    fn hostile_levels_are_left_alone() {
        for black in [-5000.0, f32::NAN] {
            let mut s = sensor_with(Some(40), 4095.0);
            s.black = BlackLevels::uniform(black);
            assert_eq!(s.saturation_points(), [None; 3]);
            assert_eq!(s.lift_clipped_samples(), 0);
        }
        for white in [f32::NAN, f32::INFINITY, 1e9, 100.0] {
            let mut s = sensor_with(Some(3567), 4095.0);
            s.white = [white; 3];
            assert_eq!(s.saturation_points(), [None; 3], "{white}");
        }
        // an active area that runs past the data (or overflows when added up) is refused
        let mut s = sensor_with(Some(3567), 4095.0);
        s.active = Rect::new(usize::MAX, 0, 2, 64);
        assert_eq!(s.saturation_points(), [None; 3]);
        s.active = Rect::new(0, 0, 64, 65);
        assert_eq!(s.lift_clipped_samples(), 0);
        let mut s = sensor_with(Some(3567), 4095.0);
        s.data.pop();
        assert_eq!(s.lift_clipped_samples(), 0);
    }
}
