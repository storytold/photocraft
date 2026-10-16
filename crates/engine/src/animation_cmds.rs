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

/// The frames a command works on: the `frames` list, else `frame`, else the selected frame; sorted,
/// without repeats.
fn frames_param(cmd: &str, p: &Value, a: &FrameAnimation) -> Result<Vec<usize>> {
    let Some(list) = p.get("frames").filter(|v| !v.is_null()) else { return Ok(vec![frame_param(cmd, p, "frame", a)?]) };
    let list = list.as_array().filter(|l| !l.is_empty()).ok_or_else(|| bad(cmd, format!("`frames` must be a non-empty list of frames, got {list}")))?;
    let mut out = Vec::with_capacity(list.len());
    for v in list {
        let i = v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < a.frames.len())
            .ok_or_else(|| bad(cmd, format!("`frames` holds {v}, which is not a frame from 0 to {}", a.frames.len() - 1)))?;
        out.push(i);
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// A delay in milliseconds from `delayMs` or `seconds`, if either is given.
fn delay_param(cmd: &str, p: &Value) -> Result<Option<u32>> {
    let ms = match (p.get("delayMs").filter(|v| !v.is_null()), p.get("seconds").filter(|v| !v.is_null())) {
        (Some(v), _) => v.as_f64().ok_or_else(|| bad(cmd, format!("`delayMs` must be a number, got {v}")))?,
        (None, Some(v)) => v.as_f64().ok_or_else(|| bad(cmd, format!("`seconds` must be a number, got {v}")))? * 1000.0,
        (None, None) => return Ok(None),
    };
    if !ms.is_finite() || !(0.0..=f64::from(MAX_DELAY_MS)).contains(&ms) {
        return Err(bad(cmd, format!("the delay must be from 0 to {} seconds", f64::from(MAX_DELAY_MS) / 1000.0)));
    }
    Ok(Some(ms.round() as u32))
}

/// Run `f` on the document without a history step (frame navigation), keeping a clean document
/// clean as selecting a layer does.
fn navigate(s: &mut Session, id: &str, f: impl FnOnce(&mut Document) -> Result<()>) -> Result<Value> {
    // Selecting a frame changes what the layers show, so it waits for a background job on the document
    // (which would discard its result) just as an edit does.
    if let Some(msg) = s.job_conflict(id, true) {
        return Err(EngineError::Other(msg));
    }
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
    // A compact reply: playback calls this on every frame, and `animation.info` has the whole animation.
    let a = s.active().and_then(|st| st.doc.frame_animation.as_ref());
    Ok(json!({"animation": {"current": a.map_or(0, |a| a.current), "frameCount": a.map_or(0, |a| a.frames.len())}}))
}

/// While an action is recorded, note the frame a navigation command selected as an absolute
/// `animation.selectFrame`; one step stands for a run of them (a click, then Next, then Next).
pub(crate) fn record_navigation(s: &mut Session, id: &str) {
    if !matches!(id, "animation.selectFrame" | "animation.nextFrame" | "animation.previousFrame") {
        return;
    }
    let Some(current) = s.active().and_then(|st| st.doc.frame_animation.as_ref()).map(|a| a.current) else { return };
    let from = s.actions.recording.map_or(0, |(from, _)| from);
    let step = ("animation.selectFrame".to_string(), json!({ "frame": current }));
    let len = s.journal.len();
    match s.journal.last_mut() {
        Some(last) if len > from && last.0 == step.0 => *last = step,
        _ => s.journal.push(step),
    }
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

/// Most layers Make Frames From Layers turns into frames.
const MAX_FRAMES_FROM_LAYERS: usize = 1000;

/// The layers in `layers` (and below) whose eye is off.
fn hidden_inside(layers: &[photocraft_doc::Layer], out: &mut Vec<photocraft_doc::LayerId>) {
    for l in layers {
        if !l.visible {
            out.push(l.id);
        }
        if let Some(ch) = l.children() {
            hidden_inside(ch, out);
        }
    }
}

/// Make Frames From Layers: one frame per top-level layer, each showing that layer alone, the
/// bottom layer first.
fn make_frames_from_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let delay = delay_param("animation.makeFramesFromLayers", p)?.unwrap_or(0);
    let n = s.active().ok_or(EngineError::NoDocument)?.doc.layers.len();
    if n == 0 {
        return Err(EngineError::Other("the document has no layers to make frames from".into()));
    }
    // Each frame lists every other layer it hides, so the animation grows with the square of the count.
    if n > MAX_FRAMES_FROM_LAYERS {
        return Err(EngineError::Other(format!("{n} layers is more than the {MAX_FRAMES_FROM_LAYERS} Make Frames From Layers handles")));
    }
    s.edit("Make Frames From Layers", |doc, _| {
        let mut a = doc.frame_animation.take().unwrap_or_else(|| FrameAnimation::new(&doc.layers));
        // Layers inside groups keep their own eye in every frame, not only in the frame that shows their group.
        let mut nested = Vec::new();
        for l in &doc.layers {
            if let Some(children) = l.children() {
                hidden_inside(children, &mut nested);
            }
        }
        a.frames = (0..n)
            .map(|shown| {
                let mut hidden: Vec<_> = doc.layers.iter().enumerate().filter(|(i, _)| *i != shown).map(|(_, l)| l.id).collect();
                hidden.extend_from_slice(&nested);
                hidden.sort();
                hidden.dedup();
                Arc::new(AnimFrame { delay_ms: delay, hidden, ..Default::default() })
            })
            .collect();
        a.fill_opacities(&doc.layers);
        a.current = 0;
        a.apply(0, &mut doc.layers);
        doc.frame_animation = Some(a);
        Ok(())
    })?;
    info(s)
}

/// Duplicate Selected Frames: copies of the `frames` (the selected frame when absent), in order,
/// after the last of them; the copies are selected.
fn new_frame(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let count = frames_param(
        "animation.newFrame",
        p,
        st.doc.frame_animation.as_ref().ok_or_else(|| bad("animation.newFrame", "no frame animation (create one first)".into()))?,
    )?
    .len();
    if st.doc.frame_animation.as_ref().is_some_and(|a| a.frames.len() + count > MAX_FRAMES) {
        return Err(EngineError::Other(format!("an animation holds at most {MAX_FRAMES} frames")));
    }
    let mut selection = Vec::new();
    s.edit("New Frame", |doc, _| {
        let a = animation(doc, "animation.newFrame")?;
        a.clamp();
        let which = frames_param("animation.newFrame", p, a)?;
        let copies: Vec<Arc<AnimFrame>> = which.iter().map(|i| a.frames[*i].clone()).collect();
        let at = which[which.len() - 1] + 1;
        a.frames.splice(at..at, copies);
        selection = (at..at + which.len()).collect();
        a.current = at + which.len() - 1;
        // Show the new selected frame: the edit records what the layers show into it afterwards.
        let cur = a.current;
        doc.select_animation_frame(cur);
        Ok(())
    })?;
    info_selecting(s, selection)
}

/// Delete the `frames` (the selected frame when absent). The selected frame becomes the one that
/// took its place; at least one frame stays.
fn delete_frame(s: &mut Session, p: &Value) -> Result<Value> {
    let mut selection = Vec::new();
    s.edit("Delete Frame", |doc, _| {
        let a = animation(doc, "animation.deleteFrame")?;
        a.clamp();
        let gone = frames_param("animation.deleteFrame", p, a)?;
        if gone.len() >= a.frames.len() {
            return Err(EngineError::Other("an animation keeps at least one frame; delete the animation instead".into()));
        }
        let kept_before = (0..a.current).filter(|i| gone.binary_search(i).is_err()).count();
        let mut i = 0;
        a.frames.retain(|_| {
            i += 1;
            gone.binary_search(&(i - 1)).is_err()
        });
        a.current = kept_before.min(a.frames.len() - 1);
        selection = vec![a.current];
        let cur = a.current;
        doc.select_animation_frame(cur);
        Ok(())
    })?;
    info_selecting(s, selection)
}

fn select_frame(s: &mut Session, p: &Value) -> Result<Value> {
    navigate(s, "animation.selectFrame", |doc| {
        let a = animation(doc, "animation.selectFrame")?;
        let v = p.get("frame").ok_or_else(|| bad("animation.selectFrame", "need `frame`".into()))?;
        let at = v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < a.frames.len())
            .ok_or_else(|| bad("animation.selectFrame", format!("`frame` must be from 0 to {}, got {v}", a.frames.len().saturating_sub(1))))?;
        doc.select_animation_frame(at);
        Ok(())
    })
}

/// Step the selected frame by `delta`; `wrap` goes round the ends (playback), else it stops there.
fn step(s: &mut Session, cmd: &'static str, delta: i64, wrap: bool) -> Result<Value> {
    navigate(s, cmd, |doc| {
        let a = animation(doc, cmd)?;
        // Playback runs this every tick, so it checks only what it needs rather than re-sorting every frame's list.
        if a.frames.is_empty() {
            return Err(EngineError::Other("the animation has no frames".into()));
        }
        let n = a.frames.len() as i64;
        let to = a.current.min(a.frames.len() - 1) as i64 + delta;
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
            // The same checks as every other command that takes frames: a list, not empty, each one a real frame.
            Some(Value::Array(_)) => frames_param("animation.setDelay", p, a)?,
            Some(v) => return Err(bad("animation.setDelay", format!("`frames` must be \"all\" or a list of frames, got {v}"))),
        };
        for i in targets {
            // A frame already at this delay stays shared with the history's copies.
            if a.frames[i].delay_ms != delay
                && let Some(f) = a.frame_mut(i)
            {
                f.delay_ms = delay;
            }
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
    // Both are required: a missing one would otherwise mean "the selected frame" and move the wrong frame.
    for key in ["from", "to"] {
        if p.get(key).is_none_or(Value::is_null) {
            return Err(bad("animation.moveFrame", format!("need `{key}`")));
        }
    }
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

/// Paste Frames: copies of `frames` ({delayMs, hidden:[layer ids]}) inserted after frame `after`
/// (the selected one when absent); the last pasted frame is selected. Ids of layers the document
/// does not have are dropped, so a frame copied from another document still pastes.
fn paste_frames(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "animation.pasteFrames";
    let list = p.get("frames").and_then(Value::as_array).filter(|l| !l.is_empty()).ok_or_else(|| bad(CMD, "`frames` must be a non-empty list".into()))?;
    let mut pasted = Vec::with_capacity(list.len());
    for f in list {
        if !f.is_object() {
            return Err(bad(CMD, format!("each of `frames` must be {{delayMs, hidden}}, got {f}")));
        }
        let delay_ms = delay_param(CMD, f)?.unwrap_or(0);
        let ids = match f.get("hidden") {
            None | Some(Value::Null) => Vec::new(),
            Some(v) => v
                .as_array()
                .and_then(|l| l.iter().map(Value::as_u64).collect::<Option<Vec<_>>>())
                .ok_or_else(|| bad(CMD, format!("`hidden` must be a list of layer ids, got {v}")))?,
        };
        let opacities = match f.get("opacities") {
            None | Some(Value::Null) => Vec::new(),
            Some(v) => v
                .as_array()
                .and_then(|l| {
                    l.iter()
                        .map(|pair| {
                            let pair = pair.as_array().filter(|p| p.len() == 2)?;
                            Some((pair[0].as_u64()?, pair[1].as_f64().filter(|o| o.is_finite())? as f32))
                        })
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or_else(|| bad(CMD, format!("`opacities` must be a list of [layer id, opacity] pairs, got {v}")))?,
        };
        pasted.push((delay_ms, ids, opacities));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if st.doc.frame_animation.as_ref().is_some_and(|a| a.frames.len() + pasted.len() > MAX_FRAMES) {
        return Err(EngineError::Other(format!("an animation holds at most {MAX_FRAMES} frames")));
    }
    let mut selection = Vec::new();
    s.edit("Paste Frames", |doc, _| {
        let live = photocraft_doc::layer_ids(&doc.layers);
        let known = |id: u64| live.contains(&photocraft_doc::LayerId(id));
        let a = animation(doc, CMD)?;
        a.clamp();
        let frames: Vec<Arc<AnimFrame>> = pasted
            .iter()
            .map(|(delay_ms, ids, opacities)| {
                let mut hidden: Vec<_> = ids.iter().copied().filter(|id| known(*id)).map(photocraft_doc::LayerId).collect();
                hidden.sort();
                hidden.dedup();
                let mut frame = AnimFrame { delay_ms: *delay_ms, hidden, ..Default::default() };
                for (id, o) in opacities.iter().filter(|(id, _)| known(*id)) {
                    let id = photocraft_doc::LayerId(*id);
                    frame.set_opacity(id, o.clamp(0.0, 1.0), a.default_opacity(id));
                }
                Arc::new(frame)
            })
            .collect();
        let after = frame_param(CMD, p, "after", a)?;
        let at = after + 1;
        let n = frames.len();
        a.frames.splice(at..at, frames);
        a.current = at + n - 1;
        selection = (at..at + n).collect();
        let mut a = a.clone();
        a.fill_opacities(&doc.layers);
        a.apply(a.current, &mut doc.layers);
        doc.frame_animation = Some(a);
        Ok(())
    })?;
    info_selecting(s, selection)
}

/// Most frames one tween adds.
pub const MAX_TWEEN: u64 = 500;

/// Tween: `frames` new frames between the selected frame and the one it is tweened `with`
/// ("previous", "next", "first" or "last"), fading each of the `layers` (all by default) from its
/// look in the earlier of the two to its look in the later one. A hidden layer counts as opacity 0,
/// so a layer shown in only one of the frames fades in or out. The new frames take the selected
/// frame's delay and are selected.
fn tween(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "animation.tween";
    let n = match p.get("frames") {
        None | Some(Value::Null) => 1,
        Some(v) => v.as_u64().filter(|n| (1..=MAX_TWEEN).contains(n)).ok_or_else(|| bad(CMD, format!("`frames` must be from 1 to {MAX_TWEEN}, got {v}")))?,
    } as usize;
    let with = match p.get("with") {
        None | Some(Value::Null) => "previous".to_string(),
        Some(Value::String(w)) => w.clone(),
        Some(v) => return Err(bad(CMD, format!("`with` must be \"previous\", \"next\", \"first\" or \"last\", got {v}"))),
    };
    let only: Option<Vec<u64>> = match p.get("layers") {
        None | Some(Value::Null) => None,
        Some(Value::String(w)) if w == "all" => None,
        Some(v) => Some(
            v.as_array()
                .and_then(|l| l.iter().map(Value::as_u64).collect::<Option<Vec<_>>>())
                .ok_or_else(|| bad(CMD, format!("`layers` must be \"all\" or a list of layer ids, got {v}")))?,
        ),
    };
    if s.active().and_then(|st| st.doc.frame_animation.as_ref()).is_some_and(|a| a.frames.len() + n > MAX_FRAMES) {
        return Err(EngineError::Other(format!("an animation holds at most {MAX_FRAMES} frames")));
    }
    let mut selection = Vec::new();
    s.edit("Tween", |doc, _| {
        fn ids(layers: &[photocraft_doc::Layer], out: &mut Vec<photocraft_doc::LayerId>) {
            for l in layers {
                out.push(l.id);
                if let Some(ch) = l.children() {
                    ids(ch, out);
                }
            }
        }
        let mut all = Vec::new();
        ids(&doc.layers, &mut all);
        let tweened: Vec<_> = all.into_iter().filter(|id| only.as_ref().is_none_or(|o| o.contains(&id.0))).collect();
        if tweened.is_empty() {
            return Err(bad(CMD, "`layers` names no layer of this document".into()));
        }
        let fallback: std::collections::HashMap<_, _> = tweened.iter().map(|id| (*id, doc.layer(*id).map_or(1.0, |l| l.opacity))).collect();
        let a = animation(doc, CMD)?;
        a.clamp();
        let (from, len) = (a.current, a.frames.len());
        let other = match with.as_str() {
            "previous" => from.checked_sub(1),
            "next" => (from + 1 < len).then_some(from + 1),
            "first" => (from != 0).then_some(0),
            "last" => (from + 1 != len).then_some(len - 1),
            w => return Err(bad(CMD, format!("`with` must be \"previous\", \"next\", \"first\" or \"last\", got \"{w}\""))),
        }
        .ok_or_else(|| EngineError::Other(format!("there is no {with} frame to tween with")))?;
        // `early` fades into `late`, and the new frames go at `at`. First and Last are the far ends of
        // the loop, so the fade runs away from (First) or back into (Last) the selected frame.
        let (early, late, at) = match with.as_str() {
            "first" => (from, other, from + 1),
            "last" => (other, from, from),
            _ => (from.min(other), from.max(other), from.min(other) + 1),
        };
        // What a layer shows in a frame: its opacity, or nothing when hidden.
        let defaults = a.clone();
        let shown = |f: &AnimFrame, id: photocraft_doc::LayerId| {
            let o = defaults.opacity_in(f, id).unwrap_or(fallback[&id]);
            (if f.hidden.binary_search(&id).is_ok() { 0.0 } else { o }, o)
        };
        let mut made = Vec::with_capacity(n);
        for k in 1..=n {
            let t = k as f32 / (n + 1) as f32;
            let mut f = (*a.frames[from]).clone();
            for id in &tweened {
                let ((e_shown, e_op), (l_shown, l_op)) = (shown(&a.frames[early], *id), shown(&a.frames[late], *id));
                let eff = e_shown + (l_shown - e_shown) * t;
                let gone = eff <= 1e-4;
                let op = if gone { if e_shown > 0.0 { e_op } else { l_op } } else { eff };
                f.set_opacity(*id, op.clamp(0.0, 1.0), defaults.default_opacity(*id));
                match (f.hidden.binary_search(id), gone) {
                    (Ok(i), false) => {
                        f.hidden.remove(i);
                    }
                    (Err(i), true) => f.hidden.insert(i, *id),
                    _ => {}
                }
            }
            made.push(Arc::new(f));
        }
        a.frames.splice(at..at, made);
        a.current = at + n - 1;
        selection = (at..at + n).collect();
        let cur = a.current;
        if let Some(a) = &mut doc.frame_animation {
            a.fill_opacities(&doc.layers);
        }
        doc.select_animation_frame(cur);
        Ok(())
    })?;
    info_selecting(s, selection)
}

/// Move the `frames` together, keeping their order, into the gap `slot` (0 is before the first
/// frame, the frame count is after the last); the moved frames are selected and the selected frame
/// stays on its own frame.
fn move_frames(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "animation.moveFrames";
    let mut selection = Vec::new();
    s.edit("Move Frames", |doc, _| {
        let a = animation(doc, CMD)?;
        a.clamp();
        let moving = frames_param(CMD, p, a)?;
        let v = p.get("slot").ok_or_else(|| bad(CMD, "need `slot`".into()))?;
        let slot = v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= a.frames.len())
            .ok_or_else(|| bad(CMD, format!("`slot` must be a gap from 0 to {}, got {v}", a.frames.len())))?;
        let mut order: Vec<usize> = (0..a.frames.len()).collect();
        let before = moving.iter().filter(|i| **i < slot).count();
        order.retain(|i| moving.binary_search(i).is_err());
        let at = slot - before;
        order.splice(at..at, moving.iter().copied());
        selection = (at..at + moving.len()).collect();
        a.current = order.iter().position(|i| *i == a.current).unwrap_or(0);
        let mut old: Vec<Option<Arc<AnimFrame>>> = std::mem::take(&mut a.frames).into_iter().map(Some).collect();
        a.frames = order.iter().filter_map(|i| old[*i].take()).collect();
        Ok(())
    })?;
    info_selecting(s, selection)
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
            "frames": a.frames.iter().map(|f| json!({
                "delayMs": f.delay_ms,
                "hidden": f.hidden.iter().map(|l| l.0).collect::<Vec<_>>(),
                "opacities": f.opacities.iter().map(|(l, o)| json!([l.0, o])).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }}),
        None => json!({"animation": null}),
    }
}

/// The animation plus the frames a command left selected (`selection`), for a UI that selects
/// several frames.
fn info_selecting(s: &mut Session, selection: Vec<usize>) -> Result<Value> {
    let mut v = info(s)?;
    v["selection"] = json!(selection);
    Ok(v)
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
        spec!("animation.newFrame", "Duplicate Selected Frame", r#"{frames?:selected} → {animation, selection}"#, has_animation, true, |s, p| new_frame(
            s, p
        )),
        spec!("animation.deleteFrame", "Delete Frame", r#"{frames?:[i..] | frame?:selected} → {animation, selection}"#, has_animation, true, |s, p| {
            delete_frame(s, p)
        }),
        spec!("animation.selectFrame", "Select Frame", r#"{frame} → {animation}"#, has_animation, false, |s, p| select_frame(s, p)),
        spec!("animation.nextFrame", "Next Frame", "{} → {animation}", has_animation, false, |s, _| step(s, "animation.nextFrame", 1, false)),
        spec!("animation.previousFrame", "Previous Frame", "{} → {animation}", has_animation, false, |s, _| step(s, "animation.previousFrame", -1, false)),
        spec!("animation.setDelay", "Frame Delay", r#"{seconds | delayMs, frames?:[i..] | "all" = selected} → {animation}"#, has_animation, true, |s, p| {
            set_delay(s, p)
        }),
        spec!("animation.setLoop", "Loop Count", r#"{loops:"once"|"forever"|plays} → {animation}"#, has_animation, true, |s, p| set_loop(s, p)),
        spec!("animation.moveFrame", "Move Frame", r#"{from, to} → {animation}"#, has_animation, true, |s, p| move_frame(s, p)),
        spec!(
            "animation.tween",
            "Tween",
            r#"{with?:"previous"|"next"|"first"|"last", frames?:1, layers?:"all"|[id..]} → {animation, selection}"#,
            has_animation,
            true,
            |s, p| tween(s, p)
        ),
        spec!("animation.moveFrames", "Move Frames", r#"{frames:[i..], slot} → {animation, selection}"#, has_animation, true, |s, p| move_frames(s, p)),
        spec!(
            "animation.pasteFrames",
            "Paste Frames",
            r#"{frames:[{delayMs, hidden:[layerId]}], after?:selected} → {animation, selection}"#,
            has_animation,
            true,
            |s, p| { paste_frames(s, p) }
        ),
        spec!("animation.reverseFrames", "Reverse Frames", "{} → {animation}", has_animation, true, |s, p| reverse_frames(s, p)),
        spec!("animation.info", "Frame Animation Info", "{} → {animation}", has_doc, false, |s, _| info(s)),
    ]
}

#[cfg(test)]
mod tests;
