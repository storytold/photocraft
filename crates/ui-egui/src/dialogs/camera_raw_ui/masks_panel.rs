//! Camera Raw's Masking tool: the Masks panel (create, list, combine, the selected mask's local
//! sliders) and drawing on the preview (brush dabs, linear and radial gradients, colour samples,
//! an object's box, and Guided Upright's guides). The maths is
//! [`photocraft_algo::camera_raw::masks`]; the masks live in [`CameraRaw::masks`], so the control
//! channel sets them with `{"ui": {"set": {"masks": […]}}}` like any other setting.
//!
//! The panel's own state (open, selection, tool, brush settings) is view state kept in egui's
//! memory, as Camera Raw keeps it per session, not in the settings.

use egui::{Color32, Pos2, Rect as ERect, Response, Stroke, TextureHandle, pos2};
use photocraft_algo::camera_raw::CameraRaw;
use photocraft_algo::camera_raw::geometry::{self, Upright};
use photocraft_algo::camera_raw::masks::{self, Component, Dab, Local, Mask, Op, PeoplePart, Shape};
use photocraft_algo::transform::Homography;

use super::{CameraRawDialog, Tokens, Value, json, widgets};

/// What a drag on the preview does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DrawTool {
    #[default]
    None,
    Brush,
    Linear,
    Radial,
    ColorPick,
    Objects,
    Guide,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    /// Painting dabs; the last dab's position (0 … 1).
    Brush([f32; 2]),
    /// Dragging out a new linear gradient (component index, start).
    NewLinear(usize, [f32; 2]),
    /// Moving a linear gradient pin: 0 = full, 1 = zero, 2 = both.
    LinearPin(usize, u8),
    NewRadial(usize, [f32; 2]),
    RadialMove(usize),
    /// Resizing a radial gradient from its outline (Ctrl: rotating it).
    RadialEdge(usize, bool),
    Objects([f32; 2]),
    Guide([f32; 2]),
}

/// How the selected mask shows on the preview (Camera Raw's overlay modes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Overlay {
    #[default]
    Color,
    ImageOnBlack,
    ImageOnWhite,
}

#[derive(Clone)]
pub(crate) struct MaskUi {
    pub(crate) open: bool,
    pub(crate) selected: Option<usize>,
    pub(crate) component: Option<usize>,
    pub(crate) tool: DrawTool,
    pub(crate) brush: Dab,
    pub(crate) erase: bool,
    pub(crate) overlay: bool,
    pub(crate) overlay_mode: Overlay,
    /// `selected: null` came from the control channel: the panel does not select the first
    /// mask again until the tool reopens or a mask is chosen.
    deselected: bool,
    /// The mask whose name is being edited.
    renaming: Option<usize>,
    drag: Option<Drag>,
    /// Add / Subtract / Intersect chosen for a drawn kind, used by the component its drag makes.
    pending_op: Option<Op>,
    /// The overlay texture and the key of the mask it shows.
    overlay_tex: Option<(u64, TextureHandle)>,
}

impl Default for MaskUi {
    fn default() -> Self {
        MaskUi {
            open: false,
            selected: None,
            component: None,
            tool: DrawTool::None,
            brush: Dab { size: 0.04, feather: 50.0, flow: 50.0, density: 100.0, ..Dab::default() },
            erase: false,
            overlay: true,
            overlay_mode: Overlay::Color,
            deselected: false,
            renaming: None,
            drag: None,
            pending_op: None,
            overlay_tex: None,
        }
    }
}

fn id() -> egui::Id {
    egui::Id::unique("camera-raw-masking")
}

pub(crate) fn load(ctx: &egui::Context) -> MaskUi {
    ctx.data(|d| d.get_temp::<MaskUi>(id())).unwrap_or_default()
}

pub(crate) fn store(ctx: &egui::Context, s: MaskUi) {
    ctx.data_mut(|d| d.insert_temp(id(), s));
}

/// A new dialog (another image, or the next raw of the filmstrip) starts in Edit with nothing
/// selected; only the brush and overlay choices carry over, as Camera Raw keeps them per session.
pub(crate) fn reset_for_new_dialog(ctx: &egui::Context) {
    let old = load(ctx);
    store(ctx, MaskUi { brush: old.brush, erase: old.erase, overlay: old.overlay, overlay_mode: old.overlay_mode, ..MaskUi::default() });
}

/// Drops a selection (mask or component) the settings `p` no longer have, so a reply never names
/// a mask that is gone (after an undo, or a `set` that removes masks).
pub(crate) fn reconcile(ctx: &egui::Context, p: &CameraRaw) {
    let mut s = load(ctx);
    let before = (s.selected, s.component);
    if s.selected.is_some_and(|i| i >= p.masks.len()) {
        s.selected = None;
    }
    let n = s.selected.and_then(|i| p.masks.get(i)).map_or(0, |m| m.components.len());
    if s.component.is_some_and(|j| j >= n) {
        s.component = None;
    }
    if (s.selected, s.component) != before {
        s.pending_op = None;
        s.drag = None;
        s.overlay_tex = None;
        store(ctx, s);
    }
}

fn trc(s: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", s)
}

const TOOLS: [(DrawTool, &str); 7] = [
    (DrawTool::None, "none"),
    (DrawTool::Brush, "brush"),
    (DrawTool::Linear, "linear"),
    (DrawTool::Radial, "radial"),
    (DrawTool::ColorPick, "colorPick"),
    (DrawTool::Objects, "objects"),
    (DrawTool::Guide, "guides"),
];
const OVERLAYS: [(Overlay, &str); 3] = [(Overlay::Color, "color"), (Overlay::ImageOnBlack, "imageOnBlack"), (Overlay::ImageOnWhite, "imageOnWhite")];

/// The Masking tool's view state for the control channel (`describe`'s `"masking"`).
pub(crate) fn describe(ctx: &egui::Context) -> Value {
    let s = load(ctx);
    json!({
        "open": s.open,
        "selected": s.selected,
        "component": s.component,
        "tool": TOOLS.iter().find(|(t, _)| *t == s.tool).map_or("none", |(_, n)| n),
        "overlay": s.overlay,
        "overlayMode": OVERLAYS.iter().find(|(o, _)| *o == s.overlay_mode).map_or("color", |(_, n)| n),
        "brush": {"size": s.brush.size, "feather": s.brush.feather, "flow": s.brush.flow, "density": s.brush.density, "erase": s.erase, "autoMask": s.brush.auto},
    })
}

/// `{"ui": {"masks": {"open", "selected", "component", "tool", "overlay", "overlayMode",
/// "brush": {"size", "feather", "flow", "density", "erase", "autoMask"}}}}`: drives the Masking
/// tool's view state. The masks themselves are settings (`{"ui": {"set": {"masks": […]}}}`).
/// Nothing changes when any field is bad.
pub(crate) fn control(ctx: &egui::Context, p: &CameraRaw, v: &Value) -> Result<(), String> {
    let obj = v.as_object().ok_or("masks: expected an object")?;
    let mut s = load(ctx);
    let index = |k: &str, val: &Value| -> Result<Option<usize>, String> {
        if val.is_null() {
            return Ok(None);
        }
        let i = val.as_u64().ok_or_else(|| format!("masks.{k}: expected an index or null"))?;
        usize::try_from(i).map(Some).map_err(|_| format!("masks.{k}: out of range"))
    };
    // "selected" before "component" (the object's keys come sorted), so a component is checked
    // against the mask this request selects.
    if let Some(val) = obj.get("selected") {
        let i = index("selected", val)?;
        if i.is_some_and(|i| i >= p.masks.len()) {
            return Err(format!("masks.selected: there are {} masks", p.masks.len()));
        }
        s.selected = i;
        s.deselected = i.is_none();
        s.component = None;
        s.pending_op = None;
    }
    if let Some(val) = obj.get("component") {
        let i = index("component", val)?;
        let n = s.selected.and_then(|m| p.masks.get(m)).map_or(0, |m| m.components.len());
        if i.is_some_and(|i| i >= n) {
            return Err(format!("masks.component: the selected mask has {n} components"));
        }
        s.component = i;
    }
    for (k, val) in obj {
        match k.as_str() {
            "open" => {
                let open = val.as_bool().ok_or("masks.open: expected true or false")?;
                // Opening the tool selects the first mask again, unless this request chooses.
                if open && !s.open && !obj.contains_key("selected") {
                    s.deselected = false;
                }
                s.open = open;
            }
            "overlay" => s.overlay = val.as_bool().ok_or("masks.overlay: expected true or false")?,
            "selected" | "component" => {}
            "tool" => {
                let name = val.as_str().ok_or("masks.tool: expected a name")?;
                s.tool = TOOLS
                    .iter()
                    .find(|(_, n)| *n == name)
                    .map(|(t, _)| *t)
                    .ok_or_else(|| format!("masks.tool: one of {}", TOOLS.map(|(_, n)| n).join(", ")))?;
            }
            "overlayMode" => {
                let name = val.as_str().ok_or("masks.overlayMode: expected a name")?;
                s.overlay_mode = OVERLAYS.iter().find(|(_, n)| *n == name).map(|(o, _)| *o).ok_or("masks.overlayMode: color, imageOnBlack or imageOnWhite")?;
            }
            "brush" => {
                let b = val.as_object().ok_or("masks.brush: expected an object")?;
                for (bk, bv) in b {
                    let num = || bv.as_f64().filter(|x| x.is_finite()).map(|x| x as f32).ok_or_else(|| format!("masks.brush.{bk}: expected a number"));
                    match bk.as_str() {
                        "size" => s.brush.size = num()?.clamp(0.001, masks::MAX_DAB_SIZE),
                        "feather" => s.brush.feather = num()?.clamp(0.0, 100.0),
                        "flow" => s.brush.flow = num()?.clamp(1.0, 100.0),
                        "density" => s.brush.density = num()?.clamp(0.0, 100.0),
                        "erase" => s.erase = bv.as_bool().ok_or("masks.brush.erase: expected true or false")?,
                        "autoMask" => s.brush.auto = bv.as_bool().ok_or("masks.brush.autoMask: expected true or false")?,
                        other => return Err(format!("masks.brush: unknown field {other}")),
                    }
                }
            }
            other => return Err(format!("masks: unknown field {other}")),
        }
    }
    s.overlay_tex = None;
    store(ctx, s);
    Ok(())
}

/// Whether the Masking tool is open (the right column shows the Masks panel).
pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    load(ctx).open
}

/// Whether a drag on the preview draws (so the preview's Zoom tool must not take the click).
/// `p`: the settings the preview shows; `before`: Before (Y) shows, which has no guides and takes
/// no mask edits (its pixels are not where the masks apply). The guide tool is governed by its own
/// test only: with Guided Upright unavailable it leaves the drag to the zoom tool.
pub(crate) fn captures_pointer(ctx: &egui::Context, p: &CameraRaw, before: bool) -> bool {
    let s = load(ctx);
    (!before && s.open && !matches!(s.tool, DrawTool::None | DrawTool::Guide) && s.selected.is_some_and(|i| i < p.masks.len()))
        || (!before && s.tool == DrawTool::Guide && Upright::parse(&p.perspective_upright) == Some(Upright::Guided))
}

/// The tool strip's Masking button (Shift+W toggles it, as in Camera Raw).
pub(crate) fn strip_button(ui: &mut egui::Ui) {
    let mut s = load(ui.ctx());
    let r = crate::icons::button(ui, "layer-mask", 30.0, s.open, trc("Masking (Shift+W)"));
    // Not while a text field has the keyboard: a capital W typed into a mask's name is a letter.
    let key = !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.modifiers.shift && i.key_pressed(egui::Key::W));
    if r.clicked() || key {
        s.open = !s.open;
        if s.open {
            s.deselected = false;
        }
        if !s.open && s.tool != DrawTool::Guide {
            s.tool = DrawTool::None;
        }
        store(ui.ctx(), s);
    }
}

fn kind_name(shape: &Shape) -> &'static str {
    match shape {
        Shape::Brush { .. } => "Brush",
        Shape::Linear { .. } => "Linear Gradient",
        Shape::Radial { .. } => "Radial Gradient",
        Shape::Color { .. } => "Color Range",
        Shape::Luminance { .. } => "Luminance Range",
        Shape::Depth { .. } => "Depth Range",
        Shape::Subject => "Subject",
        Shape::Sky => "Sky",
        Shape::Background => "Background",
        Shape::People { part: PeoplePart::Person } => "People",
        Shape::People { part: PeoplePart::Face } => "People › Face",
        Shape::People { part: PeoplePart::Skin } => "People › Skin",
        Shape::Objects { .. } => "Objects",
    }
}

/// The kinds "Create New Mask" and Add / Subtract offer, with the tool each starts.
const KINDS: [(&str, DrawTool); 10] = [
    ("Subject", DrawTool::None),
    ("Sky", DrawTool::None),
    ("Background", DrawTool::None),
    ("Objects", DrawTool::Objects),
    ("People", DrawTool::None),
    ("Brush", DrawTool::Brush),
    ("Linear Gradient", DrawTool::Linear),
    ("Radial Gradient", DrawTool::Radial),
    ("Color Range", DrawTool::ColorPick),
    ("Luminance Range", DrawTool::None),
];

/// Why People cannot be chosen in this build (no face detector: the browser build).
const NO_FACES: &str = "People needs the face detector, which the desktop app has and the browser version does not.";

/// Whether this build can make a component of `kind` (`faces`: a face detector is available,
/// [`photocraft_algo::faces::AVAILABLE`]). Without a detector People would select nothing.
fn kind_available(kind: &str, faces: bool) -> bool {
    faces || kind != "People"
}

/// A component of `kind` that needs no drawing, or `None` for the drawn kinds (made by the drag).
fn ready_shape(kind: &str) -> Option<Shape> {
    Some(match kind {
        "Subject" => Shape::Subject,
        "Sky" => Shape::Sky,
        "Background" => Shape::Background,
        "People" => Shape::People { part: PeoplePart::Person },
        "Luminance Range" => Shape::Luminance { range: [50.0, 100.0], smoothness: 50.0 },
        "Brush" => Shape::Brush { dabs: Vec::new() },
        _ => return None,
    })
}

/// Creates a mask (or, with `op`, adds a component to the selected one) of `kind`.
pub(crate) fn create(p: &mut CameraRaw, s: &mut MaskUi, kind: &str, op: Option<Op>) {
    create_with(p, s, kind, op, photocraft_algo::faces::AVAILABLE);
}

/// [`create`], with `faces` saying whether this build has a face detector (People needs one).
fn create_with(p: &mut CameraRaw, s: &mut MaskUi, kind: &str, op: Option<Op>, faces: bool) {
    if !kind_available(kind, faces) {
        return;
    }
    let tool = KINDS.iter().find(|(k, _)| *k == kind).map_or(DrawTool::None, |(_, t)| *t);
    let target = match (op, s.selected) {
        (Some(_), Some(i)) if i < p.masks.len() => i,
        _ => {
            if p.masks.len() >= masks::MAX_MASKS {
                return;
            }
            p.masks.push(Mask { name: format!("{} {}", trc("Mask"), p.masks.len() + 1), adjust: Local::default(), ..Mask::default() });
            p.masks.len() - 1
        }
    };
    s.selected = Some(target);
    s.deselected = false;
    s.tool = tool;
    s.pending_op = None;
    let Some(m) = p.masks.get_mut(target) else { return };
    if let Some(shape) = ready_shape(kind) {
        if m.components.len() < masks::MAX_COMPONENTS {
            m.components.push(Component { op: op.unwrap_or(Op::Add), invert: false, shape });
            s.component = Some(m.components.len() - 1);
        }
    } else {
        // Drawn kinds: the drag makes the component, with the operation chosen here.
        s.component = None;
        s.pending_op = op;
    }
    // A new mask starts with Exposure up a little, as Camera Raw's do not: here it starts
    // neutral and shows its overlay, so drawing is visible before any slider moves.
    s.overlay = true;
}

/// The operation for a component a drag makes: the one Add / Subtract / Intersect chose, once.
fn take_op(s: &mut MaskUi) -> Op {
    s.pending_op.take().unwrap_or(Op::Add)
}

fn op_label(op: Op) -> &'static str {
    match op {
        Op::Add => "Add",
        Op::Subtract => "Subtract",
        Op::Intersect => "Intersect",
    }
}

/// Opening the tool on existing masks selects the first, as Camera Raw does; a mask that is gone
/// gives way to the first too. An explicit `selected: null` stays deselected.
fn auto_select(s: &mut MaskUi, masks: usize) {
    if s.selected.map_or(!s.deselected, |i| i >= masks) {
        s.selected = (masks > 0).then_some(0);
        s.component = None;
    }
}

/// The Masks panel (the right column while the Masking tool is open).
pub(crate) fn panel(ui: &mut egui::Ui, d: &mut CameraRawDialog, dirty: &mut bool) {
    let mut s = load(ui.ctx());
    let p = &mut d.params;
    let before = p.masks.clone();
    auto_select(&mut s, p.masks.len());
    ui.add_space(6.0);
    widgets::section_label(ui, trc("Create New Mask"));
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (kind, _) in KINDS {
            let available = kind_available(kind, photocraft_algo::faces::AVAILABLE);
            let clicked = ui.add_enabled_ui(available, |ui| widgets::pill_tab(ui, trc(kind), false).on_disabled_hover_text(trc(NO_FACES)).clicked()).inner;
            if clicked {
                create(p, &mut s, kind, None);
            }
        }
    });
    ui.add_space(8.0);
    widgets::hairline(ui);
    ui.add_space(4.0);
    widgets::section_label(ui, trc("Masks"));
    let mut delete_mask = None;
    let mut action: Option<(usize, i32)> = None;
    let n = p.masks.len();
    for (i, m) in p.masks.iter_mut().enumerate() {
        ui.push_id(("mask", i), |ui| {
            ui.horizontal(|ui| {
                let eye = if m.visible { "eye" } else { "eye-off" };
                if crate::icons::button(ui, eye, 22.0, false, trc("Show / hide this mask")).clicked() {
                    m.visible = !m.visible;
                }
                if s.renaming == Some(i) {
                    let r = ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(120.0));
                    r.request_focus();
                    if r.lost_focus() || ui.input(|x| x.key_pressed(egui::Key::Enter)) {
                        s.renaming = None;
                    }
                } else {
                    let r = ui.selectable_label(s.selected == Some(i), &m.name).on_hover_text(trc("Double-click to rename"));
                    if r.double_clicked() {
                        s.renaming = Some(i);
                    } else if r.clicked() {
                        s.selected = Some(i);
                        s.deselected = false;
                        s.component = None;
                        s.pending_op = None;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::button(ui, "trash", 22.0, false, trc("Delete mask")).clicked() {
                        delete_mask = Some(i);
                    }
                    if crate::icons::button(ui, "copy", 22.0, false, trc("Duplicate mask")).clicked() {
                        action = Some((i, 0));
                    }
                    if i + 1 < n && crate::icons::button(ui, "chevron-down", 22.0, false, trc("Move down")).clicked() {
                        action = Some((i, 1));
                    }
                    if i > 0 && crate::icons::button(ui, "chevron-up", 22.0, false, trc("Move up")).clicked() {
                        action = Some((i, -1));
                    }
                });
            });
            if s.selected == Some(i) {
                components(ui, m, &mut s);
            }
        });
    }
    if let Some((i, op)) = action {
        apply_list_action(p, &mut s, i, op);
    }
    if let Some(i) = delete_mask {
        p.masks.remove(i);
        s.selected = None;
        s.component = None;
        s.pending_op = None;
        s.tool = DrawTool::None;
    }
    if let Some(i) = s.selected.filter(|i| *i < p.masks.len()) {
        ui.horizontal_wrapped(|ui| {
            for (label, op) in [("Add", Op::Add), ("Subtract", Op::Subtract), ("Intersect", Op::Intersect)] {
                ui.menu_button(trc(label), |ui| {
                    for (kind, _) in KINDS {
                        let available = kind_available(kind, photocraft_algo::faces::AVAILABLE);
                        if ui.add_enabled(available, egui::Button::new(trc(kind))).on_disabled_hover_text(trc(NO_FACES)).clicked() {
                            create(p, &mut s, kind, Some(op));
                            ui.close();
                        }
                    }
                });
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if widgets::checkbox(ui, &mut s.overlay, trc("Show Overlay")).changed() {
                s.overlay_tex = None;
            }
            let options =
                [(Overlay::Color, trc("Color Overlay")), (Overlay::ImageOnBlack, trc("Image on Black")), (Overlay::ImageOnWhite, trc("Image on White"))];
            if widgets::dropdown(ui, "mask-overlay", "", &mut s.overlay_mode, &options, 130.0) {
                s.overlay_tex = None;
            }
        });
        if let Some(m) = p.masks.get_mut(i) {
            component_settings(ui, m, &mut s, dirty);
            ui.add_space(6.0);
            widgets::hairline(ui);
            if widgets::checkbox(ui, &mut m.invert, trc("Invert")).changed() {
                *dirty = true;
            }
            local_sliders(ui, m, dirty);
        }
    } else {
        ui.add_space(6.0);
        let hint = if p.masks.is_empty() { "Create a mask to adjust part of the image." } else { "Select a mask to edit it." };
        ui.label(egui::RichText::new(trc(hint)).color(Tokens::get(ui.ctx()).text_faint));
    }
    if p.masks != before {
        *dirty = true;
        s.overlay_tex = None;
    }
    store(ui.ctx(), s);
}

fn components(ui: &mut egui::Ui, m: &mut Mask, s: &mut MaskUi) {
    let mut delete = None;
    for (j, c) in m.components.iter_mut().enumerate() {
        ui.push_id(("component", j), |ui| {
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                let label = format!("{} · {}", trc(op_label(c.op)), trc(kind_name(&c.shape)));
                if ui.selectable_label(s.component == Some(j), label).clicked() {
                    s.component = Some(j);
                    s.pending_op = None;
                    s.tool = match c.shape {
                        Shape::Brush { .. } => DrawTool::Brush,
                        Shape::Linear { .. } => DrawTool::Linear,
                        Shape::Radial { .. } => DrawTool::Radial,
                        Shape::Color { .. } => DrawTool::ColorPick,
                        Shape::Objects { .. } => DrawTool::Objects,
                        _ => DrawTool::None,
                    };
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::button(ui, "minus", 20.0, false, trc("Delete component")).clicked() {
                        delete = Some(j);
                    }
                });
            });
        });
    }
    if let Some(j) = delete {
        m.components.remove(j);
        s.component = None;
    }
}

fn row(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, reset: f32) {
    super::edit_panel::row(ui, dirty, label, v, range, None, reset);
}

fn component_settings(ui: &mut egui::Ui, m: &mut Mask, s: &mut MaskUi, dirty: &mut bool) {
    let Some(c) = s.component.and_then(|j| m.components.get_mut(j)) else {
        if s.tool == DrawTool::Brush {
            brush_settings(ui, s, dirty);
        }
        return;
    };
    ui.add_space(4.0);
    if widgets::checkbox(ui, &mut c.invert, trc("Invert component")).changed() {
        *dirty = true;
    }
    match &mut c.shape {
        Shape::Brush { .. } => brush_settings(ui, s, dirty),
        Shape::Radial { feather, .. } => row(ui, dirty, "Feather", feather, 0.0..=100.0, 50.0),
        Shape::People { part } => {
            let options = [(PeoplePart::Person, trc("Entire person")), (PeoplePart::Face, trc("Face")), (PeoplePart::Skin, trc("Skin"))];
            if widgets::dropdown(ui, "people-part", "", part, &options, 140.0) {
                *dirty = true;
            }
            ui.label(egui::RichText::new(trc("Found by PhotoCraft's face detector, not Adobe's models.")).color(Tokens::get(ui.ctx()).text_faint));
        }
        Shape::Color { amount, samples } => {
            row(ui, dirty, "Refine", amount, 0.0..=100.0, 50.0);
            ui.label(format!("{} {}/{}", trc("Samples:"), samples.len(), masks::MAX_SAMPLES));
        }
        Shape::Luminance { range, smoothness } => {
            let (mut lo, mut hi) = (range[0], range[1]);
            row(ui, dirty, "Range Low", &mut lo, 0.0..=100.0, 50.0);
            row(ui, dirty, "Range High", &mut hi, 0.0..=100.0, 100.0);
            *range = [lo.min(hi), hi.max(lo)];
            row(ui, dirty, "Smoothness", smoothness, 0.0..=100.0, 50.0);
        }
        _ => {}
    }
}

fn brush_settings(ui: &mut egui::Ui, s: &mut MaskUi, dirty: &mut bool) {
    let mut size = s.brush.size * 100.0;
    let mut sink = false;
    row(ui, &mut sink, "Size", &mut size, 0.1..=25.0, 4.0);
    s.brush.size = size / 100.0;
    row(ui, &mut sink, "Feather", &mut s.brush.feather, 0.0..=100.0, 50.0);
    row(ui, &mut sink, "Flow", &mut s.brush.flow, 1.0..=100.0, 50.0);
    row(ui, &mut sink, "Density", &mut s.brush.density, 0.0..=100.0, 100.0);
    let _ = widgets::checkbox(ui, &mut s.erase, trc("Erase (Alt)"));
    let _ = widgets::checkbox(ui, &mut s.brush.auto, trc("Auto Mask"));
    let _ = dirty;
}

fn local_sliders(ui: &mut egui::Ui, m: &mut Mask, dirty: &mut bool) {
    let a = &mut m.adjust;
    row(ui, dirty, "Amount", &mut m.amount, 0.0..=200.0, 100.0);
    ui.add_space(4.0);
    for (label, v, lo, hi) in [
        ("Temperature", &mut a.temperature, -100.0, 100.0),
        ("Tint", &mut a.tint, -100.0, 100.0),
        ("Exposure", &mut a.exposure, -4.0, 4.0),
        ("Contrast", &mut a.contrast, -100.0, 100.0),
        ("Highlights", &mut a.highlights, -100.0, 100.0),
        ("Shadows", &mut a.shadows, -100.0, 100.0),
        ("Whites", &mut a.whites, -100.0, 100.0),
        ("Blacks", &mut a.blacks, -100.0, 100.0),
        ("Texture", &mut a.texture, -100.0, 100.0),
        ("Clarity", &mut a.clarity, -100.0, 100.0),
        ("Dehaze", &mut a.dehaze, -100.0, 100.0),
        ("Hue", &mut a.hue, -100.0, 100.0),
        ("Saturation", &mut a.saturation, -100.0, 100.0),
        ("Sharpness", &mut a.sharpness, -100.0, 100.0),
        ("Noise", &mut a.noise, -100.0, 100.0),
        ("Moiré", &mut a.moire, 0.0, 100.0),
        ("Defringe", &mut a.defringe, 0.0, 100.0),
    ] {
        row(ui, dirty, label, v, lo..=hi, 0.0);
    }
    ui.add_space(4.0);
    widgets::section_label(ui, trc("Color"));
    ui.push_id("local-color", |ui| {
        row(ui, dirty, "Hue", &mut a.color_hue, 0.0..=360.0, 0.0);
        row(ui, dirty, "Saturation", &mut a.color_sat, 0.0..=100.0, 0.0);
    });
}

// ---------- preview ----------

fn norm(r: ERect, p: Pos2) -> [f32; 2] {
    [((p.x - r.min.x) / r.width().max(1e-3)), ((p.y - r.min.y) / r.height().max(1e-3))]
}

fn screen(r: ERect, u: [f32; 2]) -> Pos2 {
    pos2(r.min.x + u[0] * r.width(), r.min.y + u[1] * r.height())
}

/// The overlay's cache key: the mask, the size and the preview revision (Color and Luminance
/// Range coverage follows the developed pixels, which every global edit changes).
fn mask_key(m: &Mask, w: usize, h: usize, revision: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(m).unwrap_or_default().hash(&mut hs);
    (w, h, revision).hash(&mut hs);
    hs.finish()
}

/// Guided Upright's mapping between guide (source) and preview (result) coordinates, 0 … 1.
fn guide_maps(p: &CameraRaw, w: usize, h: usize) -> Option<([f64; 9], [f64; 9])> {
    let m = geometry::homography(&[], w, h, p);
    let inv = Homography(m).inverse()?.0;
    Some((m, inv))
}

fn map(m: &[f64; 9], u: [f32; 2], w: f64, h: f64) -> Option<[f32; 2]> {
    let (x, y) = (f64::from(u[0]) * w - w / 2.0, f64::from(u[1]) * h - h / 2.0);
    let z = m[6] * x + m[7] * y + m[8];
    if z.abs() < 1e-12 {
        return None;
    }
    let (gx, gy) = ((m[0] * x + m[1] * y + m[2]) / z, (m[3] * x + m[4] * y + m[5]) / z);
    Some([((gx + w / 2.0) / w) as f32, ((gy + h / 2.0) / h) as f32])
}

fn ellipse(painter: &egui::Painter, r: ERect, center: [f32; 2], radius: [f32; 2], angle: f32, stroke: Stroke) {
    let (s, c) = angle.to_radians().sin_cos();
    let pts: Vec<Pos2> = (0..=48)
        .map(|k| {
            let t = k as f32 / 48.0 * std::f32::consts::TAU;
            let (ex, ey) = (t.cos() * radius[0] * r.width(), t.sin() * radius[1] * r.height());
            let p = screen(r, center);
            pos2(p.x + ex * c - ey * s, p.y + ex * s + ey * c)
        })
        .collect();
    painter.add(egui::Shape::line(pts, stroke));
}

/// The part of the preview image `r` Guided Upright's guides are relative to, and its size in
/// proxy pixels: the canvas, when the layer extends past it (Geometry develops and warps the
/// canvas only, see `ProxyJob::run`), else the whole image.
fn guide_frame(canvas_frame: Option<[usize; 4]>, pw: usize, ph: usize, r: ERect) -> (ERect, usize, usize) {
    let (pw, ph) = (pw.max(1), ph.max(1));
    match canvas_frame {
        Some([x0, y0, x1, y1]) if x0 < x1 && y0 < y1 && x1 <= pw && y1 <= ph => {
            let at = |x: usize, y: usize| pos2(r.min.x + r.width() * x as f32 / pw as f32, r.min.y + r.height() * y as f32 / ph as f32);
            (ERect::from_min_max(at(x0, y0), at(x1, y1)), x1 - x0, y1 - y0)
        }
        _ => (r, pw, ph),
    }
}

/// Draws the selected mask's overlay, pins and guides over the preview image `r`, and turns drags
/// on `response` into mask edits. Call after painting the image. While `panning` (Space or a
/// middle drag) they are drawn but the drag edits nothing. Only a press inside `after` (the part
/// of the preview showing the After image: not Split's Before half) starts an edit, and Before
/// (Y) takes none: its pixels are not where the masks apply once Geometry moves them.
pub(crate) fn preview(ui: &mut egui::Ui, d: &mut CameraRawDialog, r: ERect, response: &Response, panning: bool, after: ERect) {
    let ctx = ui.ctx().clone();
    let mut s = load(&ctx);
    let painter = ui.painter().clone();
    let t = Tokens::get(&ctx);
    let before = d.params.clone();
    if panning {
        s.drag = None;
    }
    // A drag that started on the After image keeps going wherever the pointer goes.
    let at = response.interact_pointer_pos().or(response.hover_pos());
    let origin = ctx.input(|i| i.pointer.press_origin()).or(at);
    let starts = response.drag_started_by(egui::PointerButton::Primary) || response.clicked();
    let edits = !panning && (!starts || origin.is_some_and(|p| after.contains(p)));
    // Guided Upright: guides drawn in result coordinates, stored in source coordinates. Through
    // the geometry the preview shows: none while Before shows or the Geometry panel is hidden.
    let shown = d.effective_params();
    if !d.show_before && Upright::parse(&shown.perspective_upright) == Some(Upright::Guided) {
        let (gr, gw, gh) = guide_frame(d.canvas_frame, d.pw, d.ph, r);
        let maps = guide_maps(&shown, gw, gh);
        if let Some((fwd, _)) = maps {
            for g in &shown.upright_guides {
                if let (Some(a), Some(b)) = (map(&fwd, g[0], gw as f64, gh as f64), map(&fwd, g[1], gw as f64, gh as f64)) {
                    painter.line_segment([screen(gr, a), screen(gr, b)], Stroke::new(2.0, Color32::from_rgb(255, 200, 0)));
                }
            }
        }
        if s.tool == DrawTool::Guide && edits {
            guide_drag(&mut s, d, (gr, gw, gh), response, &painter, maps.map(|m| m.1));
        }
    }
    if s.open
        && let Some(i) = s.selected.filter(|i| *i < d.params.masks.len())
    {
        overlay(&ctx, &mut s, d, i, r, &painter);
        pins(&painter, &d.params.masks[i], s.component, r, t.accent);
        // The guide tool's drag belongs to `guide_drag` above.
        if s.tool != DrawTool::Guide && edits && !d.show_before {
            mask_drag(&mut s, d, i, r, response, &painter);
        }
    }
    if d.params != before {
        d.dirty = true;
        s.overlay_tex = None;
    }
    store(&ctx, s);
}

fn overlay(ctx: &egui::Context, s: &mut MaskUi, d: &CameraRawDialog, i: usize, r: ERect, painter: &egui::Painter) {
    if !s.overlay {
        return;
    }
    let Some(m) = d.params.masks.get(i) else { return };
    let (w, h) = (d.pw, d.ph);
    let key = mask_key(m, w, h, d.preview_revision);
    if s.overlay_tex.as_ref().is_none_or(|(k, _)| *k != key) {
        let src: &[[f32; 4]] = if d.processed.len() >= w * h { &d.processed } else { &d.proxy };
        let cov = masks::coverage(m, src, w, h);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for c in &cov {
            let c = c.clamp(0.0, 1.0);
            // Colour: a translucent red where the mask is; Image on Black / White: everything
            // outside the mask covered.
            let px: [u8; 4] = match s.overlay_mode {
                Overlay::Color => [255, 40, 40, (c * 128.0) as u8],
                Overlay::ImageOnBlack => [0, 0, 0, ((1.0 - c) * 255.0) as u8],
                Overlay::ImageOnWhite => [255, 255, 255, ((1.0 - c) * 255.0) as u8],
            };
            rgba.extend_from_slice(&px);
        }
        if rgba.len() == w * h * 4 && w > 0 && h > 0 {
            let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            s.overlay_tex = Some((key, ctx.load_texture("camera-raw-mask-overlay", img, egui::TextureOptions::LINEAR)));
        }
    }
    if let Some((_, tex)) = &s.overlay_tex {
        painter.image(tex.id(), r, ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

fn pins(painter: &egui::Painter, m: &Mask, selected: Option<usize>, r: ERect, accent: Color32) {
    for (j, c) in m.components.iter().enumerate() {
        let col = if Some(j) == selected { accent } else { Color32::from_white_alpha(200) };
        match &c.shape {
            Shape::Linear { zero, full } => {
                painter.line_segment([screen(r, *zero), screen(r, *full)], Stroke::new(1.0, col));
                painter.circle_stroke(screen(r, *full), 5.0, Stroke::new(2.0, col));
                painter.circle_stroke(screen(r, *zero), 4.0, Stroke::new(1.0, col));
            }
            Shape::Radial { center, radius, angle, .. } => {
                ellipse(painter, r, *center, *radius, *angle, Stroke::new(1.0, col));
                painter.circle_filled(screen(r, *center), 4.0, col);
            }
            Shape::Objects { rect } => {
                painter.rect_stroke(
                    ERect::from_two_pos(screen(r, [rect[0], rect[1]]), screen(r, [rect[2], rect[3]])),
                    0.0,
                    Stroke::new(1.0, col),
                    egui::StrokeKind::Inside,
                );
            }
            _ => {}
        }
    }
}

/// Whether `p` is on the outline of the ellipse (within 8 points).
fn on_outline(r: ERect, center: [f32; 2], radius: [f32; 2], angle: f32, p: Pos2) -> bool {
    let c = screen(r, center);
    let (dx, dy) = (p.x - c.x, p.y - c.y);
    let (sn, cs) = angle.to_radians().sin_cos();
    let (rx, ry) = ((radius[0] * r.width()).max(1.0), (radius[1] * r.height()).max(1.0));
    let (u, v) = ((dx * cs + dy * sn) / rx, (-dx * sn + dy * cs) / ry);
    let d = (u * u + v * v).sqrt();
    (d - 1.0).abs() * rx.min(ry) < 8.0
}

/// Duplicates (`op` 0) or moves (`op` ±1) mask `i`, keeping it selected.
pub(crate) fn apply_list_action(p: &mut CameraRaw, s: &mut MaskUi, i: usize, op: i32) {
    if i >= p.masks.len() {
        return;
    }
    match op {
        0 if p.masks.len() < masks::MAX_MASKS => {
            let mut copy = p.masks[i].clone();
            copy.name = format!("{} {}", copy.name, trc("copy"));
            p.masks.insert(i + 1, copy);
            s.selected = Some(i + 1);
        }
        -1 if i > 0 => {
            p.masks.swap(i, i - 1);
            s.selected = Some(i - 1);
        }
        1 if i + 1 < p.masks.len() => {
            p.masks.swap(i, i + 1);
            s.selected = Some(i + 1);
        }
        _ => {}
    }
    s.component = None;
    s.pending_op = None;
}

fn close(r: ERect, a: [f32; 2], p: Pos2) -> bool {
    screen(r, a).distance(p) < 9.0
}

fn mask_drag(s: &mut MaskUi, d: &mut CameraRawDialog, i: usize, r: ERect, response: &Response, painter: &egui::Painter) {
    let ctx = response.ctx.clone();
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let Some(p) = pointer else { return };
    let u = norm(r, p);
    let long = r.width().max(r.height()).max(1.0);
    if s.tool == DrawTool::Brush && response.hovered() {
        let rad = s.brush.size * long;
        painter.circle_stroke(p, rad, Stroke::new(1.0, Color32::WHITE));
        painter.circle_stroke(p, rad * (1.0 - s.brush.feather / 100.0), Stroke::new(1.0, Color32::from_white_alpha(120)));
    }
    let Some(m) = d.params.masks.get_mut(i) else { return };
    let alt = ctx.input(|x| x.modifiers.alt);
    if response.drag_started_by(egui::PointerButton::Primary) {
        s.drag = match s.tool {
            DrawTool::Brush => brush_target(m, s).map(|j| {
                dab(m, j, u, &s.brush, s.erase || alt);
                Drag::Brush(u)
            }),
            DrawTool::Linear => {
                let hit = s.component.and_then(|j| match m.components.get(j).map(|c| &c.shape) {
                    Some(Shape::Linear { full, .. }) if close(r, *full, p) => Some(Drag::LinearPin(j, 0)),
                    Some(Shape::Linear { zero, .. }) if close(r, *zero, p) => Some(Drag::LinearPin(j, 1)),
                    _ => None,
                });
                hit.or_else(|| {
                    (m.components.len() < masks::MAX_COMPONENTS).then(|| {
                        m.components.push(Component { op: take_op(s), invert: false, shape: Shape::Linear { zero: u, full: u } });
                        s.component = Some(m.components.len() - 1);
                        Drag::NewLinear(m.components.len() - 1, u)
                    })
                })
            }
            DrawTool::Radial => {
                let ctrl = ctx.input(|x| x.modifiers.command);
                let hit = s.component.and_then(|j| match m.components.get(j).map(|c| &c.shape) {
                    Some(Shape::Radial { center, .. }) if close(r, *center, p) => Some(Drag::RadialMove(j)),
                    Some(Shape::Radial { center, radius, angle, .. }) if on_outline(r, *center, *radius, *angle, p) => Some(Drag::RadialEdge(j, ctrl)),
                    _ => None,
                });
                hit.or_else(|| {
                    (m.components.len() < masks::MAX_COMPONENTS).then(|| {
                        m.components.push(Component {
                            op: take_op(s),
                            invert: false,
                            shape: Shape::Radial { center: u, radius: [0.01, 0.01], angle: 0.0, feather: 50.0 },
                        });
                        s.component = Some(m.components.len() - 1);
                        Drag::NewRadial(m.components.len() - 1, u)
                    })
                })
            }
            DrawTool::Objects => Some(Drag::Objects(u)),
            _ => None,
        };
    } else if response.dragged_by(egui::PointerButton::Primary) {
        drag_to(s, m, r, p, u, alt, painter);
    } else if response.drag_stopped_by(egui::PointerButton::Primary) {
        // The pointer can move on the release frame: the stroke, pin or outline ends where it
        // was let go.
        drag_to(s, m, r, p, u, alt, painter);
        if let Some(Drag::Objects(start)) = s.drag
            && (u[0] - start[0]).abs() > 0.01
            && (u[1] - start[1]).abs() > 0.01
            && m.components.len() < masks::MAX_COMPONENTS
        {
            m.components.push(Component {
                op: take_op(s),
                invert: false,
                shape: Shape::Objects { rect: [start[0], start[1], u[0], u[1]].map(|v| v.clamp(0.0, 1.0)) },
            });
            s.component = Some(m.components.len() - 1);
        }
        s.drag = None;
    } else if response.clicked() && s.tool == DrawTool::Brush {
        // A click without a drag paints one dab, as in Camera Raw.
        if let Some(j) = brush_target(m, s) {
            dab(m, j, u, &s.brush, s.erase || alt);
        }
    } else if response.clicked() && s.tool == DrawTool::ColorPick && r.contains(p) {
        // Only on the image: past its edge (a preview smaller than the view) there is no pixel.
        let (w, h) = (d.pw, d.ph);
        let src: &[[f32; 4]] = if d.processed.len() >= w * h { &d.processed } else { &d.proxy };
        color_pick(s, m, (src, w, h), u, ctx.input(|x| x.modifiers.shift));
    }
}

/// A Color Range click at `u`: a plain click starts the samples over, Shift adds one (up to five).
fn color_pick(s: &mut MaskUi, m: &mut Mask, (src, w, h): (&[[f32; 4]], usize, usize), u: [f32; 2], shift: bool) {
    if !(0.0..=1.0).contains(&u[0]) || !(0.0..=1.0).contains(&u[1]) {
        return;
    }
    let (x, y) = ((u[0] * w as f32) as usize, (u[1] * h as f32) as usize);
    let (x, y) = (x.min(w.saturating_sub(1)), y.min(h.saturating_sub(1)));
    let Some(q) = (x < w && y < h).then(|| src.get(y * w + x)).flatten().copied() else { return };
    let sample = [q[0], q[1], q[2]].map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    let existing = s.component.filter(|j| matches!(m.components.get(*j).map(|c| &c.shape), Some(Shape::Color { .. })));
    if let Some(Component { shape: Shape::Color { samples, .. }, .. }) = existing.and_then(|j| m.components.get_mut(j)) {
        if !shift {
            *samples = vec![sample];
        } else if samples.len() < masks::MAX_SAMPLES {
            samples.push(sample);
        }
    } else if m.components.len() < masks::MAX_COMPONENTS {
        m.components.push(Component { op: take_op(s), invert: false, shape: Shape::Color { samples: vec![sample], amount: 50.0 } });
        s.component = Some(m.components.len() - 1);
    }
}

/// Moves the drag in progress to the pointer `p` (`u` on the image `r`).
fn drag_to(s: &mut MaskUi, m: &mut Mask, r: ERect, p: Pos2, u: [f32; 2], alt: bool, painter: &egui::Painter) {
    let long = r.width().max(r.height()).max(1.0);
    match s.drag {
        Some(Drag::Brush(last)) => {
            let step = (s.brush.size * 0.25).max(0.002);
            let (dx, dy) = (u[0] - last[0], u[1] - last[1]);
            let dist = (dx * r.width()).hypot(dy * r.height()) / long;
            if dist >= step
                && let Some(j) = s.component
            {
                let n = ((dist / step) as usize).clamp(1, 64);
                for k in 1..=n {
                    let f = k as f32 / n as f32;
                    dab(m, j, [last[0] + dx * f, last[1] + dy * f], &s.brush, s.erase || alt);
                }
                s.drag = Some(Drag::Brush(u));
            }
        }
        Some(Drag::NewLinear(j, start)) => {
            if let Some(Component { shape: Shape::Linear { zero, full }, .. }) = m.components.get_mut(j) {
                *full = start;
                *zero = u;
            }
        }
        Some(Drag::LinearPin(j, which)) => {
            if let Some(Component { shape: Shape::Linear { zero, full }, .. }) = m.components.get_mut(j) {
                match which {
                    0 => *full = u,
                    _ => *zero = u,
                }
            }
        }
        Some(Drag::NewRadial(j, start)) => {
            if let Some(Component { shape: Shape::Radial { center, radius, .. }, .. }) = m.components.get_mut(j) {
                *center = start;
                *radius = [(u[0] - start[0]).abs().max(0.005), (u[1] - start[1]).abs().max(0.005)];
            }
        }
        Some(Drag::RadialMove(j)) => {
            if let Some(Component { shape: Shape::Radial { center, .. }, .. }) = m.components.get_mut(j) {
                *center = u;
            }
        }
        Some(Drag::RadialEdge(j, rotate)) => {
            if let Some(Component { shape: Shape::Radial { center, radius, angle, .. }, .. }) = m.components.get_mut(j) {
                let c = screen(r, *center);
                let (dx, dy) = (p.x - c.x, p.y - c.y);
                if rotate {
                    *angle = dy.atan2(dx).to_degrees();
                } else {
                    // The pointer's offset in the ellipse's own axes sets the nearer half axis.
                    let (sn, cs) = angle.to_radians().sin_cos();
                    let (lx, ly) = ((dx * cs + dy * sn) / r.width().max(1.0), (-dx * sn + dy * cs) / r.height().max(1.0));
                    if lx.abs() / radius[0].max(1e-3) >= ly.abs() / radius[1].max(1e-3) {
                        radius[0] = lx.abs().max(0.005);
                    } else {
                        radius[1] = ly.abs().max(0.005);
                    }
                }
            }
        }
        Some(Drag::Objects(start)) => {
            painter.rect_stroke(ERect::from_two_pos(screen(r, start), p), 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
        }
        _ => {}
    }
}

/// The brush component a stroke paints into: the selected one, or a new one (with the pending
/// operation). `None` when the mask already has as many components as it may.
fn brush_target(m: &mut Mask, s: &mut MaskUi) -> Option<usize> {
    let selected = s.component.filter(|j| matches!(m.components.get(*j).map(|c| &c.shape), Some(Shape::Brush { .. })));
    let j = match selected {
        Some(j) => j,
        None => {
            if m.components.len() >= masks::MAX_COMPONENTS {
                return None;
            }
            m.components.push(Component { op: take_op(s), invert: false, shape: Shape::Brush { dabs: Vec::new() } });
            m.components.len() - 1
        }
    };
    s.component = Some(j);
    Some(j)
}

fn dab(m: &mut Mask, j: usize, u: [f32; 2], brush: &Dab, erase: bool) {
    if let Some(Component { shape: Shape::Brush { dabs }, .. }) = m.components.get_mut(j)
        && dabs.len() < masks::MAX_DABS
    {
        dabs.push(Dab { x: u[0], y: u[1], erase, ..*brush });
    }
}

/// `frame`: the guides' rectangle on screen and its size in proxy pixels ([`guide_frame`]).
fn guide_drag(s: &mut MaskUi, d: &mut CameraRawDialog, frame: (ERect, usize, usize), response: &Response, painter: &egui::Painter, inv: Option<[f64; 9]>) {
    let (r, fw, fh) = frame;
    let Some(p) = response.interact_pointer_pos().or(response.hover_pos()) else { return };
    let u = norm(r, p);
    if response.drag_started_by(egui::PointerButton::Primary) {
        s.drag = Some(Drag::Guide(u));
    } else if let Some(Drag::Guide(start)) = s.drag {
        if response.dragged_by(egui::PointerButton::Primary) {
            painter.line_segment([screen(r, start), p], Stroke::new(2.0, Color32::from_rgb(255, 200, 0)));
        } else if response.drag_stopped_by(egui::PointerButton::Primary) {
            s.drag = None;
            let (w, h) = (fw as f64, fh as f64);
            if let Some(inv) = inv
                && let (Some(a), Some(b)) = (map(&inv, start, w, h), map(&inv, u, w, h))
            {
                let _ = super::geometry_panel::add_guide(&mut d.params, a, b);
            }
        }
    }
}

/// Turns the Guided Upright guide tool on or off (the Geometry panel's Guided button and the
/// preview share it).
pub(crate) fn set_guide_tool(ctx: &egui::Context, on: bool) {
    let mut s = load(ctx);
    s.tool = if on {
        DrawTool::Guide
    } else if s.tool == DrawTool::Guide {
        DrawTool::None
    } else {
        s.tool
    };
    store(ctx, s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_adds_masks_and_components() {
        let mut p = CameraRaw::default();
        let mut s = MaskUi::default();
        create(&mut p, &mut s, "Sky", None);
        assert_eq!(p.masks.len(), 1);
        assert_eq!(p.masks[0].components[0].shape, Shape::Sky);
        create(&mut p, &mut s, "Luminance Range", Some(Op::Intersect));
        assert_eq!(p.masks.len(), 1);
        assert_eq!(p.masks[0].components[1].op, Op::Intersect);
        create(&mut p, &mut s, "Linear Gradient", None);
        assert_eq!(p.masks.len(), 2);
        assert_eq!(s.tool, DrawTool::Linear);
        assert!(p.masks[1].components.is_empty(), "drawn kinds wait for the drag");
        for _ in 0..100 {
            create(&mut p, &mut s, "Subject", None);
        }
        assert_eq!(p.masks.len(), masks::MAX_MASKS);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn control_channel_settings_are_validated() {
        let ok = CameraRaw {
            masks: vec![Mask { components: vec![Component { op: Op::Add, invert: false, shape: Shape::Sky }], ..Mask::default() }],
            ..CameraRaw::default()
        };
        assert!(ok.validate_changes(&CameraRaw::default()).is_ok());
        let depth = CameraRaw {
            masks: vec![Mask { components: vec![Component { op: Op::Add, invert: false, shape: Shape::Depth { range: [0.0, 1.0] } }], ..Mask::default() }],
            ..CameraRaw::default()
        };
        assert!(depth.validate_changes(&CameraRaw::default()).is_err());
        let guides = CameraRaw { upright_guides: vec![[[0.1, 0.1], [0.1, 0.9]]; 5], ..CameraRaw::default() };
        assert!(guides.validate_changes(&CameraRaw::default()).is_err());
    }

    #[test]
    fn list_actions_duplicate_and_move() {
        let mut p = CameraRaw::default();
        let mut s = MaskUi::default();
        create(&mut p, &mut s, "Sky", None);
        create(&mut p, &mut s, "Subject", None);
        apply_list_action(&mut p, &mut s, 0, 0);
        assert_eq!(p.masks.len(), 3);
        assert_eq!(p.masks[1].components, p.masks[0].components);
        assert_eq!(s.selected, Some(1));
        apply_list_action(&mut p, &mut s, 2, -1);
        assert_eq!(p.masks[1].components[0].shape, Shape::Subject);
        apply_list_action(&mut p, &mut s, 2, 1); // already last: nothing
        apply_list_action(&mut p, &mut s, 9, 0); // no such mask: nothing
        assert_eq!(p.masks.len(), 3);
    }

    #[test]
    fn outline_hit_test() {
        let r = ERect::from_min_size(pos2(0.0, 0.0), egui::vec2(200.0, 100.0));
        assert!(on_outline(r, [0.5, 0.5], [0.25, 0.25], 0.0, pos2(150.0, 50.0)));
        assert!(!on_outline(r, [0.5, 0.5], [0.25, 0.25], 0.0, pos2(100.0, 50.0)));
    }

    #[test]
    fn control_channel_drives_the_tool_and_rejects_bad_input() {
        let ctx = egui::Context::default();
        let mut p = CameraRaw::default();
        let mut s = MaskUi::default();
        create(&mut p, &mut s, "Brush", None);
        control(&ctx, &p, &json!({"open": true, "selected": 0, "tool": "radial", "overlayMode": "imageOnWhite", "brush": {"size": 9, "autoMask": true}}))
            .unwrap();
        let d = describe(&ctx);
        assert_eq!(d["open"], true);
        assert_eq!(d["tool"], "radial");
        assert_eq!(d["overlayMode"], "imageOnWhite");
        assert_eq!(d["brush"]["autoMask"], true);
        assert!((d["brush"]["size"].as_f64().unwrap() - f64::from(masks::MAX_DAB_SIZE)).abs() < 1e-6);
        for bad in [json!(1), json!({"selected": 7}), json!({"tool": "lasso"}), json!({"brush": {"flow": "x"}}), json!({"nope": 1}), json!({"component": 3})] {
            assert!(control(&ctx, &p, &bad).is_err(), "{bad}");
        }
        assert_eq!(describe(&ctx)["tool"], "radial", "a refused request changes nothing");
    }

    /// The keys arrive sorted ("component" before "selected"); the component is still checked
    /// against, and kept for, the mask the same request selects.
    #[test]
    fn control_selects_the_mask_before_its_component() {
        let ctx = egui::Context::default();
        let mut p = CameraRaw::default();
        let mut s = MaskUi::default();
        create(&mut p, &mut s, "Sky", None);
        create(&mut p, &mut s, "Subject", None);
        control(&ctx, &p, &json!({"selected": 1, "component": 0})).unwrap();
        let d = describe(&ctx);
        assert_eq!((d["selected"].clone(), d["component"].clone()), (json!(1), json!(0)));
        // An index past usize (a 32-bit target) is refused, never truncated.
        assert!(control(&ctx, &p, &json!({"selected": u64::MAX})).is_err());
        assert!(control(&ctx, &p, &json!({"selected": 0, "component": u64::MAX})).is_err());
    }

    /// Subtract › Linear Gradient: the component the drag makes subtracts; the one after adds.
    #[test]
    fn a_drawn_kind_keeps_the_chosen_operation() {
        let mut p = CameraRaw::default();
        let mut s = MaskUi::default();
        create(&mut p, &mut s, "Sky", None);
        create(&mut p, &mut s, "Linear Gradient", Some(Op::Subtract));
        assert_eq!(take_op(&mut s), Op::Subtract);
        assert_eq!(take_op(&mut s), Op::Add);
        create(&mut p, &mut s, "Brush", Some(Op::Intersect));
        assert_eq!(s.pending_op, None, "a ready kind uses the operation itself");
        assert_eq!(p.masks[0].components.last().map(|c| c.op), Some(Op::Intersect));
        create(&mut p, &mut s, "Radial Gradient", Some(Op::Subtract));
        apply_list_action(&mut p, &mut s, 0, 0);
        assert_eq!(take_op(&mut s), Op::Add, "a new selection drops it");
    }

    /// A brush stroke makes a component only while the mask has room for one.
    #[test]
    fn brush_target_respects_the_component_limit() {
        let mut m = Mask { components: vec![Component { op: Op::Add, invert: false, shape: Shape::Sky }; masks::MAX_COMPONENTS], ..Mask::default() };
        let mut s = MaskUi::default();
        assert_eq!(brush_target(&mut m, &mut s), None);
        assert_eq!(m.components.len(), masks::MAX_COMPONENTS);
        m.components.pop();
        s.pending_op = Some(Op::Subtract);
        assert_eq!(brush_target(&mut m, &mut s), Some(masks::MAX_COMPONENTS - 1));
        assert_eq!(m.components.last().map(|c| c.op), Some(Op::Subtract));
        assert_eq!(brush_target(&mut m, &mut s), Some(masks::MAX_COMPONENTS - 1), "the selected brush is reused");
    }

    /// With the Geometry panel hidden (or Before shown) the preview is unwarped and shows no
    /// guides, so the guide tool does not take the drag.
    #[test]
    fn hidden_geometry_has_no_guides_to_draw() {
        let ctx = egui::Context::default();
        set_guide_tool(&ctx, true);
        let p = CameraRaw { perspective_upright: "guided".into(), upright_guides: vec![[[0.3, 0.1], [0.32, 0.9]]], ..CameraRaw::default() };
        assert!(captures_pointer(&ctx, &p, false));
        assert!(!captures_pointer(&ctx, &p, true), "Before shows no guides");
        let panels = super::super::panels::PanelState { hidden: ["geometry"].into(), ..Default::default() };
        assert!(!captures_pointer(&ctx, &panels.effective(&p), false));
    }

    #[test]
    fn the_overlay_key_follows_the_preview_revision() {
        let m = Mask::default();
        assert_ne!(mask_key(&m, 10, 10, 1), mask_key(&m, 10, 10, 2));
    }

    #[test]
    fn dabs_stop_at_the_limit() {
        let mut m = Mask { components: vec![Component { op: Op::Add, invert: false, shape: Shape::Brush { dabs: Vec::new() } }], ..Mask::default() };
        for _ in 0..masks::MAX_DABS + 10 {
            dab(&mut m, 0, [0.5, 0.5], &Dab::default(), false);
        }
        assert!(matches!(&m.components[0].shape, Shape::Brush { dabs } if dabs.len() == masks::MAX_DABS));
        dab(&mut m, 7, [0.5, 0.5], &Dab::default(), false); // no such component: nothing
    }

    #[test]
    fn guide_mapping_round_trips() {
        let p = CameraRaw {
            perspective_upright: "guided".into(),
            upright_guides: vec![[[0.3, 0.1], [0.32, 0.9]], [[0.7, 0.1], [0.66, 0.9]]],
            ..CameraRaw::default()
        };
        let (fwd, inv) = guide_maps(&p, 300, 200).expect("invertible");
        let a = map(&fwd, [0.4, 0.3], 300.0, 200.0).expect("maps");
        let b = map(&inv, a, 300.0, 200.0).expect("maps back");
        assert!((b[0] - 0.4).abs() < 1e-4 && (b[1] - 0.3).abs() < 1e-4);
    }

    /// The guide tool with Masking open and a mask selected: it captures the drag only where
    /// Guided Upright can take a guide, so Before or a hidden Geometry leave it to the zoom tool.
    /// Before takes no mask edits either.
    #[test]
    fn the_guide_tool_is_not_a_mask_tool() {
        let ctx = egui::Context::default();
        let p = CameraRaw { masks: vec![Mask::default()], ..CameraRaw::default() };
        control(&ctx, &p, &json!({"open": true, "selected": 0, "tool": "guides"})).unwrap();
        assert!(!captures_pointer(&ctx, &p, false), "no Guided Upright: nothing to draw");
        let guided = CameraRaw { perspective_upright: "guided".into(), ..p.clone() };
        assert!(captures_pointer(&ctx, &guided, false));
        assert!(!captures_pointer(&ctx, &guided, true), "Before shows no guides");
        control(&ctx, &p, &json!({"tool": "brush"})).unwrap();
        assert!(captures_pointer(&ctx, &p, false));
        assert!(!captures_pointer(&ctx, &p, true), "Before takes no mask edits");
    }

    /// `selected: null` stays deselected while the panel draws; reopening the tool selects the
    /// first mask again, and a selection past the masks falls back to the first.
    #[test]
    fn an_explicit_deselection_is_kept() {
        let ctx = egui::Context::default();
        let p = CameraRaw { masks: vec![Mask::default(), Mask::default()], ..CameraRaw::default() };
        control(&ctx, &p, &json!({"open": true})).unwrap();
        let mut s = load(&ctx);
        auto_select(&mut s, 2);
        assert_eq!(s.selected, Some(0), "opening selects the first");
        store(&ctx, s);
        control(&ctx, &p, &json!({"selected": null})).unwrap();
        let mut s = load(&ctx);
        auto_select(&mut s, 2);
        assert_eq!(s.selected, None, "deselected on purpose");
        store(&ctx, s);
        control(&ctx, &p, &json!({"open": false})).unwrap();
        control(&ctx, &p, &json!({"open": true})).unwrap();
        let mut s = load(&ctx);
        auto_select(&mut s, 2);
        assert_eq!(s.selected, Some(0), "reopened");
        s.selected = Some(5);
        auto_select(&mut s, 2);
        assert_eq!(s.selected, Some(0), "a mask that is gone");
    }

    /// A selection the settings no longer have is dropped; a new dialog starts unselected and
    /// closed, keeping the brush.
    #[test]
    fn the_selection_follows_the_settings_and_each_dialog_starts_afresh() {
        let ctx = egui::Context::default();
        let three = CameraRaw {
            masks: vec![Mask { components: vec![Component { op: Op::Add, invert: false, shape: Shape::Sky }], ..Mask::default() }; 3],
            ..CameraRaw::default()
        };
        control(&ctx, &three, &json!({"open": true, "selected": 2, "component": 0, "brush": {"size": 0.1}})).unwrap();
        reconcile(&ctx, &three);
        assert_eq!((load(&ctx).selected, load(&ctx).component), (Some(2), Some(0)), "still there");
        let one = CameraRaw { masks: three.masks[..1].to_vec(), ..CameraRaw::default() };
        reconcile(&ctx, &one);
        assert_eq!((load(&ctx).selected, load(&ctx).component), (None, None));
        control(&ctx, &three, &json!({"selected": 0, "component": 0})).unwrap();
        reconcile(&ctx, &CameraRaw { masks: vec![Mask::default()], ..CameraRaw::default() });
        assert_eq!((load(&ctx).selected, load(&ctx).component), (Some(0), None), "the component is gone");
        control(&ctx, &three, &json!({"selected": 2})).unwrap();
        reset_for_new_dialog(&ctx);
        let d = describe(&ctx);
        assert_eq!((d["open"].clone(), d["selected"].clone(), d["tool"].clone()), (json!(false), Value::Null, json!("none")));
        assert!((load(&ctx).brush.size - 0.1).abs() < 1e-6, "the brush carries over");
    }

    /// Without a face detector (the browser build) People is not offered: it would make a mask
    /// that selects nothing.
    #[test]
    fn people_needs_a_face_detector() {
        let mut p = CameraRaw::default();
        let mut s = MaskUi::default();
        create_with(&mut p, &mut s, "People", None, false);
        assert!(p.masks.is_empty(), "no detector: no People mask");
        create_with(&mut p, &mut s, "Sky", None, false);
        create_with(&mut p, &mut s, "People", Some(Op::Add), false);
        assert_eq!(p.masks[0].components.len(), 1, "nor a People component");
        create_with(&mut p, &mut s, "People", Some(Op::Add), true);
        assert_eq!(p.masks[0].components.len(), 2);
        assert!(KINDS.iter().all(|(k, _)| kind_available(k, true)));
        assert_eq!(KINDS.iter().filter(|(k, _)| !kind_available(k, false)).count(), 1);
    }

    /// Shift+W toggles Masking, but not while a text field (a mask's name) takes the typing.
    #[test]
    fn shift_w_is_a_letter_while_typing() {
        let ctx = egui::Context::default();
        let shift = egui::Modifiers::SHIFT;
        let shift_w = || egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(shift),
                egui::Event::Key { key: egui::Key::W, physical_key: None, pressed: true, repeat: false, modifiers: shift },
            ],
            ..Default::default()
        };
        let frame = |input: egui::RawInput, typing: bool| {
            ctx.run_ui(input, |ui| {
                let mut name = String::from("Mask 1");
                let r = ui.text_edit_singleline(&mut name);
                if typing {
                    r.request_focus();
                } else {
                    r.surrender_focus();
                }
                strip_button(ui);
            })
            .drop_without_applying_deltas();
        };
        frame(egui::RawInput::default(), true);
        frame(shift_w(), true);
        assert!(!is_open(&ctx), "typed into the name field");
        frame(egui::RawInput::default(), false);
        frame(shift_w(), false);
        assert!(is_open(&ctx), "the shortcut");
    }

    /// Guides are relative to the canvas when the layer extends past it (Geometry warps the
    /// canvas only), else to the whole preview.
    #[test]
    fn guides_are_relative_to_the_canvas() {
        let r = ERect::from_min_size(pos2(10.0, 20.0), egui::vec2(280.0, 160.0));
        let (gr, w, h) = guide_frame(Some([7, 0, 28, 16]), 28, 16, r);
        assert_eq!((w, h), (21, 16));
        assert_eq!(gr, ERect::from_min_max(pos2(80.0, 20.0), pos2(290.0, 180.0)));
        assert_eq!(guide_frame(None, 28, 16, r), (r, 28, 16));
        assert_eq!(guide_frame(Some([0, 0, 40, 16]), 28, 16, r), (r, 28, 16), "a frame past the proxy is ignored");
    }
}
