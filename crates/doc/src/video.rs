//! Window › Timeline. A document-level playhead over a fixed number of frames at a frame rate.
//! Video layers (Layer › Video Layers) show their content for the current frame; this type is the
//! shared clock. Frame-animation and video-timeline modes both reduce to "which frame is current".

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use photocraft_raster::Surface;

/// Where a video layer's footage came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum VideoSource {
    /// Created blank (Layer › Video Layers › New Blank Video Layer).
    Blank,
    /// Loaded from a file or image sequence.
    File { path: String },
}

/// A video layer's frame stack (Layer › Video Layers). The layer's displayed content surface is a
/// cache of `frames[timeline.current]`, which the engine keeps in sync as the playhead moves.
///
/// Frames are full raster surfaces held in memory and saved in native documents. The
/// optional original footage is kept separately from painted frames for comparison/restore.
#[derive(Clone, Debug)]
pub struct VideoData {
    pub frames: Vec<Surface>,
    /// Immutable original footage of a file-backed sequence. COW surfaces share unchanged
    /// pixel tiles with edited frames; blank video layers have no original source.
    pub original_frames: Option<Vec<Surface>>,
    pub source: VideoSource,
    pub fps: f32,
    /// Show the altered (painted) frames rather than the original footage.
    pub show_altered: bool,
}

impl VideoData {
    pub fn new(frames: Vec<Surface>, fps: f32) -> Self {
        VideoData { frames, original_frames: None, source: VideoSource::Blank, fps, show_altered: true }
    }
    pub fn len(&self) -> usize {
        self.frames.len()
    }
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

// Surfaces aren't `PartialEq` (and pixel comparison would be pointless here), so compare the stack
// by shape, not contents — enough for change detection.
impl PartialEq for VideoData {
    fn eq(&self, o: &Self) -> bool {
        self.frames.len() == o.frames.len()
            && self.original_frames.as_ref().map(Vec::len) == o.original_frames.as_ref().map(Vec::len)
            && self.source == o.source
            && self.fps == o.fps
            && self.show_altered == o.show_altered
    }
}

/// The document timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    /// Frames per second.
    pub fps: f32,
    /// Total number of frames (>= 1).
    pub duration: usize,
    /// The playhead (0-based frame index).
    pub current: usize,
    /// Work-area start frame (inclusive).
    #[serde(default)]
    pub work_start: usize,
    /// Work-area end frame (exclusive).
    pub work_end: usize,
    /// How long each frame shows, in milliseconds (a GIF stores one per frame). Empty, or short of
    /// `duration`, means the frames past it run at `fps`. `Arc`-shared like every large payload in
    /// a document (up to a million entries), so a snapshot never copies it; edit with
    /// `Arc::make_mut`.
    #[serde(default, skip_serializing_if = "no_delays")]
    pub delays_ms: Arc<Vec<u32>>,
}

impl Timeline {
    pub fn new(duration: usize, fps: f32) -> Self {
        let d = duration.max(1);
        Timeline { fps: valid_fps(fps), duration: d, current: 0, work_start: 0, work_end: d, delays_ms: Arc::default() }
    }

    /// Keep every index inside `0..duration` and the work area non-empty.
    pub fn clamp(&mut self) {
        self.duration = self.duration.max(1);
        self.fps = valid_fps(self.fps);
        self.current = self.current.min(self.duration - 1);
        self.work_start = self.work_start.min(self.duration - 1);
        self.work_end = self.work_end.clamp(self.work_start + 1, self.duration);
        if self.delays_ms.len() > self.duration {
            Arc::make_mut(&mut self.delays_ms).truncate(self.duration);
        }
    }

    /// Move the playhead `delta` frames, stopping at the first and last frame. Saturating, so it's
    /// safe on any stored timeline, even one loaded with `duration` 0 or `usize::MAX` (#1009).
    pub fn step(&mut self, delta: i64) {
        let n = usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX);
        let moved = if delta < 0 { self.current.saturating_sub(n) } else { self.current.saturating_add(n) };
        self.current = moved.min(self.duration.saturating_sub(1));
    }

    /// How long frame `f` shows, in milliseconds: its own delay when it has one, else `1000 / fps`.
    /// A stored 0 (only a hand-edited file has one) counts as none, or playback would never wait.
    pub fn frame_ms(&self, f: usize) -> u32 {
        self.delays_ms.get(f).copied().filter(|&d| d > 0).unwrap_or_else(|| (1000.0 / valid_fps(self.fps)).round().clamp(1.0, u32::MAX as f32) as u32)
    }

    /// Playhead time in seconds: the delays of the frames before it, at `fps` for those without
    /// one. Only the stored delays are summed, so a loaded `current` of any size costs nothing.
    pub fn time(&self) -> f32 {
        let fps = f64::from(valid_fps(self.fps));
        let stored = self.delays_ms.get(..self.current.min(self.delays_ms.len())).unwrap_or(&[]);
        let ms: f64 = stored.iter().map(|&d| if d > 0 { f64::from(d) } else { 1000.0 / fps }).sum();
        (ms / 1000.0 + (self.current - stored.len()) as f64 / fps) as f32
    }
}

/// No per-frame delays to save.
fn no_delays(d: &Arc<Vec<u32>>) -> bool {
    d.is_empty()
}

/// A usable frame rate: positive and finite, else the 30 fps default.
fn valid_fps(fps: f32) -> f32 {
    if fps.is_finite() && fps > 0.0 { fps } else { 30.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_clamp() {
        let t = Timeline::new(0, 0.0);
        assert_eq!(t.duration, 1);
        assert_eq!(t.fps, 30.0);
        let mut t = Timeline::new(24, 24.0);
        t.current = 100;
        t.work_end = 999;
        t.clamp();
        assert_eq!(t.current, 23);
        assert_eq!(t.work_end, 24);
        assert!((t.time() - 23.0 / 24.0).abs() < 1e-6);
    }

    /// A frame shows for its own delay; frames without one, and any loaded rate, fall back to `fps`.
    #[test]
    fn frame_delays_fall_back_to_the_rate() {
        let mut t = Timeline::new(3, 10.0);
        t.delays_ms = Arc::new(vec![50, 500]);
        assert_eq!([t.frame_ms(0), t.frame_ms(1), t.frame_ms(2), t.frame_ms(usize::MAX)], [50, 500, 100, 100]);
        t.fps = f32::NAN;
        assert_eq!(t.frame_ms(2), 33);
        t.fps = 0.001;
        assert_eq!(t.frame_ms(2), 1_000_000);
        t.duration = 1;
        t.clamp();
        assert_eq!(*t.delays_ms, [50]);
        // A stored 0 (a hand-edited file) shows at the rate, not never.
        t.fps = 10.0;
        t.delays_ms = Arc::new(vec![0]);
        assert_eq!(t.frame_ms(0), 100);
    }

    /// The playhead time adds up the delays before it: frames with their own delay show for it,
    /// the rest (and a stored 0) at the rate.
    #[test]
    fn time_follows_the_frame_delays() {
        let mut t = Timeline::new(4, 10.0);
        t.delays_ms = Arc::new(vec![50, 500]);
        let at = |t: &mut Timeline, f| {
            t.current = f;
            t.time()
        };
        for (f, s) in [(0, 0.0), (1, 0.05), (2, 0.55), (3, 0.65)] {
            assert!((at(&mut t, f) - s).abs() < 1e-6, "frame {f}: {} s, want {s}", t.time());
        }
        t.delays_ms = Arc::new(vec![0, 500]);
        assert!((at(&mut t, 2) - 0.6).abs() < 1e-6, "{}", t.time());
        // Control: without delays it is still frames / fps; a loaded huge playhead stays finite.
        t.delays_ms = Arc::default();
        assert!((at(&mut t, 3) - 0.3).abs() < 1e-6);
        t.delays_ms = Arc::new(vec![50]);
        assert!(at(&mut t, usize::MAX).is_finite());
    }

    /// A copy of the timeline (every document snapshot) shares the delays; they save as a plain
    /// list, and none at all when empty, so older files and builds read them unchanged.
    #[test]
    fn frame_delays_are_shared_and_save_as_a_list() {
        let mut t = Timeline::new(3, 10.0);
        let empty = serde_json::to_value(&t).unwrap();
        assert!(empty.get("delays_ms").is_none(), "{empty}");
        t.delays_ms = Arc::new(vec![50, 500]);
        let copy = t.clone();
        assert!(Arc::ptr_eq(&t.delays_ms, &copy.delays_ms));
        let saved = serde_json::to_value(&t).unwrap();
        assert_eq!(saved["delays_ms"], serde_json::json!([50, 500]));
        assert_eq!(serde_json::from_value::<Timeline>(saved).unwrap(), t);
        // An edit copies them once and leaves the snapshot alone.
        let mut edited = copy.clone();
        Arc::make_mut(&mut edited.delays_ms)[0] = 70;
        assert_eq!((*copy.delays_ms)[0], 50);
    }

    /// #1009: stepping never panics or leaves the timeline, whatever was stored.
    #[test]
    fn step_saturates_at_both_ends() {
        let mut t = Timeline::new(3, 24.0);
        t.step(-1);
        assert_eq!(t.current, 0);
        t.step(1);
        t.step(i64::MAX);
        assert_eq!(t.current, 2);
        t.step(i64::MIN);
        assert_eq!(t.current, 0);
        for duration in [0, usize::MAX] {
            let mut t = Timeline { fps: f32::INFINITY, duration, current: usize::MAX, work_start: 0, work_end: 0, delays_ms: Arc::default() };
            t.step(1);
            assert_eq!(t.current, duration.saturating_sub(1));
            t.clamp();
            assert_eq!(t.fps, 30.0);
            assert!(t.current < t.duration);
        }
    }

    #[test]
    fn round_trips() {
        let t = Timeline::new(48, 25.0);
        let j = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Timeline>(&j).unwrap(), t);
    }
}
