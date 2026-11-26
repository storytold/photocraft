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
use craft::ui::style::DataColor;
use egui::Color32;
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

/// What the Remove tool's "Overlay options" popup holds (#12549): the colour it picked, and the
/// opacity its field shows. A fresh start holds the theme's colour and the share the tool's own trail
/// tone paints, so the footprint is drawn exactly as it was before the popup existed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OverlayTint {
    /// The footprint colour, channels 0..=1, as the Color Picker answered it; `None` shows the theme's.
    color: Option<[f32; 3]>,
    /// The footprint's opacity in percent, 1..=100.
    opacity: f32,
}

impl Default for OverlayTint {
    /// A fresh start: the theme's colour and the share the tone paints (#12549).
    fn default() -> Self {
        Self::plain()
    }
}

impl OverlayTint {
    /// The tint of a trail painted as `tone`: only the Remove tool's (`Danger`) reads the popup's pair,
    /// every other tone paints itself. Keyed on the tone the trail was made with, not on the tool now
    /// in the bar, so a spring-loaded key cannot restyle a stroke already being painted (#12549).
    pub(crate) fn of(app: &PhotocraftApp, tone: TrailTone) -> Self {
        if tone != TrailTone::Danger {
            return Self::plain();
        }
        let o = &app.ui.tool_options;
        Self { color: o.remove_overlay_color, opacity: o.remove_overlay_opacity }
    }

    /// No colour and the share `Danger` already paints: a trail left to its tone, built without the
    /// tool options so a canvas pass costs nothing more (#12549).
    pub(crate) fn plain() -> Self {
        Self { color: None, opacity: TrailTone::Danger.share() * 100.0 }
    }

    /// Whether the tint leaves the trail's tone to paint the footprint: no colour, and the opacity the
    /// tone already shows (#12549).
    pub(crate) fn is_plain(&self) -> bool {
        self.props() == (None, None)
    }

    /// The share of its colour a fully covered texel shows at `opacity` percent; over 100 % is no more
    /// than the colour itself, under 0 % shows no footprint.
    pub(crate) fn share(opacity: f32) -> f32 {
        (opacity / 100.0).clamp(0.0, 1.0)
    }

    /// The trail props this tint stands for: the colour as the framework takes it, and the share of it.
    /// An opacity that is still the tone's own needs no share prop, so an untouched overlay paints
    /// through its tone exactly as before; `(None, None)` is that case (#12549).
    pub(crate) fn props(&self) -> (Option<DataColor>, Option<f32>) {
        let share = Self::share(self.opacity);
        // Remove's own tone is the one whose share the tool options' default carries.
        let same_as_tone = (share - TrailTone::Danger.share()).abs() < 1e-6;
        (self.color.map(data_colour), (!same_as_tone).then_some(share))
    }
}

/// The colour the person picked (channels 0..=1) as the trail's own colour.
fn data_colour(rgb: [f32; 3]) -> DataColor {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    DataColor::new(Color32::from_rgb(byte(rgb[0]), byte(rgb[1]), byte(rgb[2])))
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

    /// Shows the footprint as `tone` over the canvas. `tint` is what the Remove tool's Overlay options
    /// popup holds: with either half set, it stands in for the tone's own colour or share (#12549).
    fn show_as(&mut self, ui: &mut egui::Ui, rect: egui::Rect, label: &'static str, tone: TrailTone, tint: OverlayTint) {
        let mut trail = StrokeTrail::new(label, &mut self.mask).tone(tone);
        // An untouched overlay paints through its tone; the popup's pair stands in for it (#12549).
        if !tint.is_plain() {
            let (color, share) = tint.props();
            if let Some(color) = color {
                trail = trail.color(color);
            }
            if let Some(share) = share {
                trail = trail.share(share);
            }
        }
        trail.show_at(ui, rect);
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
/// with no trail does nothing. The Remove tool shows the colour and opacity its Overlay options popup
/// set (#12549); every other tool's trail paints its tone exactly as before.
pub(crate) fn show(ui: &mut egui::Ui, app: &mut PhotocraftApp, rect: egui::Rect) {
    // The live stroke's tint comes from the tone its own trail carries, so a spring-loaded tool key
    // pressed in the middle of a stroke cannot restyle the footprint already being painted (#12549).
    let live_tone = app.trail.as_ref().map(|trail| trail.tone);
    let live = live_tone.map_or_else(OverlayTint::plain, |tone| OverlayTint::of(app, tone));
    if let Some(trail) = app.trail.as_mut() {
        let tone = trail.tone;
        trail.show_as(ui, rect, tl!("Stroke footprint"), tone, live);
    }
    // The waiting strokes are Remove's own, whatever the bar's tool is (#12549): two field reads.
    if app.remove_trail.is_some() && crate::retouch_ui::remove_pending_live(app) {
        let waiting = OverlayTint::of(app, TrailTone::Danger);
        if let Some(trail) = app.remove_trail.as_mut() {
            trail.show_as(ui, rect, tl!("Waiting removals"), TrailTone::Danger, waiting);
        }
    }
}
