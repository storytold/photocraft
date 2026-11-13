//! The develop view's local tools on the preview (Masking, Heal, Red Eye), as LightCraft's loupe
//! has them: what a drag on the preview makes, the outlines, handles and pins drawn over it, and
//! the selected mask's red overlay. Each edits the dialog's develop settings (`masks`, `spots`,
//! `red_eye`), so undo, Before, the filmstrip and Open take them as any other setting.
//!
//! Coordinates go through the [`ViewMap`] of the frame shown: a point on the preview is a mask's
//! coordinate on the photo (before lens corrections, Upright and Transform), or the normalized
//! coordinate spots and red eyes use, and back.

use craft_develop_model::{BrushStroke, DevelopSettings, LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape, RedEye, Spot, SpotMode};
use craft_geom::Point;
use egui::{Color32, Pos2, Rect, Response, Stroke, pos2, vec2};
use photocraft_io::raw_develop::ViewMap;

use super::rail::Tool;

/// What a drag (or click) on the preview makes in Masking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Draw {
    /// Nothing: drags move the selected component's handles.
    #[default]
    None,
    Brush,
    Linear,
    Radial,
    /// Clicks sample a colour for the selected colour range.
    ColorPick,
}

/// The brush's settings (LightCraft's Size, Feather, Flow, and Add / Erase).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Brush {
    /// Radius as a fraction of the image's long edge.
    pub size: f64,
    pub feather: f64,
    pub flow: f64,
    pub erase: bool,
}

impl Default for Brush {
    fn default() -> Self {
        Brush { size: 0.03, feather: 50.0, flow: 100.0, erase: false }
    }
}

/// The settings a new spot takes (Heal's Size, Feather, Opacity and mode).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Heal {
    pub mode: SpotMode,
    pub size: f64,
    pub feather: f64,
    pub opacity: f64,
}

impl Default for Heal {
    fn default() -> Self {
        Heal { mode: SpotMode::Heal, size: 0.02, feather: 50.0, opacity: 100.0 }
    }
}

/// A gesture on the preview in progress.
#[derive(Clone, Debug, PartialEq)]
enum Gesture {
    /// Drawing the selected linear or radial component anew from a press at this mask point.
    Shape(Point),
    /// Painting a brush stroke (its index in the component).
    Stroke(usize),
    /// Moving handle `handle` of component `comp`, as it was at the press.
    Handle { comp: usize, handle: u8, shape: MaskShape },
    /// Moving spot `index`'s target (`source` false) or source, from the press at this normalized point.
    Spot { index: usize, source: bool, press: Point, was: Spot },
    /// Brushing a new spot's path.
    SpotPath(usize),
    /// Drawing red eye `index`'s ellipse from a press at this normalized point.
    Eye { index: usize, press: Point },
}

/// The local tools' state (not settings: what is selected and how the tools are set).
#[derive(Clone)]
pub(crate) struct LocalTools {
    /// The selected mask (its id).
    pub selected: Option<u32>,
    /// The selected component of the selected mask.
    pub component: usize,
    pub draw: Draw,
    pub brush: Brush,
    /// Show the selected mask's red overlay.
    pub overlay: bool,
    /// A mask being renamed, and the name typed so far.
    pub renaming: Option<(u32, String)>,
    pub heal: Heal,
    /// The selected spot.
    pub spot: Option<usize>,
    /// The selected red eye.
    pub eye: Option<usize>,
    /// New red eyes are pet eyes.
    pub pet: bool,
    /// The preview navigates (Hand, Zoom) instead of the tool until a tool is picked again.
    pub paused: bool,
    gesture: Option<Gesture>,
    /// How the shown frame maps to the settings' coordinates.
    pub view_map: Option<ViewMap>,
    /// The overlay of the shown frame, and the mask it shows.
    pub(super) overlay_tex: Option<(egui::TextureHandle, u32)>,
}

impl Default for LocalTools {
    fn default() -> Self {
        LocalTools {
            selected: None,
            component: 0,
            draw: Draw::None,
            brush: Brush::default(),
            overlay: true,
            renaming: None,
            heal: Heal::default(),
            spot: None,
            eye: None,
            pet: false,
            paused: false,
            gesture: None,
            view_map: None,
            overlay_tex: None,
        }
    }
}

impl LocalTools {
    /// Drops what no longer exists in `s` (after an undo, a reset, a mask deleted elsewhere).
    pub(crate) fn validate(&mut self, s: &DevelopSettings) {
        match self.selected.and_then(|id| s.masks.iter().find(|m| m.id == id)) {
            Some(m) => self.component = self.component.min(m.components.len().saturating_sub(1)),
            None => {
                self.selected = None;
                self.component = 0;
                if self.draw == Draw::ColorPick || self.draw == Draw::Brush {
                    self.draw = Draw::None;
                }
            }
        }
        if self.spot.is_some_and(|i| i >= s.spots.len()) {
            self.spot = None;
        }
        if self.eye.is_some_and(|i| i >= s.red_eye.len()) {
            self.eye = None;
        }
    }

    /// The mask the overlay shows this frame, if any.
    pub(crate) fn overlay_mask(&self, tool: Tool, s: &DevelopSettings) -> Option<u32> {
        let id = self.selected.filter(|_| tool == Tool::Masking && self.overlay)?;
        s.masks.iter().any(|m| m.id == id && !m.components.is_empty()).then_some(id)
    }

    /// Ends whatever gesture was going on (the tool changed).
    pub(crate) fn end_gesture(&mut self) {
        self.gesture = None;
    }
}

/// The name of a mask kind, as the panel shows it.
pub(crate) fn kind_label(s: &MaskShape) -> &'static str {
    match s {
        MaskShape::Brush { .. } => "Brush",
        MaskShape::Linear { .. } => "Linear Gradient",
        MaskShape::Radial { .. } => "Radial Gradient",
        MaskShape::ColorRange { .. } => "Color Range",
        MaskShape::LuminanceRange { .. } => "Luminance Range",
        MaskShape::DepthRange { .. } => "Depth Range",
        MaskShape::Subject => "Subject",
        MaskShape::Sky => "Sky",
        MaskShape::Background => "Background",
        MaskShape::Object { .. } => "Object",
        MaskShape::Prompt { .. } => "Describe",
        MaskShape::People { .. } => "People",
        MaskShape::Landscape { .. } => "Landscape",
    }
}

/// The non-AI mask kinds the dialog makes (the AI ones need a segmentation model PhotoCraft
/// doesn't have, so they are not offered).
pub(crate) const KINDS: [&str; 5] = ["brush", "linear", "radial", "luminanceRange", "colorRange"];

/// A new component of kind `kind` (one of [`KINDS`]) with LightCraft's defaults.
pub(crate) fn new_shape(kind: &str) -> Option<MaskShape> {
    Some(match kind {
        "brush" => MaskShape::Brush { strokes: vec![] },
        "linear" => MaskShape::Linear { start: Point::new(0.5, 0.25), end: Point::new(0.5, 0.6) },
        "radial" => MaskShape::Radial { center: Point::new(0.5, 0.5), rx: 0.22, ry: 0.16, angle: 0.0, feather: 50.0, invert: false },
        "luminanceRange" => MaskShape::LuminanceRange { lo: 0.6, hi: 1.0, lo_feather: 0.1, hi_feather: 0.1 },
        "colorRange" => MaskShape::ColorRange { samples: vec![], refine: 50.0 },
        _ => return None,
    })
}

/// What a drag on the preview does with component `shape` selected.
pub(crate) fn draw_of(shape: &MaskShape) -> Draw {
    match shape {
        MaskShape::Brush { .. } => Draw::Brush,
        MaskShape::Linear { .. } => Draw::Linear,
        MaskShape::Radial { .. } => Draw::Radial,
        MaskShape::ColorRange { .. } => Draw::ColorPick,
        _ => Draw::None,
    }
}

/// Selects mask `id` (its first component), or none.
pub(crate) fn select(st: &mut LocalTools, s: &DevelopSettings, id: Option<u32>) {
    st.selected = id.filter(|id| s.masks.iter().any(|m| m.id == *id));
    st.component = 0;
    st.draw = st.selected.and_then(|id| s.masks.iter().find(|m| m.id == id)).and_then(|m| m.components.first()).map_or(Draw::None, |c| draw_of(&c.shape));
}

/// What a drag on the preview does once a component of `kind` is made.
fn draw_for(kind: &str) -> Draw {
    match kind {
        "brush" => Draw::Brush,
        "linear" => Draw::Linear,
        "radial" => Draw::Radial,
        "colorRange" => Draw::ColorPick,
        _ => Draw::None,
    }
}

/// Adds a mask of `kind` to `s` and selects it. `Err` for a kind the dialog doesn't make.
pub(crate) fn add_mask(st: &mut LocalTools, s: &mut DevelopSettings, kind: &str) -> Result<u32, String> {
    let shape = new_shape(kind).ok_or_else(|| format!("unknown mask kind {kind}: expected one of {}", KINDS.join(", ")))?;
    let id = s.masks.iter().map(|m| m.id).max().map_or(1, |m| m.saturating_add(1));
    let n = s.masks.len() + 1;
    s.masks.push(Mask {
        id,
        name: format!("Mask {n}"),
        components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape }],
        ..Mask::default()
    });
    st.selected = Some(id);
    st.component = 0;
    st.draw = draw_for(kind);
    st.paused = false;
    Ok(id)
}

/// Adds a component of `kind` to the selected mask with `op` (Add or Subtract) and selects it.
pub(crate) fn add_component(st: &mut LocalTools, s: &mut DevelopSettings, kind: &str, op: MaskOp) -> Result<(), String> {
    let shape = new_shape(kind).ok_or_else(|| format!("unknown mask kind {kind}"))?;
    let m = st.selected.and_then(|id| s.masks.iter_mut().find(|m| m.id == id)).ok_or("no mask is selected")?;
    m.components.push(MaskComponent { name: None, op, invert: false, shape });
    st.component = m.components.len() - 1;
    st.draw = draw_for(kind);
    st.paused = false;
    Ok(())
}

pub(crate) fn selected_mask<'a>(st: &LocalTools, s: &'a mut DevelopSettings) -> Option<&'a mut Mask> {
    let id = st.selected?;
    s.masks.iter_mut().find(|m| m.id == id)
}

/// The local adjustment slider `id` of `a` (LightCraft's field names).
pub(crate) fn local_field<'a>(a: &'a mut LocalAdjustments, id: &str) -> Option<&'a mut f64> {
    Some(match id {
        "temp" => &mut a.temp,
        "tint" => &mut a.tint,
        "exposure" => &mut a.exposure,
        "contrast" => &mut a.contrast,
        "highlights" => &mut a.highlights,
        "shadows" => &mut a.shadows,
        "whites" => &mut a.whites,
        "blacks" => &mut a.blacks,
        "texture" => &mut a.texture,
        "clarity" => &mut a.clarity,
        "dehaze" => &mut a.dehaze,
        "hue" => &mut a.hue,
        "saturation" => &mut a.saturation,
        "sharpness" => &mut a.sharpness,
        "noise" => &mut a.noise,
        "moire" => &mut a.moire,
        "defringe" => &mut a.defringe,
        "amount" => &mut a.amount,
        _ => return None,
    })
}

/// The preview's image rect on screen and the frame's map: screen points ↔ settings coordinates.
pub(crate) struct Canvas<'a> {
    pub r: Rect,
    pub map: &'a ViewMap,
}

impl Canvas<'_> {
    fn unit(&self, p: Pos2) -> Point {
        Point::new(f64::from((p.x - self.r.left()) / self.r.width().max(1e-3)), f64::from((p.y - self.r.top()) / self.r.height().max(1e-3)))
    }
    fn screen(&self, u: Point) -> Pos2 {
        pos2(self.r.left() + u.x as f32 * self.r.width(), self.r.top() + u.y as f32 * self.r.height())
    }
    pub(crate) fn screen_to_mask(&self, on_photo: bool, p: Pos2) -> Point {
        self.map.to_mask(on_photo, self.unit(p))
    }
    pub(crate) fn mask_to_screen(&self, on_photo: bool, n: Point) -> Option<Pos2> {
        self.map.from_mask(on_photo, n).map(|u| self.screen(u))
    }
    pub(crate) fn screen_to_norm(&self, p: Pos2) -> Point {
        self.map.to_norm(self.unit(p))
    }
    pub(crate) fn norm_to_screen(&self, n: Point) -> Pos2 {
        self.screen(self.map.from_norm(n))
    }
    /// Screen points per unit of the image's long edge.
    pub(crate) fn pts_per_long(&self) -> f32 {
        let (w, _) = self.map.size();
        (self.map.px_per_long() * f64::from(self.r.width()) / w as f64) as f32
    }
    /// Normalized → long-edge units (isotropic).
    fn long(&self, n: Point) -> Point {
        let (sx, sy) = self.map.long_scale();
        Point::new(n.x * sx, n.y * sy)
    }
    fn norm(&self, l: Point) -> Point {
        let (sx, sy) = self.map.long_scale();
        Point::new(l.x / sx.max(1e-9), l.y / sy.max(1e-9))
    }
}

/// The handles of a gradient: (handle id, its mask coordinates).
fn handles(c: &Canvas, shape: &MaskShape) -> Vec<(u8, Point)> {
    match shape {
        MaskShape::Linear { start, end } => vec![(0, *start), (1, *end), (2, Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0))],
        MaskShape::Radial { center, rx, ry, angle, .. } => {
            let cl = c.long(*center);
            let (s, co) = angle.to_radians().sin_cos();
            let x = c.norm(Point::new(cl.x + rx * co, cl.y + rx * s));
            let y = c.norm(Point::new(cl.x - ry * s, cl.y + ry * co));
            vec![(0, *center), (1, x), (2, y)]
        }
        _ => vec![],
    }
}

/// `shape` with handle `handle` moved to mask point `at` (`press` where the drag began).
fn moved(c: &Canvas, shape: &MaskShape, handle: u8, press: Point, at: Point) -> MaskShape {
    let d = Point::new(at.x - press.x, at.y - press.y);
    match shape.clone() {
        MaskShape::Linear { start, end } => match handle {
            0 => MaskShape::Linear { start: at, end },
            1 => MaskShape::Linear { start, end: at },
            _ => MaskShape::Linear { start: Point::new(start.x + d.x, start.y + d.y), end: Point::new(end.x + d.x, end.y + d.y) },
        },
        MaskShape::Radial { center, rx, ry, angle, feather, invert } => {
            let (s, co) = angle.to_radians().sin_cos();
            let (cl, al) = (c.long(center), c.long(at));
            let v = Point::new(al.x - cl.x, al.y - cl.y);
            match handle {
                1 => MaskShape::Radial { center, rx: (v.x * co + v.y * s).abs().max(0.002), ry, angle, feather, invert },
                2 => MaskShape::Radial { center, rx, ry: (-v.x * s + v.y * co).abs().max(0.002), angle, feather, invert },
                _ => MaskShape::Radial { center: Point::new(center.x + d.x, center.y + d.y), rx, ry, angle, feather, invert },
            }
        }
        other => other,
    }
}

/// A gradient drawn anew from a press at `a` to `b` (mask points): a linear from `a` to `b`, a
/// radial centred on `a` reaching `b` (a circle with Shift).
fn drawn(c: &Canvas, shape: &MaskShape, a: Point, b: Point, circle: bool) -> MaskShape {
    match shape {
        MaskShape::Linear { .. } => MaskShape::Linear { start: a, end: b },
        MaskShape::Radial { feather, invert, .. } => {
            let (al, bl) = (c.long(a), c.long(b));
            let (mut rx, mut ry) = ((bl.x - al.x).abs().max(0.005), (bl.y - al.y).abs().max(0.005));
            if circle {
                rx = rx.hypot(ry);
                ry = rx;
            }
            MaskShape::Radial { center: a, rx, ry, angle: 0.0, feather: *feather, invert: *invert }
        }
        other => other.clone(),
    }
}

/// Where a component's pin goes (gradients and brushes), in mask coordinates.
fn pin(shape: &MaskShape) -> Option<Point> {
    match shape {
        MaskShape::Linear { start, end } => Some(Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0)),
        MaskShape::Radial { center, .. } => Some(*center),
        MaskShape::Brush { strokes } => strokes.iter().find_map(|s| s.points.first().copied()),
        _ => None,
    }
}

/// White with a dark keyline, so it reads on any photo.
fn outline(p: &egui::Painter, pts: Vec<Pos2>, closed: bool) {
    let shape = |s: Stroke| if closed { egui::Shape::closed_line(pts.clone(), s) } else { egui::Shape::line(pts.clone(), s) };
    p.add(shape(Stroke::new(3.0, Color32::from_black_alpha(110))));
    p.add(shape(Stroke::new(1.2, Color32::WHITE)));
}

fn handle_dot(p: &egui::Painter, at: Pos2, main: bool, hot: bool) {
    let r = if main { 6.5 } else { 5.0 };
    p.circle_filled(at, r + 1.0, Color32::from_black_alpha(140));
    p.circle_filled(at, r, if main { Color32::from_rgb(0x1f, 0x6f, 0xe8) } else { Color32::from_gray(225) });
    p.circle_stroke(at, r, Stroke::new(if hot { 2.0 } else { 1.2 }, Color32::WHITE));
}

/// The outline of `shape` on screen.
fn draw_shape(p: &egui::Painter, c: &Canvas, on_photo: bool, shape: &MaskShape) {
    match shape {
        MaskShape::Radial { center, rx, ry, angle, feather, .. } => {
            let cl = c.long(*center);
            let (s, co) = angle.to_radians().sin_cos();
            let ring = |k: f64| -> Vec<Pos2> {
                (0..72)
                    .filter_map(|i| {
                        let th = f64::from(i) / 72.0 * std::f64::consts::TAU;
                        let (x, y) = (rx * k * th.cos(), ry * k * th.sin());
                        c.mask_to_screen(on_photo, c.norm(Point::new(cl.x + x * co - y * s, cl.y + x * s + y * co)))
                    })
                    .collect()
            };
            outline(p, ring(1.0), true);
            // the feather's inner edge, faint
            let inner = 1.0 - (feather / 100.0).clamp(0.0, 0.99);
            if inner < 0.98 {
                p.add(egui::Shape::closed_line(ring(inner), Stroke::new(1.0, Color32::from_white_alpha(90))));
            }
        }
        MaskShape::Linear { start, end } => {
            let (a, b) = (c.long(*start), c.long(*end));
            let d = Point::new(b.x - a.x, b.y - a.y);
            let len = d.x.hypot(d.y).max(1e-9);
            let n = Point::new(-d.y / len * 2.0, d.x / len * 2.0);
            for (k, at) in [(0, a), (1, Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)), (2, b)] {
                let ends = [c.norm(Point::new(at.x - n.x, at.y - n.y)), c.norm(Point::new(at.x + n.x, at.y + n.y))];
                let pts: Vec<Pos2> = (0..=16)
                    .filter_map(|i| {
                        let t = f64::from(i) / 16.0;
                        c.mask_to_screen(on_photo, Point::new(ends[0].x + (ends[1].x - ends[0].x) * t, ends[0].y + (ends[1].y - ends[0].y) * t))
                    })
                    .collect();
                if k == 1 {
                    p.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, Color32::from_white_alpha(170)), 6.0, 4.0));
                } else {
                    outline(p, pts, false);
                }
            }
        }
        _ => {}
    }
}

/// Paints the selected mask's red overlay over the image rect `r`.
pub(crate) fn paint_overlay(ui: &egui::Ui, st: &LocalTools, r: Rect, tool: Tool, s: &DevelopSettings) {
    if let (Some((tex, id)), Some(shown)) = (&st.overlay_tex, st.overlay_mask(tool, s))
        && *id == shown
    {
        ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// The overlay image of a mask alpha (`w`×`h`, 0..255): LightCraft's red tint at half strength.
pub(crate) fn overlay_image(alpha: &[u8], w: usize, h: usize) -> Option<egui::ColorImage> {
    if alpha.len() != w.checked_mul(h)? {
        return None;
    }
    let px = alpha.iter().map(|a| Color32::from_rgba_unmultiplied(0xe0, 0x20, 0x30, (u16::from(*a) * 110 / 255) as u8)).collect();
    Some(egui::ColorImage { size: [w, h], source_size: vec2(w as f32, h as f32), pixels: px })
}

/// What a frame of a local tool did on the preview.
#[derive(Default)]
pub(crate) struct PreviewOut {
    /// The settings changed.
    pub edited: bool,
}

/// The local tool `tool` on the preview: its outlines and handles over the image rect `r`, and
/// what the pointer does there (`response`: the preview's, `panning` while the Hand pans).
/// `sample`: a colour range's colour at a normalized point.
// (the tool, its state and the settings it edits, the preview's rect and response, and how a
// colour is sampled: what each tool's gesture reads)
#[allow(clippy::too_many_arguments)]
pub(crate) fn preview(
    ui: &mut egui::Ui,
    st: &mut LocalTools,
    tool: Tool,
    s: &mut DevelopSettings,
    r: Rect,
    response: &Response,
    panning: bool,
    sample: &dyn Fn(&DevelopSettings, Point) -> Option<[f64; 3]>,
) -> PreviewOut {
    let mut out = PreviewOut::default();
    let Some(map) = st.view_map.clone() else { return out };
    let c = Canvas { r, map: &map };
    let p = ui.painter().clone();
    let live = !panning && !st.paused;
    let press = response.drag_started() || response.clicked();
    // a drag starts where it was pressed, not where egui saw it move far enough to be one
    let origin = if response.drag_started() { ui.input(|i| i.pointer.press_origin()) } else { None };
    let pointer = origin.or(response.interact_pointer_pos()).or(response.hover_pos());
    // Delete (or Backspace) removes the selected mask, spot or red eye
    let delete = !ui.ctx().text_edit_focused()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace));
    if delete {
        out.edited = delete_selected(st, tool, s);
    }
    match tool {
        Tool::Edit => {}
        Tool::Masking => masking(ui, st, s, &c, &p, response, live, press, pointer, sample, &mut out),
        Tool::Heal => heal(ui, st, s, &c, &p, response, live, press, pointer, &mut out),
        Tool::RedEye => red_eye(st, s, &c, &p, response, live, press, pointer, &mut out),
    }
    if !(response.dragged() || response.drag_started()) || !live {
        st.gesture = None;
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn masking(
    ui: &mut egui::Ui,
    st: &mut LocalTools,
    s: &mut DevelopSettings,
    c: &Canvas,
    p: &egui::Painter,
    response: &Response,
    live: bool,
    press: bool,
    pointer: Option<Pos2>,
    sample: &dyn Fn(&DevelopSettings, Point) -> Option<[f64; 3]>,
    out: &mut PreviewOut,
) {
    // the other masks' pins: a click selects that mask
    let grabbed = 10.0;
    let mut pins: Vec<(u32, Pos2)> = Vec::new();
    for m in s.masks.iter().filter(|m| Some(m.id) != st.selected) {
        if let Some(at) = m.components.iter().find_map(|k| pin(&k.shape)).and_then(|n| c.mask_to_screen(m.on_photo, n)) {
            pins.push((m.id, at));
            p.circle_filled(at, 5.5, Color32::from_black_alpha(140));
            p.circle_stroke(at, 4.5, Stroke::new(1.5, Color32::WHITE));
        }
    }
    let shift = ui.input(|i| i.modifiers.shift);
    let alt = ui.input(|i| i.modifiers.alt);
    if live
        && press
        && st.gesture.is_none()
        && let Some(at) = pointer
        && let Some((id, _)) = pins.iter().find(|(_, q)| q.distance(at) <= grabbed)
    {
        select(st, s, Some(*id));
        return;
    }
    let draw = st.draw;
    let comp = st.component;
    let brush = st.brush;
    let Some(m) = selected_mask(st, s) else { return };
    let on_photo = m.on_photo;
    for (i, k) in m.components.iter().enumerate() {
        draw_shape(p, c, on_photo, &k.shape);
        if i == comp {
            for (h, at) in handles(c, &k.shape) {
                if let Some(q) = c.mask_to_screen(on_photo, at) {
                    let hot = pointer.is_some_and(|ptr| ptr.distance(q) <= grabbed);
                    handle_dot(p, q, h == 0 && matches!(k.shape, MaskShape::Radial { .. }) || h == 2 && matches!(k.shape, MaskShape::Linear { .. }), hot);
                }
            }
        }
    }
    let Some(at) = pointer else { return };
    let mask_at = c.screen_to_mask(on_photo, at);
    if draw == Draw::Brush && response.hovered() {
        // the brush's size and feather under the pointer
        let rad = (brush.size as f32 * c.pts_per_long()).max(1.0);
        p.circle_stroke(at, rad, Stroke::new(1.0, Color32::WHITE));
        p.circle_stroke(at, rad * (1.0 - (brush.feather as f32 / 100.0) * 0.5), Stroke::new(1.0, Color32::from_white_alpha(120)));
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if !live {
        return;
    }
    // a handle of the selected component under the press
    if press && st.gesture.is_none() {
        let shape = m.components.get(comp).map(|k| k.shape.clone());
        if let Some(shape) = shape
            && let Some((h, _)) = handles(c, &shape).into_iter().find(|(_, n)| c.mask_to_screen(on_photo, *n).is_some_and(|q| q.distance(at) <= grabbed))
        {
            st.gesture = Some(Gesture::Handle { comp, handle: h, shape });
        } else {
            match draw {
                Draw::Linear | Draw::Radial => st.gesture = Some(Gesture::Shape(mask_at)),
                Draw::Brush => {
                    if let Some(MaskShape::Brush { strokes }) = m.components.get_mut(comp).map(|k| &mut k.shape) {
                        strokes.push(BrushStroke {
                            points: vec![mask_at],
                            size: brush.size,
                            feather: brush.feather,
                            flow: brush.flow,
                            erase: brush.erase || alt,
                            ..BrushStroke::default()
                        });
                        st.gesture = Some(Gesture::Stroke(strokes.len() - 1));
                        out.edited = true;
                    }
                }
                Draw::ColorPick => {
                    if let Some(lab) = sample(s, c.screen_to_norm(at)) {
                        let Some(m) = selected_mask(st, s) else { return };
                        if let Some(MaskShape::ColorRange { samples, .. }) = m.components.get_mut(comp).map(|k| &mut k.shape) {
                            // a click picks the colour, ⇧-click adds one (up to five, as LightCraft)
                            if !shift {
                                samples.clear();
                            }
                            if samples.len() < 5 {
                                samples.push(lab);
                            }
                            out.edited = true;
                        }
                    }
                    return;
                }
                Draw::None => {}
            }
        }
    }
    let Some(m) = selected_mask(st, s) else { return };
    match st.gesture.clone() {
        Some(Gesture::Handle { comp, handle, shape }) if response.dragged() => {
            let press_at = ui.input(|i| i.pointer.press_origin()).map(|o| c.screen_to_mask(on_photo, o)).unwrap_or(mask_at);
            if let Some(k) = m.components.get_mut(comp) {
                let next = moved(c, &shape, handle, press_at, mask_at);
                if k.shape != next {
                    k.shape = next;
                    out.edited = true;
                }
            }
        }
        Some(Gesture::Shape(a)) if response.dragged() => {
            if let Some(k) = m.components.get_mut(comp) {
                let next = drawn(c, &k.shape, a, mask_at, shift);
                if k.shape != next {
                    k.shape = next;
                    out.edited = true;
                }
            }
        }
        Some(Gesture::Stroke(i)) if response.dragged() => {
            if let Some(MaskShape::Brush { strokes }) = m.components.get_mut(comp).map(|k| &mut k.shape)
                && let Some(stroke) = strokes.get_mut(i)
            {
                // a dab every quarter of the brush's radius
                let spacing = (brush.size * 0.25).max(1e-4);
                let far = stroke.points.last().is_none_or(|q| {
                    let (a, b) = (c.long(*q), c.long(mask_at));
                    (a.x - b.x).hypot(a.y - b.y) >= spacing
                });
                if far && stroke.points.len() < 4096 {
                    stroke.points.push(mask_at);
                    out.edited = true;
                }
            }
        }
        _ => {}
    }
}

/// Removes the selected mask (Masking), spot (Heal) or red eye (Red Eye). Says whether it did.
pub(crate) fn delete_selected(st: &mut LocalTools, tool: Tool, s: &mut DevelopSettings) -> bool {
    let removed = match tool {
        Tool::Masking => st.selected.is_some_and(|id| {
            let n = s.masks.len();
            s.masks.retain(|m| m.id != id);
            s.masks.len() != n
        }),
        Tool::Heal => st.spot.take().filter(|i| *i < s.spots.len()).map(|i| s.spots.remove(i)).is_some(),
        Tool::RedEye => st.eye.take().filter(|i| *i < s.red_eye.len()).map(|i| s.red_eye.remove(i)).is_some(),
        Tool::Edit => false,
    };
    st.validate(s);
    removed
}

/// Spot radius and source offset defaults: the source sits beside the target, 2.5 radii away,
/// on the side with more room.
fn default_offset(c: &Canvas, at: Point, size: f64) -> Point {
    let (sx, _) = c.map.long_scale();
    let dx = 2.5 * size / sx.max(1e-9);
    Point::new(if at.x + dx <= 1.0 { dx } else { -dx }, 0.0)
}

/// Places a spot at normalized point `at` with the Heal settings, and selects it.
pub(crate) fn add_spot(st: &mut LocalTools, s: &mut DevelopSettings, at: Point, offset: Option<Point>) -> usize {
    let h = st.heal;
    let source_offset = if h.mode == SpotMode::Remove || h.mode == SpotMode::Fill { None } else { offset };
    s.spots.push(Spot { mode: h.mode, points: vec![at], size: h.size, feather: h.feather, opacity: h.opacity, source_offset });
    st.spot = Some(s.spots.len() - 1);
    s.spots.len() - 1
}

#[allow(clippy::too_many_arguments)]
fn heal(
    ui: &mut egui::Ui,
    st: &mut LocalTools,
    s: &mut DevelopSettings,
    c: &Canvas,
    p: &egui::Painter,
    response: &Response,
    live: bool,
    press: bool,
    pointer: Option<Pos2>,
    out: &mut PreviewOut,
) {
    let ppl = c.pts_per_long();
    // every spot: its target, and the selected one's source joined to it
    let mut grips: Vec<(usize, bool, Pos2, f32)> = Vec::new();
    for (i, spot) in s.spots.iter().enumerate() {
        let rad = (spot.size as f32 * ppl).max(3.0);
        let sel = st.spot == Some(i);
        let pts: Vec<Pos2> = spot.points.iter().map(|n| c.norm_to_screen(*n)).collect();
        let Some(first) = pts.first().copied() else { continue };
        if pts.len() > 1 {
            p.add(egui::Shape::line(pts.clone(), Stroke::new(rad * 2.0, Color32::from_white_alpha(if sel { 60 } else { 30 }))));
        }
        p.circle_stroke(first, rad + 1.0, Stroke::new(2.5, Color32::from_black_alpha(110)));
        p.circle_stroke(first, rad, Stroke::new(if sel { 2.0 } else { 1.2 }, Color32::WHITE));
        grips.push((i, false, first, rad));
        if sel && let Some(o) = spot.source_offset {
            let src_n = spot.points.first().map(|t| Point::new(t.x + o.x, t.y + o.y));
            if let Some(src) = src_n.map(|n| c.norm_to_screen(n)) {
                p.line_segment([first, src], Stroke::new(1.0, Color32::from_white_alpha(170)));
                p.circle_stroke(src, rad + 1.0, Stroke::new(2.5, Color32::from_black_alpha(110)));
                p.add(egui::Shape::dashed_line(
                    &(0..=48)
                        .map(|k| {
                            let a = k as f32 / 48.0 * std::f32::consts::TAU;
                            src + vec2(a.cos(), a.sin()) * rad
                        })
                        .collect::<Vec<_>>(),
                    Stroke::new(1.5, Color32::WHITE),
                    4.0,
                    3.0,
                ));
                grips.push((i, true, src, rad));
            }
        }
    }
    let Some(at) = pointer else { return };
    if response.hovered() && st.gesture.is_none() {
        let rad = (st.heal.size as f32 * ppl).max(3.0);
        p.circle_stroke(at, rad, Stroke::new(1.0, Color32::from_white_alpha(200)));
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if !live {
        return;
    }
    let n = c.screen_to_norm(at);
    if press && st.gesture.is_none() {
        // a spot's source (on top) or target under the press: select and drag it
        let hit = grips.iter().rev().find(|(_, _, q, rad)| q.distance(at) <= rad.max(6.0)).copied();
        if let Some((i, source, _, _)) = hit {
            st.spot = Some(i);
            if let Some(was) = s.spots.get(i).cloned() {
                st.gesture = Some(Gesture::Spot { index: i, source, press: n, was });
            }
        } else {
            let offset = default_offset(c, n, st.heal.size);
            let i = add_spot(st, s, n, Some(offset));
            st.gesture = Some(Gesture::SpotPath(i));
            out.edited = true;
        }
        return;
    }
    if !response.dragged() {
        return;
    }
    match st.gesture.clone() {
        Some(Gesture::Spot { index, source, press, was }) => {
            if let Some(spot) = s.spots.get_mut(index) {
                let d = Point::new(n.x - press.x, n.y - press.y);
                let next = if source {
                    let o = was.source_offset.unwrap_or(Point::new(0.0, 0.0));
                    Spot { source_offset: Some(Point::new(o.x + d.x, o.y + d.y)), ..was.clone() }
                } else {
                    Spot { points: was.points.iter().map(|q| Point::new(q.x + d.x, q.y + d.y)).collect(), ..was.clone() }
                };
                if *spot != next {
                    *spot = next;
                    out.edited = true;
                }
            }
        }
        Some(Gesture::SpotPath(i)) => {
            if let Some(spot) = s.spots.get_mut(i) {
                // a drag brushes the spot along a path (a dab every half radius)
                let (sx, sy) = c.map.long_scale();
                let far = spot.points.last().is_none_or(|q| ((q.x - n.x) * sx).hypot((q.y - n.y) * sy) >= spot.size * 0.5);
                if far && spot.points.len() < 512 {
                    spot.points.push(n);
                    out.edited = true;
                }
            }
        }
        _ => {}
    }
}

/// Adds a red eye at normalized point `at` (radii as fractions of the long edge) and selects it.
pub(crate) fn add_eye(st: &mut LocalTools, s: &mut DevelopSettings, at: Point, rx: f64, ry: f64) -> usize {
    s.red_eye.push(RedEye { center: at, rx, ry, pet: st.pet, ..RedEye::default() });
    st.eye = Some(s.red_eye.len() - 1);
    s.red_eye.len() - 1
}

#[allow(clippy::too_many_arguments)]
fn red_eye(
    st: &mut LocalTools,
    s: &mut DevelopSettings,
    c: &Canvas,
    p: &egui::Painter,
    response: &Response,
    live: bool,
    press: bool,
    pointer: Option<Pos2>,
    out: &mut PreviewOut,
) {
    let ppl = c.pts_per_long();
    let mut grips = Vec::new();
    for (i, e) in s.red_eye.iter().enumerate() {
        let at = c.norm_to_screen(e.center);
        let r = vec2((e.rx as f32 * ppl).max(3.0), (e.ry as f32 * ppl).max(3.0));
        let pts: Vec<Pos2> = (0..48)
            .map(|k| {
                let a = k as f32 / 48.0 * std::f32::consts::TAU;
                at + vec2(a.cos() * r.x, a.sin() * r.y)
            })
            .collect();
        outline(p, pts, true);
        if st.eye == Some(i) {
            handle_dot(p, at, true, false);
        }
        grips.push((i, at, r.x.max(r.y)));
    }
    let Some(at) = pointer else { return };
    if !live {
        return;
    }
    let n = c.screen_to_norm(at);
    if press && st.gesture.is_none() {
        if let Some((i, _, _)) = grips.iter().rev().find(|(_, q, rad)| q.distance(at) <= rad.max(6.0)) {
            st.eye = Some(*i);
            return;
        }
        let i = add_eye(st, s, n, 0.02, 0.015);
        st.gesture = Some(Gesture::Eye { index: i, press: n });
        out.edited = true;
        return;
    }
    if response.dragged()
        && let Some(Gesture::Eye { index, press }) = st.gesture.clone()
        && let Some(e) = s.red_eye.get_mut(index)
    {
        // a drag draws the eye's ellipse from its centre
        let (sx, sy) = c.map.long_scale();
        let (rx, ry) = (((n.x - press.x) * sx).abs().max(0.004), ((n.y - press.y) * sy).abs().max(0.004));
        if (e.rx, e.ry) != (rx, ry) {
            (e.rx, e.ry) = (rx, ry);
            out.edited = true;
        }
    }
}

/// Paints brush stroke `points` (mask coordinates on a `size` brush) into the selected brush
/// component, as a drag on the preview would: the control channel's `brush`.
pub(crate) fn paint_stroke(st: &mut LocalTools, s: &mut DevelopSettings, points: Vec<Point>) -> Result<(), String> {
    let brush = st.brush;
    let comp = st.component;
    let m = selected_mask(st, s).ok_or("no mask is selected")?;
    match m.components.get_mut(comp).map(|k| &mut k.shape) {
        Some(MaskShape::Brush { strokes }) => {
            strokes.push(BrushStroke { points, size: brush.size, feather: brush.feather, flow: brush.flow, erase: brush.erase, ..BrushStroke::default() });
            Ok(())
        }
        _ => Err("the selected mask component is not a brush".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_are_added_selected_and_validated() {
        let mut st = LocalTools::default();
        let mut s = DevelopSettings::default();
        assert!(add_mask(&mut st, &mut s, "subject").is_err(), "AI kinds are not made here");
        let a = add_mask(&mut st, &mut s, "radial").unwrap();
        let b = add_mask(&mut st, &mut s, "brush").unwrap();
        assert_eq!((a, b, st.selected, st.draw), (1, 2, Some(2), Draw::Brush));
        assert!(paint_stroke(&mut st, &mut s, vec![Point::new(0.2, 0.2), Point::new(0.3, 0.2)]).is_ok());
        add_component(&mut st, &mut s, "linear", MaskOp::Subtract).unwrap();
        assert_eq!(s.masks[1].components.len(), 2);
        assert_eq!(s.masks[1].components[1].op, MaskOp::Subtract);
        assert!(paint_stroke(&mut st, &mut s, vec![]).is_err(), "a linear component takes no strokes");
        // an undo that drops the mask drops the selection
        s.masks.pop();
        st.validate(&s);
        assert_eq!(st.selected, None);
        assert_eq!(st.overlay_mask(Tool::Masking, &s), None);
        st.selected = Some(1);
        assert_eq!(st.overlay_mask(Tool::Masking, &s), Some(1));
        assert_eq!(st.overlay_mask(Tool::Edit, &s), None, "only while masking");
        let mut a = LocalAdjustments::default();
        *local_field(&mut a, "exposure").unwrap() = 0.5;
        assert_eq!(a.exposure, 0.5);
        assert!(local_field(&mut a, "nope").is_none());
        assert!(overlay_image(&[0, 255], 2, 1).is_some());
        assert!(overlay_image(&[0, 255], 3, 1).is_none(), "a wrong size is no image, never a panic");
        // Delete takes the selected one of the tool's
        add_spot(&mut st, &mut s, Point::new(0.5, 0.5), None);
        assert!(delete_selected(&mut st, Tool::Heal, &mut s) && s.spots.is_empty());
        assert!(!delete_selected(&mut st, Tool::Heal, &mut s), "nothing selected");
        assert!(delete_selected(&mut st, Tool::Masking, &mut s) && s.masks.is_empty() && st.selected.is_none());
    }
}
