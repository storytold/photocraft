//! Arrow tool (U): quick annotation arrows. Drag from the tail to the tip. An arrow is an
//! ordinary shape layer (one open two-knot path, no fill, a stroke with arrowheads), so it renders,
//! saves and transforms like any shape and stays editable with Direct Selection and Properties.
//!
//! The options bar leads with quick styles that set colour, width, line and heads in one click;
//! every control also edits the selected arrow. With the Arrow tool the active arrow shows three
//! handles: drag an end to move it, drag the middle one to bend the arrow into a curve. Each
//! handle drag is one history step.

use egui::{Color32, Modifiers, Pos2, Stroke, vec2};
use photocraft_doc::{Arrowhead, LayerContent, LineCap, ShapeLayer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::shape_stroke_ui::{ArrowEnds, StrokeOptions};
use crate::state::Tool;
use crate::theme::Tokens;

/// Screen points within which a handle can be grabbed.
const GRAB: f64 = 9.0;

/// Head sizes offered in the options bar: (name, width, length) in stroke widths.
const SIZES: [(&str, f32, f32); 3] = [("Small", 3.0, 3.0), ("Medium", 4.0, 4.5), ("Large", 5.5, 6.0)];

/// Head shapes: (name, concavity).
const SHAPES: [(&str, f32); 2] = [("Triangle", 0.0), ("Barbed", 0.35)];

/// Line patterns, as the stroke options name them.
const LINES: [&str; 3] = ["Solid", "Dashed", "Dotted"];

/// The look of the next arrow (and of the selected one while it is edited).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArrowOptions {
    /// Straight RGBA.
    pub color: [f32; 4],
    /// Line width in px (the options bar shows it in points at the document's resolution).
    pub width: f32,
    /// Line pattern, caps and heads.
    pub style: StrokeOptions,
    /// A thin contrasting outline (a Stroke layer style) so the arrow reads on dark and bright
    /// parts of a scan alike.
    pub outline: bool,
    /// New arrows start bent (a gentle curve) instead of straight.
    pub curved: bool,
}

impl Default for ArrowOptions {
    fn default() -> Self {
        QUICK_STYLES[0].options(SCREEN_PX_PER_PT)
    }
}

impl ArrowOptions {
    /// `stroke` params for `shape.create` / `shape.edit`.
    pub fn stroke_params(&self) -> Value {
        let mut p = self.style.params();
        p["width"] = json!(self.width);
        p["color"] = json!(self.color);
        p
    }

    fn stroke(&self) -> photocraft_doc::ShapeStroke {
        let c = self.color;
        self.style.stroke(self.width, photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(c[0], c[1], c[2], c[3])))
    }

    fn of(stroke: &photocraft_doc::ShapeStroke, outline: bool) -> Option<Self> {
        let color = crate::shape_paint_ui::color_of(&stroke.paint)?;
        Some(ArrowOptions { color, width: stroke.width, style: StrokeOptions::from(stroke), outline, curved: false })
    }

    /// `layer.layerStyle.stroke` params for the outline of arrow `layer`: about a fifth of the
    /// line width, black on light colours and white on dark ones.
    fn outline_params(&self, layer: u64) -> Value {
        let [r, g, b, _] = self.color;
        let light = 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.45;
        json!({"layer": layer, "size": (self.width * 0.2).max(1.0).round(), "position": "outside", "color": if light { "#000000" } else { "#ffffff" }})
    }

    fn line(&self) -> &'static str {
        LINES
            .into_iter()
            .find(|name| {
                let mut s = self.style.clone();
                s.preset(name);
                s.dashes == self.style.dashes && (*name != "Dotted" || s.cap == self.style.cap)
            })
            .unwrap_or("Custom")
    }

    fn head(&self) -> Arrowhead {
        self.style.arrow_end.or(self.style.arrow_start).unwrap_or(Arrowhead { width: SIZES[1].1, length: SIZES[1].2, concavity: 0.0 })
    }

    fn set_head(&mut self, h: Arrowhead) {
        self.style.arrow_start = self.style.arrow_start.map(|_| h);
        self.style.arrow_end = self.style.arrow_end.map(|_| h);
    }
}

/// A one-click look, like Snagit's quick styles.
struct QuickStyle {
    name: &'static str,
    color: [f32; 4],
    /// Line width in points.
    width: f32,
    outline: bool,
    curved: bool,
    line: &'static str,
    ends: ArrowEnds,
    size: usize,
    shape: usize,
}

impl QuickStyle {
    /// The look at `px_per_pt` pixels per point (the document's resolution).
    fn options(&self, px_per_pt: f32) -> ArrowOptions {
        let mut style = StrokeOptions { cap: LineCap::Round, ..StrokeOptions::default() };
        style.preset(self.line);
        let (_, w, l) = SIZES[self.size.min(SIZES.len() - 1)];
        let (_, c) = SHAPES[self.shape.min(SHAPES.len() - 1)];
        style.arrow_end = Some(Arrowhead { width: w, length: l, concavity: c });
        style.set_arrow_ends(self.ends);
        ArrowOptions { color: self.color, width: (self.width * px_per_pt * 4.0).round() / 4.0, style, outline: self.outline, curved: self.curved }
    }
}

/// Pixels per point on screen-resolution documents (72 dpi screenshots): widths are not allowed
/// to get thinner than at 144 dpi, so a 2 pt arrow is still 4 px on a screenshot.
pub const SCREEN_PX_PER_PT: f32 = 2.0;

/// Pixels per point in the active document (at least [`SCREEN_PX_PER_PT`]): 300 dpi gives 4.17.
pub fn px_per_pt(app: &PhotocraftApp) -> f32 {
    app.session.active().map_or(SCREEN_PX_PER_PT, |st| (st.doc.resolution_dpi / 72.0).max(SCREEN_PX_PER_PT))
}

const RED: [f32; 4] = [0.894, 0.153, 0.122, 1.0];
const YELLOW: [f32; 4] = [1.0, 0.8, 0.0, 1.0];
const QUICK_STYLES: [QuickStyle; 7] = [
    QuickStyle { name: "Red arrow", color: RED, width: 2.0, outline: false, curved: false, line: "Solid", ends: ArrowEnds::End, size: 1, shape: 0 },
    QuickStyle { name: "Red double arrow", color: RED, width: 2.0, outline: false, curved: false, line: "Solid", ends: ArrowEnds::Both, size: 1, shape: 0 },
    QuickStyle { name: "Red curved arrow", color: RED, width: 2.0, outline: false, curved: true, line: "Solid", ends: ArrowEnds::End, size: 1, shape: 1 },
    QuickStyle { name: "Red dashed arrow", color: RED, width: 1.75, outline: false, curved: false, line: "Dashed", ends: ArrowEnds::End, size: 1, shape: 0 },
    QuickStyle {
        name: "Black dotted arrow",
        color: [0.0, 0.0, 0.0, 1.0],
        width: 1.75,
        outline: false,
        curved: false,
        line: "Dotted",
        ends: ArrowEnds::End,
        size: 1,
        shape: 0,
    },
    QuickStyle {
        name: "Yellow bold arrow with outline",
        color: YELLOW,
        width: 3.5,
        outline: true,
        curved: false,
        line: "Solid",
        ends: ArrowEnds::End,
        size: 1,
        shape: 1,
    },
    QuickStyle {
        name: "White arrow with outline",
        color: [1.0, 1.0, 1.0, 1.0],
        width: 2.5,
        outline: true,
        curved: false,
        line: "Solid",
        ends: ArrowEnds::End,
        size: 1,
        shape: 0,
    },
];

/// An arrow's geometry: tail, the tail's and tip's control points, tip (one cubic segment).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    pub p0: [f64; 2],
    pub c0: [f64; 2],
    pub c1: [f64; 2],
    pub p1: [f64; 2],
}

impl Geom {
    pub fn straight(p0: [f64; 2], p1: [f64; 2]) -> Self {
        Geom { p0, c0: p0, c1: p1, p1 }
    }

    /// The point halfway along the curve (`t = 0.5`): the bend handle.
    pub fn mid(&self) -> [f64; 2] {
        let f = |i: usize| (self.p0[i] + 3.0 * self.c0[i] + 3.0 * self.c1[i] + self.p1[i]) / 8.0;
        [f(0), f(1)]
    }

    /// The curve from the same ends through `m` (a quadratic bend, written as a cubic).
    pub fn bent_through(&self, m: [f64; 2]) -> Self {
        let q = |i: usize| 2.0 * m[i] - (self.p0[i] + self.p1[i]) * 0.5;
        let c = [q(0), q(1)];
        let toward = |p: [f64; 2]| [p[0] + (c[0] - p[0]) * 2.0 / 3.0, p[1] + (c[1] - p[1]) * 2.0 / 3.0];
        Geom { p0: self.p0, c0: toward(self.p0), c1: toward(self.p1), p1: self.p1 }
    }

    /// The same bend (the middle's offset from the chord's midpoint) between new ends.
    pub fn with_ends(&self, p0: [f64; 2], p1: [f64; 2]) -> Self {
        let m = self.mid();
        let off = [m[0] - (self.p0[0] + self.p1[0]) * 0.5, m[1] - (self.p0[1] + self.p1[1]) * 0.5];
        let mid = [(p0[0] + p1[0]) * 0.5 + off[0], (p0[1] + p1[1]) * 0.5 + off[1]];
        Geom::straight(p0, p1).bent_through(mid)
    }

    /// `path` param for `shape.create` / `shape.edit`.
    pub fn path_json(&self) -> Value {
        json!({"subpaths": [{"closed": false, "knots": [
            {"anchor": self.p0, "in": self.p0, "out": self.c0, "smooth": false},
            {"anchor": self.p1, "in": self.c1, "out": self.p1, "smooth": false},
        ]}]})
    }

    fn of(sh: &ShapeLayer) -> Option<Self> {
        if sh.fill.is_some() || sh.stroke.is_none() {
            return None;
        }
        let [sp] = sh.path.subpaths.as_slice() else { return None };
        let [a, b] = sp.knots.as_slice() else { return None };
        if sp.closed {
            return None;
        }
        let p = |q: photocraft_geom::Point| [q.x, q.y];
        Some(Geom { p0: p(a.anchor), c0: p(a.out_ctrl), c1: p(b.in_ctrl), p1: p(b.anchor) })
    }
}

/// Which handle is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Tail,
    Bend,
    Tip,
}

/// A handle drag on the active arrow.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    layer: u64,
    handle: Handle,
    base: Geom,
    start: [f64; 2],
    coalesce: String,
}

/// The active layer when it is an arrow (or a plain line drawn with the Arrow tool).
fn target(app: &PhotocraftApp) -> Option<(u64, Geom, ArrowOptions)> {
    let st = app.session.active()?;
    let id = st.active_layer?;
    let layer = st.doc.layer(id)?;
    if !layer.visible {
        return None;
    }
    let LayerContent::Shape(sh) = &layer.content else { return None };
    let outline = layer.effects.items.iter().any(|e| matches!(e, photocraft_doc::Effect::Stroke(_)));
    Some((id.0, Geom::of(sh)?, ArrowOptions::of(sh.stroke.as_ref()?, outline)?))
}

/// Where `layer`'s outline (its Stroke effect) is in its layer style.
fn outline_index(app: &PhotocraftApp, layer: u64) -> Option<usize> {
    let l = app.session.active()?.doc.layer(photocraft_doc::LayerId(layer))?;
    l.effects.items.iter().position(|e| matches!(e, photocraft_doc::Effect::Stroke(_)))
}

fn handles(g: &Geom) -> [(Handle, [f64; 2]); 3] {
    [(Handle::Tail, g.p0), (Handle::Bend, g.mid()), (Handle::Tip, g.p1)]
}

fn hit(app: &PhotocraftApp, p: [f64; 2]) -> Option<(u64, Geom, Handle)> {
    if app.ui.tool != Tool::Arrow {
        return None;
    }
    let (layer, g, _) = target(app)?;
    let tol = GRAB / f64::from(app.point_zoom().max(0.01));
    handles(&g)
        .into_iter()
        .map(|(h, q)| (h, (q[0] - p[0]).hypot(q[1] - p[1])))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| (layer, g, h))
}

/// Is a handle under document point `p`, or one being dragged? The canvas shows a hand then.
pub fn hovers(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.ui.arrow_drag.is_some() || hit(app, p).is_some()
}

/// Canvas pointer events for the arrow handles; false when the event is not theirs (a press
/// elsewhere draws a new arrow).
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if mods.command || mods.alt {
                return false;
            }
            let Some((layer, base, handle)) = hit(app, [x, y]) else { return false };
            let coalesce = format!("arrow-handle-{}", app.ui.alloc_id());
            app.ui.arrow_drag = Some(Drag { layer, handle, base, start: [x, y], coalesce });
            app.prefs_rt.snap = None;
            app.prefs_rt.snap_lines.clear();
            true
        }
        ToolEvent::Move { x, y, .. } => {
            let Some(d) = app.ui.arrow_drag.clone() else { return false };
            apply(app, &d, [x, y], mods);
            true
        }
        ToolEvent::Up { x, y } => {
            let Some(d) = app.ui.arrow_drag.take() else { return false };
            apply(app, &d, [x, y], mods);
            true
        }
    }
}

/// Moves the dragged handle to follow the pointer at `p` (⇧ snaps a moved end to 45°).
fn apply(app: &mut PhotocraftApp, d: &Drag, p: [f64; 2], mods: Modifiers) {
    let delta = [p[0] - d.start[0], p[1] - d.start[1]];
    if delta == [0.0, 0.0] {
        return;
    }
    let g = d.base;
    let moved = |q: [f64; 2]| [q[0] + delta[0], q[1] + delta[1]];
    let next = match d.handle {
        Handle::Bend => g.bent_through(moved(g.mid())),
        Handle::Tail => g.with_ends(snap(g.p1, moved(g.p0), mods.shift), g.p1),
        Handle::Tip => g.with_ends(g.p0, snap(g.p0, moved(g.p1), mods.shift)),
    };
    if let Err(e) = app.run("shape.edit", json!({"layer": d.layer, "path": next.path_json(), "coalesce": d.coalesce})) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// `p`, rotated about `anchor` to the nearest 45° when `on`.
fn snap(anchor: [f64; 2], p: [f64; 2], on: bool) -> [f64; 2] {
    if !on {
        return p;
    }
    let (dx, dy) = (p[0] - anchor[0], p[1] - anchor[1]);
    let a = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
    let len = dx.hypot(dy);
    [anchor[0] + len * a.cos(), anchor[1] + len * a.sin()]
}

/// A new straight arrow from `start` to `end` with the options bar's look (⇧: 45° steps).
pub fn geometry(start: [f64; 2], end: [f64; 2], mods: Modifiers) -> Option<Geom> {
    let end = snap(start, end, mods.shift);
    ((end[0] - start[0]).hypot(end[1] - start[1]) >= 2.0).then(|| Geom::straight(start, end))
}

/// The arrow a drag would draw, for the canvas preview.
pub fn preview(app: &PhotocraftApp, g: &Geom) -> Option<ShapeLayer> {
    let g = shaped(app, g);
    let path = photocraft_engine::vector_cmds::parse_path(&g.path_json()).ok()?;
    Some(ShapeLayer { path, fill: None, stroke: Some(app.ui.tool_options.arrow.stroke()), ..Default::default() })
}

/// Creates an arrow layer for a finished drag.
pub fn create(app: &mut PhotocraftApp, g: &Geom) {
    let g = shaped(app, g);
    let name = app.session.active().map_or_else(|| "Arrow".to_string(), |st| st.doc.next_layer_name("Arrow"));
    // The layer and its outline are one history step.
    let key = format!("arrow-create-{}", app.ui.alloc_id());
    let o = app.ui.tool_options.arrow.clone();
    let p = json!({"kind": "path", "path": g.path_json(), "fill": null, "stroke": o.stroke_params(), "name": name, "coalesce": key});
    let r = app.run("shape.create", p).and_then(|info| match info["layer"].as_u64() {
        Some(layer) if o.outline => {
            let mut q = o.outline_params(layer);
            q["coalesce"] = json!(key);
            app.run("layer.layerStyle.stroke", q)
        }
        _ => Ok(info),
    });
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// `g` as the options draw it: bent into a gentle curve when Curved is on.
fn shaped(app: &PhotocraftApp, g: &Geom) -> Geom {
    if !app.ui.tool_options.arrow.curved {
        return *g;
    }
    let (dx, dy) = (g.p1[0] - g.p0[0], g.p1[1] - g.p0[1]);
    let m = g.mid();
    // A fifth of the length to the left of the direction of travel.
    g.bent_through([m[0] + dy * 0.2, m[1] - dx * 0.2])
}

/// Handles of the active arrow.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if app.ui.tool != Tool::Arrow {
        return;
    }
    let Some((_, g, _)) = target(app) else { return };
    let t = Tokens::get(painter.ctx());
    let scr = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let hovered = painter.ctx().input(|i| i.pointer.hover_pos()).map(|p| xf.to_doc(p)).and_then(|p| hit(app, p)).map(|(.., h)| h);
    let dragged = app.ui.arrow_drag.as_ref().map(|d| d.handle);
    for (h, p) in handles(&g) {
        let c = scr(p);
        let hot = hovered == Some(h) || dragged == Some(h);
        let r = if hot { 6.5 } else { 5.0 };
        // The bend handle is filled with the accent so it reads as "pull me".
        let fill = if h == Handle::Bend { t.accent } else { Color32::WHITE };
        painter.circle_filled(c, r + 1.5, Color32::from_black_alpha(140));
        painter.circle(c, r, fill, Stroke::new(1.5, if h == Handle::Bend { Color32::WHITE } else { t.accent }));
    }
}

// ---------------------------------------------------------------------------------------------
// Options bar

/// The Arrow tool's options bar: quick styles, colour, width, line, heads, head size and shape.
/// Edits apply to the selected arrow and to the next one.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    let t = Tokens::get(ui.ctx());
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(s).color(t.text_dim).size(12.0));
    };
    let target = target(app);
    let current =
        target.as_ref().map_or_else(|| app.ui.tool_options.arrow.clone(), |(.., o)| ArrowOptions { curved: app.ui.tool_options.arrow.curved, ..o.clone() });
    let mut next = current.clone();
    let mut coalesce: Option<&str> = None;
    lbl(ui, tl!("Quick styles:"));
    let scale = px_per_pt(app);
    for q in &QUICK_STYLES {
        let o = q.options(scale);
        if style_button(ui, &o, o == current).on_hover_text(tl!(q.name)).clicked() {
            next = o;
        }
    }
    crate::widgets::vline(ui, 22.0);
    lbl(ui, tl!("Color:"));
    if let Some(c) = crate::marker_ui::color_swatch(app, ui, current.color, tl!("Arrow color")) {
        next.color = c;
        coalesce = Some("color");
    }
    lbl(ui, tl!("Width:"));
    let k = px_per_pt(app);
    let mut pt = current.width / k;
    if crate::widgets::value_field(ui, &mut pt, 0.25..=40.0, "pt", 58.0).changed() {
        next.width = (pt * k * 4.0).round() / 4.0;
        coalesce = Some("width");
    }
    if crate::widgets::checkbox(ui, &mut next.outline, tl!("Outline")).changed() {
        coalesce = None;
    }
    if crate::widgets::checkbox(ui, &mut next.curved, tl!("Curved")).changed() {
        coalesce = None;
    }
    crate::widgets::vline(ui, 22.0);
    lbl(ui, tl!("Line:"));
    let mut line = current.line();
    let lines: Vec<(&str, &str)> = LINES.iter().map(|n| (*n, *n)).chain((line == "Custom").then_some(("Custom", "Custom"))).collect();
    if crate::widgets::dropdown(ui, "arrow-line", tl!("Line:"), &mut line, &lines, 82.0) && line != "Custom" {
        next.style.preset(line);
        if line != "Dotted" {
            next.style.cap = LineCap::Round;
        }
    }
    lbl(ui, tl!("Heads:"));
    let mut ends = current.style.arrow_ends();
    let opts = [(ArrowEnds::End, "End"), (ArrowEnds::Start, "Start"), (ArrowEnds::Both, "Both"), (ArrowEnds::None, "None")];
    if crate::widgets::dropdown(ui, "arrow-ends", tl!("Heads:"), &mut ends, &opts, 70.0) {
        let head = next.head();
        next.style.arrow_end = Some(head);
        next.style.set_arrow_ends(ends);
    }
    let head = current.head();
    let enabled = ends != ArrowEnds::None;
    ui.add_enabled_ui(enabled, |ui| {
        lbl(ui, tl!("Size:"));
        let mut size = SIZES.iter().position(|(_, w, l)| (*w - head.width).abs() < 1e-3 && (*l - head.length).abs() < 1e-3).unwrap_or(usize::MAX);
        let mut sizes: Vec<(usize, &str)> = SIZES.iter().enumerate().map(|(i, (n, ..))| (i, *n)).collect();
        if size == usize::MAX {
            sizes.push((usize::MAX, "Custom"));
        }
        if crate::widgets::dropdown(ui, "arrow-size", tl!("Size:"), &mut size, &sizes, 78.0)
            && let Some((_, w, l)) = SIZES.get(size)
        {
            next.set_head(Arrowhead { width: *w, length: *l, ..head });
        }
        let mut shape = SHAPES.iter().position(|(_, c)| (*c - head.concavity).abs() < 1e-3).unwrap_or(usize::MAX);
        let mut shapes: Vec<(usize, &str)> = SHAPES.iter().enumerate().map(|(i, (n, _))| (i, *n)).collect();
        if shape == usize::MAX {
            shapes.push((usize::MAX, "Custom"));
        }
        if crate::widgets::dropdown(ui, "arrow-shape", tl!("Shape"), &mut shape, &shapes, 82.0)
            && let Some((_, c)) = SHAPES.get(shape)
        {
            next.set_head(Arrowhead { concavity: *c, ..head });
        }
    });
    crate::widgets::vline(ui, 22.0);
    lbl(ui, tl!("Drag to draw · Drag the blue dot to bend · ⇧ 45°"));
    if next != current {
        app.ui.tool_options.arrow = next.clone();
        if let Some((layer, ..)) = target {
            let key = format!("arrow-options-{layer}-{}", coalesce.unwrap_or("style"));
            let restyled = (&next.color, next.width, &next.style) != (&current.color, current.width, &current.style);
            let mut r = Ok(Value::Null);
            if restyled {
                r = app.run("shape.edit", json!({"layer": layer, "stroke": next.stroke_params(), "coalesce": key}));
            }
            // The outline follows the colour and width; Curved only shapes new arrows.
            if r.is_ok() && (next.outline != current.outline || next.outline && restyled) {
                let q = if next.outline {
                    Some(("layer.layerStyle.stroke", next.outline_params(layer)))
                } else {
                    // Only the outline (the Stroke effect): the arrow's other effects stay.
                    outline_index(app, layer).map(|i| ("layer.layerStyle.clear", json!({"layer": layer, "effect": i})))
                };
                if let Some((cmd, mut q)) = q {
                    q["coalesce"] = json!(key);
                    r = app.run(cmd, q);
                }
            }
            if let Err(e) = r {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
    }
    true
}

/// A quick-style button: a small arrow drawn in that style.
fn style_button(ui: &mut egui::Ui, o: &ArrowOptions, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 22.0), egui::Sense::click());
    let _ = crate::icons::button_chrome(ui, rect, selected, resp.hovered());
    let c = o.color.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    let color = Color32::from_rgb(c[0], c[1], c[2]);
    let w = (o.width / 3.0).clamp(1.5, 3.5);
    let (a, b) = (rect.left_center() + vec2(5.0, 0.0), rect.right_center() - vec2(5.0, 0.0));
    let head = (w * 2.4).max(5.0);
    let ends = o.style.arrow_ends();
    let a2 = if matches!(ends, ArrowEnds::Start | ArrowEnds::Both) { a + vec2(head * 0.8, 0.0) } else { a };
    let b2 = if matches!(ends, ArrowEnds::End | ArrowEnds::Both) { b - vec2(head * 0.8, 0.0) } else { b };
    match o.line() {
        "Dashed" => dashes(ui.painter(), a2, b2, w * 2.2, w * 1.2, Stroke::new(w, color)),
        "Dotted" => {
            let mut x = a2.x;
            while x <= b2.x {
                ui.painter().circle_filled(Pos2::new(x, a2.y), w * 0.6, color);
                x += w * 2.0;
            }
        }
        _ => {
            ui.painter().line_segment([a2, b2], Stroke::new(w, color));
        }
    }
    let tri = |tip: Pos2, dir: f32| {
        let base = tip - vec2(dir * head, 0.0);
        vec![tip, base + vec2(0.0, head * 0.55), base - vec2(0.0, head * 0.55)]
    };
    if matches!(ends, ArrowEnds::End | ArrowEnds::Both) {
        ui.painter().add(egui::Shape::convex_polygon(tri(b, 1.0), color, Stroke::NONE));
    }
    if matches!(ends, ArrowEnds::Start | ArrowEnds::Both) {
        ui.painter().add(egui::Shape::convex_polygon(tri(a, -1.0), color, Stroke::NONE));
    }
    resp
}

fn dashes(painter: &egui::Painter, a: Pos2, b: Pos2, on: f32, off: f32, stroke: Stroke) {
    let mut x = a.x;
    while x < b.x {
        painter.line_segment([Pos2::new(x, a.y), Pos2::new((x + on).min(b.x), a.y)], stroke);
        x += on + off;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bend_handle_lies_on_the_curve_and_bending_keeps_the_ends() {
        let g = Geom::straight([0.0, 0.0], [100.0, 0.0]);
        assert_eq!(g.mid(), [50.0, 0.0]);
        let b = g.bent_through([50.0, 30.0]);
        assert_eq!((b.p0, b.p1), (g.p0, g.p1));
        let m = b.mid();
        assert!((m[0] - 50.0).abs() < 1e-9 && (m[1] - 30.0).abs() < 1e-9, "{m:?}");
    }

    #[test]
    fn moving_an_end_keeps_the_bend() {
        let b = Geom::straight([0.0, 0.0], [100.0, 0.0]).bent_through([50.0, 20.0]);
        let moved = b.with_ends([0.0, 0.0], [200.0, 0.0]);
        let m = moved.mid();
        assert!((m[0] - 100.0).abs() < 1e-9 && (m[1] - 20.0).abs() < 1e-9, "{m:?}");
    }

    #[test]
    fn quick_styles_are_valid_strokes() {
        for q in &QUICK_STYLES {
            let o = q.options(SCREEN_PX_PER_PT);
            assert!(o.stroke().validate().is_ok(), "{}", q.name);
            assert_eq!(o.line(), q.line, "{}", q.name);
            assert_eq!(o.style.arrow_ends(), q.ends, "{}", q.name);
        }
    }

    #[test]
    fn print_widths_outlines_and_curves() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 1200, "height": 800, "background": "black", "resolution": 300})).unwrap();
        assert!((px_per_pt(&app) - 300.0 / 72.0).abs() < 1e-4);
        // The yellow outlined quick style at 300 dpi: 3.5 pt = 14.5 px, with a black outline.
        app.ui.tool_options.arrow = QUICK_STYLES[5].options(px_per_pt(&app));
        create(&mut app, &Geom::straight([100.0, 100.0], [500.0, 100.0]));
        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let LayerContent::Shape(sh) = &layer.content else { panic!("an arrow is a shape") };
        assert_eq!(sh.stroke.as_ref().unwrap().width, 14.5);
        assert!(layer.effects.items.iter().any(|e| matches!(e, photocraft_doc::Effect::Stroke(_))), "outlined");
        let steps = st.history.past_len();
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().history.past_len() < steps, "arrow and outline are one step");
        // Curved: new arrows bend to the left of travel.
        app.ui.tool_options.arrow = QUICK_STYLES[2].options(px_per_pt(&app));
        create(&mut app, &Geom::straight([100.0, 300.0], [500.0, 300.0]));
        let (_, g, _) = target(&app).unwrap();
        assert!((g.mid()[1] - 220.0).abs() < 1e-6, "bent 80 px up: {:?}", g.mid());
    }

    #[test]
    fn turning_the_outline_off_keeps_other_effects() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 200, "background": "white"})).unwrap();
        app.ui.tool = Tool::Arrow;
        app.ui.tool_options.arrow = QUICK_STYLES[5].options(SCREEN_PX_PER_PT);
        create(&mut app, &Geom::straight([20.0, 100.0], [300.0, 100.0]));
        let (layer, ..) = target(&app).unwrap();
        app.run("layer.layerStyle.dropShadow", json!({"layer": layer, "add": true})).unwrap();
        let effects = |app: &PhotocraftApp| app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().effects.items.len();
        assert_eq!(effects(&app), 2);
        assert!(target(&app).unwrap().2.outline);
        // What the options bar sends when Outline is unticked.
        let i = outline_index(&app, layer).unwrap();
        app.run("layer.layerStyle.clear", json!({"layer": layer, "effect": i})).unwrap();
        assert_eq!(effects(&app), 1, "the drop shadow stays");
        assert!(outline_index(&app, layer).is_none());
    }

    #[test]
    fn tiny_drags_draw_nothing_and_shift_snaps() {
        assert!(geometry([10.0, 10.0], [10.5, 10.5], Modifiers::NONE).is_none());
        let g = geometry([0.0, 0.0], [100.0, 7.0], Modifiers::SHIFT).unwrap();
        assert!(g.p1[1].abs() < 1e-9, "{:?}", g.p1);
    }
}
