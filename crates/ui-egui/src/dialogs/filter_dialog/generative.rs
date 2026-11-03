//! The Edit menu's generative dialogs (Generative Fill, Generate Image, Generative Expand,
//! Generative Edit, Remove Background, Split into Layers), laid out like the Generate panel:
//! a one-line summary of what the command makes, the prompt in a framed box, the choices that
//! matter as segmented controls and labelled rows, and the sampler's knobs (steps, guidance,
//! context) folded under Advanced with Auto in place of a slider stuck at 0.
//!
//! The body reads and writes the same `fields` as the schema form it replaces, so confirming,
//! Remember settings, `ui.dialog.set` and the command's parameters are unchanged. It is driven
//! by the command's spec: a parameter it has no place for is drawn as a plain row under
//! Advanced, and one the spec drops is simply not drawn.

use egui::{Rect, RichText, Sense, Stroke, StrokeKind, vec2};
use serde_json::{Map, Value, json};

use super::{Kind, Param, label, parse_spec};
use crate::generate_panel::{LABEL_W, SECTION_GAP, Seg, disclosure, section, segmented, setting_row};
use crate::theme::Tokens;

/// The dialogs this body draws.
pub const COMMANDS: &[&str] = &["generate.fill", "generate.image", "generate.expand", "generate.edit", "generate.removeBackground", "generate.splitLayers"];

/// The dialog's open/closed Advanced section and negative prompt (dialog state, never sent).
const ADVANCED: &str = "__advanced";
const NEGATIVE: &str = "__showNegative";
/// The document's size `[w, h]` and whether it has a selection, given by the opener.
pub const DOC_SIZE: &str = "__docSize";
pub const SELECTION: &str = "__selection";

/// Keys drawn in their own places (prompt, sides, settings); the rest go to Advanced.
const PLACED: &[&str] = &[
    "prompt",
    "negative",
    "template",
    "left",
    "top",
    "right",
    "bottom",
    "width",
    "height",
    "target",
    "transparent",
    "variations",
    "edge",
    "layers",
    "tries",
    "grow",
    "asSelection",
    "mode",
    "sampleAllLayers",
    "refine",
    "wholePicture",
    "fillAsEdit",
    "steps",
    "guidance",
    "margin",
];

pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key("__filter") && !f.contains_key("__spec") && f.get("__command").and_then(Value::as_str).is_some_and(|c| COMMANDS.contains(&c))
}

/// What the dialog's default button says.
pub fn ok_label(f: &Map<String, Value>) -> Option<&'static str> {
    if !owns(f) {
        return None;
    }
    Some(match command(f) {
        "generate.removeBackground" => tl!("Remove Background"),
        "generate.splitLayers" => tl!("Split"),
        "generate.expand" => tl!("Expand"),
        _ => tl!("Generate"),
    })
}

/// Whether OK can run: Expand needs at least one side to grow.
pub fn can_confirm(f: &Map<String, Value>) -> bool {
    !owns(f) || command(f) != "generate.expand" || added(f) != [0; 4] || int(f, "width") > 0 || int(f, "height") > 0
}

fn command(f: &Map<String, Value>) -> &str {
    f.get("__command").and_then(Value::as_str).unwrap_or_default()
}

fn int(f: &Map<String, Value>, key: &str) -> i64 {
    f.get(key).and_then(Value::as_f64).map_or(0, |v| v.round() as i64)
}

fn flag(f: &Map<String, Value>, key: &str) -> bool {
    f.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The pixels Expand adds, left, top, right, bottom.
fn added(f: &Map<String, Value>) -> [u32; 4] {
    ["left", "top", "right", "bottom"].map(|k| int(f, k).clamp(0, 4096) as u32)
}

fn doc_size(f: &Map<String, Value>) -> Option<(u32, u32)> {
    let a = f.get(DOC_SIZE)?.as_array()?;
    Some((a.first()?.as_u64()? as u32, a.get(1)?.as_u64()? as u32))
}

/// The summary line under the title: what the command makes, and where.
fn intro(cmd: &str) -> (&'static str, &'static str) {
    match cmd {
        "generate.fill" => ("square-dashed", tl!("Paints the selection from your prompt. The result is a new layer, masked to the selection.")),
        "generate.image" => ("image", tl!("Makes a new picture from your prompt, as a layer or a new document.")),
        "generate.expand" => ("scaling", tl!("Grows the canvas and paints the new area to continue the picture.")),
        "generate.edit" => ("wand-sparkles", tl!("Changes the picture by instruction. The result is a new layer above it.")),
        "generate.removeBackground" => ("eraser", tl!("Masks out the background and keeps the subject. No pixels are changed.")),
        _ => ("layers", tl!("Puts each subject on its own layer above the background.")),
    }
}

/// The prompt section's title and the empty box's hint.
fn prompt_text(cmd: &str) -> (&'static str, &'static str) {
    match cmd {
        "generate.fill" => (tl!("Prompt"), tl!("What to paint in the selection, e.g. a wooden bench. Empty: fill in the surroundings.")),
        "generate.image" => (tl!("Prompt"), tl!("Describe the picture: subject, setting, light, style…")),
        "generate.expand" => (tl!("Prompt (optional)"), tl!("What the new area shows. Empty: continue the scene.")),
        "generate.edit" => (tl!("Instruction"), tl!("What to change, e.g. make the sky stormy, turn the boat blue…")),
        "generate.removeBackground" => (tl!("Subject"), tl!("What to keep, e.g. the lighthouse. Empty: the main subject.")),
        _ => (tl!("Subject"), tl!("What to cut out, e.g. the dog. Empty: the main subject.")),
    }
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let cmd = command(f).to_string();
    let Some(spec) = photocraft_engine::commands::find(&cmd) else { return };
    let params = parse_spec(spec.params);
    let has = |k: &str| params.iter().any(|p| p.key == k);
    ui.spacing_mut().item_spacing.y = 6.0;
    // Wide enough for the prompt, the sides diagram and the segmented rows.
    ui.set_width(ui.available_width().max(380.0));

    let (icon, text) = intro(&cmd);
    summary(ui, &t, icon, text);
    ui.add_space(SECTION_GAP - 4.0);

    if has("prompt") {
        let (title, hint) = prompt_text(&cmd);
        section(ui, &t, title, None);
        prompt_box(ui, &t, f, &cmd, hint);
        if has("negative") {
            let open = flag(f, NEGATIVE) || f.get("negative").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
            if disclosure(ui, &t, tl!("Negative prompt"), open).clicked() {
                f.insert(NEGATIVE.into(), json!(!open));
            }
            if open {
                let mut v = f.get("negative").and_then(Value::as_str).unwrap_or_default().to_string();
                let r = ui.add(egui::TextEdit::singleline(&mut v).hint_text(tl!("What to avoid…")).desired_width(f32::INFINITY).id_salt((&cmd, "negative")));
                crate::widgets::field_tab::register(ui.ctx(), r.id);
                if v.is_empty() {
                    f.remove("negative");
                } else {
                    f.insert("negative".into(), json!(v));
                }
            }
        }
        ui.add_space(SECTION_GAP);
    }

    if cmd == "generate.expand" && has("left") {
        section(ui, &t, tl!("Canvas"), None);
        expand_sides(ui, &t, f);
        ui.add_space(SECTION_GAP);
    }

    section(ui, &t, tl!("Settings"), None);
    settings(ui, &t, f, &cmd, &params);
    ui.add_space(SECTION_GAP - 4.0);

    advanced(ui, &t, f, &cmd, &params);
}

/// The tinted line under the title.
fn summary(ui: &mut egui::Ui, t: &Tokens, icon: &str, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        crate::icons::paint(ui, r, icon, 16.0, t.accent);
        ui.add(egui::Label::new(RichText::new(text).color(t.text_dim)).wrap());
    });
}

/// The prompt in a framed box that grows with it (the accent frame while it has the keyboard),
/// with a character count. Opens with the keyboard in it.
fn prompt_box(ui: &mut egui::Ui, t: &Tokens, f: &mut Map<String, Value>, cmd: &str, hint: &str) {
    let mut v = f.get("prompt").and_then(Value::as_str).unwrap_or_default().to_string();
    let id = egui::Id::unique(("generative-dialog-prompt", cmd));
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    // Expand's prompt is optional and usually empty: a shorter box keeps the dialog on screen.
    let least = if cmd == "generate.expand" { 2 } else { 3 };
    let rows = v.lines().count().clamp(least, 8);
    egui::Frame::new()
        .fill(t.field)
        .stroke(Stroke::new(1.0, if focused { t.accent } else { t.field_border }))
        .corner_radius(t.radius_sm)
        .inner_margin(egui::Margin { left: 8, right: 6, top: 6, bottom: 4 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let resp = ui.add(
                egui::TextEdit::multiline(&mut v)
                    .hint_text(RichText::new(hint).color(t.text_faint))
                    .desired_rows(rows)
                    .desired_width(f32::INFINITY)
                    .char_limit(crate::generate_panel::MAX_PROMPT * 2)
                    .frame(egui::Frame::NONE)
                    .id(id),
            );
            crate::widgets::field_tab::register(ui.ctx(), resp.id);
            // The dialog opens with the keyboard in the prompt, not in the first number field.
            if crate::widgets::take_first_field(ui.ctx()) {
                resp.request_focus();
            }
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let n = v.chars().count();
                    let max = crate::generate_panel::MAX_PROMPT;
                    let color = if n > max { t.danger } else { t.text_faint };
                    ui.label(RichText::new(format!("{n} / {max}")).small().color(color));
                });
            });
        });
    if v.is_empty() {
        f.insert("prompt".into(), json!(""));
    } else {
        f.insert("prompt".into(), json!(v));
    }
}

/// The one-click amounts: a share of the picture on every side, the aspect ratios the picture
/// can grow into, and Clear.
fn expand_presets(dw: u32, dh: u32) -> Vec<(String, &'static str, [u32; 4])> {
    let mut out = Vec::new();
    for pct in [10u32, 25, 50] {
        let (x, y) = (dw * pct / 100, dh * pct / 100);
        out.push((format!("+{pct}%"), tl!("Add this share of the picture on every side"), [x, y, x, y]));
    }
    // An aspect ratio wider or taller than the picture: grow the short way, evenly.
    for (name, num, den, tip) in
        [("1:1", 1, 1, tl!("Make it square")), ("4:3", 4, 3, tl!("Grow to 4:3")), ("16:9", 16, 9, tl!("Grow to 16:9")), ("9:16", 9, 16, tl!("Grow to 9:16"))]
    {
        let want_w = dh * num / den;
        let want_h = dw * den / num;
        let sides = if want_w > dw {
            let add = want_w - dw;
            [add / 2, 0, add - add / 2, 0]
        } else if want_h > dh {
            let add = want_h - dh;
            [0, add / 2, 0, add - add / 2]
        } else {
            continue;
        };
        if sides.iter().all(|v| *v <= 4096) && sides != [0; 4] {
            out.push((name.to_string(), tip, sides));
        }
    }
    out
}

/// Expand's sides: a to-scale diagram of the grown canvas (the picture inside, each added band
/// labelled with its pixels) beside the four fields, one-click amounts above, the new size below.
fn expand_sides(ui: &mut egui::Ui, t: &Tokens, f: &mut Map<String, Value>) {
    let mut e = added(f);
    let (dw, dh) = doc_size(f).unwrap_or((1024, 1024));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (name, tip, sides) in expand_presets(dw, dh) {
            if chip(ui, t, &name, e == sides).on_hover_text(tip).clicked() {
                e = sides;
            }
        }
        if e != [0; 4] && chip(ui, t, tl!("Clear"), false).on_hover_text(tl!("Add nothing")).clicked() {
            e = [0; 4];
        }
    });
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(t.field.gamma_multiply(0.5))
        .stroke(Stroke::new(1.0, t.separator))
        .corner_radius(t.radius_sm)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let fields_w = 150.0;
                let side = (ui.available_width() - fields_w - 12.0).clamp(120.0, 200.0);
                let (r, _) = ui.allocate_exact_size(vec2(side, 124.0), Sense::hover());
                diagram(ui, t, r, (dw, dh), e);
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    egui::Grid::new("generative-expand-sides").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        for (i, name) in [(1usize, tl!("Top")), (3, tl!("Bottom")), (0, tl!("Left")), (2, tl!("Right"))] {
                            ui.label(RichText::new(name).color(t.text_dim));
                            let mut x = e[i] as f32;
                            let r = crate::widgets::value_field_named(ui, name, &mut x, 0.0..=4096.0, "px", 88.0);
                            crate::widgets::field_tab::register(ui.ctx(), r.id);
                            e[i] = x.round().clamp(0.0, 4096.0) as u32;
                            ui.end_row();
                        }
                    });
                });
            });
            if doc_size(f).is_some() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tl!("Canvas")).small().color(t.text_faint));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let [l, tp, r, b] = e;
                        let text = format!("{dw} × {dh} px  →  {} × {} px", dw + l + r, dh + tp + b);
                        ui.label(RichText::new(text).small().color(if e == [0; 4] { t.text_faint } else { t.text_dim }));
                    });
                });
            }
        });
    if e == [0; 4] && int(f, "width") == 0 && int(f, "height") == 0 {
        ui.label(RichText::new(tl!("Pick an amount or type how many pixels to add on a side.")).small().color(t.text_faint));
    }
    for (k, v) in ["left", "top", "right", "bottom"].into_iter().zip(e) {
        f.insert(k.into(), json!(v));
    }
}

/// The grown canvas to scale in `r`: the added bands tinted with their pixels written in them,
/// the picture in the middle (never smaller than a mark you can see), the new edge dashed.
fn diagram(ui: &egui::Ui, t: &Tokens, r: Rect, (dw, dh): (u32, u32), e: [u32; 4]) {
    let p = ui.painter();
    let (nw, nh) = ((dw + e[0] + e[2]) as f32, (dh + e[1] + e[3]) as f32);
    let s = ((r.width() - 4.0) / nw).min((r.height() - 4.0) / nh).max(0.0);
    let outer = Rect::from_center_size(r.center(), vec2(nw * s, nh * s));
    // The picture's place by its share of the new canvas, at least 18 points so it stays visible.
    let pw = (dw as f32 * s).max(18.0).min(outer.width());
    let ph = (dh as f32 * s).max(18.0).min(outer.height());
    let fx = if e[0] + e[2] == 0 { 0.5 } else { e[0] as f32 / (e[0] + e[2]) as f32 };
    let fy = if e[1] + e[3] == 0 { 0.5 } else { e[1] as f32 / (e[1] + e[3]) as f32 };
    let inner = Rect::from_min_size(outer.min + vec2((outer.width() - pw) * fx, (outer.height() - ph) * fy), vec2(pw, ph));
    if e != [0; 4] {
        p.rect_filled(outer, 3.0, t.accent.gamma_multiply(0.16));
        dashed(p, outer, Stroke::new(1.0, t.accent));
    }
    p.rect_filled(inner, 2.0, t.text_faint.gamma_multiply(0.9));
    p.rect_stroke(inner, 2.0, Stroke::new(1.0, t.text_dim), StrokeKind::Inside);
    if inner.width() >= 22.0 && inner.height() >= 22.0 {
        crate::icons::paint(ui, Rect::from_center_size(inner.center(), vec2(16.0, 16.0)), "image", 16.0, t.card);
    }
    // Each band's pixels, where it is wide enough to hold them.
    let font = egui::FontId::proportional(10.0);
    let bands = [
        (e[0], Rect::from_min_max(outer.min, egui::pos2(inner.left(), outer.bottom())), true),
        (e[1], Rect::from_min_max(outer.min, egui::pos2(outer.right(), inner.top())), false),
        (e[2], Rect::from_min_max(egui::pos2(inner.right(), outer.top()), outer.max), true),
        (e[3], Rect::from_min_max(egui::pos2(outer.left(), inner.bottom()), outer.max), false),
    ];
    for (v, band, vertical) in bands {
        if v == 0 {
            continue;
        }
        let text = format!("+{v}");
        let g = p.layout_no_wrap(text, font.clone(), t.accent);
        let room = if vertical { band.width() } else { band.height() };
        let need = if vertical { g.size().x + 4.0 } else { g.size().y + 2.0 };
        if room >= need {
            let c = if vertical { egui::pos2(band.center().x, inner.center().y) } else { egui::pos2(inner.center().x, band.center().y) };
            p.galley(c - g.size() / 2.0, g, t.accent);
        }
    }
}

/// A dashed rectangle outline.
fn dashed(p: &egui::Painter, r: Rect, stroke: Stroke) {
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in corners.windows(2) {
        p.extend(egui::Shape::dashed_line(&[w[0], w[1]], stroke, 4.0, 3.0));
    }
}

/// A small toggle chip (the Expand amounts).
fn chip(ui: &mut egui::Ui, t: &Tokens, text: &str, on: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), crate::theme::medium(11.0), if on { t.text } else { t.text_dim });
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 20.0), Sense::click());
    let fill = if on {
        t.accent.gamma_multiply(0.25)
    } else if resp.hovered() {
        t.hover
    } else {
        t.field
    };
    ui.painter().rect_filled(rect, 10.0, fill);
    ui.painter().rect_stroke(rect, 10.0, Stroke::new(1.0, if on { t.accent } else { t.field_border }), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, text));
    resp
}

/// The choices that shape the result, as labelled rows.
fn settings(ui: &mut egui::Ui, t: &Tokens, f: &mut Map<String, Value>, cmd: &str, params: &[Param]) {
    let has = |k: &str| params.iter().any(|p| p.key == k);
    if f.contains_key("__models") && has("template") {
        setting_row(ui, tl!("Model"), |ui| {
            let w = ui.available_width();
            super::model_pill(ui, f, cmd, w);
        });
    }
    if has("target") {
        let cur = f.get("target").and_then(Value::as_str).unwrap_or("auto").to_string();
        setting_row(ui, tl!("Output"), |ui| {
            let options = [
                Seg::new("auto".to_string(), tl!("Auto")).tip(tl!("A layer over the picture when one is open, else a new document")),
                Seg::new("layer".to_string(), tl!("Layer")).icon("layers").tip(tl!("A new layer over the open picture")),
                Seg::new("document".to_string(), tl!("Document")).icon("image").tip(tl!("A new document")),
            ];
            if let Some(v) = segmented(ui, &cur, &options, 0.0) {
                f.insert("target".into(), json!(v));
            }
        });
    }
    if cmd == "generate.image" && has("width") && has("height") {
        image_size(ui, t, f);
    }
    if has("layers") {
        let range = params.iter().find(|p| p.key == "layers").and_then(|p| match p.kind {
            Kind::Range { min, max, .. } => Some((min as i64, max as i64)),
            _ => None,
        });
        let (lo, hi) = range.unwrap_or((2, 6));
        let hi = hi.min(lo + 6);
        let cur = int(f, "layers").clamp(lo, hi);
        let labels: Vec<String> = (lo..=hi).map(|n| n.to_string()).collect();
        setting_row(ui, tl!("Layers"), |ui| {
            let options: Vec<Seg<'_, i64>> = (lo..=hi).zip(labels.iter()).map(|(n, l)| Seg::new(n, l.as_str())).collect();
            if let Some(n) = segmented(ui, &cur, &options, 0.0) {
                f.insert("layers".into(), json!(n));
            }
        });
        f.insert("layers".into(), json!(int(f, "layers").clamp(lo, hi)));
    }
    if has("variations") {
        count_row(ui, f, "variations", tl!("Variations"), tl!("Results with consecutive seeds, each a layer; the first is shown"));
    }
    if has("tries") {
        count_row(ui, f, "tries", tl!("Tries"), tl!("Seeds run in a row; the split that separates best is kept"));
    }
    if has("asSelection") {
        let as_sel = flag(f, "asSelection");
        setting_row(ui, tl!("Output"), |ui| {
            let options = [
                Seg::new(false, tl!("Layer Mask")).icon("layers").tip(tl!("The matte becomes the layer's mask")),
                Seg::new(true, tl!("Selection")).icon("square-dashed").tip(tl!("The matte becomes the selection")),
            ];
            if let Some(v) = segmented(ui, &as_sel, &options, 0.0) {
                f.insert("asSelection".into(), json!(v));
            }
        });
        if as_sel && has("mode") {
            let cur = f.get("mode").and_then(Value::as_str).unwrap_or("replace").to_string();
            setting_row(ui, "", |ui| {
                let options = [
                    Seg::new("replace".to_string(), tl!("New")),
                    Seg::new("add".to_string(), tl!("Add")),
                    Seg::new("subtract".to_string(), tl!("Subtract")),
                    Seg::new("intersect".to_string(), tl!("Intersect")),
                ];
                if let Some(v) = segmented(ui, &cur, &options, 0.0) {
                    f.insert("mode".into(), json!(v));
                }
            });
        }
    }
    // The edge only matters where the result is masked: a fill, an expand, an edit with a selection.
    let masked = cmd != "generate.edit" || flag(f, SELECTION);
    if has("edge") && masked {
        let cur = f.get("edge").and_then(Value::as_str).unwrap_or("soft").to_string();
        setting_row(ui, tl!("Edge"), |ui| {
            let options = [
                Seg::new("soft".to_string(), tl!("Soft")).tip(tl!("The result fades into the picture across a thin band, with no visible line")),
                Seg::new("hard".to_string(), tl!("Hard")).tip(tl!("The result stops exactly at the edge")),
            ];
            if let Some(v) = segmented(ui, &cur, &options, 0.0) {
                f.insert("edge".into(), json!(v));
            }
        });
    }
    if has("grow") {
        setting_row(ui, tl!("Widen"), |ui| {
            let mut g = int(f, "grow") as f32;
            let r = crate::widgets::value_field(ui, &mut g, 0.0..=512.0, "px", 72.0)
                .on_hover_text(tl!("Widen the selection by this many pixels first, so a bulkier replacement has room"));
            crate::widgets::field_tab::register(ui.ctx(), r.id);
            ui.label(RichText::new(tl!("around the selection")).small().color(t.text_faint));
            f.insert("grow".into(), json!(g.round().clamp(0.0, 512.0) as i64));
        });
    }
    // Switches, each with a line on what it does.
    let switches: [(&str, &str, &str); 5] = [
        ("transparent", tl!("Transparent background"), tl!("The subject alone, where the model can make one")),
        ("wholePicture", tl!("Ignore the selection"), tl!("The edit shows over the whole picture")),
        ("fillAsEdit", tl!("Fill by instruction"), tl!("Qwen-Image 2.1: the selection is replaced by name, e.g. replace the cup with a vase")),
        ("sampleAllLayers", tl!("Sample all layers"), tl!("Use what is visible, not the active layer alone")),
        ("refine", tl!("Refine edges"), tl!("Soften the detector's hard edge, for hair and fur")),
    ];
    let shown: Vec<_> = switches.iter().filter(|(k, _, _)| has(k)).collect();
    if !shown.is_empty() {
        ui.add_space(2.0);
        for (k, name, tip) in shown {
            let default = params.iter().find(|p| p.key == *k).is_some_and(|p| matches!(p.kind, Kind::Bool(true)));
            let mut b = f.get(*k).and_then(Value::as_bool).unwrap_or(default);
            ui.horizontal(|ui| {
                ui.add_space(LABEL_W + 6.0);
                crate::widgets::checkbox(ui, &mut b, name).on_hover_text(*tip);
            });
            f.insert((*k).into(), json!(b));
        }
    }
}

/// A 1–4 count as a segmented row.
fn count_row(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, name: &str, tip: &str) {
    let cur = int(f, key).clamp(1, 4);
    setting_row(ui, name, |ui| {
        let options: Vec<Seg<'_, i64>> = [(1, "1"), (2, "2"), (3, "3"), (4, "4")].into_iter().map(|(n, l)| Seg::new(n, l).tip(tip)).collect();
        if let Some(n) = segmented(ui, &cur, &options, 0.0) {
            f.insert(key.into(), json!(n));
        }
    });
    f.insert(key.into(), json!(int(f, key).clamp(1, 4)));
}

/// Generate Image's size: the document's (or the template's default), or one given.
fn image_size(ui: &mut egui::Ui, t: &Tokens, f: &mut Map<String, Value>) {
    let custom = int(f, "width") > 0 || int(f, "height") > 0 || flag(f, "__customSize");
    setting_row(ui, tl!("Size"), |ui| {
        let options = [Seg::new(false, tl!("Auto")).tip(tl!("The document's size, or the model's when there is none")), Seg::new(true, tl!("Custom"))];
        if let Some(c) = segmented(ui, &custom, &options, 0.0) {
            f.insert("__customSize".into(), json!(c));
            if c {
                let (w, h) = doc_size(f).unwrap_or((1024, 1024));
                f.insert("width".into(), json!(w));
                f.insert("height".into(), json!(h));
            } else {
                f.insert("width".into(), json!(0));
                f.insert("height".into(), json!(0));
            }
        }
    });
    if int(f, "width") > 0 || int(f, "height") > 0 || flag(f, "__customSize") {
        setting_row(ui, "", |ui| {
            let mut w = int(f, "width") as f32;
            let mut h = int(f, "height") as f32;
            let a = crate::widgets::value_field_named(ui, tl!("Width"), &mut w, 64.0..=4096.0, "px", 84.0);
            ui.label(RichText::new("×").color(t.text_dim));
            let b = crate::widgets::value_field_named(ui, tl!("Height"), &mut h, 64.0..=4096.0, "px", 84.0);
            crate::widgets::field_tab::register(ui.ctx(), a.id);
            crate::widgets::field_tab::register(ui.ctx(), b.id);
            f.insert("width".into(), json!(w.round() as i64));
            f.insert("height".into(), json!(h.round() as i64));
        });
    } else if let Some((w, h)) = doc_size(f) {
        setting_row(ui, "", |ui| {
            ui.label(RichText::new(format!("{w} × {h} px")).small().color(t.text_faint));
        });
    }
}

/// Steps, guidance and the rest, folded away; Auto means the model's own.
fn advanced(ui: &mut egui::Ui, t: &Tokens, f: &mut Map<String, Value>, cmd: &str, params: &[Param]) {
    let rest: Vec<&Param> = params.iter().filter(|p| !PLACED.contains(&p.key.as_str()) && !matches!(p.kind, Kind::Json)).collect();
    let knobs: Vec<&Param> = params.iter().filter(|p| matches!(p.key.as_str(), "steps" | "guidance" | "margin")).collect();
    if knobs.is_empty() && rest.is_empty() {
        return;
    }
    let open = flag(f, ADVANCED);
    let tuned = ["steps", "guidance"].iter().any(|k| int_or_float(f, k) > 0.0);
    let title = if tuned && !open { format!("{} · {}", tl!("Advanced"), tl!("custom")) } else { tl!("Advanced").to_string() };
    if disclosure(ui, t, &title, open).clicked() {
        f.insert(ADVANCED.into(), json!(!open));
    }
    if !open {
        return;
    }
    for p in knobs {
        match (p.key.as_str(), &p.kind) {
            ("steps", Kind::Range { max, .. }) => auto_number(ui, f, "steps", tl!("Steps"), 1.0..=*max, 20.0, 0),
            ("guidance", Kind::Range { max, .. }) => auto_number(ui, f, "guidance", tl!("Guidance"), 0.5..=*max, 4.0, 1),
            ("margin", Kind::Range { min, max, default }) => {
                let mut v = f.get("margin").and_then(Value::as_f64).unwrap_or(*default as f64) as f32 * 100.0;
                setting_row(ui, tl!("Context"), |ui| {
                    let r = crate::widgets::value_field(ui, &mut v, min * 100.0..=max * 100.0, "%", 72.0)
                        .on_hover_text(tl!("How much of the picture around the selection the model sees, as a share of its larger side"));
                    crate::widgets::field_tab::register(ui.ctx(), r.id);
                    ui.label(RichText::new(tl!("around the selection")).small().color(t.text_faint));
                });
                f.insert("margin".into(), json!((v / 100.0 * 100.0).round() / 100.0));
            }
            _ => {}
        }
    }
    // Anything else the spec has: a plain row, so a new parameter is never unreachable.
    for p in rest {
        let name = label(&p.key);
        match &p.kind {
            Kind::Range { min, max, default } => {
                let mut v = f.get(&p.key).and_then(Value::as_f64).unwrap_or(*default as f64) as f32;
                crate::widgets::slider_row_with(ui, &name, &mut v, *min..=*max, "", None, Default::default());
                f.insert(p.key.clone(), json!(v));
            }
            Kind::Bool(default) => {
                let mut b = f.get(&p.key).and_then(Value::as_bool).unwrap_or(*default);
                crate::widgets::checkbox(ui, &mut b, &name);
                f.insert(p.key.clone(), json!(b));
            }
            Kind::Int { default } => {
                let mut v = f.get(&p.key).and_then(Value::as_f64).unwrap_or(*default as f64) as f32;
                setting_row(ui, &name, |ui| {
                    crate::widgets::value_field(ui, &mut v, -30000.0..=30000.0, "", 80.0);
                });
                f.insert(p.key.clone(), json!(v.round() as i64));
            }
            Kind::Choice { options, default } => {
                let mut cur = f.get(&p.key).and_then(Value::as_str).or(default.as_deref()).or(options.first().map(String::as_str)).unwrap_or("").to_string();
                let labels: Vec<String> = options.iter().map(|o| super::choice_label(o)).collect();
                let opts: Vec<(String, &str)> = options.iter().cloned().zip(labels.iter().map(String::as_str)).collect();
                setting_row(ui, &name, |ui| {
                    crate::widgets::dropdown(ui, &format!("gen-{cmd}-{}", p.key), &name, &mut cur, &opts, ui.available_width());
                });
                f.insert(p.key.clone(), json!(cur));
            }
            Kind::Text => {
                let mut v = f.get(&p.key).and_then(Value::as_str).unwrap_or_default().to_string();
                setting_row(ui, &name, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut v).desired_width(f32::INFINITY));
                });
                if v.is_empty() {
                    f.remove(&p.key);
                } else {
                    f.insert(p.key.clone(), json!(v));
                }
            }
            _ => {}
        }
    }
}

fn int_or_float(f: &Map<String, Value>, key: &str) -> f64 {
    f.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// A number whose 0 means "the model's own": Auto, or Custom with a field.
fn auto_number(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, name: &str, range: std::ops::RangeInclusive<f32>, start: f32, decimals: u32) {
    let mut v = int_or_float(f, key) as f32;
    let custom = v > 0.0;
    setting_row(ui, name, |ui| {
        let seg_w = 128.0_f32.min(ui.available_width());
        let options = [Seg::new(false, tl!("Auto")).tip(tl!("The model's own")), Seg::new(true, tl!("Custom"))];
        if let Some(c) = segmented(ui, &custom, &options, seg_w) {
            v = if c { start } else { 0.0 };
        }
        if v > 0.0 {
            let r = crate::widgets::value_field_prec(ui, &mut v, range.clone(), "", 72.0, decimals).accessible_name(name);
            crate::widgets::field_tab::register(ui.ctx(), r.id);
        }
    });
    let scale = 10f32.powi(decimals as i32);
    f.insert(key.into(), json!(((v * scale).round() / scale) as f64));
}
