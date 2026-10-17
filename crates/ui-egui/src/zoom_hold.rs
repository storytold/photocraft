//! Animated Zoom: press and hold with the Zoom tool to zoom continuously (#6210).
//!
//! With Preferences › General › Animated Zoom on (the default, as in Photoshop) a press held
//! still past [`DELAY`] zooms in smoothly about the press point until the button comes up, or out
//! with ⌥. The release is then no click step. A quick click still steps the zoom (the canvas), a
//! drag still scrubs or draws a zoom rectangle (`zoom_tool.rs`), and with the preference off a
//! hold does nothing more than a click.
//!
//! The rate is per second of wall time, so the zoom grows the same at any frame rate.

use egui::Pos2;

use crate::canvas::ViewXform;
use crate::state::View;

/// Seconds a press must be held still before it zooms continuously (a click is shorter).
pub const DELAY: f64 = 0.3;
/// Doublings (or halvings, zooming out) per second while held.
pub const RATE: f32 = 2.0;
/// Screen points the pointer may wander before the press counts as a drag instead.
const SLOP: f32 = 6.0;
/// The longest frame gap a hold zooms over: a stalled frame never jumps the zoom far.
const MAX_DT: f64 = 0.25;

/// A Zoom-tool press being held.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hold {
    /// Screen point where the button went down.
    anchor: Pos2,
    /// The document point under it (it stays under the anchor).
    doc: [f64; 2],
    /// When the button went down (egui seconds).
    t0: f64,
    /// The time the zoom last advanced to, once zooming.
    last: Option<f64>,
    /// The pointer moved before the hold began: this press is a drag, never a hold.
    dragged: bool,
}

fn id() -> egui::Id {
    egui::Id::new("pc-zoom-tool-hold")
}

/// The zoom after holding for `dt` seconds at `zoom` (out: shrinking), within the range for a
/// document of `size` px. A non-finite or negative `dt` changes nothing.
pub fn hold_zoom(zoom: f32, dt: f64, out: bool, size: [u32; 2]) -> f32 {
    let dt = if dt.is_finite() { dt.clamp(0.0, MAX_DT) as f32 } else { 0.0 };
    let e = if out { -RATE * dt } else { RATE * dt };
    crate::zoom_levels::clamp(zoom * e.exp2(), size)
}

/// Is a held press zooming right now? (It owns the gesture: no scrub, no click step.)
pub fn active(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Hold>(id())).is_some_and(|h| h.last.is_some())
}

/// Advance a Zoom-tool hold by one frame. `enabled` is the Animated Zoom preference, `out` the
/// zoom direction (⌥ or a temporary zoom-out key, read live). Returns true when the press is a
/// hold zoom (on its release frame too), so the canvas skips the scrub and the click step.
pub fn update(ctx: &egui::Context, view: &mut View, xf: &ViewXform, enabled: bool, out: bool, ppp: f32) -> bool {
    let (down, origin, start, pos, now) =
        ctx.input(|i| (i.pointer.primary_down(), i.pointer.press_origin(), i.pointer.press_start_time(), i.pointer.latest_pos(), i.time));
    let held = ctx.data(|d| d.get_temp::<Hold>(id()));
    if !enabled || !down {
        if held.is_some() {
            ctx.data_mut(|d| d.remove::<Hold>(id()));
        }
        return enabled && held.is_some_and(|h| h.last.is_some());
    }
    let mut h = match held {
        Some(h) => h,
        None => {
            // A press that began on the canvas (not one dragged in from a panel).
            let Some(anchor) = origin.filter(|p| xf.rect.contains(*p)) else { return false };
            let t0 = start.filter(|t| t.is_finite()).unwrap_or(now);
            let h = Hold { anchor, doc: xf.to_doc(anchor), t0, last: None, dragged: false };
            ctx.data_mut(|d| d.insert_temp(id(), h));
            h
        }
    };
    if h.dragged {
        return false;
    }
    if h.last.is_none() {
        // Moved before the hold began: a drag (scrubby zoom or rectangle), not a hold.
        if pos.is_some_and(|p| p.distance(h.anchor) > SLOP) {
            h.dragged = true;
            ctx.data_mut(|d| d.insert_temp(id(), h));
            return false;
        }
        let begin = h.t0 + DELAY;
        if now < begin {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((begin - now).clamp(0.0, DELAY)));
            return false;
        }
        h.last = Some(begin);
    }
    let dt = now - h.last.unwrap_or(now);
    let zoom = hold_zoom(view.zoom, dt, out, view.doc_size);
    crate::zoom_tool::anchor_view(view, xf, h.anchor, h.doc, zoom, ppp);
    h.last = Some(now);
    ctx.data_mut(|d| d.insert_temp(id(), h));
    ctx.request_repaint();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PhotocraftApp;
    use egui::{Event, PointerButton, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    const DOC: [u32; 2] = [400, 300];

    #[test]
    fn hold_zoom_is_frame_rate_independent_and_safe() {
        // One second in one step or in 60 steps: two doublings either way.
        let mut z = 1.0;
        for _ in 0..60 {
            z = hold_zoom(z, 1.0 / 60.0, false, DOC);
        }
        let big = hold_zoom(hold_zoom(hold_zoom(hold_zoom(1.0, 0.25, false, DOC), 0.25, false, DOC), 0.25, false, DOC), 0.25, false, DOC);
        assert!((z - 4.0).abs() < 1e-3 && (big - 4.0).abs() < 1e-3, "{z} {big}");
        assert!((hold_zoom(1.0, 0.25, true, DOC) - 2f32.powf(-0.5)).abs() < 1e-5);
        // Nothing for no time, a negative or broken dt, or a non-finite zoom (100 %).
        assert_eq!(hold_zoom(1.5, 0.0, false, DOC), 1.5);
        assert_eq!(hold_zoom(1.5, -3.0, false, DOC), 1.5);
        assert_eq!(hold_zoom(1.5, f64::NAN, true, DOC), 1.5);
        assert_eq!(hold_zoom(1.5, f64::INFINITY, true, DOC), 1.5);
        assert_eq!(hold_zoom(f32::NAN, 0.1, false, DOC), 1.0);
        // A stall is capped; the range is kept.
        assert!((hold_zoom(1.0, 10.0, false, DOC) - 2f32.powf(0.5)).abs() < 1e-5);
        assert_eq!(hold_zoom(crate::zoom_levels::MAX, 0.2, false, DOC), crate::zoom_levels::MAX);
        let min = crate::zoom_levels::min(DOC);
        assert_eq!(hold_zoom(min, 0.2, true, DOC), min);
        assert_eq!(hold_zoom(1.0, 0.1, false, [0, 0]), hold_zoom(1.0, 0.1, false, [0, 0]));
    }

    fn harness(animated: bool) -> Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("prefs.set", json!({"values": {"general.animatedZoom": animated, "tools.zoomClickedPointToCenter": false}})).unwrap();
        app.sync_views();
        app.ui.extras.rulers = false;
        app.ui.tool = crate::state::Tool::Zoom;
        app.ui.tool_options.zoom_scrubby = true;
        // Each step is 0.1 s of egui time.
        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(0.1).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        let view = &mut h.state_mut().ui.views[0];
        view.zoom = 1.0;
        view.center = [200.0, 150.0];
        view.fit_pending = false;
        view.fill_pending = false;
        h.run_steps(2);
        h
    }

    fn xf(h: &Harness<'static, PhotocraftApp>) -> ViewXform {
        ViewXform::active(h.state()).unwrap()
    }

    fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, pressed: bool, alt: bool) {
        let modifiers = egui::Modifiers { alt, ..Default::default() };
        h.event(Event::ModifiersChanged(modifiers));
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers });
        h.step();
    }

    /// Press at `p`, hold still for `frames` steps, release; returns the zoom after every step,
    /// then after the release step and two more frames.
    fn hold(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, frames: usize, alt: bool) -> Vec<f32> {
        h.event(Event::PointerMoved(p));
        h.step();
        button(h, p, true, alt);
        let mut zs = vec![h.state().current_zoom()];
        for _ in 0..frames {
            h.step();
            zs.push(h.state().current_zoom());
        }
        button(h, p, false, alt);
        zs.push(h.state().current_zoom());
        h.run_steps(2);
        zs.push(h.state().current_zoom());
        zs
    }

    #[test]
    fn holding_zooms_in_continuously_about_the_press_point() {
        let mut h = harness(true);
        let p = h.state().last_canvas_rect.center() + vec2(-120.0, 70.0);
        let doc = xf(&h).to_doc(p);
        let zs = hold(&mut h, p, 8, false);
        // Nothing during the delay, then more every frame; the release adds no click step.
        assert_eq!(zs[0], 1.0, "{zs:?}");
        let rising = &zs[4..=8];
        assert!(rising.windows(2).all(|w| w[1] > w[0]), "{zs:?}");
        // The release frame ends it: no click step, nothing more afterwards.
        assert_eq!(zs[10], zs[9], "{zs:?}");
        assert!(zs[9] < zs[8] * 1.2, "{zs:?}");
        // About 0.5 s held past the delay at RATE doublings per second.
        let expected = (RATE * 0.5).exp2();
        assert!((zs[8] - expected).abs() < 0.2, "{zs:?} vs {expected}");
        assert!(xf(&h).to_screen(doc[0] as f32, doc[1] as f32).distance(p) < 0.5);
        assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
    }

    #[test]
    fn alt_hold_zooms_out() {
        let mut h = harness(true);
        let p = h.state().last_canvas_rect.center() + vec2(50.0, -30.0);
        let doc = xf(&h).to_doc(p);
        let zs = hold(&mut h, p, 8, true);
        assert!(zs[4..=8].windows(2).all(|w| w[1] < w[0]), "{zs:?}");
        assert_eq!(zs[10], zs[9], "{zs:?}");
        assert!(zs[9] > zs[8] / 1.2, "{zs:?}");
        assert!(xf(&h).to_screen(doc[0] as f32, doc[1] as f32).distance(p) < 0.5);
    }

    #[test]
    fn with_animated_zoom_off_a_hold_is_one_click_step() {
        let mut h = harness(false);
        let p = h.state().last_canvas_rect.center();
        // Shorter than egui's longest click, so the release is a click.
        let zs = hold(&mut h, p, 5, false);
        assert!(zs[..=5].iter().all(|&z| z == 1.0), "{zs:?}");
        assert_eq!(*zs.last().unwrap(), crate::zoom_levels::step(1.0, 1, DOC));
    }

    #[test]
    fn a_quick_click_steps_and_a_drag_still_scrubs() {
        let mut h = harness(true);
        let p = h.state().last_canvas_rect.center();
        let zs = hold(&mut h, p, 0, false);
        assert_eq!(*zs.last().unwrap(), crate::zoom_levels::step(1.0, 1, DOC), "{zs:?}");
        // Scrubby Zoom: a drag zooms by its distance however long it is held, never by time.
        let z0 = h.state().current_zoom();
        h.event(Event::PointerMoved(p));
        h.step();
        button(&mut h, p, true, false);
        h.event(Event::PointerMoved(p + vec2(50.0, 0.0)));
        h.step();
        for _ in 0..6 {
            h.step();
        }
        let expected = crate::zoom_tool::scrub_zoom(z0, 50.0, DOC);
        assert!((h.state().current_zoom() - expected).abs() < 1e-4, "{} vs {expected}", h.state().current_zoom());
        button(&mut h, p + vec2(50.0, 0.0), false, false);
        h.run_steps(2);
        assert!((h.state().current_zoom() - expected).abs() < 1e-4);
        assert!(!active(&h.ctx));
    }
}
