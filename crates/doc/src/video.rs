//! Window › Timeline. A document-level playhead over a fixed number of frames at a frame rate.
//! Video layers (Layer › Video Layers) show their content for the current frame; this type is the
//! shared clock. Frame-animation and video-timeline modes both reduce to "which frame is current".

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use photocraft_raster::Surface;

use crate::plate::Plate;

/// Where a video layer's footage came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
pub enum VideoSource {
    /// Created blank (Layer › Video Layers › New Blank Video Layer).
    Blank,
    /// Loaded from a file or image sequence.
    File { path: String },
}

/// The frames of the video timeline a layer shows on: its bar in the Timeline panel, from `start`
/// up to but not including `end`. A paint layer for one frame of a shot spans just that frame.
///
/// Outside the span the layer is hidden. The engine keeps [`Layer::visible`](crate::Layer) at
/// `eye && contains(current frame)` whenever the playhead or a span changes, so every renderer,
/// exporter and panel reads that one flag; `eye` holds the visibility the user chose (the Layers
/// panel's eye), which the playhead never changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, craft_ui_macros::PlainData)]
pub struct FrameSpan {
    /// First frame shown (0-based).
    pub start: usize,
    /// The frame after the last one shown; always above `start`.
    pub end: usize,
    /// The user's own visibility for the layer.
    pub eye: bool,
}

impl FrameSpan {
    /// The frames `start..end`, at least one frame long, with the eye on.
    pub fn new(start: usize, end: usize) -> Self {
        FrameSpan { start, end: end.max(start.saturating_add(1)), eye: true }
    }

    /// Whether `frame` is one the layer shows on.
    pub fn contains(&self, frame: usize) -> bool {
        (self.start..self.end).contains(&frame)
    }

    /// Number of frames spanned.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// A span always holds at least one frame.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One frame of a video layer's stack.
#[derive(Clone, Debug, craft_ui_macros::PlainData)]
pub enum VideoFrame {
    /// Pixels the document holds: a painted frame, a blank one, or footage loaded whole.
    Pixels(Surface),
    /// Frame `n` of the layer's [`Plate`], as its file holds it: read when shown, never stored in
    /// the document.
    Plate(usize),
}

impl VideoFrame {
    /// The frame's pixels, when the document holds them.
    pub fn pixels(&self) -> Option<&Surface> {
        match self {
            VideoFrame::Pixels(s) => Some(s),
            VideoFrame::Plate(_) => None,
        }
    }

    /// The frame's pixels, when the document holds them.
    pub fn pixels_mut(&mut self) -> Option<&mut Surface> {
        match self {
            VideoFrame::Pixels(s) => Some(s),
            VideoFrame::Plate(_) => None,
        }
    }

    /// The plate frame this frame reads, when it is one.
    pub fn plate_index(&self) -> Option<usize> {
        match self {
            VideoFrame::Plate(i) => Some(*i),
            VideoFrame::Pixels(_) => None,
        }
    }
}

impl From<Surface> for VideoFrame {
    fn from(s: Surface) -> Self {
        VideoFrame::Pixels(s)
    }
}

/// The plate frame a video layer's displayed content was last set from: a frame storing finds
/// still sharing every tile with it is unpainted, and stays a plate reference.
#[derive(Clone, Debug, craft_ui_macros::PlainData)]
pub struct ShownPlateFrame {
    pub index: usize,
    pub surface: Surface,
}

/// A video layer's frame stack (Layer › Video Layers). The layer's displayed content surface is a
/// cache of `frames[timeline.current]`, which the engine keeps in sync as the playhead moves.
///
/// Frames are raster surfaces held in memory and saved in native documents, except the frames of
/// an image sequence's [`Plate`] nobody has painted: those are read from their files when shown
/// and saved as references to them. The optional original footage is kept separately from
/// painted frames for comparison/restore.
#[derive(Clone, Debug, craft_ui_macros::PlainData)]
pub struct VideoData {
    pub frames: Vec<VideoFrame>,
    /// Immutable original footage of a file-backed sequence. COW surfaces share unchanged
    /// pixel tiles with edited frames; blank video layers have no original source.
    pub original_frames: Option<Vec<VideoFrame>>,
    /// The files [`VideoFrame::Plate`] frames read; `None` when the layer has no such frames.
    pub plate: Option<Arc<Plate>>,
    pub source: VideoSource,
    pub fps: f32,
    /// Show the altered (painted) frames rather than the original footage.
    pub show_altered: bool,
    /// The plate frame on screen, if the displayed frame came from the plate.
    pub shown: Option<ShownPlateFrame>,
    /// Why the frame at the playhead could not be read from the plate (it is shown empty), while
    /// that is so.
    pub plate_problem: Option<String>,
}

impl VideoData {
    pub fn new(frames: Vec<Surface>, fps: f32) -> Self {
        VideoData {
            frames: frames.into_iter().map(VideoFrame::Pixels).collect(),
            original_frames: None,
            plate: None,
            source: VideoSource::Blank,
            fps,
            show_altered: true,
            shown: None,
            plate_problem: None,
        }
    }

    /// Every frame of `plate`, unpainted, as both the altered and the original footage.
    pub fn from_plate(plate: Arc<Plate>, fps: f32) -> Self {
        let frames: Vec<VideoFrame> = (0..plate.len()).map(VideoFrame::Plate).collect();
        VideoData { original_frames: Some(frames.clone()), frames, plate: Some(plate), ..VideoData::new(Vec::new(), fps) }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Every frame, altered and original.
    pub fn all_frames(&self) -> impl Iterator<Item = &VideoFrame> {
        self.frames.iter().chain(self.original_frames.iter().flatten())
    }

    /// Every frame, altered and original.
    pub fn all_frames_mut(&mut self) -> impl Iterator<Item = &mut VideoFrame> {
        self.frames.iter_mut().chain(self.original_frames.iter_mut().flatten())
    }

    /// The pixels of every frame the document holds, altered and original.
    pub fn held_pixels(&self) -> impl Iterator<Item = &Surface> {
        self.all_frames().filter_map(VideoFrame::pixels)
    }

    /// Whether any frame is read from the plate.
    pub fn reads_plate(&self) -> bool {
        self.plate.is_some() && self.all_frames().any(|f| f.plate_index().is_some())
    }
}

// Surfaces aren't `PartialEq` (and pixel comparison would be pointless here), so compare the stack
// by shape, not contents — enough for change detection.
impl PartialEq for VideoData {
    fn eq(&self, o: &Self) -> bool {
        let kinds = |a: &[VideoFrame], b: &[VideoFrame]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.plate_index() == y.plate_index());
        kinds(&self.frames, &o.frames)
            && match (&self.original_frames, &o.original_frames) {
                (Some(a), Some(b)) => kinds(a, b),
                (None, None) => true,
                _ => false,
            }
            && match (&self.plate, &o.plate) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b) || (a.files == b.files && a.format == b.format && a.size == b.size),
                (None, None) => true,
                _ => false,
            }
            && self.source == o.source
            && self.fps == o.fps
            && self.show_altered == o.show_altered
            && self.plate_problem == o.plate_problem
    }
}

/// The document timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
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
}

impl Timeline {
    pub fn new(duration: usize, fps: f32) -> Self {
        let d = duration.max(1);
        Timeline { fps: valid_fps(fps), duration: d, current: 0, work_start: 0, work_end: d }
    }

    /// Keep every index inside `0..duration` and the work area non-empty.
    pub fn clamp(&mut self) {
        self.duration = self.duration.max(1);
        self.fps = valid_fps(self.fps);
        self.current = self.current.min(self.duration - 1);
        self.work_start = self.work_start.min(self.duration - 1);
        self.work_end = self.work_end.clamp(self.work_start + 1, self.duration);
    }

    /// Move the playhead `delta` frames, stopping at the first and last frame. Saturating, so it's
    /// safe on any stored timeline, even one loaded with `duration` 0 or `usize::MAX` (#1009).
    pub fn step(&mut self, delta: i64) {
        let n = usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX);
        let moved = if delta < 0 { self.current.saturating_sub(n) } else { self.current.saturating_add(n) };
        self.current = moved.min(self.duration.saturating_sub(1));
    }

    /// Playhead time in seconds.
    pub fn time(&self) -> f32 {
        self.current as f32 / self.fps.max(1e-3)
    }
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
            let mut t = Timeline { fps: f32::INFINITY, duration, current: usize::MAX, work_start: 0, work_end: 0 };
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
