//! Window › Timeline. A document-level playhead over a fixed number of frames at a frame rate.
//! Video layers (Layer › Video Layers) show their content for the current frame; this type is the
//! shared clock. Frame-animation and video-timeline modes both reduce to "which frame is current".

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use photocraft_raster::Surface;

/// Where a video layer's footage came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
pub enum VideoSource {
    /// Created blank (Layer › Video Layers › New Blank Video Layer).
    Blank,
    /// Loaded from a file or image sequence.
    File { path: String },
}

/// What a movie file holds, as read when it was linked: its picture size, how many frames it has
/// and at what rate. Kept in the document so a layer whose file is missing still knows its shape.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
pub struct FootageInfo {
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    pub fps: f32,
}

/// A movie file a video layer reads its frames from, decoded when a frame is shown rather than
/// held in memory. The document keeps only this link: the decoder and its frame cache live in the
/// engine, keyed by `path`, so the document stays plain data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
pub struct FootageLink {
    /// The file, as given when it was linked.
    pub path: String,
    pub info: FootageInfo,
    /// Footage pixel → document pixel. Whole-image edits (Image Size, Canvas Size, crop, canvas
    /// turns) change it as they change the stored pixels, so a decoded frame lands where the
    /// painted frames are.
    pub placement: photocraft_geom::Affine,
    /// The part of the footage frame that is kept, as a convex polygon in footage pixels: a crop
    /// that deleted the cropped pixels, mapped back through `placement`, so it stays exact when
    /// the footage is turned or scaled. A footage pixel is kept when its centre is inside.
    /// `None` keeps the whole frame.
    pub clip: Option<Vec<[f64; 2]>>,
}

impl FootageLink {
    pub fn new(path: String, info: FootageInfo) -> Self {
        FootageLink { path, info, placement: photocraft_geom::Affine::IDENTITY, clip: None }
    }
}

/// The frames of a video layer that come from a movie file: for each frame of the layer, which
/// footage frame it shows (`None` for a frame inserted blank) and whether it has been painted on.
/// A painted frame's pixels are the layer's own (`VideoData::frames`); an unpainted one's are
/// decoded from the file, and its `frames` entry is an empty placeholder.
///
/// The tables are shared between copies of the document (history states, the copy each playhead
/// move makes) and copied only when one of them changes, so a long clip's tables cost nothing to
/// carry until a frame is painted or the stack changes.
#[derive(Clone, Debug, PartialEq, craft_ui_macros::PlainData)]
pub struct FootageFrames {
    pub link: FootageLink,
    /// Same length as `VideoData::frames`.
    pub origin: Arc<Vec<Option<usize>>>,
    /// Same length as `VideoData::frames`.
    pub altered: Arc<Vec<bool>>,
}

/// What a video layer shows for one of its frames.
#[derive(Clone, Copy, Debug)]
pub enum FrameView<'a> {
    /// Pixels the layer holds (a painted frame, a blank frame, or any frame of footage that was
    /// loaded whole).
    Own(&'a Surface),
    /// Frame `n` of the linked movie, to be decoded.
    Footage(usize),
    /// Nothing: the original of a frame that was inserted blank.
    Blank,
}

/// A video layer's frame stack (Layer › Video Layers). The layer's displayed content surface is a
/// cache of the frame at the timeline playhead, which the engine keeps in sync as the playhead moves.
///
/// Frames of blank layers, image sequences and animated images are full raster surfaces held in
/// memory and saved in native documents, with the original footage kept separately from painted
/// frames for comparison/restore. Frames of a movie file ([`FootageFrames`]) are decoded when
/// shown; only the painted ones are held and saved.
#[derive(Clone, Debug, craft_ui_macros::PlainData)]
pub struct VideoData {
    /// Shared between copies of the document and copied on the first change (see
    /// [`FootageFrames`]); change it through [`Arc::make_mut`].
    pub frames: Arc<Vec<Surface>>,
    /// Immutable original footage of a file-backed sequence. COW surfaces share unchanged
    /// pixel tiles with edited frames; blank video layers have no original source.
    pub original_frames: Option<Vec<Surface>>,
    pub source: VideoSource,
    pub fps: f32,
    /// Show the altered (painted) frames rather than the original footage.
    pub show_altered: bool,
    /// The movie file the unpainted frames are decoded from; `None` for in-memory footage.
    pub footage: Option<FootageFrames>,
    /// The pixel state (`Surface::generation`) of the decoded frame last shown, so storing the
    /// displayed content back can tell an untouched decoded frame from a painted one. Not saved.
    pub shown: Option<u64>,
}

impl VideoData {
    pub fn new(frames: Vec<Surface>, fps: f32) -> Self {
        VideoData { frames: Arc::new(frames), original_frames: None, source: VideoSource::Blank, fps, show_altered: true, footage: None, shown: None }
    }

    /// A layer that shows the frames of a linked movie, one layer frame per footage frame, none
    /// of them painted. `format` is the document's pixel format, for the empty placeholders.
    pub fn from_footage(link: FootageLink, format: photocraft_color::PixelFormat) -> Self {
        let n = link.info.frames;
        let fps = link.info.fps;
        let path = link.path.clone();
        let footage = FootageFrames { link, origin: Arc::new((0..n).map(Some).collect()), altered: Arc::new(vec![false; n]) };
        VideoData {
            frames: Arc::new((0..n).map(|_| Surface::new(format)).collect()),
            original_frames: None,
            source: VideoSource::File { path },
            fps,
            show_altered: true,
            footage: Some(footage),
            shown: None,
        }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// What frame `at` shows: the painted (altered) frame, or the original footage when
    /// `original` is set (Show Altered Video off). Clamped to the last frame; `None` for a layer
    /// without frames.
    pub fn view(&self, at: usize, original: bool) -> Option<FrameView<'_>> {
        let at = at.min(self.frames.len().checked_sub(1)?);
        if let Some(f) = &self.footage {
            let altered = f.altered.get(at).copied().unwrap_or(true);
            if altered && !original {
                return Some(FrameView::Own(&self.frames[at]));
            }
            return Some(match f.origin.get(at).copied().flatten() {
                Some(n) => FrameView::Footage(n),
                // A frame inserted blank has nothing behind it.
                None => FrameView::Blank,
            });
        }
        let stack = if original { self.original_frames.as_ref().unwrap_or(&self.frames) } else { &self.frames };
        stack.get(at.min(stack.len().saturating_sub(1))).map(FrameView::Own)
    }

    /// Keep `content` as frame `at`'s painted pixels. For linked footage, an unpainted frame whose
    /// content is still the decoded frame last shown (`shown`) is left unpainted, so moving the
    /// playhead doesn't copy every visited frame into the document.
    pub fn store(&mut self, at: usize, content: &Surface) {
        let Some(last) = self.frames.len().checked_sub(1) else { return };
        let at = at.min(last);
        if let Some(f) = &mut self.footage {
            let altered = f.altered.get(at).copied().unwrap_or(true);
            if !altered && self.shown.is_some_and(|g| g == content.generation()) {
                return;
            }
            if let Some(a) = Arc::make_mut(&mut f.altered).get_mut(at) {
                *a = true;
            }
        }
        Arc::make_mut(&mut self.frames)[at] = content.clone();
    }

    /// After loading a document: the layer's saved content stands for the frame at `at` as it was
    /// last shown, so an unpainted footage frame there stays unpainted until the content is
    /// actually edited (a save keeps no `shown`, the pixel state of the decode).
    pub fn adopt_loaded(&mut self, at: usize, content: &Surface) {
        let Some(f) = &self.footage else { return };
        let at = at.min(self.frames.len().saturating_sub(1));
        if !f.altered.get(at).copied().unwrap_or(true) {
            self.shown = Some(content.generation());
        }
    }

    /// Insert a blank frame at `at` (clamped to the end). A blank frame has no footage behind it.
    pub fn insert_blank(&mut self, at: usize, format: photocraft_color::PixelFormat) {
        let at = at.min(self.frames.len());
        Arc::make_mut(&mut self.frames).insert(at, Surface::new(format));
        if let Some(original) = &mut self.original_frames {
            // The stacks stay aligned, but never index past a shorter one.
            original.insert(at.min(original.len()), Surface::new(format));
        }
        if let Some(f) = &mut self.footage {
            let at_origin = at.min(f.origin.len());
            Arc::make_mut(&mut f.origin).insert(at_origin, None);
            // Its own (empty) pixels are what it shows.
            let at_altered = at.min(f.altered.len());
            Arc::make_mut(&mut f.altered).insert(at_altered, true);
        }
    }

    /// Insert a copy of frame `at` after it, painted pixels and footage frame alike.
    pub fn duplicate(&mut self, at: usize) {
        let Some(frame) = self.frames.get(at).cloned() else { return };
        if let Some(original) = &mut self.original_frames {
            let orig = original.get(at).cloned().unwrap_or_else(|| Surface::new(frame.format()));
            original.insert((at + 1).min(original.len()), orig);
        }
        if let Some(f) = &mut self.footage {
            let origin = f.origin.get(at).copied().flatten();
            let altered = f.altered.get(at).copied().unwrap_or(true);
            let (o, a) = ((at + 1).min(f.origin.len()), (at + 1).min(f.altered.len()));
            Arc::make_mut(&mut f.origin).insert(o, origin);
            Arc::make_mut(&mut f.altered).insert(a, altered);
        }
        Arc::make_mut(&mut self.frames).insert(at + 1, frame);
    }

    /// Remove frame `at`, keeping at least one frame.
    pub fn remove(&mut self, at: usize) {
        if self.frames.len() <= 1 || at >= self.frames.len() {
            return;
        }
        Arc::make_mut(&mut self.frames).remove(at);
        if let Some(original) = &mut self.original_frames
            && at < original.len()
        {
            original.remove(at);
        }
        if let Some(f) = &mut self.footage {
            if at < f.origin.len() {
                Arc::make_mut(&mut f.origin).remove(at);
            }
            if at < f.altered.len() {
                Arc::make_mut(&mut f.altered).remove(at);
            }
        }
    }

    /// Put frame `at` back to its original footage (blank for a frame without footage).
    pub fn restore(&mut self, at: usize, format: photocraft_color::PixelFormat) {
        if at >= self.frames.len() {
            return;
        }
        if let Some(f) = &mut self.footage {
            let has_origin = f.origin.get(at).copied().flatten().is_some();
            if let Some(a) = Arc::make_mut(&mut f.altered).get_mut(at) {
                // A blank frame shows its own (now empty) pixels again.
                *a = !has_origin;
            }
            Arc::make_mut(&mut self.frames)[at] = Surface::new(format);
            return;
        }
        let original = self.original_frames.as_ref().and_then(|frames| frames.get(at)).cloned();
        Arc::make_mut(&mut self.frames)[at] = original.unwrap_or_else(|| Surface::new(format));
    }

    /// The footage frame count the layer as displayed reads from its file: one more than the
    /// highest footage frame shown (painted frames read none while Show Altered Video is on);
    /// 0 when nothing shown comes from the file.
    pub fn footage_frames_shown(&self) -> usize {
        self.footage_frames_shown_at(0..self.frames.len())
    }

    /// [`Self::footage_frames_shown`] over the playhead positions `at` only (each clamped to the
    /// last frame, as the display is), for output that visits only those.
    pub fn footage_frames_shown_at(&self, at: impl IntoIterator<Item = usize>) -> usize {
        at.into_iter()
            .filter_map(|i| match self.view(i, !self.show_altered) {
                Some(FrameView::Footage(n)) => Some(n.saturating_add(1)),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }

    /// The footage frame count this layer needs from its file: one more than the highest footage
    /// frame any of its frames shows (0 without linked footage).
    pub fn footage_frames_used(&self) -> usize {
        self.footage.as_ref().map_or(0, |f| f.origin.iter().flatten().map(|n| n.saturating_add(1)).max().unwrap_or(0))
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
            && self.footage == o.footage
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
