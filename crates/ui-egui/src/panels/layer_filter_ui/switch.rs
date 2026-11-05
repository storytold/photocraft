//! Session-only ownership and the Layers filter's switch. Criteria stay in `Filter`.
use std::sync::Weak;

use egui::{Color32, Sense, Stroke, pos2, vec2};

use super::Filter;
use crate::PhotocraftApp;

/// Filter state belongs to one open editing instance, including when its snapshot changes.
#[derive(Clone, Debug)]
pub struct DocumentFilter {
    pub filter: Filter,
    /// Last criteria applied to selection in this open document, not the last active tab.
    applied: Option<Filter>,
    instance: Weak<()>,
}

impl PartialEq for DocumentFilter {
    fn eq(&self, other: &Self) -> bool {
        // Document identity is bookkeeping, separate from the UI filter value.
        self.filter == other.filter
    }
}

impl DocumentFilter {
    fn belongs_to(&self, state: &photocraft_engine::DocState) -> bool {
        self.instance.ptr_eq(&state.open_instance())
    }
}

pub(super) fn current(app: &PhotocraftApp) -> Filter {
    let Some(state) = app.session.active() else { return Filter::default() };
    app.ui.layer_filters.get(&state.doc.id).filter(|saved| saved.belongs_to(state)).map(|saved| saved.filter.clone()).unwrap_or_default()
}

pub(super) fn retain_open(app: &mut PhotocraftApp) {
    let documents = app.session.documents();
    app.ui.layer_filters.retain(|id, saved| documents.iter().any(|state| state.doc.id == *id && saved.belongs_to(state)));
}

pub(super) fn apply(app: &mut PhotocraftApp, filter: Filter) {
    retain_open(app);
    if let Some(state) = app.session.active() {
        app.ui.layer_filters.entry(state.doc.id).and_modify(|saved| saved.filter = filter.clone()).or_insert_with(|| DocumentFilter {
            filter,
            applied: None,
            instance: state.open_instance(),
        });
    }
}

/// Remember selection application in the same lifecycle-owned record as the criteria.
pub(super) fn changed_since_draw(app: &mut PhotocraftApp, filter: &Filter) -> bool {
    apply(app, filter.clone());
    let Some(id) = app.session.active().map(|state| state.doc.id) else { return false };
    let Some(saved) = app.ui.layer_filters.get_mut(&id) else { return false };
    let changed = saved.applied.as_ref() != Some(filter);
    saved.applied = Some(filter.clone());
    changed
}

fn toggle(ui: &mut egui::Ui, on: &mut bool) {
    let label = tl!("Turn layer filtering on or off");
    let (rect, mut response) = ui.allocate_exact_size(vec2(16.0, 22.0), Sense::click());
    let keyboard = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter));
    if response.clicked() || keyboard {
        *on = !*on;
        response.request_focus();
        response.mark_changed();
    }
    ui.painter().rect_filled(rect.shrink(1.0), 7.0, ui.visuals().extreme_bg_color);
    ui.painter().rect_stroke(rect.shrink(1.0), 7.0, Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color), egui::StrokeKind::Inside);
    let centre = pos2(rect.center().x, if *on { rect.top() + 7.0 } else { rect.bottom() - 7.0 });
    ui.painter().circle_filled(centre, 4.5, if *on { Color32::from_rgb(220, 50, 50) } else { Color32::WHITE });
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, label));
    response.on_hover_text(label);
}

pub(super) fn bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    retain_open(app);
    let Some(document) = app.session.active().map(|state| state.doc.id) else { return };
    let mut filter = Filter::of(app);
    let before = filter.clone();
    ui.push_id(("layer-filter", document.0), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = (ui.available_width() - 26.0).max(0.0);
            ui.allocate_ui_with_layout(vec2(width, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add_enabled_ui(filter.on, |ui| super::controls(&mut filter, ui));
            });
            filter.activate_changed(&before);
            ui.add_space(4.0);
            toggle(ui, &mut filter.on);
        });
    });
    if filter != before {
        apply(app, filter);
    }
}
