//! The model picker (ShotoPhopEX, ArtCraft-style): a pill showing the chosen template's family
//! glyph and short name; its popover lists the templates a page offers, Auto first, newest first,
//! grouped by family into submenus once there are many, each row with the glyph, the name, the
//! one-line description and badges (the time estimate, `research` for research-licence models,
//! which stay greyed until Preferences allows them, and `not installed` when the server lacks a
//! file: still selectable, the engine's error then says what is missing).
//!
//! One component for the Generate panel, the task bar and the generated dialogs (`template`).
//! The data is plain [`Item`]s built by each caller from `generate.models`, so the picker itself
//! draws and decides nothing about the server.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::theme::Tokens;

/// Families are grouped into submenus from this many items on (with the preference on).
pub const GROUP_FROM: usize = 8;

/// One template the picker can offer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub id: String,
    /// The template's full name ("Generative Fill (Qwen-Image-Edit-2511, Lightning 8 steps)").
    pub name: String,
    pub family: String,
    pub description: String,
    /// A research-only licence.
    pub research: bool,
    /// Selectable (research models only with Preferences › Allow research-only models).
    pub allowed: bool,
    /// The server has every file (None = unknown).
    pub installed: Option<bool>,
    /// Seconds for the current request, when known.
    pub eta: Option<f32>,
}

/// How a family looks: its display name, glyph colour and initial.
pub struct FamilyStyle {
    pub name: &'static str,
    pub color: Option<Color32>,
    pub initial: char,
}

/// The glyph and display name of a family (`None` colour = the theme's dim text).
pub fn family_style(family: &str) -> FamilyStyle {
    let hex = |v: u32| Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
    match family {
        "qwen-image-edit-2511" => FamilyStyle { name: "Qwen Image Edit", color: hex(0x7c3aed), initial: 'Q' },
        "qwen-image-2.1" => FamilyStyle { name: "Qwen Image 2.1", color: hex(0x9333ea), initial: 'Q' },
        "krea2" => FamilyStyle { name: "Krea 2", color: hex(0xf97316), initial: 'K' },
        "sam3.1" => FamilyStyle { name: "SAM 3.1", color: hex(0x10b981), initial: 'S' },
        "qwen-image-layered" => FamilyStyle { name: "Qwen Image Layered", color: hex(0xeab308), initial: 'L' },
        "qwen3-vl" => FamilyStyle { name: "Qwen3-VL", color: hex(0x64748b), initial: 'V' },
        "moss-image" => FamilyStyle { name: "MOSS-Image", color: hex(0x0d9488), initial: 'M' },
        f if f.starts_with("flux") => FamilyStyle { name: "FLUX", color: hex(0x0ea5e9), initial: 'F' },
        _ => FamilyStyle { name: "Other", color: None, initial: '•' },
    }
}

/// The part of a template name a row needs: what is inside its last parentheses
/// ("Qwen-Image-Edit-2511, Lightning 8 steps"), else the name.
pub fn short_name(name: &str) -> &str {
    match (name.rfind('('), name.rfind(')')) {
        (Some(a), Some(b)) if b > a + 1 => name[a + 1..b].trim(),
        _ => name,
    }
}

/// The names rows show: the short name, or the full name where two items' short names collide
/// ("Generate Image (Krea 2 Turbo)" and "Generate Image with a Style Reference (Krea 2 Turbo)"
/// both shorten to "Krea 2 Turbo").
pub fn display_names(items: &[Item]) -> Vec<String> {
    items
        .iter()
        .map(|it| {
            let s = short_name(&it.name);
            if items.iter().filter(|o| short_name(&o.name) == s).count() > 1 { it.name.clone() } else { s.to_string() }
        })
        .collect()
}

/// Compare names so the higher version sorts first: digit runs compare as numbers
/// ("Krea 10" before "Krea 2" before "Krea 1").
pub fn newest_first(a: &str, b: &str) -> std::cmp::Ordering {
    natural(b, a)
}

fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (n, m) = (take(&mut x), take(&mut y));
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_ascii_lowercase().cmp(&d.to_ascii_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// One line of the popover.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Item(usize),
    /// A family's submenu: its display name and its items (indices, newest first).
    Family(String, Vec<usize>),
}

/// The popover's lines for `items`: one per item, or (with `group` and at least
/// [`GROUP_FROM`] items) one per family; families in first-seen order, items newest first.
pub fn layout(items: &[Item], group: bool) -> Vec<Entry> {
    let mut families: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        match families.iter_mut().find(|(f, _)| *f == it.family) {
            Some((_, v)) => v.push(i),
            None => families.push((it.family.clone(), vec![i])),
        }
    }
    for (_, v) in &mut families {
        v.sort_by(|a, b| newest_first(&items[*a].name, &items[*b].name));
    }
    if group && items.len() >= GROUP_FROM {
        families.into_iter().map(|(f, v)| if v.len() > 1 { Entry::Family(family_style(&f).name.to_string(), v) } else { Entry::Item(v[0]) }).collect()
    } else {
        families.into_iter().flat_map(|(_, v)| v.into_iter().map(Entry::Item)).collect()
    }
}

/// Can the row be chosen?
pub fn selectable(it: &Item) -> bool {
    it.allowed
}

/// A row's badges, in order: the estimate, `research`, `not installed`.
pub fn badges(it: &Item) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(e) = it.eta.map(crate::generate_panel::eta_label).filter(|s| !s.is_empty()) {
        out.push(e);
    }
    if it.research {
        out.push(tl!("research").to_string());
    }
    if it.installed == Some(false) {
        out.push(tl!("not installed").to_string());
    }
    out
}

/// A family glyph: a rounded square in the family's colour with its initial.
pub fn glyph(ui: &Ui, rect: Rect, family: &str, t: &Tokens) {
    let s = family_style(family);
    let fill = s.color.unwrap_or(t.text_dim);
    ui.painter().rect_filled(rect, 3.0, fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, s.initial, FontId::proportional(rect.height() * 0.7), Color32::WHITE);
}

/// The pill and its popover. `current` is a template id, or empty for Auto; `auto` is the short
/// name of the model a run without a choice uses (the preference's default, else what `auto`
/// picks), shown on the Auto row. Under 60 pt wide the pill is the glyph alone (one-row forms).
/// Returns true when the choice changed.
pub fn pill(ui: &mut Ui, id_salt: &str, items: &[Item], current: &mut String, auto: Option<&str>, group: bool, width: f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let names = display_names(items);
    let name_of = |i: usize| names.get(i).cloned().unwrap_or_default();
    let chosen_at = items.iter().position(|i| i.id == *current);
    let chosen = chosen_at.and_then(|i| items.get(i));
    let label = chosen_at.map_or_else(|| tl!("Auto").to_string(), name_of);
    let h = 24.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    let frame = ui.ctx().cumulative_frame_nr();
    ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Drawn>(egui::Id::unique("model-pickers")).insert(id_salt.to_string(), (rect, frame, label.clone())));
    let p = ui.painter();
    p.rect_filled(rect, h / 2.0, if resp.hovered() { t.hover } else { t.field });
    p.rect_stroke(rect, h / 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    let glyph_only = width < 60.0;
    let g = if glyph_only {
        Rect::from_center_size(rect.center(), vec2(14.0, 14.0))
    } else {
        Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - 7.0), vec2(14.0, 14.0))
    };
    match chosen {
        Some(c) => glyph(ui, g, &c.family, &t),
        None => crate::icons::paint(ui, g, "sparkles", 12.0, t.accent),
    }
    if !glyph_only {
        let text_w = (rect.width() - 40.0).max(10.0);
        let galley = ui.painter().layout(label.clone(), FontId::proportional(12.0), t.text, text_w);
        let row = galley.rows.first().map(|r| r.text()).unwrap_or_default().to_string();
        let shown = if galley.rows.len() > 1 { format!("{}…", row.trim_end()) } else { label.clone() };
        ui.painter().text(pos2(g.right() + 6.0, rect.center().y), Align2::LEFT_CENTER, shown, FontId::proportional(12.0), t.text);
        crate::icons::paint(ui, Rect::from_min_size(pos2(rect.right() - 18.0, rect.center().y - 6.0), vec2(12.0, 12.0)), "chevron-down", 10.0, t.text_dim);
    }
    let spoken = format!("{}: {label}", tl!("Model"));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), &spoken));
    let resp = resp.on_hover_text(chosen.map_or_else(|| auto_description(auto), |c| c.name.clone()));
    let mut changed = false;
    egui::Popup::menu(&resp).id(popup_id(id_salt)).show(|ui| {
        ui.set_min_width(300.0);
        // Auto first.
        if row_ui(ui, None, "", auto, current.is_empty(), &t).clicked() && !current.is_empty() {
            current.clear();
            changed = true;
        }
        ui.separator();
        for entry in layout(items, group) {
            match entry {
                Entry::Item(i) => {
                    let Some(it) = items.get(i) else { continue };
                    if row_ui(ui, Some(it), &name_of(i), None, *current == it.id, &t).clicked() && selectable(it) && *current != it.id {
                        *current = it.id.clone();
                        changed = true;
                    }
                }
                Entry::Family(name, members) => {
                    let selected = members.iter().copied().find(|i| items.get(*i).is_some_and(|it| it.id == *current)).map(name_of);
                    let title = match &selected {
                        Some(s) => format!("{name} · {s}"),
                        None => name.clone(),
                    };
                    ui.menu_button(title, |ui| {
                        ui.set_min_width(300.0);
                        for i in members {
                            let Some(it) = items.get(i) else { continue };
                            if row_ui(ui, Some(it), &name_of(i), None, *current == it.id, &t).clicked() && selectable(it) && *current != it.id {
                                *current = it.id.clone();
                                changed = true;
                            }
                        }
                    });
                }
            }
        }
    });
    changed
}

/// The pickers drawn so far: id salt → (rect, frame, label).
type Drawn = std::collections::BTreeMap<String, (Rect, u64, String)>;

/// `ui.inspect.modelPickers`: the pickers drawn in the last two frames, with their rect (points),
/// label and whether their popover is open (scripts click them with `ui.click`).
pub fn inspect(ctx: &egui::Context) -> serde_json::Value {
    let now = ctx.cumulative_frame_nr();
    let drawn: Drawn = ctx.data(|d| d.get_temp(egui::Id::unique("model-pickers"))).unwrap_or_default();
    drawn
        .into_iter()
        .filter(|(_, (_, f, _))| now.saturating_sub(*f) <= 2)
        .map(|(id, (r, _, label))| serde_json::json!({"id": id, "rect": [r.left(), r.top(), r.width(), r.height()], "label": label, "open": is_open(ctx, &id)}))
        .collect()
}

fn popup_id(id_salt: &str) -> egui::Id {
    egui::Id::unique(("model-picker", id_salt))
}

/// Is the popover of the picker drawn with `id_salt` open? (Callers ask the server for the
/// `not installed` badges then.)
pub fn is_open(ctx: &egui::Context, id_salt: &str) -> bool {
    egui::Popup::is_id_open(ctx, popup_id(id_salt))
}

/// What the Auto row says: the model it resolves to, when known.
pub fn auto_description(auto: Option<&str>) -> String {
    match auto.filter(|a| !a.is_empty()) {
        Some(a) => crate::i18n::fmt(tl!("Uses: {name}"), &[("name", a)]),
        None => tl!("The fastest permissive model the server has").to_string(),
    }
}

/// One popover row showing `name` (`None` = Auto, described by `auto`).
fn row_ui(ui: &mut Ui, it: Option<&Item>, name: &str, auto: Option<&str>, checked: bool, t: &Tokens) -> egui::Response {
    let w = ui.available_width().max(280.0);
    let enabled = it.is_none_or(selectable);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 40.0), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    if resp.hovered() && enabled {
        p.rect_filled(rect, 4.0, t.hover);
    }
    let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let g = Rect::from_min_size(pos2(rect.left() + 6.0, rect.top() + 7.0), vec2(14.0, 14.0));
    match it {
        Some(i) => glyph(ui, g, &i.family, t),
        None => crate::icons::paint(ui, g, "sparkles", 12.0, t.accent),
    }
    let (name, desc) = match it {
        Some(i) => (name.to_string(), i.description.clone()),
        None => (tl!("Auto").to_string(), auto_description(auto)),
    };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, checked, &name));
    p.text(pos2(g.right() + 8.0, rect.top() + 13.0), Align2::LEFT_CENTER, name, FontId::proportional(12.5), dim(t.text));
    p.text(pos2(g.right() + 8.0, rect.top() + 29.0), Align2::LEFT_CENTER, desc, FontId::proportional(10.5), dim(t.text_faint));
    // Badges from the right edge; the check left of them.
    let mut x = rect.right() - 6.0;
    if let Some(i) = it {
        for b in badges(i).into_iter().rev() {
            let color = if b == tl!("research") {
                t.warning
            } else if b == tl!("not installed") {
                t.danger
            } else {
                t.text_dim
            };
            let galley = ui.painter().layout_no_wrap(b, FontId::proportional(10.0), color);
            let r = Rect::from_min_size(pos2(x - galley.size().x - 10.0, rect.top() + 6.0), vec2(galley.size().x + 10.0, 15.0));
            ui.painter().rect_stroke(r, 7.0, Stroke::new(1.0, color.gamma_multiply(0.6)), egui::StrokeKind::Inside);
            ui.painter().galley(pos2(r.left() + 5.0, r.center().y - galley.size().y / 2.0), galley, color);
            x = r.left() - 4.0;
        }
    }
    if checked {
        crate::icons::paint(ui, Rect::from_center_size(pos2(x - 8.0, rect.top() + 13.0), vec2(14.0, 14.0)), "check", 12.0, t.accent);
    }
    let tip = match it {
        Some(i) if !selectable(i) => format!("{}\n{}", i.name, tl!("Research licence: turn on Allow research-only models in Preferences › AI Integrations")),
        Some(i) => i.name.clone(),
        None => String::new(),
    };
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

#[cfg(test)]
#[path = "model_picker_tests.rs"]
mod tests;
