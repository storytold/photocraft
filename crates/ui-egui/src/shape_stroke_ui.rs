//! Shape stroke geometry controls shared by the tool defaults and Properties.
use egui::{Sense, Stroke, vec2};
use photocraft_doc::{LineCap, LineJoin, ShapeStroke, StrokeAlign};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::theme::Tokens;

/// Colour and width stay in their existing controls. These options never replace a shape's paint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StrokeOptions {
    pub align: StrokeAlign,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub dashes: Vec<f32>,
    pub dash_offset: f32,
    pub opacity: f32,
}

impl Default for StrokeOptions {
    fn default() -> Self {
        Self::from(&ShapeStroke::default())
    }
}

impl From<&ShapeStroke> for StrokeOptions {
    fn from(s: &ShapeStroke) -> Self {
        Self { align: s.align, cap: s.cap, join: s.join, miter_limit: s.miter_limit, dashes: s.dashes.clone(), dash_offset: s.dash_offset, opacity: s.opacity }
    }
}

impl StrokeOptions {
    pub fn params(&self) -> Value {
        json!({"align": format!("{:?}", self.align).to_ascii_lowercase(),
            "cap": format!("{:?}", self.cap).to_ascii_lowercase(), "join": format!("{:?}", self.join).to_ascii_lowercase(),
            "miterLimit": self.miter_limit, "dashes": self.dashes, "dashOffset": self.dash_offset, "opacity": self.opacity * 100.0})
    }

    pub fn stroke(&self, width: f32, paint: photocraft_doc::Fill) -> ShapeStroke {
        ShapeStroke {
            width,
            paint,
            align: self.align,
            cap: self.cap,
            join: self.join,
            miter_limit: self.miter_limit,
            dashes: self.dashes.clone(),
            dash_offset: self.dash_offset,
            opacity: self.opacity,
        }
    }

    fn label(&self) -> &'static str {
        if self.dashes.is_empty() {
            "Solid"
        } else if self.dashes == [4.0, 2.0] {
            "Dashed"
        } else if self.dashes == [0.0, 2.0] && self.cap == LineCap::Round {
            "Dotted"
        } else {
            "Custom"
        }
    }

    fn preset(&mut self, name: &str) {
        self.dash_offset = 0.0;
        self.dashes = match name {
            "Dashed" => vec![4.0, 2.0],
            "Dotted" => {
                self.cap = LineCap::Round;
                vec![0.0, 2.0]
            }
            _ => Vec::new(),
        };
    }

    fn valid(&self) -> bool {
        self.stroke(3.0, photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK)).validate().is_ok()
    }
}

#[derive(Clone)]
struct DragCache {
    shape: photocraft_doc::ShapeLayer,
    clip: egui::Rect,
    image_rect: egui::Rect,
    texture: egui::TextureHandle,
}

/// The drag uses the committed vector rasterizer. Cache by geometry/style/visible region, and
/// cap proxy resolution so oversized windows cannot allocate a document-sized preview.
pub fn paint_shape_preview(painter: &egui::Painter, shape: &photocraft_doc::ShapeLayer) {
    let clip = painter.clip_rect();
    if !clip.is_finite() || clip.is_negative() {
        return;
    }
    let id = egui::Id::new("shape-drag-render");
    let cached = painter.ctx().data(|d| d.get_temp::<DragCache>(id));
    let cache = cached.filter(|c| c.shape == *shape && c.clip == clip).or_else(|| {
        let scale = (2048.0 / clip.width().max(clip.height()).max(1.0)).min(1.0);
        let mut proxy = shape.clone();
        let tr = photocraft_geom::Affine { m: [scale as f64, 0.0, 0.0, scale as f64, -(clip.left() * scale) as f64, -(clip.top() * scale) as f64] };
        proxy.path = proxy.path.transform(&tr);
        if let Some(stroke) = proxy.stroke.as_mut() {
            stroke.width *= scale;
        }
        let canvas = photocraft_geom::Rect::from_size(photocraft_geom::Size::new((clip.width() * scale).ceil() as u32, (clip.height() * scale).ceil() as u32));
        let compiled = photocraft_vector::CompiledShape::new(&proxy, photocraft_vector::DEFAULT_TOLERANCE, canvas);
        let bounds = compiled.bounds()?.intersect(&canvas);
        if bounds.is_empty() {
            return None;
        }
        let rgba = compiled.render_rgba(bounds);
        let bytes: Vec<u8> = rgba.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        let texture = painter.ctx().load_texture(
            "shape-drag",
            egui::ColorImage::from_rgba_unmultiplied([bounds.width() as usize, bounds.height() as usize], &bytes),
            egui::TextureOptions::LINEAR,
        );
        let screen = |x: i32, y: i32| clip.min + vec2(x as f32, y as f32) / scale;
        Some(DragCache {
            shape: shape.clone(),
            clip,
            image_rect: egui::Rect::from_min_max(screen(bounds.x0, bounds.y0), screen(bounds.x1, bounds.y1)),
            texture,
        })
    });
    if let Some(cache) = cache {
        painter.image(cache.texture.id(), cache.image_rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        painter.ctx().data_mut(|d| d.insert_temp(id, cache));
    }
}

/// Transient editor content is still plain serde state, visible to inspection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeEditor {
    pub key: String,
    pub draft: StrokeOptions,
    pub original: StrokeOptions,
    pub target: Option<(photocraft_doc::DocId, photocraft_doc::LayerId)>,
}

pub(crate) struct ShapeStrokePreview {
    doc: photocraft_doc::DocId,
    revision: u64,
    target: photocraft_doc::LayerId,
    options: StrokeOptions,
    shown: std::sync::Arc<photocraft_doc::Document>,
    key: u64,
}

/// Cache the selected shape preview by revision and draft. Cancel simply drops the snapshot.
pub(crate) fn display_doc(app: &mut crate::PhotocraftApp, idx: usize) -> Option<(std::sync::Arc<photocraft_doc::Document>, u64)> {
    let active = app.session.active_index() == Some(idx);
    if !active {
        return None;
    }
    let editor = app.ui.stroke_editor.as_ref().filter(|e| active && e.target.is_some() && e.draft != e.original && e.draft.valid());
    let Some(editor) = editor else {
        app.shape_stroke_preview = None;
        return None;
    };
    let (doc, target) = editor.target?;
    let st = app.session.documents().get(idx)?;
    let current = st.doc.layer(target).and_then(|l| match &l.content {
        photocraft_doc::LayerContent::Shape(sh) => sh.stroke.as_ref().map(StrokeOptions::from),
        _ => None,
    });
    let locks = st.doc.effective_locks(target);
    if st.doc.id != doc || st.active_layer != Some(target) || locks.all || locks.pixels || current.as_ref() != Some(&editor.original) {
        app.shape_stroke_preview = None;
        app.ui.stroke_editor = None;
        return None;
    }
    let fresh =
        app.shape_stroke_preview.as_ref().is_some_and(|p| p.doc == st.doc.id && p.revision == st.revision && p.target == target && p.options == editor.draft);
    if !fresh {
        let shown = photocraft_engine::vector_cmds::preview_shape_stroke(&st.doc, target, &editor.draft.params()).ok()?;
        // Hash all cache inputs so returning to an earlier draft is deterministic, and a
        // reopened editor cannot alias a different document revision's GPU output.
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (st.doc.id, st.revision, target, editor.key.as_str(), editor.draft.params().to_string()).hash(&mut hash);
        let key = (1u64 << 60) | (hash.finish() & ((1u64 << 60) - 1));
        app.shape_stroke_preview =
            Some(ShapeStrokePreview { doc: st.doc.id, revision: st.revision, target, options: editor.draft.clone(), shown: std::sync::Arc::new(shown), key });
    }
    app.shape_stroke_preview.as_ref().map(|p| (p.shown.clone(), p.key))
}

/// Staged editor: outside click/Escape cancels; Apply returns one complete command patch.
pub fn button(
    ui: &mut egui::Ui,
    key: &str,
    current: &StrokeOptions,
    editor: &mut Option<StrokeEditor>,
    closed: bool,
    corners: bool,
    target: Option<(photocraft_doc::DocId, photocraft_doc::LayerId)>,
) -> Option<StrokeOptions> {
    let response = ui.add_sized(vec2(96.0, 22.0), egui::Button::new(tl!(current.label()))).on_hover_text(tl!("Stroke Options"));
    let c = response.rect.right_center() - vec2(10.0, 0.0);
    ui.painter().add(egui::Shape::line(vec![c - vec2(3.0, 1.5), c + vec2(0.0, 1.5), c + vec2(3.0, -1.5)], Stroke::new(1.0, Tokens::get(ui.ctx()).text_dim)));
    if response.clicked() {
        let draft = if current.valid() { current.clone() } else { StrokeOptions::default() };
        *editor = Some(StrokeEditor { key: key.into(), draft, original: current.clone(), target });
    }
    let mut result = None;
    let mut dismissed = false;
    egui::Popup::from_toggle_button_response(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let Some(e) = editor.as_mut().filter(|e| e.key == key && e.original == *current && e.target == target) else {
            ui.close();
            return;
        };
        let s = &mut e.draft;
        ui.set_width(260.0);
        ui.label(egui::RichText::new(tl!("Stroke Options")).strong());
        let body_height = (ui.ctx().content_rect().height() - 150.0).clamp(120.0, 520.0);
        // Keep the footer stable when presets add/remove dash fields or a scroll bar appears.
        egui::ScrollArea::vertical().max_height(body_height).min_scrolled_height(body_height).auto_shrink([false, false]).show(ui, |ui| {
            for name in ["Solid", "Dashed", "Dotted"] {
                let mut sample = s.clone();
                sample.preset(name);
                ui.horizontal(|ui| {
                    if ui.selectable_label(s.label() == name, tl!(name)).clicked() {
                        s.preset(name);
                    }
                    preview(ui, &sample, 154.0, false);
                });
            }
            ui.separator();
            ui.add_enabled_ui(closed, |ui| {
                choice(ui, "Alignment", &mut s.align, &[(StrokeAlign::Inside, "Inside"), (StrokeAlign::Center, "Center"), (StrokeAlign::Outside, "Outside")]);
            });
            if !closed {
                ui.label(egui::RichText::new(tl!("Open paths use centered strokes")).small());
            }
            ui.add_enabled_ui(!closed || !s.dashes.is_empty(), |ui| {
                choice(ui, "Caps", &mut s.cap, &[(LineCap::Butt, "Butt / Flat"), (LineCap::Round, "Round"), (LineCap::Square, "Square")]);
            });
            ui.add_enabled_ui(corners, |ui| {
                choice(ui, "Corners", &mut s.join, &[(LineJoin::Miter, "Miter"), (LineJoin::Round, "Round"), (LineJoin::Bevel, "Bevel")]);
            });
            ui.add_enabled_ui(corners && s.join == LineJoin::Miter, |ui| {
                number(ui, "Miter limit", &mut s.miter_limit, 1.0..=500.0, "×");
            });
            let mut opacity = s.opacity * 100.0;
            number(ui, "Opacity", &mut opacity, 0.0..=100.0, "%");
            s.opacity = opacity / 100.0;
            ui.separator();
            let mut dashed = !s.dashes.is_empty();
            if ui.checkbox(&mut dashed, tl!("Dashed line")).changed() {
                s.preset(if dashed { "Dashed" } else { "Solid" });
            }
            if dashed {
                ui.label(egui::RichText::new(tl!("Dash / gap lengths × stroke width")).small());
                for (i, pair) in s.dashes.chunks_mut(2).enumerate() {
                    ui.push_id(i, |ui| {
                        ui.horizontal(|ui| {
                            for (j, value) in pair.iter_mut().enumerate() {
                                ui.label(tl!(if j == 0 { "Dash" } else { "Gap" }));
                                crate::widgets::value_field(ui, value, 0.0..=10000.0, "×", 68.0);
                            }
                        });
                    });
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(s.dashes.len() < 32, egui::Button::new(tl!("Add pair"))).clicked() {
                        if s.dashes.len() % 2 == 1 {
                            s.dashes.push(2.0);
                        } else {
                            s.dashes.extend([4.0, 2.0]);
                        }
                    }
                    if ui.add_enabled(s.dashes.len() > 2, egui::Button::new(tl!("Remove pair"))).clicked() {
                        s.dashes.truncate(s.dashes.len().saturating_sub(2));
                    }
                });
                number(ui, "Dash offset", &mut s.dash_offset, -1_000_000.0..=1_000_000.0, "×");
            }
            preview(ui, s, 260.0, true);
        });
        let valid = s.valid();
        if !valid {
            ui.colored_label(Tokens::get(ui.ctx()).text_dim, tl!("Use finite lengths and a positive pattern total"));
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(valid, egui::Button::new(tl!("Apply"))).clicked() {
                result = Some(s.clone());
                dismissed = true;
                ui.close();
            }
            if ui.button(tl!("Cancel")).clicked() {
                dismissed = true;
                ui.close();
            }
        });
    });
    if dismissed {
        egui::Popup::close_id(ui.ctx(), egui::Popup::default_response_id(&response));
        *editor = None;
    }
    if editor.as_ref().is_some_and(|e| e.key == key) && !egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response)) {
        *editor = None;
    }
    result
}

fn choice<T: Copy + PartialEq>(ui: &mut egui::Ui, label: &str, value: &mut T, choices: &[(T, &str)]) {
    ui.horizontal(|ui| {
        ui.label(tl!(label));
        let (press, close) = crate::press_menu::PressCombo::before(ui, label);
        let combo = egui::ComboBox::from_id_salt(label)
            .width(140.0)
            .selected_text(choices.iter().find(|(v, _)| v == value).map_or("", |(_, n)| tl!(*n)))
            .close_behavior(close)
            .show_ui(ui, |ui| {
                for (v, n) in choices {
                    let item = ui.selectable_label(*value == *v, tl!(*n));
                    if press.chosen(ui, &item) {
                        *value = *v;
                    }
                }
            });
        press.after(&combo.response);
    });
}

fn number(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str) {
    ui.horizontal(|ui| {
        ui.label(tl!(label));
        crate::widgets::value_field(ui, value, range, unit, 78.0);
    });
}

#[derive(Clone)]
struct SampleCache {
    options: StrokeOptions,
    width: usize,
    closed: bool,
    color: egui::Color32,
    texture: egui::TextureHandle,
}

/// A tiny cached raster sample uses the actual vector renderer, including AA, opacity and joins.
fn preview(ui: &mut egui::Ui, options: &StrokeOptions, width: f32, corners: bool) {
    let t = Tokens::get(ui.ctx());
    let height = if corners { 68.0 } else { 22.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().rect_filled(rect, t.radius_sm, t.field);
    let id = response.id.with("stroke-preview");
    let w = width as usize;
    let mut sample = options.clone();
    if !corners {
        sample.align = StrokeAlign::Center;
    }
    let cached = ui.data(|d| d.get_temp::<SampleCache>(id));
    let cache = cached.filter(|c| c.options == sample && c.width == w && c.closed == corners && c.color == t.text).unwrap_or_else(|| {
        let path = if corners {
            photocraft_vector::shapes::rounded_rect(16.0, 16.0, width as f64 - 32.0, 36.0, [0.0; 4])
        } else {
            let mut sp = photocraft_doc::Subpath::polygon(&[(12.0, 11.0), (width as f64 - 12.0, 11.0)]);
            sp.closed = false;
            photocraft_doc::Path::new(vec![sp])
        };
        let c = t.text;
        let shape =
            photocraft_doc::ShapeLayer {
                path,
                stroke: Some(sample.stroke(
                    3.0,
                    photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, 1.0)),
                )),
                ..Default::default()
            };
        let r = photocraft_geom::Rect::from_size(photocraft_geom::Size::new(w as u32, height as u32));
        let pixels = photocraft_vector::CompiledShape::new(&shape, photocraft_vector::DEFAULT_TOLERANCE, r).render_rgba(r);
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        let texture =
            ui.ctx().load_texture("stroke-preview", egui::ColorImage::from_rgba_unmultiplied([w, height as usize], &bytes), egui::TextureOptions::LINEAR);
        SampleCache { options: sample, width: w, closed: corners, color: t.text, texture }
    });
    ui.painter().image(cache.texture.id(), rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    ui.data_mut(|d| d.insert_temp(id, cache));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    struct State {
        current: StrokeOptions,
        editor: Option<StrokeEditor>,
        applied: usize,
    }
    fn harness() -> Harness<'static, State> {
        Harness::builder().with_size(vec2(500.0, 800.0)).build_ui_state(
            |ui, s: &mut State| {
                if let Some(next) = button(ui, "test", &s.current, &mut s.editor, true, true, None) {
                    s.current = next;
                    s.applied += 1;
                }
            },
            State { current: Default::default(), editor: None, applied: 0 },
        )
    }

    #[test]
    fn preset_preview_is_staged_cancelled_or_applied_once() {
        let mut h = harness();
        h.run();
        h.get_by_label("Solid").click();
        h.run();
        h.get_by_label("Dotted").click();
        h.run_steps(3);
        assert!(h.state().current.dashes.is_empty());
        assert_eq!(h.state().editor.as_ref().unwrap().draft.dashes, [0.0, 2.0]);
        let cancel_rect = h.get_by_label("Cancel").rect();
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert!(h.state().editor.is_none(), "Cancel at {cancel_rect:?}; remaining: {:?}", h.query_all_by_label("Cancel").map(|n| n.rect()).collect::<Vec<_>>());
        assert_eq!(h.state().applied, 0);
        h.get_by_label("Solid").click();
        h.run();
        h.get_by_label("Dotted").click();
        h.run_steps(3);
        h.get_by_label("Apply").click();
        h.run();
        assert_eq!(h.state().current.cap, LineCap::Round);
        assert_eq!(h.state().current.dashes, [0.0, 2.0]);
        assert_eq!(h.state().applied, 1);
    }

    #[test]
    fn options_serde_defaults_keep_old_tool_settings_and_full_stroke() {
        assert_eq!(serde_json::from_value::<StrokeOptions>(json!({})).unwrap(), StrokeOptions::default());
        let mut style = StrokeOptions::default();
        style.preset("Dotted");
        style.dashes = vec![0.0, 2.0, 4.0, 1.0];
        style.dash_offset = -0.5;
        let stroke = style.stroke(5.0, photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK));
        assert_eq!(StrokeOptions::from(&stroke), style);
        assert_eq!(serde_json::from_value::<StrokeOptions>(serde_json::to_value(&style).unwrap()).unwrap(), style);
    }

    #[test]
    fn canvas_preview_is_cached_and_matches_commit_at_all_depths() {
        for depth in [8, 16, 32] {
            let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width":80,"height":60,"depth":depth})).unwrap();
            let id = app.run("shape.create", json!({"rect":[15,15,45,30],"stroke":{"width":5,"color":"#ff0000"}})).unwrap()["layer"].as_u64().unwrap();
            let target = photocraft_doc::LayerId(id);
            let original = StrokeOptions::default();
            let mut draft = original.clone();
            draft.preset("Dotted");
            draft.join = LineJoin::Round;
            draft.opacity = 0.5;
            let committed = app.session.active().unwrap().doc.clone();
            let count = app.session.active().unwrap().history.past_len();
            app.ui.stroke_editor = Some(StrokeEditor { key: "test".into(), original, draft: draft.clone(), target: Some((committed.id, target)) });
            let idx = app.session.active_index().unwrap();
            let first = display_doc(&mut app, idx).unwrap();
            let next = display_doc(&mut app, idx).unwrap();
            assert!(std::sync::Arc::ptr_eq(&first.0, &next.0));
            assert_eq!(first.1, next.1);
            assert_eq!(app.session.active().unwrap().doc, committed);
            assert_eq!(app.session.active().unwrap().history.past_len(), count);
            app.ui.stroke_editor = None;
            assert!(display_doc(&mut app, idx).is_none());
            app.run("shape.edit", json!({"layer":id,"stroke":draft.params()})).unwrap();
            assert_eq!(app.session.active().unwrap().doc, first.0);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc, committed);
            app.ui.stroke_editor = Some(StrokeEditor { key: "test".into(), original: StrokeOptions::default(), draft, target: Some((committed.id, target)) });
            app.run("file.new", json!({"width":80,"height":60,"depth":depth})).unwrap();
            let other = app.session.active_index().unwrap();
            assert!(display_doc(&mut app, other).is_none());
            assert!(app.ui.stroke_editor.is_none());
        }
    }
}
