//! Relative device motion supplies candidate interior points; OS positions remain authoritative.
//! Online only: no waiting, timestamp calibration, prediction or retrospective edits.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

// Conservative fallback heuristics, not guarantees about a device's sampling rate.
const MAX_SAMPLES: usize = 64;
const MAX_PENDING_SEGMENTS: usize = 128;
const MAX_OS_AGE_SECONDS: f64 = 0.025;
const MAX_INTERVAL_SECONDS: f64 = 0.05;
const ENDPOINT_EPSILON_POINTS: f64 = 0.02;
const OS_CHORD_POINTS: std::ops::RangeInclusive<f64> = 0.25..=512.0;
const RAW_DELTA_MAX: f64 = 4096.0; // device units, before OS acceleration
const RAW_NET_MIN: f64 = 0.01;
const GAIN: std::ops::RangeInclusive<f64> = 0.05..=8.0;
const MIN_NET_ARC_RATIO: f64 = 0.25; // reject cancelling loops
const MIN_ALIGNMENT_COS: f64 = std::f64::consts::FRAC_1_SQRT_2; // at most 45 degrees
const MAX_PATH_CHORD_RATIO: f64 = 3.0;

#[derive(Clone, Copy)]
struct Anchor {
    point: [f64; 2],
    event_time: f64,
    receipt: f64,
}
#[derive(Default)]
pub struct Aligner {
    anchor: Option<Anchor>,
    deltas: Vec<[f64; 2]>,
    device: Option<u64>,
    invalid: bool,
}
impl Aligner {
    pub fn reset(&mut self) {
        self.anchor = None;
        self.deltas.clear();
        self.device = None;
        self.invalid = false;
    }
    pub fn push(&mut self, device: u64, delta: [f64; 2]) {
        if self.anchor.is_none() {
            return;
        }
        if self.deltas.len() >= MAX_SAMPLES || !finite(delta) || length(delta) > RAW_DELTA_MAX || self.device.is_some_and(|d| d != device) {
            self.invalid = true;
            self.deltas.clear();
            return;
        }
        if !self.invalid {
            self.device = Some(device);
            self.deltas.push(delta);
        }
    }
    /// Candidate points strictly before `to`; the caller always delivers the OS endpoint.
    pub fn endpoint(&mut self, to: [f64; 2], event_time: f64, receipt: f64) -> Vec<[f64; 2]> {
        self.endpoint_with_clock(to, event_time, receipt, false)
    }
    /// Window-event streams expose only a frame clock. Events in the same frame retain their
    /// arrival order, but their individual age and device identity cannot be established.
    pub fn frame_endpoint(&mut self, to: [f64; 2], frame_time: f64) -> Vec<[f64; 2]> {
        self.endpoint_with_clock(to, frame_time, frame_time, true)
    }
    fn endpoint_with_clock(&mut self, to: [f64; 2], event_time: f64, receipt: f64, same_frame: bool) -> Vec<[f64; 2]> {
        let previous = self.anchor.replace(Anchor { point: to, event_time, receipt });
        let points = self.fit(previous, to, event_time, receipt, same_frame);
        self.deltas.clear();
        self.device = None;
        self.invalid = false;
        if !finite(to) || !event_time.is_finite() || !receipt.is_finite() || receipt < event_time || receipt - event_time > MAX_OS_AGE_SECONDS {
            self.anchor = None;
        }
        points
    }
    fn fit(&self, previous: Option<Anchor>, to: [f64; 2], event_time: f64, receipt: f64, same_frame: bool) -> Vec<[f64; 2]> {
        let Some(a) = previous else { return Vec::new() };
        if self.invalid
            || self.deltas.len() < 2
            || !finite(to)
            || !event_time.is_finite()
            || !receipt.is_finite()
            || event_time < a.event_time
            || (!same_frame && event_time == a.event_time)
            || event_time - a.event_time > MAX_INTERVAL_SECONDS
            || receipt < a.receipt
            || receipt < event_time
            || receipt - event_time > MAX_OS_AGE_SECONDS
        {
            return Vec::new();
        }
        let chord = [to[0] - a.point[0], to[1] - a.point[1]];
        let distance = length(chord);
        if !OS_CHORD_POINTS.contains(&distance) {
            return Vec::new();
        }
        let net = self.deltas.iter().fold([0.0; 2], |p, d| [p[0] + d[0], p[1] + d[1]]);
        let raw_len = length(net);
        let arc = self.deltas.iter().map(|d| length(*d)).sum::<f64>();
        let gain = distance / raw_len;
        if raw_len < RAW_NET_MIN
            || raw_len < arc * MIN_NET_ARC_RATIO
            || !GAIN.contains(&gain)
            || (net[0] * chord[0] + net[1] * chord[1]) / (raw_len * distance) < MIN_ALIGNMENT_COS
        {
            return Vec::new();
        }
        // Complex multiplication rotates and scales cumulative raw deltas onto the OS chord.
        let norm = raw_len * raw_len;
        let re = (net[0] * chord[0] + net[1] * chord[1]) / norm;
        let im = (net[0] * chord[1] - net[1] * chord[0]) / norm;
        let mut raw = [0.0; 2];
        let mut out = Vec::with_capacity(self.deltas.len().saturating_sub(1));
        for d in self.deltas.iter().take(self.deltas.len().saturating_sub(1)) {
            raw = [raw[0] + d[0], raw[1] + d[1]];
            let p = [a.point[0] + re * raw[0] - im * raw[1], a.point[1] + im * raw[0] + re * raw[1]];
            if !finite(p) || length([p[0] - a.point[0], p[1] - a.point[1]]) + length([p[0] - to[0], p[1] - to[1]]) > distance * MAX_PATH_CHORD_RATIO {
                return Vec::new();
            }
            out.push(p);
        }
        out
    }
}
fn finite(p: [f64; 2]) -> bool {
    p.iter().all(|v| v.is_finite())
}
fn length(p: [f64; 2]) -> f64 {
    p[0].hypot(p[1])
}

struct Segment {
    button: Option<u8>,
    from: [f64; 2],
    to: [f64; 2],
    points: Vec<[f64; 2]>,
}
#[derive(Default)]
struct Pending {
    view: usize,
    segments: VecDeque<Segment>,
}
/// Window-scoped transport. Both endpoint coordinates must match; each segment is used once.
#[derive(Clone, Default)]
pub struct Feed(Arc<Mutex<Pending>>);
impl Feed {
    pub fn bind_view(&self, view: usize) {
        let mut p = self.lock();
        p.view = view;
        p.segments.clear();
    }
    pub fn view(&self) -> usize {
        self.lock().view
    }
    pub fn clear(&self) {
        self.lock().segments.clear();
    }
    pub fn publish(&self, from: [f64; 2], to: [f64; 2], points: Vec<[f64; 2]>) {
        if points.is_empty() {
            return;
        }
        if !finite(from) || !finite(to) || points.len() > MAX_SAMPLES || points.iter().any(|p| !finite(*p)) {
            self.clear();
            return;
        }
        let mut p = self.lock();
        if p.segments.len() >= MAX_PENDING_SEGMENTS {
            p.segments.clear();
            return;
        }
        p.segments.push_back(Segment { button: None, from, to, points });
    }
    /// Native press marker: button 0 is primary, 1 is secondary (egui-compatible).
    /// Retain later moves even when down/up share a UI frame.
    pub fn press(&self, point: [f64; 2], button: u8) {
        if !finite(point) {
            self.clear();
            return;
        }
        let mut p = self.lock();
        if p.segments.len() >= MAX_PENDING_SEGMENTS {
            p.segments.clear();
        }
        p.segments.push_back(Segment { button: Some(button), from: point, to: point, points: Vec::new() });
    }
    pub fn begin(&self, point: [f64; 2], button: u8) {
        let mut p = self.lock();
        if let Some(i) = p.segments.iter().position(|s| {
            s.button == Some(button) && (s.to[0] - point[0]).abs() < ENDPOINT_EPSILON_POINTS && (s.to[1] - point[1]).abs() < ENDPOINT_EPSILON_POINTS
        }) {
            p.segments.drain(..=i);
        } else {
            p.segments.clear();
        }
    }
    pub fn take(&self, from: [f64; 2], to: [f64; 2]) -> Vec<[f64; 2]> {
        let mut p = self.lock();
        let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < ENDPOINT_EPSILON_POINTS && (a[1] - b[1]).abs() < ENDPOINT_EPSILON_POINTS;
        // A later press belongs to a separate gesture, even if its coordinates repeat.
        let Some(i) = p.segments.iter().take_while(|s| s.button.is_none()).position(|s| close(s.from, from) && close(s.to, to)) else { return Vec::new() };
        let segment = p.segments.drain(..=i).next_back();
        segment.map_or_else(Vec::new, |s| s.points)
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn start() -> Aligner {
        let mut a = Aligner::default();
        a.endpoint([10.0, 20.0], 1.0, 1.0);
        a
    }
    #[test]
    fn frame_clock_accepts_ordered_same_frame_events_without_relaxing_native_timestamps() {
        for native in [false, true] {
            let mut a = start();
            a.push(1, [1.0, 1.0]);
            a.push(1, [1.0, -1.0]);
            let points = if native { a.endpoint([14.0, 20.0], 1.0, 1.0) } else { a.frame_endpoint([14.0, 20.0], 1.0) };
            assert_eq!(points, if native { vec![] } else { vec![[12.0, 22.0]] });
        }
        for time in [0.9, 1.1, f64::NAN, f64::INFINITY] {
            let mut a = start();
            a.push(1, [1.0, 1.0]);
            a.push(1, [1.0, -1.0]);
            assert!(a.frame_endpoint([14.0, 20.0], time).is_empty());
        }
    }
    #[test]
    fn online_curve_fits_without_waiting_or_including_endpoint() {
        let mut a = start();
        a.push(1, [1.0, 1.0]);
        a.push(1, [1.0, -1.0]);
        assert_eq!(a.endpoint([14.0, 20.0], 1.01, 1.012), vec![[12.0, 22.0]]);
        assert!(a.endpoint([15.0, 20.0], 1.02, 1.02).is_empty());
    }
    #[test]
    fn ambiguous_intervals_keep_only_the_os_endpoint() {
        let cases = [
            "mixed devices",
            "cancelling deltas",
            "nonfinite delta",
            "reset",
            "single sample",
            "overflow",
            "long interval",
            "old OS event",
            "opposite direction",
            "excessive curvature",
        ];
        for (case, name) in cases.into_iter().enumerate() {
            let mut a = start();
            a.push(1, [1.0, 1.0]);
            match case {
                0 => a.push(2, [1.0, -1.0]),
                1 => a.push(1, [-1.0, -1.0]),
                2 => a.push(1, [f64::NAN, 0.0]),
                3 => {
                    a.reset();
                    a.push(1, [1.0, -1.0]);
                }
                4 => {}
                5 => {
                    for _ in 0..65 {
                        a.push(1, [1.0, 0.0]);
                    }
                }
                6 => a.push(1, [1.0, -1.0]),
                7 => a.push(1, [1.0, -1.0]),
                8 => a.push(1, [-2.0, -1.0]),
                _ => {
                    a.push(1, [1000.0, 1000.0]);
                    a.push(1, [-1000.0, -1000.0]);
                    a.push(1, [1.0, -1.0]);
                }
            }
            let (event, receipt) = if case == 6 {
                (1.1, 1.1)
            } else if case == 7 {
                (1.01, 1.2)
            } else {
                (1.01, 1.01)
            };
            assert!(a.endpoint([14.0, 20.0], event, receipt).is_empty(), "{name}");
        }
    }
    #[test]
    fn ignored_secondary_press_cannot_supply_primary_curve() {
        let f = Feed::default();
        let (a, b) = ([0.0; 2], [4.0, 0.0]);
        f.press(a, 1);
        f.publish(a, b, vec![[2.0, 1.0]]);
        f.press(a, 0);
        f.publish(a, b, vec![[2.0, -1.0]]);
        f.begin(a, 0);
        assert_eq!(f.take(a, b), vec![[2.0, -1.0]]);
        // Frame-end cleanup also discards unused primary markers from non-painting tools.
        f.press(a, 0);
        f.publish(a, b, vec![[2.0, 1.0]]);
        f.clear();
        f.press(a, 0);
        f.publish(a, b, vec![[2.0, -1.0]]);
        f.begin(a, 0);
        assert_eq!(f.take(a, b), vec![[2.0, -1.0]]);
    }

    #[test]
    fn invalid_anchor_and_warp_cannot_seed_a_later_curve() {
        let mut a = start();
        a.endpoint([f64::INFINITY, 0.0], 1.01, 1.01);
        a.push(1, [1.0, 0.0]);
        a.push(1, [1.0, 0.0]);
        assert!(a.endpoint([14.0, 20.0], 1.02, 1.02).is_empty());
        let mut a = start();
        a.push(1, [1.0, 0.0]);
        a.push(1, [1.0, 0.0]);
        assert!(a.endpoint([2000.0, 20.0], 1.01, 1.01).is_empty());
    }
    #[test]
    fn transport_matches_both_endpoints_and_consumes_once() {
        let f = Feed::default();
        f.publish([0.0; 2], [4.0, 0.0], vec![[2.0, 1.0]]);
        assert!(f.take([1.0, 0.0], [4.0, 0.0]).is_empty());
        assert_eq!(f.take([0.0; 2], [4.0, 0.0]), vec![[2.0, 1.0]]);
        assert!(f.take([0.0; 2], [4.0, 0.0]).is_empty());
        f.publish([0.0; 2], [4.0, 0.0], vec![[2.0, 1.0]]);
        f.bind_view(7);
        assert!(f.take([0.0; 2], [4.0, 0.0]).is_empty());
    }
    #[test]
    fn press_markers_keep_same_frame_moves_and_separate_repeated_strokes() {
        let f = Feed::default();
        let a = [0.0; 2];
        let b = [4.0, 0.0];
        f.press(a, 0);
        f.publish(a, b, vec![[2.0, 1.0]]);
        f.press(a, 0);
        f.publish(a, b, vec![[2.0, -1.0]]);
        assert!(f.take(a, b).is_empty(), "a native press must first match the UI press");
        f.begin(a, 0);
        assert_eq!(f.take(a, b), vec![[2.0, 1.0]]);
        assert!(f.take(a, b).is_empty(), "never borrow the next stroke's identical interval");
        f.begin(a, 0);
        assert_eq!(f.take(a, b), vec![[2.0, -1.0]]);
    }
    #[test]
    fn transport_rejects_invalid_or_unbounded_supplemental_input() {
        let f = Feed::default();
        let a = [0.0; 2];
        let b = [4.0, 0.0];
        for bad in [vec![[f64::NAN, 0.0]], vec![[2.0, 0.0]; 65]] {
            f.publish(a, b, vec![[2.0, 1.0]]);
            f.publish(a, b, bad);
            assert!(f.take(a, b).is_empty());
        }
    }
    #[test]
    fn late_deltas_never_rewrite_an_already_closed_interval() {
        let mut a = start();
        assert!(a.endpoint([14.0, 20.0], 1.01, 1.01).is_empty());
        a.push(1, [1.0, 1.0]);
        a.push(1, [1.0, -1.0]);
        assert_eq!(a.endpoint([18.0, 20.0], 1.02, 1.02), vec![[16.0, 22.0]]);
        // GC supplies no timestamp with which to identify these deltas' physical interval.
        // Callback-order assignment is explicitly an estimate, never retrospective repair.
    }
}
