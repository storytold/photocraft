//! Layer › Frame Range: the frames of the video timeline a layer shows on, so a paint layer can
//! cover one frame of a shot, or a few, while the plate plays beneath it. The range is the layer's
//! [`FrameSpan`]; outside it the layer is hidden (see `video_cmds::apply_spans`). The Timeline
//! panel draws each layer's range as a bar and its drags run these commands, so agents and the
//! control channel can do everything the panel does.
//!
//! Frames are 0-based, and `end` is the frame after the last one shown, as the timeline's
//! `workEnd` is.

use photocraft_doc::{Document, FrameSpan, Layer, LayerId};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::video_cmds::{show_spans, store_spans};
use crate::{EngineError, Result, Session};

const MENU: &[&str] = &["Layer", "Frame Range"];

fn has_timeline(s: &Session) -> std::result::Result<(), String> {
    match s.active() {
        Some(d) if d.doc.timeline.is_some() => Ok(()),
        Some(_) => Err("no timeline (Window › Timeline › Create Video Timeline)".into()),
        None => Err("no document".into()),
    }
}

/// A timeline and an active layer.
fn has_layer(s: &Session) -> std::result::Result<(), String> {
    has_timeline(s)?;
    s.active().and_then(|d| d.active_layer).map(|_| ()).ok_or_else(|| "no active layer".into())
}

/// The active layer has a frame range of its own.
fn has_range(s: &Session) -> std::result::Result<(), String> {
    has_layer(s)?;
    let d = s.active().ok_or("no document")?;
    match d.active_layer.and_then(|id| d.doc.layer(id)) {
        Some(l) if l.span.is_some() => Ok(()),
        _ => Err("the layer shows on every frame".into()),
    }
}

/// The timeline's playhead and length.
fn clock(doc: &Document) -> Result<(usize, usize)> {
    let t = doc.timeline.as_ref().ok_or_else(|| EngineError::Other("no timeline (Window › Timeline › Create Video Timeline)".into()))?;
    Ok((t.current, t.duration.max(1)))
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// A whole-number frame param.
fn frame_param(cmd: &str, p: &Value, key: &str) -> Result<Option<usize>> {
    match p.get(key).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => v.as_u64().and_then(|f| usize::try_from(f).ok()).map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be a whole frame number, got {v}"))),
    }
}

/// The range a layer shows on now: its own, or every frame of the timeline.
fn range_of(l: &Layer, duration: usize) -> FrameSpan {
    l.span.unwrap_or(FrameSpan { start: 0, end: duration, eye: l.visible })
}

/// Changes the frame range of `layer` as one history step: the eyes are recorded first, so the
/// visibility the layer gets afterwards is its eye on its new frames.
fn edit_range(s: &mut Session, label: &str, layer: LayerId, f: impl FnOnce(Option<FrameSpan>, usize, usize) -> Result<Option<FrameSpan>>) -> Result<Value> {
    s.edit(label, |doc, _| {
        let (cur, duration) = clock(doc)?;
        store_spans(doc);
        let l = doc.layer_mut(layer).ok_or_else(|| EngineError::Other("no such layer".into()))?;
        let eye = l.span.map_or(l.visible, |sp| sp.eye);
        let next = f(l.span, cur, duration)?;
        // Leaving a range shows the layer as its eye says, on every frame.
        l.visible = eye;
        l.span = next.map(|sp| FrameSpan { eye, ..sp });
        show_spans(doc);
        Ok(())
    })?;
    info(s, layer)
}

/// `{layer, range: {start, end} | null, visible}` for `layer`.
fn info(s: &Session, layer: LayerId) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(layer).ok_or_else(|| EngineError::Other("no such layer".into()))?;
    Ok(json!({
        "layer": layer.0,
        "range": l.span.map(|sp| json!({"start": sp.start, "end": sp.end})),
        "visible": l.visible,
    }))
}

/// `layer.frameRange.set {layer?, start, end}`: the layer shows on frames `start..end`.
fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.frameRange.set";
    let layer = layer_param(s, p)?;
    let start = frame_param(CMD, p, "start")?.ok_or_else(|| bad(CMD, "need `start`"))?;
    let end = frame_param(CMD, p, "end")?.ok_or_else(|| bad(CMD, "need `end`"))?;
    edit_range(s, "Set Frame Range", layer, |_, _, duration| {
        if start >= end {
            return Err(bad(CMD, format!("`end` ({end}) must be after `start` ({start})")));
        }
        if end > duration {
            return Err(bad(CMD, format!("`end` ({end}) is past the timeline's {duration} frames")));
        }
        Ok(Some(FrameSpan::new(start, end)))
    })
}

/// `layer.frameRange.clear {layer?}`: the layer shows on every frame again.
fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    let layer = layer_param(s, p)?;
    edit_range(s, "Show on Every Frame", layer, |_, _, _| Ok(None))
}

/// `layer.frameRange.startAtPlayhead {layer?}`: the range starts at the playhead, keeping its end
/// (the timeline's end for a layer on every frame). A playhead at or past the end gives a range
/// of that one frame.
fn start_at_playhead(s: &mut Session, p: &Value) -> Result<Value> {
    let layer = layer_param(s, p)?;
    edit_range(s, "Start at Playhead", layer, |span, cur, duration| {
        let end = span.map_or(duration, |sp| sp.end);
        Ok(Some(FrameSpan::new(cur, end.max(cur + 1).min(duration.max(cur + 1)))))
    })
}

/// `layer.frameRange.endAtPlayhead {layer?}`: the range ends with the playhead's frame, keeping its
/// start (frame 0 for a layer on every frame). A playhead before the start gives a range of that
/// one frame.
fn end_at_playhead(s: &mut Session, p: &Value) -> Result<Value> {
    let layer = layer_param(s, p)?;
    edit_range(s, "End at Playhead", layer, |span, cur, _| {
        let start = span.map_or(0, |sp| sp.start).min(cur);
        Ok(Some(FrameSpan::new(start, cur + 1)))
    })
}

/// `layer.frameRange.newLayerForFrame {}`: a new empty pixel layer above the active one, shown on
/// the playhead's frame only, named after that frame (1-based, as the Timeline panel counts) and
/// made active, ready to paint.
fn new_layer_for_frame(s: &mut Session, _p: &Value) -> Result<Value> {
    let id = s.edit("New Layer for This Frame", |doc, active| {
        let (cur, _) = clock(doc)?;
        let fmt = doc.pixel_format();
        let base = format!("Frame {}", cur + 1);
        let taken = doc.walk().into_iter().any(|(_, _, l)| l.name == base);
        let name = if taken { doc.copy_name(&base) } else { base };
        let mut l = Layer::raster(name, fmt);
        l.span = Some(FrameSpan::new(cur, cur + 1));
        let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::AutoEditable);
        *active = Some(id);
        Ok(id)
    })?;
    info(s, id)
}

/// `layer.frameRange.split {layer?}`: cuts the layer's range at the playhead into two layers: the
/// layer keeps the frames before the playhead and a copy of it (same pixels, mask and style) takes
/// the playhead's frame onward, made active. The playhead must be inside the range, after its
/// first frame.
fn split(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.frameRange.split";
    let layer = layer_param(s, p)?;
    let id = s.edit("Split at Playhead", |doc, active| {
        let (cur, duration) = clock(doc)?;
        store_spans(doc);
        if doc.is_background(layer) {
            return Err(bad(CMD, "the Background can't be split"));
        }
        let l = doc.layer_mut(layer).ok_or_else(|| EngineError::Other("no such layer".into()))?;
        let range = range_of(l, duration);
        if cur <= range.start || cur >= range.end {
            return Err(bad(CMD, format!("move the playhead inside the layer's frames ({}–{}), after the first one", range.start + 1, range.end)));
        }
        l.span = Some(FrameSpan { end: cur, ..range });
        let mut copy = l.duplicate();
        copy.span = Some(FrameSpan { start: cur, ..range });
        let id = crate::layer_place::insert_new(doc, Some(layer), copy, crate::layer_place::Place::Above);
        show_spans(doc);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layers": [info(s, layer)?, info(s, id)?]}))
}

/// The ranges of every layer that has one, top to bottom: `{layers: [{layer, name, range}]}`.
fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    fn walk(layers: &[Layer], out: &mut Vec<Value>) {
        for l in layers.iter().rev() {
            if let Some(sp) = l.span {
                out.push(json!({"layer": l.id.0, "name": l.name, "range": {"start": sp.start, "end": sp.end}, "visible": l.visible}));
            }
            if let Some(ch) = l.children() {
                walk(ch, out);
            }
        }
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let mut out = Vec::new();
    walk(&d.doc.layers, &mut out);
    Ok(json!({"layers": out}))
}

macro_rules! spec {
    ($id:expr, $label:expr, $menu:expr, $params:expr, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $en, journal: true, run: $run }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    let layer = r#"{layer?:id|name (default: the active layer)} → {layer, range:{start,end}|null, visible}"#;
    vec![
        spec!(
            "layer.frameRange.newLayerForFrame",
            "New Layer for This Frame",
            MENU,
            r#"{} → {layer, range:{start,end}, visible} (an empty layer shown on the playhead's frame only)"#,
            has_timeline,
            new_layer_for_frame
        ),
        spec!("layer.frameRange.startAtPlayhead", "Start at Playhead", MENU, layer, has_layer, start_at_playhead),
        spec!("layer.frameRange.endAtPlayhead", "End at Playhead", MENU, layer, has_layer, end_at_playhead),
        spec!(
            "layer.frameRange.split",
            "Split at Playhead",
            MENU,
            r#"{layer?:id|name} → {layers:[before, after]} (the copy from the playhead on is made active)"#,
            has_layer,
            split
        ),
        spec!("layer.frameRange.clear", "Show on Every Frame", MENU, layer, has_range, clear),
        spec!(
            "layer.frameRange.set",
            "Set Frame Range",
            &[],
            r#"{layer?:id|name, start:frame (0-based), end:frame (exclusive, at most the timeline's length)} → {layer, range:{start,end}, visible}"#,
            has_layer,
            set
        ),
        CommandSpec {
            id: "layer.frameRange.list",
            label: "List Frame Ranges",
            menu: &[],
            shortcut: None,
            params: r#"{} → {layers:[{layer, name, range:{start,end}, visible}]} (top to bottom; layers on every frame are left out)"#,
            enabled: crate::cmd::always,
            journal: false,
            run: list,
        },
    ]
}

#[cfg(test)]
mod tests;
