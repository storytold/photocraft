//! The translucent footprint trail the retouching tools (Dodge, Blur, Clone Stamp, …) and Quick
//! Selection show while dragging: the union of the brush's round footprint along the drag.
//!
//! The footprint is a framework [`CoverageMask`] over the document, stamped as the pointer moves and
//! shown by the framework's `StrokeTrail`, which uploads only the changed rows. The Spot Healing
//! Brush's trail is the coverage of the brush's own dabs ([`Trail::feed_dabs`]): hardness, pressure
//! and shape dynamics show as they heal (#2364). The Remove Tool's is where those dabs reach at all
//! ([`Trail::feed_reach`]), which is what it removes.

use craft::behavior::kurbo::{Point, Rect};
use craft::ui::canvas_controls::brush::{CoverageMask, StrokeTrail, TrailTone};
use craft::ui::contract::Show as _;
use photocraft_paint::{BrushSettings, StrokePoint, StrokeRenderer};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// A drag's footprint mask.
pub struct Trail {
    mask: CoverageMask,
    /// Drag points stamped so far.
    fed: usize,
    /// The stroke's dabs, when the trail shows their coverage ([`Self::feed_dabs`]).
    dabs: Option<Dabs>,
    /// What the footprint shows as.
    tone: TrailTone,
}

/// The brush stroke behind a [`Trail::feed_dabs`] trail.
struct Dabs {
    renderer: StrokeRenderer,
    /// Any coverage counts in full ([`Trail::feed_reach`]).
    reach: bool,
    /// Document pixels the last "finish now" tail (a lone first dab, smoothing catch-up) covered.
    tail: photocraft_geom::Rect,
}

impl Trail {
    /// An empty trail over a document of `doc` pixels. Big documents are stamped at a reduced
    /// scale (the mask's longest side is 1024 texels).
    /// `serials` keeps its texture apart from earlier trails' (the app's [`PhotocraftApp::texture_serials`]).
    pub fn new(serials: &mut craft::ui::primitives::Serials, doc: [u32; 2]) -> Self {
        let bounds = Rect::new(0.0, 0.0, f64::from(doc[0]), f64::from(doc[1]));
        Self { mask: CoverageMask::new(serials, bounds, 1024), fed: 0, dabs: None, tone: TrailTone::Light }
    }

    /// The trail shown as `tone`.
    fn toned(mut self, tone: TrailTone) -> Self {
        self.tone = tone;
        self
    }

    /// Coverage 0..=255 at a mask texel (tests).
    pub fn coverage(&self, x: usize, y: usize) -> u8 {
        let s = self.mask.scale();
        // `coverage_at` returns the byte, 0..=255.
        self.mask.coverage_at(Point::new((x as f64 + 0.5) * s, (y as f64 + 0.5) * s))
    }

    /// Document pixels per mask texel.
    pub fn scale(&self) -> f32 {
        self.mask.scale() as f32
    }

    /// Stamp the drag points not seen yet: round footprints of `diameter` document pixels joined
    /// into a capsule per segment (coverage is the max, so overlaps don't add up).
    pub fn feed(&mut self, points: &[[f64; 3]], diameter: f32) {
        if self.fed >= points.len() {
            return;
        }
        let radius = f64::from(diameter.max(1.0)) / 2.0;
        let at = |p: &[f64; 3]| Point::new(p[0], p[1]);
        for i in self.fed..points.len() {
            let Some(b) = points.get(i).map(at) else { break };
            let a = i.checked_sub(1).and_then(|j| points.get(j)).map_or(b, at);
            self.mask.stamp_capsule(a, b, radius);
        }
        self.fed = points.len();
    }

    /// Show the coverage the brush's own dabs leave along the drag, as the stroke commit renders
    /// it (`photocraft_paint::retouch::stroke_coverage`): soft edges for a soft brush, the size and
    /// shape each dab takes from pen pressure and dynamics. `brush` is asked for once, on the first
    /// call. What finishing the stroke now would add (the press's lone first dab) shows too.
    pub fn feed_dabs(&mut self, points: &[[f64; 3]], brush: impl FnOnce() -> BrushSettings) {
        self.feed_dabs_with(points, brush, false);
    }

    /// Show where the brush's own dabs reach at all, in full: the Remove Tool removes every pixel a
    /// dab covers, however faintly (`retouch_cmds::remove`'s hole), so a soft tip's whole reach is
    /// tinted, not just its nominal radius or its opaque core.
    pub fn feed_reach(&mut self, points: &[[f64; 3]], brush: impl FnOnce() -> BrushSettings) {
        self.feed_dabs_with(points, brush, true);
    }

    /// [`Self::feed_reach`] for a whole new drag into the same mask (the Remove Tool's waiting
    /// strokes, #6555): the earlier strokes' tint stays under it.
    pub fn feed_stroke_reach(&mut self, points: &[[f64; 3]], brush: impl FnOnce() -> BrushSettings) {
        self.mask.begin_stroke();
        self.dabs = None;
        self.fed = 0;
        self.feed_reach(points, brush);
    }

    fn feed_dabs_with(&mut self, points: &[[f64; 3]], brush: impl FnOnce() -> BrushSettings, reach: bool) {
        if self.dabs.is_some() && self.fed >= points.len() {
            return;
        }
        let mut dabs =
            self.dabs.take().unwrap_or_else(|| Dabs { renderer: StrokeRenderer::new(&brush(), None, 1.0), reach, tail: photocraft_geom::Rect::EMPTY });
        let new: Vec<StrokePoint> = points
            .get(self.fed..)
            .unwrap_or(&[])
            .iter()
            .filter(|p| p.iter().all(|v| v.is_finite()) && p[0].abs() < 1e7 && p[1].abs() < 1e7)
            .map(|p| StrokePoint::new(p[0], p[1], p[2] as f32))
            .collect();
        self.fed = points.len();
        dabs.renderer.push(&new);
        let mut changed = dabs.renderer.take_dirty_rect().union(&dabs.tail);
        let tail = dabs.renderer.tail_preview().map(|mut t| (t.take_dirty_rect(), t));
        dabs.tail = tail.as_ref().map_or(photocraft_geom::Rect::EMPTY, |(r, _)| *r);
        changed = changed.union(&dabs.tail);
        let tail = tail.map(|(_, t)| t);
        if !changed.is_empty() {
            let renderer = &dabs.renderer;
            let reach = dabs.reach;
            let rect = Rect::new(f64::from(changed.x0), f64::from(changed.y0), f64::from(changed.x1), f64::from(changed.y1));
            self.mask.restamp(rect, |p| {
                let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
                let c = renderer.coverage_at(x, y).max(tail.as_ref().map_or(0.0, |t| t.coverage_at(x, y)));
                if reach && c > 0.0 { 1.0 } else { c }
            });
        }
        self.dabs = Some(dabs);
    }

    /// Shows the footprint as `tone` over the canvas.
    fn show_as(&mut self, ui: &mut egui::Ui, rect: egui::Rect, label: &'static str, tone: TrailTone) {
        StrokeTrail::new(label, &mut self.mask).tone(tone).show_at(ui, rect);
    }
}

/// How `tool`'s trail shows: the Remove Tool's as what will go, the Spot Healing Brush's dark as
/// Photoshop's, the others light.
fn tone_of(tool: Tool) -> TrailTone {
    match tool {
        Tool::Remove => TrailTone::Danger,
        Tool::SpotHealing => TrailTone::Shade,
        _ => TrailTone::Light,
    }
}

/// Stamps the new points of the painting `tool`'s stroke into `app.trail`, which the first call of
/// a stroke creates. `points` are the whole stroke's points (the painting tool passes them because
/// `app.canvas_tools` is out of the app while its family runs). Quick Selection shows the brush
/// footprint, the Remove Tool the dabs' whole reach, every other tool the dabs' own coverage.
/// A pass that feeds the same points again stamps nothing.
pub(crate) fn feed(app: &mut PhotocraftApp, tool: Tool, points: &[[f64; 3]]) {
    let Some(st) = app.session.active() else { return };
    let size = [st.doc.size.width, st.doc.size.height];
    let brush = (*app.session.tools.brush).clone();
    let trail = app.trail.get_or_insert_with(|| Trail::new(&mut app.texture_serials, size).toned(tone_of(tool)));
    if tool == Tool::QuickSelection {
        trail.feed(points, brush.size);
        return;
    }
    let entry = json!({"size": brush.size, "hardness": brush.hardness * 100.0, "spacing": brush.spacing * 100.0});
    let dabs_brush = || photocraft_engine::retouch_cmds::stroke_brush(&brush, &entry);
    if tool == Tool::Remove {
        trail.feed_reach(points, dabs_brush);
    } else {
        trail.feed_dabs(points, dabs_brush);
    }
}

/// Shows the live stroke's trail and, while Remove strokes wait for Enter, their tint. A part
/// with no trail does nothing.
pub(crate) fn show(ui: &mut egui::Ui, app: &mut PhotocraftApp, rect: egui::Rect) {
    if let Some(trail) = app.trail.as_mut() {
        let tone = trail.tone;
        trail.show_as(ui, rect, tl!("Stroke footprint"), tone);
    }
    if app.remove_trail.is_some()
        && crate::retouch_ui::remove_pending_live(app)
        && let Some(trail) = app.remove_trail.as_mut()
    {
        trail.show_as(ui, rect, tl!("Waiting removals"), TrailTone::Danger);
    }
}
