//! Window › Timeline: create and drive the document timeline (playhead over N frames at a frame
//! rate). Navigation doesn't add history steps. Video layers (Layer › Video Layers, added later)
//! read `current` to show their frame. Headless and scriptable so agents can render frame ranges.

use std::sync::Arc;

use photocraft_doc::Timeline;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Longest timeline, in frames: every frame becomes a surface in a new video layer and a file in
/// Render Video, so `duration` is capped well before it can exhaust memory or overflow (#1009).
pub(crate) const MAX_DURATION: usize = 1_000_000;
/// Highest frame rate; larger values only come from mistakes and become `inf` as `f32` (#1009).
const MAX_FPS: f32 = 1000.0;

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document".into())
}

fn has_timeline(s: &Session) -> std::result::Result<(), String> {
    match s.active() {
        Some(d) if d.doc.timeline.is_some() => Ok(()),
        Some(_) => Err("no timeline (create one first)".into()),
        None => Err("no document".into()),
    }
}

/// Mutate the document timeline without a history step (navigation / metadata).
fn with_timeline(s: &mut Session, f: impl FnOnce(&mut Option<Timeline>)) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let mut doc = (*st.doc).clone();
    crate::video_cmds::store(&mut doc);
    f(&mut doc.timeline);
    if let Some(t) = &mut doc.timeline {
        t.clamp();
    }
    crate::video_cmds::sync(&mut doc);
    st.doc = Arc::new(doc);
    st.revision += 1;
    info(s)
}

fn bad(cmd: &str, msg: String) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg }
}

/// The optional `duration` param, rejected unless it's a whole number in `1..=MAX_DURATION`.
/// Before #1009 any `u64` was kept and stepping panicked on it.
fn duration_param(cmd: &str, p: &Value) -> Result<Option<usize>> {
    let Some(v) = p.get("duration").filter(|v| !v.is_null()) else { return Ok(None) };
    v.as_u64()
        .and_then(|d| usize::try_from(d).ok())
        .filter(|d| (1..=MAX_DURATION).contains(d))
        .map(Some)
        .ok_or_else(|| bad(cmd, format!("`duration` must be a whole number of frames from 1 to {MAX_DURATION}, got {v}")))
}

/// The optional `fps` param, rejected unless it's a finite number in `(0, MAX_FPS]` (#1009).
pub(crate) fn fps_param(cmd: &str, p: &Value) -> Result<Option<f32>> {
    let Some(v) = p.get("fps").filter(|v| !v.is_null()) else { return Ok(None) };
    v.as_f64()
        .filter(|f| f.is_finite() && *f > 0.0 && *f <= f64::from(MAX_FPS))
        .map(|f| Some(f as f32))
        .ok_or_else(|| bad(cmd, format!("`fps` must be a number above 0 and at most {MAX_FPS}, got {v}")))
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let duration = duration_param("timeline.create", p)?.unwrap_or(30);
    let fps = fps_param("timeline.create", p)?.unwrap_or(30.0);
    with_timeline(s, |t| *t = Some(Timeline::new(duration, fps)))
}

fn delete(s: &mut Session) -> Result<Value> {
    with_timeline(s, |t| *t = None)
}

fn set_frame(s: &mut Session, p: &Value) -> Result<Value> {
    let f =
        p.get("frame").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: "timeline.setFrame".into(), msg: "need `frame`".into() })? as usize;
    with_timeline(s, |t| {
        if let Some(t) = t {
            t.current = f;
        }
    })
}

fn step(s: &mut Session, delta: i64) -> Result<Value> {
    with_timeline(s, |t| {
        if let Some(t) = t {
            t.step(delta);
        }
    })
}

/// Shortest and longest frame delay, in milliseconds. Browsers show anything under 20 ms slower, and
/// a minute is far past any animation's frame.
const DELAY_MS: std::ops::RangeInclusive<u64> = 10..=60_000;

/// Sets how long a frame shows (`ms`): the playhead's frame, `frame`, or every frame with `all`.
/// `all` sets the frame rate and drops the per-frame delays; one frame stores delays up to it, the
/// frames before it keeping what they showed for.
fn set_frame_delay(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "timeline.setFrameDelay";
    let ms = p.get("ms").and_then(Value::as_u64).filter(|v| DELAY_MS.contains(v)).ok_or_else(|| {
        bad(cmd, format!("`ms` must be a whole number from {} to {}, got {}", DELAY_MS.start(), DELAY_MS.end(), p.get("ms").unwrap_or(&Value::Null)))
    })? as u32;
    let all = match p.get("all").filter(|v| !v.is_null()) {
        Some(v) => v.as_bool().ok_or_else(|| bad(cmd, format!("`all` must be true or false, got {v}")))?,
        None => false,
    };
    let frame = match p.get("frame").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_u64().and_then(|f| usize::try_from(f).ok()).ok_or_else(|| bad(cmd, format!("`frame` must be a frame index, got {v}")))?),
        None => None,
    };
    // Delays are stored up to the frame, so a loaded timeline longer than any command makes (a
    // crafted file) is refused before it can size that allocation.
    let duration = s.active().and_then(|d| d.doc.timeline.as_ref()).map_or(0, |t| t.duration);
    if duration > MAX_DURATION {
        return Err(bad(cmd, format!("the timeline is {duration} frames; frame delays are limited to {MAX_DURATION} frames")));
    }
    with_timeline(s, |t| {
        let Some(t) = t else { return };
        if all {
            t.fps = 1000.0 / ms as f32;
            t.delays_ms = Arc::default();
            return;
        }
        let at = frame.unwrap_or(t.current).min(t.duration.saturating_sub(1));
        let shown: Vec<u32> = (t.delays_ms.len()..=at).map(|f| t.frame_ms(f)).collect();
        let delays = Arc::make_mut(&mut t.delays_ms);
        delays.extend(shown);
        if let Some(d) = delays.get_mut(at) {
            *d = ms;
        }
    })
}

fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    let fps = fps_param("timeline.setProps", p)?;
    let duration = duration_param("timeline.setProps", p)?;
    with_timeline(s, |t| {
        if let Some(t) = t {
            if let Some(fps) = fps {
                t.fps = fps;
            }
            if let Some(d) = duration {
                // A work area that reached the old end follows the new one, as in
                // `video_cmds::grow_duration` (#4170); a shrink is left to `Timeline::clamp`.
                if t.work_end >= t.duration {
                    t.work_end = d;
                }
                t.duration = d;
            }
            if let Some(ws) = p.get("workStart").and_then(Value::as_u64) {
                t.work_start = ws as usize;
            }
            if let Some(we) = p.get("workEnd").and_then(Value::as_u64) {
                t.work_end = we as usize;
            }
        }
    })
}

fn info(s: &mut Session) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    Ok(match &st.doc.timeline {
        Some(t) => {
            json!({"timeline": {"fps": t.fps, "duration": t.duration, "current": t.current, "workStart": t.work_start, "workEnd": t.work_end, "time": t.time(), "frameMs": t.frame_ms(t.current)}})
        }
        None => json!({"timeline": null}),
    })
}

macro_rules! spec {
    ($id:expr, $label:expr, $params:expr, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: $en, journal: false, run: $run }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("timeline.create", "Create Video Timeline", r#"{duration?:30, fps?:30} → {timeline}"#, has_doc, |s, p| create(s, p)),
        spec!("timeline.delete", "Delete Timeline", "{} → {timeline:null}", has_timeline, |s, _| delete(s)),
        spec!("timeline.setFrame", "Go to Frame", r#"{frame} → {timeline}"#, has_timeline, |s, p| set_frame(s, p)),
        spec!("timeline.nextFrame", "Next Frame", "{} → {timeline}", has_timeline, |s, _| step(s, 1)),
        spec!("timeline.previousFrame", "Previous Frame", "{} → {timeline}", has_timeline, |s, _| step(s, -1)),
        spec!("timeline.setProps", "Timeline Settings", r#"{fps?, duration?, workStart?, workEnd?} → {timeline}"#, has_timeline, |s, p| set_props(s, p)),
        spec!("timeline.info", "Timeline Info", "{} → {timeline}", has_doc, |s, _| info(s)),
        spec!("timeline.setFrameDelay", "Frame Delay", r#"{ms, frame?, all?} → {timeline}"#, has_timeline, |s, p| set_frame_delay(s, p)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_navigate_delete() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"], Value::Null);
        let r = s.execute("timeline.create", json!({"duration": 24, "fps": 24.0})).unwrap();
        assert_eq!(r["timeline"]["duration"], 24);
        assert_eq!(r["timeline"]["current"], 0);
        // Navigate.
        s.execute("timeline.setFrame", json!({"frame": 10})).unwrap();
        s.execute("timeline.nextFrame", json!({})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["current"], 11);
        s.execute("timeline.previousFrame", json!({})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["current"], 10);
        // Clamp beyond the end.
        let r = s.execute("timeline.setFrame", json!({"frame": 999})).unwrap();
        assert_eq!(r["timeline"]["current"], 23);
        // Props.
        s.execute("timeline.setProps", json!({"fps": 30.0})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"]["fps"], 30.0);
        // Delete.
        s.execute("timeline.delete", json!({})).unwrap();
        assert_eq!(s.execute("timeline.info", json!({})).unwrap()["timeline"], Value::Null);
    }

    /// The document's timeline, revision and history depth, to show a rejected command changed nothing.
    fn snapshot(s: &Session) -> (Option<Timeline>, u64, usize) {
        let st = s.active().unwrap();
        (st.doc.timeline.clone(), st.revision, st.history.past_len())
    }

    fn current(s: &mut Session) -> Value {
        s.execute("timeline.info", json!({})).unwrap()["timeline"]["current"].clone()
    }

    /// #1009: `timeline.create` kept any `duration` (even `u64::MAX`) and `fps`, and stepping then
    /// panicked in `i64::clamp`. Bad values are now rejected before anything changes.
    #[test]
    fn create_rejects_out_of_range_params() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("timeline.create", json!({"duration": 24, "fps": 24.0})).unwrap();
        s.execute("timeline.setFrame", json!({"frame": 5})).unwrap();
        let before = snapshot(&s);
        let too_long = MAX_DURATION as u64 + 1;
        for p in [
            json!({"duration": 0}),
            json!({"duration": u64::MAX}),
            json!({"duration": i64::MAX as u64 + 1}),
            json!({"duration": too_long}),
            json!({"duration": -1}),
            json!({"duration": 2.5}),
            json!({"duration": "24"}),
            json!({"fps": 0.0}),
            json!({"fps": -24.0}),
            json!({"fps": MAX_FPS as f64 + 1.0}),
            json!({"fps": 1e300}),
            json!({"fps": "fast"}),
        ] {
            let e = s.execute("timeline.create", p.clone()).expect_err(&format!("{p} must be rejected"));
            assert!(matches!(e, EngineError::BadParams { .. }), "{p}: {e}");
            assert_eq!(snapshot(&s), before, "{p}: nothing changes");
        }
        // The issue's repro: stepping still works on the surviving timeline.
        s.execute("timeline.nextFrame", json!({})).unwrap();
        assert_eq!(current(&mut s), 6);
        s.execute("timeline.previousFrame", json!({})).unwrap();
        assert_eq!(current(&mut s), 5);
        // Control: the largest accepted values still create a timeline that steps at both ends.
        let r = s.execute("timeline.create", json!({"duration": MAX_DURATION, "fps": MAX_FPS as f64})).unwrap();
        assert_eq!(r["timeline"]["duration"], MAX_DURATION);
        s.execute("timeline.previousFrame", json!({})).unwrap();
        assert_eq!(current(&mut s), 0);
        s.execute("timeline.setFrame", json!({"frame": MAX_DURATION})).unwrap();
        s.execute("timeline.nextFrame", json!({})).unwrap();
        assert_eq!(current(&mut s), MAX_DURATION - 1);
        // Control: `null` means "use the default", as before.
        let r = s.execute("timeline.create", json!({"duration": null, "fps": null})).unwrap();
        assert_eq!((r["timeline"]["duration"].as_u64(), r["timeline"]["fps"].as_f64()), (Some(30), Some(30.0)));
    }

    /// #1009: `timeline.setProps` could store the same out-of-range `duration` and `fps`.
    #[test]
    fn set_props_rejects_out_of_range_params() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("timeline.create", json!({"duration": 24, "fps": 24.0})).unwrap();
        let before = snapshot(&s);
        for p in [json!({"duration": 0}), json!({"duration": u64::MAX}), json!({"fps": 0.0}), json!({"fps": -1.0}), json!({"fps": 30.0, "duration": 0})] {
            let e = s.execute("timeline.setProps", p.clone()).expect_err(&format!("{p} must be rejected"));
            assert!(matches!(e, EngineError::BadParams { .. }), "{p}: {e}");
            assert_eq!(snapshot(&s), before, "{p}: nothing changes");
        }
        s.execute("timeline.nextFrame", json!({})).unwrap();
        assert_eq!(current(&mut s), 1);
    }

    /// A longer `duration` moves a work area that reached the old end to the new end, like
    /// `video_cmds::grow_duration` (#4170); a narrowed one and an explicit `workEnd` stay put.
    #[test]
    fn set_props_longer_duration_moves_full_work_area() {
        let work = |r: &Value| (r["timeline"]["workStart"].as_u64(), r["timeline"]["workEnd"].as_u64());
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("timeline.create", json!({"duration": 30})).unwrap();
        let r = s.execute("timeline.setProps", json!({"duration": 60})).unwrap();
        assert_eq!(work(&r), (Some(0), Some(60)), "the full work area follows the new end");
        let r = s.execute("timeline.setProps", json!({"duration": 90, "workEnd": 45})).unwrap();
        assert_eq!(work(&r), (Some(0), Some(45)), "an explicit workEnd wins");
        s.execute("timeline.setProps", json!({"workStart": 2, "workEnd": 10})).unwrap();
        let r = s.execute("timeline.setProps", json!({"duration": 120})).unwrap();
        assert_eq!(work(&r), (Some(2), Some(10)), "a narrowed work area stays put");
    }

    /// #1009: a timeline stored with `duration` 0 or `u64::MAX` (an old or hand-edited file) steps
    /// without panicking and lands on a valid frame.
    #[test]
    fn step_is_safe_on_any_stored_timeline() {
        for duration in [0, 1, usize::MAX] {
            for current in [0, usize::MAX] {
                for (cmd, delta) in [("timeline.nextFrame", 1i64), ("timeline.previousFrame", -1)] {
                    let mut s = Session::new();
                    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
                    let st = s.active_mut().unwrap();
                    let mut doc = (*st.doc).clone();
                    doc.timeline = Some(Timeline { fps: f32::NAN, duration, current, work_start: usize::MAX, work_end: 0, delays_ms: Arc::default() });
                    st.doc = Arc::new(doc);
                    let r = s.execute(cmd, json!({})).unwrap_or_else(|e| panic!("{cmd} d={duration} c={current}: {e}"));
                    let t = &r["timeline"];
                    let d = t["duration"].as_u64().unwrap();
                    let c = t["current"].as_u64().unwrap();
                    assert!(d >= 1 && c < d, "{cmd} d={duration} c={current}: {t}");
                    if duration == usize::MAX && current == 0 {
                        assert_eq!(c, if delta > 0 { 1 } else { 0 });
                    }
                }
            }
        }
    }

    #[test]
    fn navigation_without_timeline_errors() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(s.execute("timeline.setFrame", json!({"frame": 1})).is_err());
    }

    /// A frame's delay is stored up to that frame; the frames before it keep the rate's time, and
    /// `all` sets the rate instead.
    #[test]
    fn set_frame_delay_sets_one_frame_or_all() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("timeline.create", json!({"duration": 4, "fps": 10.0})).unwrap();
        s.execute("timeline.setFrameDelay", json!({"frame": 2, "ms": 500})).unwrap();
        let t = s.active().unwrap().doc.timeline.clone().unwrap();
        assert_eq!(*t.delays_ms, [100, 100, 500]);
        assert_eq!(t.frame_ms(3), 100);
        // Without `frame`, the playhead's frame; past the end, the last frame.
        s.execute("timeline.setFrame", json!({"frame": 1})).unwrap();
        let r = s.execute("timeline.setFrameDelay", json!({"ms": 40})).unwrap();
        assert_eq!(r["timeline"]["frameMs"], 40);
        s.execute("timeline.setFrameDelay", json!({"frame": 99, "ms": 70})).unwrap();
        assert_eq!(*s.active().unwrap().doc.timeline.as_ref().unwrap().delays_ms, [100, 40, 500, 70]);
        // Navigation leaves history alone, and so does this metadata.
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        let r = s.execute("timeline.setFrameDelay", json!({"ms": 50, "all": true})).unwrap();
        assert_eq!(r["timeline"]["fps"], 20.0);
        assert!(s.active().unwrap().doc.timeline.as_ref().unwrap().delays_ms.is_empty());
        // A shorter timeline drops the delays past its end.
        s.execute("timeline.setFrameDelay", json!({"frame": 3, "ms": 200})).unwrap();
        s.execute("timeline.setProps", json!({"duration": 2})).unwrap();
        assert_eq!(*s.active().unwrap().doc.timeline.as_ref().unwrap().delays_ms, [50, 50]);
    }

    /// A loaded timeline longer than `MAX_DURATION` (a crafted file) is refused before delays are
    /// stored up to its playhead, which would size an allocation from the file.
    #[test]
    fn set_frame_delay_refuses_an_oversized_loaded_timeline() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let st = s.active_mut().unwrap();
        let mut doc = (*st.doc).clone();
        doc.timeline = Some(Timeline { fps: 10.0, duration: usize::MAX, current: usize::MAX - 1, work_start: 0, work_end: 1, delays_ms: Arc::default() });
        st.doc = Arc::new(doc);
        let before = snapshot(&s);
        for p in [json!({"ms": 100}), json!({"ms": 100, "frame": u64::MAX})] {
            let e = s.execute("timeline.setFrameDelay", p.clone()).expect_err(&format!("{p} must be rejected"));
            assert!(matches!(e, EngineError::BadParams { .. }), "{p}: {e}");
            assert_eq!(snapshot(&s), before, "{p}: nothing changes");
        }
        // Control: `all` stores nothing per frame, but is refused the same way.
        assert!(s.execute("timeline.setFrameDelay", json!({"ms": 100, "all": true})).is_err());
    }

    /// Bad `ms` or `frame` values are rejected and change nothing.
    #[test]
    fn set_frame_delay_rejects_bad_params() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(s.execute("timeline.setFrameDelay", json!({"ms": 100})).is_err(), "no timeline");
        s.execute("timeline.create", json!({"duration": 3, "fps": 10.0})).unwrap();
        let before = snapshot(&s);
        for p in [
            json!({}),
            json!({"ms": 0}),
            json!({"ms": 9}),
            json!({"ms": 60_001}),
            json!({"ms": -5}),
            json!({"ms": 1.5}),
            json!({"ms": "fast"}),
            json!({"ms": u64::MAX}),
            json!({"ms": 100, "frame": -1}),
            json!({"ms": 100, "frame": "first"}),
            json!({"ms": 100, "all": "true"}),
            json!({"ms": 100, "all": 1}),
        ] {
            let e = s.execute("timeline.setFrameDelay", p.clone()).expect_err(&format!("{p} must be rejected"));
            assert!(matches!(e, EngineError::BadParams { .. }), "{p}: {e}");
            assert_eq!(snapshot(&s), before, "{p}: nothing changes");
        }
    }
}
