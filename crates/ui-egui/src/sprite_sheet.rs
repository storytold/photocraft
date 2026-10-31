use std::time::{Duration, Instant};

use photocraft_doc::LayerContent;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub const COMMAND: &str = "file.export.spriteSheet";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimelineFrame {
    frame: usize,
    duration_ticks: u32,
}

impl TimelineFrame {
    fn new(frame: usize) -> Self {
        Self { frame, duration_ticks: 1 }
    }

    fn json(self) -> Value {
        json!({"frame": self.frame, "durationTicks": self.duration_ticks})
    }
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key("__spriteSheet")
}

pub fn open(app: &mut PhotocraftApp) -> Result<u64, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    let mut fields = Map::new();
    fields.insert("__spriteSheet".into(), Value::Bool(true));
    fields.insert("__command".into(), json!(COMMAND));
    fields.insert("__label".into(), json!("Export Sprite Sheet"));
    fields.insert("source".into(), json!("layers"));
    fields.insert("frameWidth".into(), json!(32));
    fields.insert("frameHeight".into(), json!(32));
    fields.insert("columns".into(), json!(8));
    fields.insert("padding".into(), json!(0));
    fields.insert("path".into(), json!("Sprite Sheet.png"));
    Ok(app.ui.open_dialog(crate::state::DialogKind::Command, fields))
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>, dialog: u64) {
    ui.horizontal(|ui| {
        ui.label("Source");
        craft_ui::ComboBox::from_id_salt((COMMAND, dialog, "source"))
            .selected_text(if fields.get("source").and_then(Value::as_str) == Some("grid") { "Canvas grid" } else { "Visible layers" })
            .show_ui(ui, |ui| {
                if ui.selectable_label(fields.get("source").and_then(Value::as_str) != Some("grid"), "Visible layers").clicked() {
                    fields.insert("source".into(), json!("layers"));
                }
                if ui.selectable_label(fields.get("source").and_then(Value::as_str) == Some("grid"), "Canvas grid").clicked() {
                    fields.insert("source".into(), json!("grid"));
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Frame size");
        number(ui, fields, "frameWidth", 1, 65_536);
        number(ui, fields, "frameHeight", 1, 65_536);
    });
    ui.horizontal(|ui| {
        ui.label("Grid columns");
        number(ui, fields, "columns", 1, 65_536);
        ui.label("Padding");
        number(ui, fields, "padding", 0, 65_536);
    });
    ui.separator();
    preview(app, ui, fields, dialog);
    ui.vertical(|ui| {
        ui.label("Output");
        let mut path = fields.get("path").and_then(Value::as_str).unwrap_or("Sprite Sheet.png").to_string();
        if ui.text_edit_singleline(&mut path).changed() {
            fields.insert("path".into(), json!(path));
        }
        if ui.button("Browse…").clicked() {
            let suggested = fields.get("path").and_then(Value::as_str).unwrap_or("Sprite Sheet.png").to_string();
            let result = app.pick_save(&suggested, move |app, path| {
                let d = app.ui.dialog_mut(dialog).ok_or("the sprite sheet dialog was closed")?;
                d.fields.insert("path".into(), json!(path));
                Ok(Value::Null)
            });
            if let Err(e) = result {
                app.ui.status = e;
            }
        }
    });

    #[derive(Clone)]
    struct Playback {
        last_tick: Instant,
        step: usize,
        tick_count: u32,
    }

    fn preview(app: &PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>, dialog: u64) {
        let frame_w = fields.get("frameWidth").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(32);
        let frame_h = fields.get("frameHeight").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(32);
        let columns = fields.get("columns").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(8).max(1);
        let source = fields.get("source").and_then(Value::as_str).unwrap_or("layers");
        let frame_count = if source == "grid" {
            let Some(doc) = app.session.active().map(|d| &d.doc) else { return };
            let bounds = doc.bounds();
            let cols = bounds.width().saturating_add(frame_w.saturating_sub(1)) / frame_w.max(1);
            let rows = bounds.height().saturating_add(frame_h.saturating_sub(1)) / frame_h.max(1);
            usize::try_from(cols.saturating_mul(rows).max(1)).unwrap_or(1)
        } else {
            app.session.active().map(|d| d.doc.layers.iter().filter(|l| l.visible && !matches!(l.content, LayerContent::Adjustment(_))).count()).unwrap_or(0)
        };
        if frame_count == 0 {
            ui.label("No visible raster frames available.");
            return;
        }
        let selected_value = fields.entry("selectedFrames").or_insert_with(|| json!([])).clone();
        let mut selected_frames = selected_value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| {
                value.as_u64().and_then(|frame| usize::try_from(frame).ok()).map(TimelineFrame::new).or_else(|| {
                    let object = value.as_object()?;
                    let frame = object.get("frame")?.as_u64().and_then(|v| usize::try_from(v).ok())?;
                    let duration_ticks =
                        object.get("durationTicks").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).filter(|v| (1..=60).contains(v)).unwrap_or(1);
                    Some(TimelineFrame { frame, duration_ticks })
                })
            })
            .filter(|item| item.frame < frame_count)
            .collect::<Vec<_>>();
        ui.label("Layout preview (click frames to add or remove them from playback order)");
        let cell_size = egui::vec2(48.0, 48.0);
        let rows = frame_count.div_ceil(columns);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(columns as f32 * cell_size.x, rows as f32 * cell_size.y), egui::Sense::click());
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let col = ((pos.x - rect.left()) / cell_size.x) as usize;
            let row = ((pos.y - rect.top()) / cell_size.y) as usize;
            let index = row.saturating_mul(columns).saturating_add(col);
            if index < frame_count {
                if let Some(position) = selected_frames.iter().position(|item| item.frame == index) {
                    selected_frames.remove(position);
                } else {
                    selected_frames.push(TimelineFrame::new(index));
                }
                fields.insert("selectedFrames".into(), Value::Array(selected_frames.iter().copied().map(TimelineFrame::json).collect()));
            }
        }
        let painter = ui.painter_at(rect);
        for index in 0..frame_count {
            let col = index % columns;
            let row = index / columns;
            let cell = egui::Rect::from_min_size(rect.min + egui::vec2(col as f32 * cell_size.x, row as f32 * cell_size.y), cell_size);
            let order = selected_frames.iter().position(|item| item.frame == index);
            painter.rect_filled(cell, 0.0, egui::Color32::from_gray(38 + (index as u8 % 3) * 12));
            painter.rect_stroke(
                cell,
                0.0,
                egui::Stroke::new(1.0, order.map_or(egui::Color32::from_gray(100), |_| egui::Color32::from_rgb(0, 190, 255))),
                egui::StrokeKind::Inside,
            );
            painter.text(
                cell.left_top() + egui::vec2(4.0, 4.0),
                egui::Align2::LEFT_TOP,
                index.to_string(),
                egui::FontId::proportional(11.0),
                egui::Color32::WHITE,
            );
            if let Some(order) = order {
                painter.text(
                    cell.right_bottom() - egui::vec2(4.0, 4.0),
                    egui::Align2::RIGHT_BOTTOM,
                    (order + 1).to_string(),
                    egui::FontId::proportional(11.0),
                    egui::Color32::YELLOW,
                );
            }
        }
        let mut playing = fields.get("playing").and_then(Value::as_bool).unwrap_or(true);
        let mut fps = fields.get("fps").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(8).clamp(1, 60);
        ui.horizontal(|ui| {
            if ui.checkbox(&mut playing, "Play").changed() {
                fields.insert("playing".into(), json!(playing));
            }
            if ui.add(egui::Slider::new(&mut fps, 1..=60).text("FPS")).changed() {
                fields.insert("fps".into(), json!(fps));
            }
            if ui.button("Clear Frames").clicked() {
                selected_frames.clear();
                fields.insert("selectedFrames".into(), json!([]));
            }
        });
        if selected_frames.is_empty() {
            ui.label("Select frames to preview an animation.");
            return;
        }
        let key = egui::Id::unique(("sprite-sheet-playback", dialog));
        let mut playback = ui.ctx().data(|data| data.get_temp::<Playback>(key)).unwrap_or(Playback { last_tick: Instant::now(), step: 0, tick_count: 0 });
        let interval = Duration::from_secs_f32(1.0 / fps as f32);
        if playing && playback.last_tick.elapsed() >= interval {
            playback.tick_count = playback.tick_count.saturating_add(1);
            playback.last_tick = Instant::now();
            let duration_ticks = selected_frames[playback.step % selected_frames.len()].duration_ticks;
            if playback.tick_count >= duration_ticks {
                playback.tick_count = 0;
                playback.step = (playback.step + 1) % selected_frames.len();
            }
        }
        ui.ctx().data_mut(|data| data.insert_temp(key, playback.clone()));
        if playing {
            ui.ctx().request_repaint_after(interval);
        }
        ui.label(format!("Animation preview: frame {}", selected_frames[playback.step % selected_frames.len()].frame));
        timeline(ui, fields, dialog, &selected_frames, playback.step);
    }

    fn timeline(ui: &mut egui::Ui, fields: &mut Map<String, Value>, dialog: u64, selected: &[TimelineFrame], active_step: usize) {
        ui.add_space(6.0);
        ui.strong(format!("Animation Timeline ({} frames)", selected.len()));
        if selected.is_empty() {
            ui.label("Timeline is empty. Click frames above to append them.");
            return;
        }
        let drag_key = egui::Id::unique(("sprite-sheet-drag", dialog));
        let mut swap = None;
        let mut remove = None;
        egui::ScrollArea::horizontal().id_salt(("sprite-sheet-timeline", dialog)).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (position, item) in selected.iter().copied().enumerate() {
                    ui.vertical(|ui| {
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::click_and_drag());
                        let painter = ui.painter_at(rect);
                        let active = position == active_step % selected.len();
                        painter.rect_filled(rect, 3.0, egui::Color32::from_gray(30));
                        painter.rect_stroke(
                            rect,
                            3.0,
                            egui::Stroke::new(2.0, if active { egui::Color32::from_rgb(0, 200, 255) } else { egui::Color32::from_gray(90) }),
                            egui::StrokeKind::Inside,
                        );
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            item.frame.to_string(),
                            egui::FontId::proportional(14.0),
                            egui::Color32::WHITE,
                        );
                        if response.drag_started() {
                            ui.ctx().data_mut(|data| data.insert_temp(drag_key, position));
                        }
                        if response.hovered() && ui.input(|input| input.pointer.any_released()) {
                            let from = ui.ctx().data(|data| data.get_temp::<usize>(drag_key));
                            if let Some(from) = from.filter(|&from| from != position) {
                                swap = Some((from, position));
                            }
                        }
                        if ui.small_button("Remove").clicked() {
                            remove = Some(position);
                        }
                        let mut duration_ticks = item.duration_ticks;
                        if ui.add(egui::DragValue::new(&mut duration_ticks).range(1..=60).speed(0.1).suffix(" ticks")).changed() {
                            let mut updated = selected.to_vec();
                            updated[position].duration_ticks = duration_ticks;
                            fields.insert("selectedFrames".into(), Value::Array(updated.iter().copied().map(TimelineFrame::json).collect()));
                        }
                    });
                }
            });
        });
        if ui.input(|input| input.pointer.any_released()) {
            ui.ctx().data_mut(|data| data.remove::<usize>(drag_key));
        }
        let mut updated = selected.to_vec();
        if let Some(position) = remove {
            if position < updated.len() {
                updated.remove(position);
            }
        } else if let Some((from, to)) = swap
            && from < updated.len()
            && to < updated.len()
        {
            let frame = updated.remove(from);
            updated.insert(to, frame);
        }
        if updated != selected {
            fields.insert("selectedFrames".into(), Value::Array(updated.iter().copied().map(TimelineFrame::json).collect()));
        }
    }
}

fn number(ui: &mut egui::Ui, fields: &mut Map<String, Value>, key: &str, min: u32, max: u32) {
    let mut value = fields.get(key).and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(min);
    if ui.add(egui::DragValue::new(&mut value).range(min..=max)).changed() {
        fields.insert(key.into(), json!(value));
    }
}

pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    fields.get("path").and_then(Value::as_str).filter(|p| !p.trim().is_empty()).ok_or("choose an output PNG path")?;
    let params = Value::Object(fields.iter().filter(|(key, _)| !key.starts_with("__")).map(|(key, value)| (key.clone(), value.clone())).collect());
    let result = app.run(COMMAND, params);
    if result.is_ok() {
        app.ui.status = "Sprite sheet exported".into();
    }
    result
}
