//! The font family menu, as in Photoshop (#540): a search field, the recently used fonts first
//! (Preferences › Type › Number of Recent Fonts to Display), then every family, each with a
//! "Sample" drawn in its own face at the Font Preview Size (Type › Font Preview Size, or
//! Preferences › Type; Off shows names only).
//!
//! It stays fast with thousands of installed fonts: only the rows on screen are laid out, a sample
//! is rendered the first time its row shows (a few per frame, so opening never stalls), and
//! samples are cached per family and size, the least recently used dropped past a cap.

use std::collections::{HashMap, VecDeque};

use egui::{Rect, Sense, TextureHandle, TextureOptions, pos2, vec2};
use photocraft_engine::prefs::FontPreview;

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The word drawn in each family's face (Photoshop's).
const SAMPLE: &str = "Sample";
/// Samples rendered per frame; the rest follow on the next frames.
const RENDERS_PER_FRAME: usize = 6;
/// Cached sample textures.
const CACHE_CAP: usize = 400;
/// The most families remembered as recently used (the preference's maximum).
const RECENT_MAX: usize = 50;

/// Sample height in points for a Font Preview Size; `None` for Off.
pub fn sample_height(size: FontPreview) -> Option<f32> {
    match size {
        FontPreview::Off => None,
        FontPreview::Small => Some(14.0),
        FontPreview::Medium => Some(18.0),
        FontPreview::Large => Some(24.0),
        FontPreview::ExtraLarge => Some(32.0),
        FontPreview::Huge => Some(44.0),
    }
}

/// The recently used families still installed, newest first, as many as the preference shows.
pub fn recent(app: &PhotocraftApp, installed: &[String]) -> Vec<String> {
    let t = &app.session.prefs().type_;
    t.recent_font_list.iter().filter(|f| installed.contains(f)).take(t.recent_fonts as usize).cloned().collect()
}

/// Remember `family` as the most recently used font (persisted with the preferences).
pub fn note_used(app: &mut PhotocraftApp, family: &str) {
    app.session.prefs.edit(|p| {
        let list = &mut p.type_.recent_font_list;
        list.retain(|f| f != family);
        list.insert(0, family.to_string());
        list.truncate(RECENT_MAX);
    });
}

/// Sample textures by (family, pixel height), with their use order for eviction.
#[derive(Clone, Default)]
struct Samples {
    map: HashMap<(String, u32), Option<TextureHandle>>,
    order: VecDeque<(String, u32)>,
}

impl Samples {
    fn get(&mut self, key: &(String, u32)) -> Option<Option<TextureHandle>> {
        let v = self.map.get(key)?.clone();
        if let Some(i) = self.order.iter().position(|k| k == key) {
            self.order.remove(i);
        }
        self.order.push_back(key.clone());
        Some(v)
    }
    fn insert(&mut self, key: (String, u32), tex: Option<TextureHandle>) {
        while self.order.len() >= CACHE_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, tex);
    }
}

fn samples_id() -> egui::Id {
    egui::Id::new("font-menu-samples")
}

/// "Sample" in `family`, `px` pixels tall (white coverage, tinted when drawn). `None` when this
/// frame's render budget is spent (`budget` counts down) or the face draws nothing.
fn sample(ctx: &egui::Context, family: &str, px: u32, budget: &mut usize) -> Option<TextureHandle> {
    let key = (family.to_string(), px);
    if let Some(t) = ctx.data_mut(|d| d.get_temp_mut_or_default::<Samples>(samples_id()).get(&key)) {
        return t;
    }
    if *budget == 0 {
        ctx.request_repaint();
        return None;
    }
    *budget -= 1;
    let style = photocraft_doc::text::CharStyle { font_family: family.to_string(), ..Default::default() };
    let (w, h, alpha) = {
        let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
        photocraft_text::glyphs::preview_text(&mut eng, &style, SAMPLE, px)
    };
    let tex = alpha.iter().any(|a| *a > 0).then(|| {
        let rgba: Vec<u8> = alpha.iter().flat_map(|a| [255, 255, 255, *a]).collect();
        let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
        ctx.load_texture(format!("font-sample-{family}-{px}"), img, TextureOptions::LINEAR)
    });
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Samples>(samples_id()).insert(key, tex.clone()));
    tex
}

enum Row<'a> {
    Family(&'a String),
    Divider,
}

/// One family row: the name in the interface font, its sample on the right. Returns whether it
/// was clicked.
fn family_row(ui: &mut egui::Ui, family: &str, selected: bool, h: f32, sample_h: Option<f32>, budget: &mut usize) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, family));
    if !ui.is_rect_visible(rect) {
        return false;
    }
    if selected {
        ui.painter().rect_filled(rect, t.radius_sm, t.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, t.radius_sm, t.hover);
    }
    let color = if selected { t.accent_text } else { t.text };
    let name_w = rect.width() * 0.5;
    let mut job = egui::text::LayoutJob::simple_singleline(family.to_string(), egui::TextStyle::Button.resolve(ui.style()), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((name_w - 12.0).max(0.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0), galley, color);
    if let Some(sh) = sample_h {
        let ppp = ui.ctx().pixels_per_point();
        let px = (sh * ppp).round().max(4.0) as u32;
        if let Some(tex) = sample(ui.ctx(), family, px, budget) {
            let size = tex.size_vec2() / ppp;
            let room = rect.right() - 6.0 - (rect.left() + name_w);
            // A wide sample is cut at the row's end, never squeezed.
            let shown_w = size.x.min(room.max(0.0));
            let r = Rect::from_min_size(pos2(rect.left() + name_w, rect.center().y - size.y / 2.0), vec2(shown_w, size.y));
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(if size.x > 0.0 { shown_w / size.x } else { 1.0 }, 1.0));
            ui.painter().image(tex.id(), r, uv, color);
        }
    }
    resp.clicked()
}

/// A searchable font family combo box (Type options bar, Character panel, style options, Glyphs).
/// An empty `current` shows the default family. Returns true when a family was picked, which
/// also becomes the most recently used font.
pub fn picker(app: &mut PhotocraftApp, ui: &mut egui::Ui, salt: &str, current: &mut String, width: f32) -> bool {
    let shown = if current.is_empty() { photocraft_text::fonts::DEFAULT_FAMILY.to_string() } else { current.clone() };
    let installed = crate::type_tool::families();
    let recent = recent(app, installed);
    let sample_h = sample_height(app.session.prefs().type_.font_preview);
    let search_id = egui::Id::new(("font-search", salt));
    let mut picked: Option<String> = None;
    egui::ComboBox::from_id_salt(salt).selected_text(shown.clone()).width(width).height(480.0).icon(crate::widgets::chevron_icon).show_ui(ui, |ui| {
        let mut q: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
        let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text(tl!("Search fonts")).desired_width(f32::INFINITY));
        if !r.has_focus() && q.is_empty() {
            r.request_focus();
        }
        ui.data_mut(|d| d.insert_temp(search_id, q.clone()));
        let ql = q.to_lowercase();
        // Recently used fonts first, above a divider (not while searching), as in Photoshop.
        let mut rows: Vec<Row> = Vec::new();
        if ql.is_empty() && !recent.is_empty() {
            rows.extend(recent.iter().map(Row::Family));
            rows.push(Row::Divider);
        }
        rows.extend(installed.iter().filter(|f| ql.is_empty() || f.to_lowercase().contains(&ql)).map(Row::Family));
        let text_h = ui.text_style_height(&egui::TextStyle::Button);
        let row_h = sample_h.unwrap_or(0.0).max(text_h) + 6.0;
        ui.set_min_width(width.max(320.0));
        let mut budget = RENDERS_PER_FRAME;
        egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show_rows(ui, row_h, rows.len(), |ui, range| {
            for row in rows.get(range).unwrap_or_default() {
                match row {
                    Row::Family(f) => {
                        if family_row(ui, f, **f == shown, row_h, sample_h, &mut budget) {
                            picked = Some((*f).clone());
                        }
                    }
                    Row::Divider => {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                        let t = Tokens::get(ui.ctx());
                        ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.separator));
                    }
                }
            }
        });
        if picked.is_some() {
            ui.data_mut(|d| d.remove::<String>(search_id));
        }
    });
    let Some(f) = picked else { return false };
    note_used(app, &f);
    *current = f;
    true
}

#[cfg(test)]
#[path = "font_menu_tests.rs"]
mod tests;
