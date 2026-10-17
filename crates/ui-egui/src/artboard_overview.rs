//! A contact sheet for every artboard in the active document.

use egui::{Color32, TextureHandle};
use photocraft_doc::{DocId, LayerId};

use crate::PhotocraftApp;

#[derive(Default)]
pub(crate) struct Overview {
    pub open: bool,
    doc: Option<(DocId, u64)>,
    cards: Vec<Card>,
    next_preview: usize,
    selected_index: usize,
    diagnostics_board: Option<LayerId>,
    diagnostics_all: bool,
    canvas_diagnostics: Option<((DocId, u64), Vec<CanvasMarker>)>,
}

#[derive(Clone, Copy)]
pub(crate) struct CanvasMarker {
    pub bounds: photocraft_geom::Rect,
    pub kind: MarkerKind,
}

#[derive(Clone, Copy)]
pub(crate) enum MarkerKind {
    Overflow,
    Collision,
    SafeArea,
}

const MAX_CANVAS_MARKERS: usize = 5_000;
const MAX_OVERVIEW_FINDINGS: usize = 5_000;

pub(crate) fn refresh_canvas_diagnostics(app: &mut PhotocraftApp) {
    if !app.ui.extras.responsive_warnings {
        return;
    }
    let Some(state) = app.session.active() else {
        app.artboard_overview.canvas_diagnostics = None;
        return;
    };
    let identity = (state.doc.id, state.revision);
    if app.artboard_overview.canvas_diagnostics.as_ref().is_some_and(|(cached, _)| *cached == identity) {
        return;
    }
    let doc = state.doc.clone();
    let mut markers = Vec::new();
    'boards: for (board, _, _) in doc.artboards() {
        let Ok(diagnostics) = photocraft_engine::artboard_cmds::diagnose_document_artboard(&doc, board, identity.1) else { continue };
        let mut add_layer = |id: LayerId, kind: MarkerKind| {
            if markers.len() >= MAX_CANVAS_MARKERS {
                return;
            }
            if let Some(bounds) = doc.layer(id).and_then(photocraft_engine::layer_multi_cmds::layer_bounds) {
                markers.push(CanvasMarker { bounds, kind });
            }
        };
        if let Some(overflow) = diagnostics.get("overflow").and_then(serde_json::Value::as_array) {
            for item in overflow {
                if let Some(id) = item.get("layer").and_then(serde_json::Value::as_u64).map(LayerId) {
                    add_layer(id, MarkerKind::Overflow);
                }
            }
        }
        if let Some(collisions) = diagnostics.get("collisions").and_then(serde_json::Value::as_array) {
            for item in collisions {
                for id in ["first", "second"].into_iter().filter_map(|key| item.get(key).and_then(serde_json::Value::as_u64).map(LayerId)) {
                    add_layer(id, MarkerKind::Collision);
                }
            }
        }
        if let Some(areas) = diagnostics.get("safeAreas").and_then(serde_json::Value::as_array) {
            for item in areas.iter().flat_map(|area| area.get("violations").and_then(serde_json::Value::as_array).into_iter().flatten()) {
                if let Some(id) = item.get("layer").and_then(serde_json::Value::as_u64).map(LayerId) {
                    add_layer(id, MarkerKind::SafeArea);
                }
            }
        }
        if markers.len() >= MAX_CANVAS_MARKERS {
            break 'boards;
        }
    }
    app.artboard_overview.canvas_diagnostics = Some((identity, markers));
}

pub(crate) fn canvas_markers(overview: &Overview, doc: DocId, revision: u64) -> &[CanvasMarker] {
    overview
        .canvas_diagnostics
        .as_ref()
        .filter(|((cached_doc, cached_revision), _)| *cached_doc == doc && *cached_revision == revision)
        .map_or(&[], |(_, markers)| markers)
}

struct Card {
    id: LayerId,
    name: String,
    width: u32,
    height: u32,
    preview: Option<TextureHandle>,
    overflow_count: usize,
    collision_count: usize,
    safe_area_count: usize,
    findings: Vec<(LayerId, FindingKind)>,
    findings_truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FindingKind {
    Overflow,
    Collision,
    SafeArea,
}

fn navigate(current: usize, last: usize, columns: usize, key: egui::Key) -> usize {
    match key {
        egui::Key::ArrowLeft => current.saturating_sub(1),
        egui::Key::ArrowRight => current.saturating_add(1).min(last),
        egui::Key::ArrowUp => current.saturating_sub(columns.max(1)),
        egui::Key::ArrowDown => current.saturating_add(columns.max(1)).min(last),
        _ => current.min(last),
    }
}

fn all_findings(cards: &[Card]) -> (Vec<(String, LayerId, FindingKind)>, bool) {
    let mut findings = Vec::new();
    let mut truncated = false;
    for card in cards {
        truncated |= card.findings_truncated;
        for (layer, kind) in &card.findings {
            if findings.len() >= MAX_OVERVIEW_FINDINGS {
                truncated = true;
                return (findings, truncated);
            }
            findings.push((card.name.clone(), *layer, *kind));
        }
    }
    (findings, truncated)
}

fn finding_filter_chips(ui: &mut egui::Ui, filter: &mut usize) {
    ui.horizontal_wrapped(|ui| {
        for (value, label) in [(0, "All findings"), (1, "Outside artboard"), (2, "Semantic collision"), (3, "Safe-area violation")] {
            ui.selectable_value(filter, value, crate::i18n::t(label));
        }
    });
}

fn rebuild(ctx: &egui::Context, overview: &mut Overview, doc: &photocraft_doc::Document, revision: u64) {
    let identity = (doc.id, revision);
    if overview.doc != Some(identity) {
        let mut cards = Vec::new();
        for (id, name, artboard) in doc.artboards() {
            let (width, height) = (artboard.rect.width(), artboard.rect.height());
            let diagnostics = photocraft_engine::artboard_cmds::diagnose_document_artboard(doc, id, revision).ok();
            let count = |key: &str| diagnostics.as_ref().and_then(|value| value.get(key)).and_then(serde_json::Value::as_array).map_or(0, Vec::len);
            let safe_area_count = diagnostics
                .as_ref()
                .and_then(|value| value.get("safeAreas"))
                .and_then(serde_json::Value::as_array)
                .map_or(0, |areas| areas.iter().map(|area| area.get("violations").and_then(serde_json::Value::as_array).map_or(0, Vec::len)).sum());
            let mut findings = Vec::new();
            let mut findings_truncated = diagnostics.as_ref().and_then(|value| value.get("truncated")).and_then(serde_json::Value::as_bool).unwrap_or(false);
            if let Some(items) = diagnostics.as_ref().and_then(|value| value.get("overflow")).and_then(serde_json::Value::as_array) {
                for item in items {
                    if let Some(layer) = item.get("layer").and_then(serde_json::Value::as_u64).map(LayerId) {
                        if findings.len() < 500 {
                            findings.push((layer, FindingKind::Overflow));
                        } else {
                            findings_truncated = true;
                            break;
                        }
                    }
                }
            }
            if let Some(items) = diagnostics.as_ref().and_then(|value| value.get("collisions")).and_then(serde_json::Value::as_array) {
                for item in items {
                    for layer in ["first", "second"].into_iter().filter_map(|key| item.get(key).and_then(serde_json::Value::as_u64).map(LayerId)) {
                        if findings.len() >= 500 {
                            findings_truncated = true;
                            break;
                        }
                        findings.push((layer, FindingKind::Collision));
                    }
                    if findings.len() >= 500 {
                        break;
                    }
                }
            }
            if let Some(areas) = diagnostics.as_ref().and_then(|value| value.get("safeAreas")).and_then(serde_json::Value::as_array) {
                for item in areas.iter().flat_map(|area| area.get("violations").and_then(serde_json::Value::as_array).into_iter().flatten()) {
                    if let Some(layer) = item.get("layer").and_then(serde_json::Value::as_u64).map(LayerId) {
                        if findings.len() < 500 {
                            findings.push((layer, FindingKind::SafeArea));
                        } else {
                            findings_truncated = true;
                            break;
                        }
                    }
                }
            }
            cards.push(Card {
                id,
                name: name.to_owned(),
                width,
                height,
                preview: None,
                overflow_count: count("overflow"),
                collision_count: count("collisions"),
                safe_area_count,
                findings,
                findings_truncated,
            });
        }
        overview.doc = Some(identity);
        overview.cards = cards;
        overview.next_preview = 0;
    }
    // Render at most one thumbnail per frame. Opening a document with many large artboards
    // therefore keeps input and scrolling responsive instead of synchronously rendering all.
    if let Some(card) = overview.cards.get_mut(overview.next_preview) {
        let id = card.id;
        let width = card.width;
        let height = card.height;
        card.preview = photocraft_engine::artboard_cmds::artboard_document(doc, id).ok().flatten().map(|one| {
            let scale = (280.0 / width as f32).min(220.0 / height as f32).min(1.0);
            let (w, h) = ((width as f32 * scale).round().max(1.0) as u32, (height as f32 * scale).round().max(1.0) as u32);
            let buffer = photocraft_compose::render_reduced(&one, w, h);
            let pixels = buffer
                .px
                .iter()
                .map(|p| {
                    Color32::from_rgba_unmultiplied(
                        (p[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (p[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (p[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (p[3].clamp(0.0, 1.0) * 255.0).round() as u8,
                    )
                })
                .collect();
            let image = egui::ColorImage::new([w as usize, h as usize], pixels);
            ctx.load_texture(format!("artboard-overview-{}-{}", doc.id.0, id.0), image, egui::TextureOptions::LINEAR)
        });
        overview.next_preview += 1;
        if overview.next_preview < overview.cards.len() {
            ctx.request_repaint();
        }
    }
}

pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.artboard_overview.open {
        return;
    }
    let Some(st) = app.session.active() else {
        app.artboard_overview.open = false;
        return;
    };
    let (doc, revision) = (st.doc.clone(), st.revision);
    rebuild(ctx, &mut app.artboard_overview, &doc, revision);

    let mut selected = None;
    let mut zoom_to_selected = false;
    let mut selected_index = app.artboard_overview.selected_index;
    egui::Window::new(crate::i18n::t("Artboard Overview · All formats"))
        .open(&mut app.artboard_overview.open)
        .default_size([1240.0, 850.0])
        .resizable(true)
        .show(ctx, |ui| {
            ui.label(format!("{} · {} {}", doc.name, app.artboard_overview.cards.len(), crate::i18n::t("artboards")));
            let finding_count: usize = app.artboard_overview.cards.iter().map(|card| card.findings.len()).sum();
            if finding_count > 0 && ui.small_button(format!("{} ({finding_count})", crate::i18n::t("Review all findings"))).clicked() {
                app.artboard_overview.diagnostics_board = None;
                app.artboard_overview.diagnostics_all = true;
            }
            ui.separator();
            ui.label(crate::i18n::t("Arrow keys navigate; Enter selects and zooms"));
            if app.artboard_overview.cards.is_empty() {
                ui.label(crate::i18n::t("This document has no artboards yet."));
                return;
            }
            if app.artboard_overview.next_preview < app.artboard_overview.cards.len() {
                let total = app.artboard_overview.cards.len();
                ui.add(
                    egui::ProgressBar::new(app.artboard_overview.next_preview as f32 / total as f32)
                        .text(format!("{} / {total}", app.artboard_overview.next_preview)),
                );
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                let columns = (ui.available_width() / 300.0).floor().clamp(1.0, 4.0) as usize;
                let last = app.artboard_overview.cards.len().saturating_sub(1);
                selected_index = selected_index.min(last);
                let mut keyboard_moved = false;
                if !ui.ctx().egui_wants_keyboard_input()
                    && let Some(key) = ui.input(|input| {
                        [egui::Key::ArrowLeft, egui::Key::ArrowRight, egui::Key::ArrowUp, egui::Key::ArrowDown].into_iter().find(|key| input.key_pressed(*key))
                    })
                {
                    selected_index = navigate(selected_index, last, columns, key);
                    keyboard_moved = true;
                }
                if !ui.ctx().egui_wants_keyboard_input()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    && let Some(card) = app.artboard_overview.cards.get(selected_index)
                {
                    selected = Some(card.id);
                    zoom_to_selected = true;
                }
                egui::Grid::new("artboard-overview-grid").num_columns(columns).spacing([12.0, 14.0]).show(ui, |ui| {
                    for (index, card) in app.artboard_overview.cards.iter().enumerate() {
                        let card_response = egui::Frame::group(ui.style())
                            .stroke(if index == selected_index {
                                egui::Stroke::new(2.0, ui.visuals().selection.stroke.color)
                            } else {
                                ui.visuals().widgets.noninteractive.bg_stroke
                            })
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(280.0);
                                    let available = egui::vec2(268.0, 220.0);
                                    ui.add_space(6.0);
                                    if let Some(preview) = &card.preview {
                                        let source = preview.size_vec2();
                                        let size = source * (available.x / source.x).min(available.y / source.y).min(1.0);
                                        let response = ui.add_sized(size, egui::Image::new(preview).fit_to_exact_size(size));
                                        if response.double_clicked() {
                                            selected = Some(card.id);
                                            zoom_to_selected = true;
                                        }
                                    } else {
                                        ui.allocate_ui_with_layout(available, egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                                            ui.label(crate::i18n::t("Preview unavailable"));
                                        });
                                    }
                                    ui.strong(&card.name);
                                    ui.label(format!("{} × {} px", card.width, card.height));
                                    if card.overflow_count + card.collision_count + card.safe_area_count > 0 {
                                        let response = ui.label(format!("⚠ {} · ◇ {} · ⌂ {}", card.overflow_count, card.collision_count, card.safe_area_count));
                                        response.on_hover_text(format!(
                                            "{}: {} · {}: {} · {}: {}",
                                            crate::i18n::t("Overflow"),
                                            card.overflow_count,
                                            crate::i18n::t("Collisions"),
                                            card.collision_count,
                                            crate::i18n::t("Safe-area violations"),
                                            card.safe_area_count
                                        ));
                                    } else {
                                        ui.label("✓").on_hover_text(crate::i18n::t("No layout issues"));
                                    }
                                    if !card.findings.is_empty()
                                        && ui.small_button(format!("{} ({})", crate::i18n::t("Review findings"), card.findings.len())).clicked()
                                    {
                                        app.artboard_overview.diagnostics_board = Some(card.id);
                                    }
                                    if ui.button(crate::i18n::t("Select on canvas")).clicked() {
                                        selected = Some(card.id);
                                        selected_index = index;
                                    }
                                });
                            });
                        if keyboard_moved && index == selected_index {
                            ui.scroll_to_rect(card_response.response.rect, Some(egui::Align::Center));
                        }
                        if (index + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }
                });
            });
        });
    app.artboard_overview.selected_index = selected_index;
    if app.artboard_overview.diagnostics_all {
        let mut open = true;
        let mut filter = usize::from(app.ui.responsive_findings_filter.min(3));
        let mut jump_to = None;
        let (all_findings, findings_truncated) = all_findings(&app.artboard_overview.cards);
        egui::Window::new(crate::i18n::t("Review all findings")).open(&mut open).default_size([600.0, 520.0]).show(ctx, |ui| {
            finding_filter_chips(ui, &mut filter);
            if findings_truncated {
                ui.label(crate::i18n::t("Results are capped; some findings are omitted"));
            }
            let visible: Vec<_> = all_findings
                .iter()
                .filter(|(_, _, kind)| {
                    filter == 0 || matches!((filter, kind), (1, FindingKind::Overflow) | (2, FindingKind::Collision) | (3, FindingKind::SafeArea))
                })
                .collect();
            if visible.is_empty() {
                ui.label(crate::i18n::t("No findings to review"));
            }
            egui::ScrollArea::vertical().show_rows(ui, 22.0, visible.len(), |ui, rows| {
                for index in rows {
                    if let Some((board_name, layer, kind)) = visible.get(index) {
                        let finding = crate::i18n::t(match kind {
                            FindingKind::Overflow => "Outside artboard",
                            FindingKind::Collision => "Semantic collision",
                            FindingKind::SafeArea => "Safe-area violation",
                        });
                        let layer_name = doc.layer(*layer).map_or_else(|| format!("Layer {}", layer.0), |layer| layer.name.clone());
                        if ui.selectable_label(false, format!("⚠ {finding}: {layer_name} · {board_name}")).clicked() {
                            jump_to = Some(*layer);
                        }
                    }
                }
            });
        });
        app.ui.responsive_findings_filter = filter as u8;
        if !open || jump_to.is_some() {
            app.artboard_overview.diagnostics_all = false;
        }
        if let Some(layer) = jump_to {
            selected = Some(layer);
        }
    }
    if let Some(board) = app.artboard_overview.diagnostics_board {
        let Some(card) = app.artboard_overview.cards.iter().find(|card| card.id == board) else {
            app.artboard_overview.diagnostics_board = None;
            return;
        };
        let card_name = card.name.clone();
        let findings = card.findings.clone();
        let findings_truncated = card.findings_truncated;
        let mut filter = usize::from(app.ui.responsive_findings_filter.min(3));
        let mut open = true;
        let mut jump_to = None;
        egui::Window::new(format!("{} · {}", crate::i18n::t("Review findings"), card_name)).open(&mut open).default_size([520.0, 480.0]).show(ctx, |ui| {
            finding_filter_chips(ui, &mut filter);
            if findings_truncated {
                ui.label(crate::i18n::t("Results are capped; some findings are omitted"));
            }
            let visible: Vec<_> = findings
                .iter()
                .filter(|(_, kind)| {
                    filter == 0 || matches!((filter, kind), (1, FindingKind::Overflow) | (2, FindingKind::Collision) | (3, FindingKind::SafeArea))
                })
                .collect();
            if visible.is_empty() {
                ui.label(crate::i18n::t("No findings to review"));
            }
            egui::ScrollArea::vertical().show_rows(ui, 22.0, visible.len(), |ui, rows| {
                for index in rows {
                    if let Some((layer, kind)) = visible.get(index) {
                        let finding = crate::i18n::t(match kind {
                            FindingKind::Overflow => "Outside artboard",
                            FindingKind::Collision => "Semantic collision",
                            FindingKind::SafeArea => "Safe-area violation",
                        });
                        let name = doc.layer(*layer).map_or_else(|| format!("Layer {}", layer.0), |layer| layer.name.clone());
                        if ui.selectable_label(false, format!("⚠ {finding}: {name}")).clicked() {
                            jump_to = Some(*layer);
                        }
                    }
                }
            });
        });
        app.ui.responsive_findings_filter = filter as u8;
        if !open || jump_to.is_some() {
            app.artboard_overview.diagnostics_board = None;
        }
        if let Some(layer) = jump_to {
            selected = Some(layer);
        }
    }
    if let Some(id) = selected
        && let Err(error) = app.run("layer.select", serde_json::json!({"layer": id.0}))
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    if zoom_to_selected && let Err(error) = crate::artboard_ui::fit_artboard(app) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn overview_keyboard_navigation_moves_by_grid_and_stays_in_bounds() {
        assert_eq!(navigate(4, 8, 3, egui::Key::ArrowUp), 1);
        assert_eq!(navigate(1, 8, 3, egui::Key::ArrowDown), 4);
        assert_eq!(navigate(0, 8, 3, egui::Key::ArrowLeft), 0);
        assert_eq!(navigate(8, 8, 3, egui::Key::ArrowRight), 8);
        assert_eq!(navigate(2, 2, 0, egui::Key::ArrowDown), 2);
    }

    #[test]
    fn all_findings_keeps_each_board_and_caps_the_global_list() {
        let make_card = |id: u64, name: &str, findings: Vec<(LayerId, FindingKind)>| Card {
            id: LayerId(id),
            name: name.to_owned(),
            width: 10,
            height: 10,
            preview: None,
            overflow_count: findings.len(),
            collision_count: 0,
            safe_area_count: 0,
            findings,
            findings_truncated: false,
        };
        let cards =
            vec![make_card(1, "Portrait", vec![(LayerId(11), FindingKind::Overflow)]), make_card(2, "Landscape", vec![(LayerId(22), FindingKind::SafeArea)])];
        let (items, truncated) = all_findings(&cards);
        assert_eq!(items, vec![("Portrait".to_owned(), LayerId(11), FindingKind::Overflow), ("Landscape".to_owned(), LayerId(22), FindingKind::SafeArea)]);
        assert!(!truncated);

        let oversized = make_card(3, "Many", vec![(LayerId(33), FindingKind::Collision); MAX_OVERVIEW_FINDINGS + 1]);
        let (items, truncated) = all_findings(&[oversized]);
        assert_eq!(items.len(), MAX_OVERVIEW_FINDINGS);
        assert!(truncated);
    }

    #[test]
    fn overview_renders_each_artboard_once_per_document_revision() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("shape.create", json!({"kind":"rect", "rect":[0,0,24,36], "fill":"#3568c0"})).unwrap();
        app.run("layer.new.artboardFromLayers", json!({"name":"Portrait"})).unwrap();
        app.run("layer.new.artboard", json!({"rect":[80,0,36,24], "name":"Landscape"})).unwrap();
        let ctx = egui::Context::default();
        let state = app.session.active().unwrap();
        let doc = state.doc.clone();
        let revision = state.revision;

        rebuild(&ctx, &mut app.artboard_overview, &doc, revision);
        assert_eq!(app.artboard_overview.cards.len(), 2);
        rebuild(&ctx, &mut app.artboard_overview, &doc, revision);
        let texture_ids: Vec<_> = app.artboard_overview.cards.iter().map(|card| card.preview.as_ref().map(TextureHandle::id)).collect();
        rebuild(&ctx, &mut app.artboard_overview, &doc, revision);
        assert_eq!(app.artboard_overview.cards.iter().map(|card| card.preview.as_ref().map(TextureHandle::id)).collect::<Vec<_>>(), texture_ids);
        let preview = app.artboard_overview.cards[0].preview.as_ref().unwrap();
        assert!(preview.size()[0] > 0);
        assert!(preview.size()[0] <= 280);
    }

    #[test]
    fn canvas_diagnostics_cache_updates_only_after_a_document_revision() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        let board = app.run("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap()["layer"].as_u64().unwrap();
        let layer = app.run("type.create", json!({"box":[90,10,130,30],"text":"overflow","size":16})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.moveTo", json!({"layer":layer,"target":board,"position":"into"})).unwrap();
        let ctx = egui::Context::default();
        assert!(crate::menus::invoke(&mut app, &ctx, "view.show.responsiveWarnings", serde_json::json!({})).is_ok());
        assert!(app.ui.extras.responsive_warnings);
        refresh_canvas_diagnostics(&mut app);
        let state = app.session.active().unwrap();
        let markers = canvas_markers(&app.artboard_overview, state.doc.id, state.revision);
        assert!(markers.iter().any(|marker| matches!(marker.kind, MarkerKind::Overflow)));
        let count = markers.len();
        refresh_canvas_diagnostics(&mut app);
        let state = app.session.active().unwrap();
        assert_eq!(canvas_markers(&app.artboard_overview, state.doc.id, state.revision).len(), count);
    }

    #[test]
    fn findings_include_both_layers_in_a_semantic_collision() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        let board = app.run("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap()["layer"].as_u64().unwrap();
        let first = app.run("shape.create", json!({"kind":"rect", "rect":[10,10,50,30], "fill":"#3568c0"})).unwrap()["layer"].as_u64().unwrap();
        let second = app.run("shape.create", json!({"kind":"rect", "rect":[20,20,50,30], "fill":"#c03568"})).unwrap()["layer"].as_u64().unwrap();
        for layer in [first, second] {
            app.run("layer.moveTo", json!({"layer":layer,"target":board,"position":"into"})).unwrap();
        }
        app.run("layer.artboard.semanticRole.set", json!({"layer":first,"role":"headline"})).unwrap();
        app.run("layer.artboard.semanticRole.set", json!({"layer":second,"role":"cta"})).unwrap();
        let state = app.session.active().unwrap();
        let doc = state.doc.clone();
        rebuild(&egui::Context::default(), &mut app.artboard_overview, &doc, state.revision);
        let card = app.artboard_overview.cards.iter().find(|card| card.id.0 == board).unwrap();
        let collision_layers: Vec<_> = card.findings.iter().filter(|(_, kind)| *kind == FindingKind::Collision).map(|(layer, _)| layer.0).collect();
        assert!(collision_layers.contains(&first));
        assert!(collision_layers.contains(&second));
    }
}
