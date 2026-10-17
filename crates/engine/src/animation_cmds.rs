//! Window › Timeline in frame-animation mode: create and drive the document's [`FrameAnimation`]
//! (the frame strip). Each frame holds a delay and which layers it shows; selecting a frame shows
//! it on the layers. Navigation adds no history step; every change to the frames does one.
//! Visibility edits made while a frame is selected change that frame (see `Session::edit`).

use std::sync::Arc;

use photocraft_doc::{
    AnimFrame, Document, FrameAnimation, LoopCount,
    frame_animation::{MAX_DELAY_MS, MAX_FRAMES},
};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document".into())
}

fn has_animation(s: &Session) -> std::result::Result<(), String> {
    match s.active() {
        Some(d) if d.doc.frame_animation.is_some() => Ok(()),
        Some(_) => Err("no frame animation (create one first)".into()),
        None => Err("no document".into()),
    }
}

fn bad(cmd: &str, msg: String) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg }
}

fn animation<'a>(doc: &'a mut Document, cmd: &str) -> Result<&'a mut FrameAnimation> {
    doc.frame_animation.as_mut().ok_or_else(|| bad(cmd, "no frame animation (create one first)".into()))
}

/// `frame` (or the selected frame when absent) as an index into `a`.
fn frame_param(cmd: &str, p: &Value, key: &str, a: &FrameAnimation) -> Result<usize> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(a.current) };
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n < a.frames.len())
        .ok_or_else(|| bad(cmd, format!("`{key}` must be a frame from 0 to {}, got {v}", a.frames.len() - 1)))
}

/// A delay in milliseconds from `delayMs` or `seconds`, if either is given.
fn delay_param(cmd: &str, p: &Value) -> Result<Option<u32>> {
    let ms = match (p.get("delayMs").filter(|v| !v.is_null()), p.get("seconds").filter(|v| !v.is_null())) {
        (Some(v), _) => v.as_f64().ok_or_else(|| bad(cmd, format!("`delayMs` must be a number, got {v}")))?,
        (None, Some(v)) => v.as_f64().ok_or_else(|| bad(cmd, format!("`seconds` must be a number, got {v}")))? * 1000.0,
        (None, None) => return Ok(None),
    };
    if !ms.is_finite() || !(0.0..=f64::from(MAX_DELAY_MS)).contains(&ms) {
        return Err(bad(cmd, format!("the delay must be from 0 to {} seconds", MAX_DELAY_MS / 1000)));
    }
    Ok(Some(ms.round() as u32))
}

/// Run `f` on the document without a history step (frame navigation), keeping a clean document
/// clean as selecting a layer does.
fn navigate(s: &mut Session, f: impl FnOnce(&mut Document) -> Result<()>) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let mut doc = (*st.doc).clone();
    f(&mut doc)?;
    let clean = st.saved_revision == st.revision;
    st.doc = Arc::new(doc);
    st.revision += 1;
    if clean {
        st.saved_revision = st.revision;
    }
    st.last_damage = None;
    info(s)
}

fn create(s: &mut Session, _p: &Value) -> Result<Value> {
    if s.active().ok_or(EngineError::NoDocument)?.doc.frame_animation.is_some() {
        return Err(EngineError::Other("the document already has a frame animation".into()));
    }
    s.edit("Create Frame Animation", |doc, _| {
        doc.frame_animation = Some(FrameAnimation::new(&doc.layers));
        Ok(())
    })?;
    info(s)
}

fn delete(s: &mut Session, _p: &Value) -> Result<Value> {
    s.edit("Delete Frame Animation", |doc, _| {
        // The layers keep what the selected frame showed.
        doc.frame_animation = None;
        Ok(())
    })?;
    info(s)
}

/// Make Frames From Layers: one frame per top-level layer, each showing that layer alone, the
/// bottom layer first.
fn make_frames_from_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let delay = delay_param("animation.makeFramesFromLayers", p)?.unwrap_or(0);
    let n = s.active().ok_or(EngineError::NoDocument)?.doc.layers.len();
    if n == 0 {
        return Err(EngineError::Other("the document has no layers to make frames from".into()));
    }
    if n > MAX_FRAMES {
        return Err(EngineError::Other(format!("{n} layers is more than the {MAX_FRAMES} frames an animation holds")));
    }
    s.edit("Make Frames From Layers", |doc, _| {
        let mut a = doc.frame_animation.take().unwrap_or_else(|| FrameAnimation::new(&doc.layers));
        a.frames = (0..n)
            .map(|shown| {
                let mut hidden: Vec<_> = doc.layers.iter().enumerate().filter(|(i, _)| *i != shown).map(|(_, l)| l.id).collect();
                hidden.sort();
                AnimFrame { delay_ms: delay, hidden }
            })
            .collect();
        a.current = 0;
        a.apply(0, &mut doc.layers);
        doc.frame_animation = Some(a);
        Ok(())
    })?;
    info(s)
}

/// Duplicate Selected Frame: a copy after it, selected.
fn new_frame(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if st.doc.frame_animation.as_ref().is_some_and(|a| a.frames.len() >= MAX_FRAMES) {
        return Err(EngineError::Other(format!("an animation holds at most {MAX_FRAMES} frames")));
    }
    s.edit("New Frame", |doc, _| {
        let a = animation(doc, "animation.newFrame")?;
        a.clamp();
        let copy = a.frames[a.current].clone();
        a.current += 1;
        a.frames.insert(a.current, copy);
        Ok(())
    })?;
    info(s)
}

fn delete_frame(s: &mut Session, p: &Value) -> Result<Value> {
    s.edit("Delete Frame", |doc, _| {
        let a = animation(doc, "animation.deleteFrame")?;
        a.clamp();
        let at = frame_param("animation.deleteFrame", p, "frame", a)?;
        if a.frames.len() < 2 {
            return Err(EngineError::Other("an animation keeps at least one frame; delete the animation instead".into()));
        }
        a.frames.remove(at);
        a.current = if at < a.current || a.current >= a.frames.len() { a.current.saturating_sub(1) } else { a.current };
        let cur = a.current;
        doc.select_animation_frame(cur);
        Ok(())
    })?;
    info(s)
}

fn select_frame(s: &mut Session, p: &Value) -> Result<Value> {
    navigate(s, |doc| {
        let a = animation(doc, "animation.selectFrame")?;
        a.clamp();
        let v = p.get("frame").ok_or_else(|| bad("animation.selectFrame", "need `frame`".into()))?;
        let at = v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < a.frames.len())
            .ok_or_else(|| bad("animation.selectFrame", format!("`frame` must be from 0 to {}, got {v}", a.frames.len() - 1)))?;
        doc.select_animation_frame(at);
        Ok(())
    })
}

/// Step the selected frame by `delta`; `wrap` goes round the ends (playback), else it stops there.
fn step(s: &mut Session, delta: i64, wrap: bool) -> Result<Value> {
    navigate(s, |doc| {
        let a = animation(doc, "animation.nextFrame")?;
        a.clamp();
        let n = a.frames.len() as i64;
        let to = a.current as i64 + delta;
        let to = if wrap { to.rem_euclid(n) } else { to.clamp(0, n - 1) };
        doc.select_animation_frame(to as usize);
        Ok(())
    })
}

fn set_delay(s: &mut Session, p: &Value) -> Result<Value> {
    let delay = delay_param("animation.setDelay", p)?.ok_or_else(|| bad("animation.setDelay", "need `seconds` or `delayMs`".into()))?;
    s.edit("Frame Delay", |doc, _| {
        let a = animation(doc, "animation.setDelay")?;
        a.clamp();
        let targets: Vec<usize> = match p.get("frames") {
            None | Some(Value::Null) => vec![a.current],
            Some(Value::String(w)) if w == "all" => (0..a.frames.len()).collect(),
            Some(Value::Array(v)) => {
                let mut out = Vec::new();
                for i in v {
                    let i = i
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .filter(|n| *n < a.frames.len())
                        .ok_or_else(|| bad("animation.setDelay", format!("`frames` holds {i}, which is not a frame from 0 to {}", a.frames.len() - 1)))?;
                    out.push(i);
                }
                out
            }
            Some(v) => return Err(bad("animation.setDelay", format!("`frames` must be \"all\" or a list of frames, got {v}"))),
        };
        for i in targets {
            a.frames[i].delay_ms = delay;
        }
        Ok(())
    })?;
    info(s)
}

fn set_loop(s: &mut Session, p: &Value) -> Result<Value> {
    let loops = match p.get("loops") {
        Some(Value::String(w)) if w == "forever" => LoopCount::Forever,
        Some(Value::String(w)) if w == "once" => LoopCount::Once,
        Some(v) => match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
            Some(n) => LoopCount::from_plays(n),
            None => return Err(bad("animation.setLoop", format!("`loops` must be \"once\", \"forever\" or a number of plays, got {v}"))),
        },
        None => return Err(bad("animation.setLoop", "need `loops`".into())),
    };
    s.edit("Loop Count", |doc, _| {
        animation(doc, "animation.setLoop")?.loops = loops;
        Ok(())
    })?;
    info(s)
}

/// Move frame `from` to position `to`; the selection stays on the same frame.
fn move_frame(s: &mut Session, p: &Value) -> Result<Value> {
    s.edit("Move Frame", |doc, _| {
        let a = animation(doc, "animation.moveFrame")?;
        a.clamp();
        let from = frame_param("animation.moveFrame", p, "from", a)?;
        let to = frame_param("animation.moveFrame", p, "to", a)?;
        let f = a.frames.remove(from);
        a.frames.insert(to, f);
        if a.current == from {
            a.current = to;
        } else if from < a.current && a.current <= to {
            a.current -= 1;
        } else if to <= a.current && a.current < from {
            a.current += 1;
        }
        Ok(())
    })?;
    info(s)
}

fn reverse_frames(s: &mut Session, _p: &Value) -> Result<Value> {
    s.edit("Reverse Frames", |doc, _| {
        let a = animation(doc, "animation.reverseFrames")?;
        a.clamp();
        a.frames.reverse();
        a.current = a.frames.len() - 1 - a.current;
        Ok(())
    })?;
    info(s)
}

pub(crate) fn info_of(doc: &Document) -> Value {
    match &doc.frame_animation {
        Some(a) => json!({"animation": {
            "current": a.current,
            "loops": match a.loops { LoopCount::Once => json!("once"), LoopCount::Forever => json!("forever"), LoopCount::Times(n) => json!(n) },
            "durationMs": a.duration_ms(),
            "frames": a.frames.iter().map(|f| json!({"delayMs": f.delay_ms, "hidden": f.hidden.iter().map(|l| l.0).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        }}),
        None => json!({"animation": null}),
    }
}

fn info(s: &mut Session) -> Result<Value> {
    Ok(info_of(&s.active().ok_or(EngineError::NoDocument)?.doc))
}

macro_rules! spec {
    ($id:expr, $label:expr, $params:expr, $en:expr, $journal:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: $en, journal: $journal, run: $run }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("animation.create", "Create Frame Animation", "{} → {animation}", has_doc, true, |s, p| create(s, p)),
        spec!("animation.delete", "Delete Frame Animation", "{} → {animation:null}", has_animation, true, |s, p| delete(s, p)),
        spec!("animation.makeFramesFromLayers", "Make Frames From Layers", r#"{seconds?:0, delayMs?} → {animation}"#, has_doc, true, |s, p| {
            make_frames_from_layers(s, p)
        }),
        spec!("animation.newFrame", "Duplicate Selected Frame", "{} → {animation}", has_animation, true, |s, p| new_frame(s, p)),
        spec!("animation.deleteFrame", "Delete Frame", r#"{frame?:selected} → {animation}"#, has_animation, true, |s, p| delete_frame(s, p)),
        spec!("animation.selectFrame", "Select Frame", r#"{frame} → {animation}"#, has_animation, false, |s, p| select_frame(s, p)),
        spec!("animation.nextFrame", "Next Frame", "{} → {animation}", has_animation, false, |s, _| step(s, 1, false)),
        spec!("animation.previousFrame", "Previous Frame", "{} → {animation}", has_animation, false, |s, _| step(s, -1, false)),
        spec!("animation.setDelay", "Frame Delay", r#"{seconds | delayMs, frames?:[i..] | "all" = selected} → {animation}"#, has_animation, true, |s, p| {
            set_delay(s, p)
        }),
        spec!("animation.setLoop", "Loop Count", r#"{loops:"once"|"forever"|plays} → {animation}"#, has_animation, true, |s, p| set_loop(s, p)),
        spec!("animation.moveFrame", "Move Frame", r#"{from, to} → {animation}"#, has_animation, true, |s, p| move_frame(s, p)),
        spec!("animation.reverseFrames", "Reverse Frames", "{} → {animation}", has_animation, true, |s, p| reverse_frames(s, p)),
        spec!("animation.info", "Frame Animation Info", "{} → {animation}", has_doc, false, |s, _| info(s)),
    ]
}

#[cfg(test)]
mod tests;
