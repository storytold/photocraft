//! How an animated GIF or WebP opens (Preferences › File Handling, or the prompt File › Open
//! shows when they say Ask). The file reads as a frame animation, a layer per frame
//! (`photocraft_io`'s flat import), and the choice reshapes that document before it opens: kept
//! as frames, turned into one video layer on the timeline, or cut to its first frame.

use photocraft_doc::{Document, Layer, LayerContent, Timeline, VideoData, VideoSource};
use serde_json::{Value, json};

use crate::prefs::{AnimatedOpen, Preferences};
use crate::{EngineError, Result};

/// The preference that answers for a file called `name` (`"animatedGif"`), by its extension;
/// `None` for a format that has none.
pub fn pref_key(name: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "gif" => Some("animatedGif"),
        "webp" => Some("animatedWebp"),
        _ => None,
    }
}

/// The preferred choice for `name`.
fn preferred(prefs: &Preferences, name: &str) -> Option<AnimatedOpen> {
    Some(match pref_key(name)? {
        "animatedGif" => prefs.file_handling.animated_gif,
        _ => prefs.file_handling.animated_webp,
    })
}

/// Whether `doc`, just read from `name`, is an animation the choice applies to.
fn is_animation(name: &str, doc: &Document) -> bool {
    pref_key(name).is_some() && doc.frame_animation.as_ref().is_some_and(|a| a.frames.len() > 1)
}

/// What opening `doc` (just read from `name`) has to ask: `{format, key, frames}` when it is
/// an animation and the preference says Ask, else `None`.
pub fn question(prefs: &Preferences, name: &str, doc: &Document) -> Option<Value> {
    if !is_animation(name, doc) || preferred(prefs, name) != Some(AnimatedOpen::Ask) {
        return None;
    }
    let key = pref_key(name)?;
    let format = if key == "animatedGif" { "GIF" } else { "WebP" };
    let frames = doc.frame_animation.as_ref().map_or(0, |a| a.frames.len());
    Some(json!({"format": format, "key": key, "frames": frames}))
}

/// Applies the preferred choice to `doc` when it is an animation and the preference names one
/// (Ask leaves it for [`question`]). `path` is the file, for a video layer to reload from.
pub fn apply_preferred(prefs: &Preferences, name: &str, doc: &mut Document, path: Option<&str>) -> Result<()> {
    match preferred(prefs, name) {
        Some(choice) if choice != AnimatedOpen::Ask && is_animation(name, doc) => apply(doc, choice, path),
        _ => Ok(()),
    }
}

/// Reshapes a just-read frame animation (a layer per frame, "Frame 1" at the bottom) as
/// `choice` says. Frames and Ask keep it as it is.
pub fn apply(doc: &mut Document, choice: AnimatedOpen, path: Option<&str>) -> Result<()> {
    match choice {
        AnimatedOpen::Ask | AnimatedOpen::Frames => Ok(()),
        AnimatedOpen::FirstFrame => {
            doc.frame_animation = None;
            doc.layers.truncate(1);
            if let Some(l) = doc.layers.first_mut() {
                // As a still GIF or a WebP with transparency opens.
                l.name = "Layer 0".into();
                l.visible = true;
            }
            Ok(())
        }
        AnimatedOpen::Video => to_video(doc, path),
    }
}

/// One video layer holding every frame, on a timeline at the rate the delays give.
fn to_video(doc: &mut Document, path: Option<&str>) -> Result<()> {
    let Some(animation) = doc.frame_animation.take() else { return Ok(()) };
    let delays: Vec<u32> = animation.frames.iter().map(|f| f.delay_ms).collect();
    let fps = crate::video_cmds::fps_from_delays(&delays).unwrap_or(10.0);
    let frames: Vec<_> = std::mem::take(&mut doc.layers)
        .into_iter()
        .filter_map(|l| match l.content {
            LayerContent::Raster(s) => Some(s),
            _ => None,
        })
        .collect();
    if frames.is_empty() {
        return Err(EngineError::Other("the animation has no frames".into()));
    }
    let n = frames.len();
    let stem = std::path::Path::new(&doc.name).file_stem().and_then(|s| s.to_str()).unwrap_or("Video").to_string();
    let mut layer = Layer::raster(stem, doc.pixel_format());
    let mut v = VideoData::new(frames, fps);
    v.original_frames = Some(v.frames.clone());
    if let Some(p) = path {
        v.source = VideoSource::File { path: p.to_string() };
    }
    layer.video = Some(v);
    doc.layers.push(layer);
    doc.timeline = Some(Timeline::new(n, fps));
    crate::video_cmds::sync(doc);
    Ok(())
}

#[cfg(test)]
mod tests;
