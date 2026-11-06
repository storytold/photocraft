//! How an animated GIF or WebP opens. File › Open follows Preferences › File Handling, or shows a
//! prompt when they say Ask. Each choice reads the file its own way: a frame animation with a layer
//! per frame (`photocraft_io`'s flat import with [`photocraft_io::AnimationImport::Frames`]), one
//! video layer on the timeline ([`open_as_video`]), or the first frame like any flat image. Asking
//! reads the first frame and its frame count, and only Frames or Video reads the file again.
//! Headless opens (scripts, the CLI, MCP) read the first frame only, unless the caller asks for
//! frames or a video layer ([`requested`]).

use photocraft_codecs::DecodeWarning;
use photocraft_doc::{ColorMode, Document, Layer, Timeline, VideoData, VideoSource};
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

/// File Handling's choice for `name`; `None` for a format that has none.
pub fn preferred(prefs: &Preferences, name: &str) -> Option<AnimatedOpen> {
    Some(match pref_key(name)? {
        "animatedGif" => prefs.file_handling.animated_gif,
        _ => prefs.file_handling.animated_webp,
    })
}

/// How File › Open first reads `name`: an animated GIF's or WebP's frames when File Handling says
/// to open them as frames, else the first frame like any flat image.
pub fn import_mode(prefs: &Preferences, name: &str) -> photocraft_io::AnimationImport {
    preferred(prefs, name).map_or(photocraft_io::AnimationImport::FirstFrame, import_for)
}

/// What the import has to read for `choice`: a layer per frame for a frame animation, else the
/// first frame (a video layer reads its frames as footage, [`open_as_video`]).
pub fn import_for(choice: AnimatedOpen) -> photocraft_io::AnimationImport {
    match choice {
        AnimatedOpen::Frames => photocraft_io::AnimationImport::Frames,
        AnimatedOpen::Ask | AnimatedOpen::Video | AnimatedOpen::FirstFrame => photocraft_io::AnimationImport::FirstFrame,
    }
}

/// How an automation caller (MCP and RPC `doc.open`, the bridge's `app.open`) asks an animated
/// GIF or WebP to open, from its `animation` parameter: `firstFrame` (also when it doesn't say,
/// as before frame animations), `frames` or `video`. Automation never prompts, so `ask` is refused.
pub fn requested(param: Option<&str>) -> Result<AnimatedOpen> {
    match param {
        None => Ok(AnimatedOpen::FirstFrame),
        Some(v) => AnimatedOpen::parse(v)
            .filter(|c| *c != AnimatedOpen::Ask)
            .ok_or_else(|| EngineError::BadParams { cmd: "doc.open".into(), msg: format!("`animation` is firstFrame, frames or video, not `{v}`") }),
    }
}

/// Whether a read's warnings say the file has frames past the one that was read.
pub fn more_frames(warnings: &[String]) -> bool {
    warnings.iter().any(|w| DecodeWarning::more_frames_in(w).is_some())
}

/// What opening `name` has to ask, given the warnings of reading its first frame:
/// `{format, key, frames}` when it is an animated GIF or WebP. `frames` is null for a damaged file
/// whose frames couldn't be counted.
pub fn question(name: &str, warnings: &[String]) -> Option<Value> {
    let key = pref_key(name)?;
    let frames = warnings.iter().find_map(|w| DecodeWarning::more_frames_in(w))?;
    let format = if key == "animatedGif" { "GIF" } else { "WebP" };
    Some(json!({"format": format, "key": key, "frames": frames}))
}

/// Turns `doc`, the first frame just read from the animated GIF or WebP in `bytes`, into one video
/// layer holding every frame, on a timeline at the rate the delays give, as File › Open and
/// automation ask for. The frames are read as footage is ([`crate::video_cmds::animation_footage`]),
/// so the footage limits apply rather than the layer budget of a frame animation. Anything else
/// (a still image, a native document) is left alone. `path` is the file, for the layer to reload
/// from; `ctl` stops the read between frames.
pub fn open_as_video(
    doc: &mut Document,
    warnings: &mut Vec<String>,
    bytes: &[u8],
    path: Option<&str>,
    opts: &photocraft_io::ImportOptions,
    ctl: &photocraft_raster::Interrupt,
) -> Result<()> {
    let Some(total) = warnings.iter().find_map(|w| DecodeWarning::more_frames_in(w)) else { return Ok(()) };
    if !matches!(photocraft_codecs::detect(bytes), Some(photocraft_codecs::Format::Gif | photocraft_codecs::Format::WebP)) {
        return Ok(());
    }
    // A video layer's frames are full colour, whatever the first frame's palette was.
    doc.mode = ColorMode::Rgb;
    doc.color_table = None;
    let fmt = doc.pixel_format();
    let footage = crate::video_cmds::animation_footage(&doc.name.clone(), bytes, fmt, opts, ctl)?;
    let n = footage.frames.len();
    let fps = footage.fps.unwrap_or(10.0);
    let stem = std::path::Path::new(&doc.name).file_stem().and_then(|s| s.to_str()).unwrap_or("Video").to_string();
    let mut layer = Layer::raster(stem, fmt);
    let mut v = VideoData::new(footage.frames, fps);
    v.original_frames = Some(v.frames.clone());
    if let Some(p) = path {
        v.source = VideoSource::File { path: p.to_string() };
    }
    layer.video = Some(v);
    doc.layers = vec![layer];
    doc.timeline = Some(Timeline::new(n, fps));
    crate::video_cmds::sync(doc);
    warnings.retain(|w| DecodeWarning::more_frames_in(w).is_none());
    if total != u32::try_from(n).ok() {
        warnings.push(format!("the animation is damaged: only {n} of its frames could be read"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
