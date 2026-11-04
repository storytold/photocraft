//! Shared recently-picked strip for every asset picker using the standard shell.

use crate::pickers::asset_browser::{BrowserAction, BrowserAdapter, GroupView, ItemPaintState};
use crate::pickers::picker_state::{PresetUi, RecentPickerItem};
use egui::{Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

pub(crate) const HEIGHT: f32 = 18.0;
const CARD_SIZE: f32 = 18.0;
const CARD_INSET: f32 = 1.0;
const END_DROP_HEIGHT: f32 = 3.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Options {
    pub(crate) allow_arranging: bool,
}

impl Options {
    pub(crate) fn for_panel(panel: &str) -> Self {
        Self { allow_arranging: panel != "swatches" }
    }
}

fn insertion_indicator_x(strip: Rect, remaining: &[Rect], insertion: usize) -> f32 {
    remaining.get(insertion).map_or_else(|| remaining.last().map_or(strip.left(), |rect| rect.right()), |rect| rect.left())
}

fn released_on_origin(origin: Rect, release: Option<egui::Pos2>) -> bool {
    release.is_some_and(|position| origin.contains(position))
}

fn insertion_index(strip: Rect, remaining: &[Rect], pointer: egui::Pos2) -> usize {
    if pointer.y >= strip.bottom() - END_DROP_HEIGHT { remaining.len() } else { remaining.iter().take_while(|rect| pointer.x > rect.center().x).count() }
}

fn reconcile_recent_items(recent: &mut Vec<RecentPickerItem>, groups: &[GroupView]) {
    recent.retain_mut(|identity| {
        let exact = groups
            .iter()
            .any(|group| group.name == identity.group && group.items.iter().any(|item| item.key == identity.key && item.group_path == identity.group_path));
        if exact {
            return true;
        }

        let same_group: Vec<_> = groups
            .iter()
            .filter(|group| group.name == identity.group)
            .flat_map(|group| group.items.iter().filter(|item| item.key == identity.key).map(|item| (group.name.as_str(), item.group_path.as_slice())))
            .collect();
        let replacement = if same_group.len() == 1 {
            same_group.first().copied()
        } else if same_group.is_empty() {
            let matches: Vec<_> = groups
                .iter()
                .flat_map(|group| group.items.iter().filter(|item| item.key == identity.key).map(|item| (group.name.as_str(), item.group_path.as_slice())))
                .collect();
            if matches.len() == 1 { matches.first().copied() } else { None }
        } else {
            None
        };

        if let Some((group, group_path)) = replacement {
            identity.group = group.to_owned();
            identity.group_path = group_path.to_vec();
            true
        } else {
            // Ambiguous keys are retained rather than silently dropping history or binding it to
            // a different asset. They become resolvable again if the browser data disambiguates.
            same_group.len() > 1 || groups.iter().flat_map(|group| &group.items).filter(|item| item.key == identity.key).count() > 1
        }
    });
}

#[derive(Clone, Debug)]
struct RecentDrag {
    item: RecentPickerItem,
    origin: Rect,
    grab_offset: Vec2,
}

/// Draw the shared one-row history and return normal browser selection events for clicked cards.
pub(crate) fn show(ui: &mut egui::Ui, state: &mut PresetUi, adapter: &mut BrowserAdapter<'_>, options: Options) -> Vec<BrowserAction> {
    let panel = adapter.panel;
    if let Some(recent) = state.recent_items.get_mut(panel) {
        reconcile_recent_items(recent, adapter.groups);
        recent.truncate(crate::pickers::picker_state::MAX_RECENT_PICKER_ITEMS);
    }
    let recent = state.recent_items.get(panel).cloned().unwrap_or_default();
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    let tokens = crate::theme::Tokens::get(ui.ctx());
    ui.painter().rect_filled(strip, 0.0, tokens.dock);

    let mut events = Vec::new();
    let mut dragged = None;
    let mut drag_finished = None;
    let mut card_rects = Vec::new();
    let mut x = strip.left();
    for (index, identity) in recent.iter().enumerate() {
        let Some((group_index, item_index)) = adapter.groups.iter().enumerate().find_map(|(gi, group)| {
            (group.name == identity.group)
                .then(|| group.items.iter().position(|item| item.key == identity.key && item.group_path == identity.group_path).map(|ii| (gi, ii)))
                .flatten()
        }) else {
            continue;
        };
        let rect = Rect::from_min_size(pos2(x, strip.top()), vec2(CARD_SIZE, CARD_SIZE));
        x += CARD_SIZE;
        card_rects.push(rect);
        ui.painter().rect_filled(rect, 0.0, tokens.card);
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, tokens.card_border), StrokeKind::Inside);
        let preview = rect.shrink(CARD_INSET);
        (adapter.paint_item)(
            ui,
            preview,
            group_index,
            item_index,
            // Recent picks select the engine's active asset, not the browser's independent
            // multi-selection model. Keep the strip visually independent of browser selection.
            ItemPaintState::Item { selected: false },
        );
        let response = ui.interact(rect, ui.id().with(("recent-picker-item", panel, index)), Sense::click_and_drag());
        let cursor = if response.is_pointer_button_down_on() {
            Some(egui::CursorIcon::Grabbing)
        } else if panel == "gradients" {
            Some(egui::CursorIcon::PointingHand)
        } else {
            None
        };
        let response = if let Some(cursor) = cursor { response.on_hover_cursor(cursor) } else { response };
        if response.clicked() {
            events.push(BrowserAction::SelectItem { group: identity.group.clone(), group_path: identity.group_path.clone(), key: identity.key.clone() });
        }
        if response.dragged() {
            dragged = Some((index, rect));
        }
        if response.drag_stopped() {
            drag_finished = Some(index);
        }
    }

    let drag_id = ui.id().with(("recent-picker-drag", panel));
    if let Some((index, rect)) = dragged {
        let press = ui.input(|input| input.pointer.press_origin());
        let grab_offset = press.filter(|point| rect.contains(*point)).map(|point| point - rect.min).unwrap_or(rect.size() * 0.5);
        if let Some(item) = recent.get(index) {
            ui.ctx().data_mut(|data| data.insert_temp(drag_id, RecentDrag { item: item.clone(), origin: rect, grab_offset }));
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }

    let active_drag = ui.ctx().data(|data| data.get_temp::<RecentDrag>(drag_id));
    if let Some(drag) = active_drag
        && let Some(pointer) = ui.ctx().pointer_interact_pos()
    {
        let pointer_ghost = Rect::from_min_size(pointer - drag.grab_offset, drag.origin.size());
        let ghost = if options.allow_arranging && strip.contains(pointer) {
            let max_x = (strip.right() - pointer_ghost.width()).max(strip.left());
            Rect::from_min_size(pos2(pointer_ghost.left().clamp(strip.left(), max_x), strip.top()), pointer_ghost.size())
        } else {
            pointer_ghost
        };
        if let Some((group_index, item_index)) = adapter.groups.iter().enumerate().find_map(|(gi, group)| {
            (group.name == drag.item.group)
                .then(|| group.items.iter().position(|item| item.key == drag.item.key && item.group_path == drag.item.group_path).map(|ii| (gi, ii)))
                .flatten()
        }) {
            // Paint on a foreground layer with the full app content clip. The strip's parent
            // panel may be narrow or scroll-clipped, but the drag preview must remain visible
            // anywhere inside the PhotoCraft window.
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(("recent-picker-drag", panel)));
            let mut ghost_ui =
                egui::Ui::new(ui.ctx().clone(), egui::Id::new(("recent-picker-drag-ui", panel)), egui::UiBuilder::new().layer_id(layer).max_rect(ghost));
            ghost_ui.set_clip_rect(ui.ctx().content_rect());
            ghost_ui.set_opacity(0.5);
            ghost_ui.painter().rect_filled(ghost, 0.0, tokens.card);
            (adapter.paint_item)(&ghost_ui, ghost.shrink(CARD_INSET), group_index, item_index, ItemPaintState::DragGhost);
            ghost_ui.painter().rect_stroke(ghost, 0.0, Stroke::new(1.0, tokens.card_border), StrokeKind::Inside);
        }
        if options.allow_arranging
            && let Some(source_index) = recent.iter().position(|item| item == &drag.item)
            && strip.contains(pointer)
        {
            let remaining: Vec<Rect> = card_rects.iter().enumerate().filter_map(|(index, rect)| (index != source_index).then_some(*rect)).collect();
            let insertion = insertion_index(strip, &remaining, pointer);
            let indicator_x = insertion_indicator_x(strip, &remaining, insertion);
            ui.painter().rect_filled(Rect::from_min_size(pos2(indicator_x - 2.0, strip.top()), vec2(4.0, HEIGHT)), 0.0, tokens.accent);
        }
    }

    if let Some(from) = drag_finished {
        // egui suppresses `clicked` once a click-and-drag response crosses the drag threshold.
        // Treat a drag released on the strip as a selection for non-arrangeable pickers. For
        // arrangeable pickers, returning to the source card or dropping into the final position
        // selects the dragged asset. The ghost is paint-only; use the actual release position.
        let release = ui.ctx().input(|input| input.pointer.interact_pos());
        if let Some(pointer) = release.filter(|pointer| strip.contains(*pointer)) {
            let to = if options.allow_arranging {
                let remaining: Vec<Rect> = card_rects.iter().enumerate().filter_map(|(index, rect)| (index != from).then_some(*rect)).collect();
                insertion_index(strip, &remaining, pointer).min(recent.len().saturating_sub(1))
            } else {
                from
            };
            let selects = !options.allow_arranging
                || released_on_origin(card_rects.get(from).copied().unwrap_or(Rect::NOTHING), Some(pointer))
                || (options.allow_arranging && to == recent.len().saturating_sub(1));
            if selects && let Some(identity) = recent.get(from) {
                events.push(BrowserAction::SelectItem { group: identity.group.clone(), group_path: identity.group_path.clone(), key: identity.key.clone() });
            }
            if options.allow_arranging {
                state.reorder_recent_items(panel, from, to);
            }
        }
        ui.ctx().data_mut(|data| data.remove::<RecentDrag>(drag_id));
    }
    events
}

#[cfg(test)]
mod tests {
    use crate::pickers::asset_browser::{GroupView, ItemView};
    use crate::pickers::picker_state::PresetUi;
    use egui::{Rect, pos2, vec2};

    #[test]
    fn insertion_indicator_covers_both_outer_edges() {
        let strip = Rect::from_min_size(pos2(10.0, 20.0), vec2(54.0, 18.0));
        let cards = [Rect::from_min_size(pos2(10.0, 20.0), vec2(18.0, 18.0)), Rect::from_min_size(pos2(28.0, 20.0), vec2(18.0, 18.0))];

        assert_eq!(super::insertion_indicator_x(strip, &cards, 0), 10.0);
        assert_eq!(super::insertion_indicator_x(strip, &cards, cards.len()), 46.0);
    }

    #[test]
    fn bottom_three_pixels_target_the_final_insertion_position() {
        let strip = Rect::from_min_size(pos2(10.0, 20.0), vec2(54.0, 18.0));
        let cards = [Rect::from_min_size(pos2(10.0, 20.0), vec2(18.0, 18.0)), Rect::from_min_size(pos2(28.0, 20.0), vec2(18.0, 18.0))];

        assert_eq!(super::insertion_index(strip, &cards, pos2(12.0, 35.0)), 2);
        assert_eq!(super::insertion_index(strip, &cards, pos2(12.0, 34.9)), 0);
        assert_eq!(super::insertion_index(strip, &cards, pos2(50.0, 25.0)), 2);
    }

    #[test]
    fn dragging_away_and_releasing_back_on_the_source_card_selects_it() {
        let source = Rect::from_min_size(pos2(10.0, 20.0), vec2(18.0, 18.0));
        let away = pos2(80.0, 60.0);

        assert!(!super::released_on_origin(source, Some(away)));
        assert!(super::released_on_origin(source, Some(source.center())));
        assert!(!super::released_on_origin(source, None));
    }

    #[test]
    fn swatches_disable_drag_snapping_and_insertion_indicator() {
        assert_eq!(super::Options::for_panel("swatches"), super::Options { allow_arranging: false });
        assert_eq!(super::Options::for_panel("gradients"), super::Options { allow_arranging: true });
    }

    #[test]
    fn selecting_moves_existing_identity_to_front_and_caps_history_at_fifteen() {
        let mut state = PresetUi::default();
        for index in 0..16 {
            state.record_recent_item("gradients", "Group", &[], &index.to_string());
        }
        assert_eq!(state.recent_items["gradients"].len(), 15);
        assert_eq!(state.recent_items["gradients"][0].key, "15");
        assert_eq!(state.recent_items["gradients"][14].key, "1");
        state.record_recent_item("gradients", "Group", &[], "8");
        assert_eq!(state.recent_items["gradients"].len(), 15);
        assert_eq!(state.recent_items["gradients"][0].key, "8");
        assert_eq!(state.recent_items["gradients"].iter().filter(|item| item.key == "8").count(), 1);
    }

    #[test]
    fn recent_reordering_preserves_the_selected_identity() {
        let mut state = PresetUi::default();
        for key in ["a", "b", "c"] {
            state.record_recent_item("panel", "Group", &[], key);
        }
        state.reorder_recent_items("panel", 2, 0);
        assert_eq!(state.recent_items["panel"].iter().map(|item| item.key.as_str()).collect::<Vec<_>>(), ["a", "c", "b"]);
    }

    #[test]
    fn recent_history_persists_full_asset_identity() {
        let mut state = PresetUi::default();
        state.record_recent_item("panel", "Group", &["Folder".into()], "shared-key");
        state.record_recent_item("panel", "Group", &["Other folder".into()], "shared-key");

        let encoded = serde_json::to_value(&state).expect("picker preferences serialize");
        let restored: PresetUi = serde_json::from_value(encoded).expect("picker preferences deserialize");

        assert_eq!(restored.recent_items["panel"].len(), 2);
        assert_eq!(restored.recent_items["panel"][0].group_path, ["Other folder"]);
        assert_eq!(restored.recent_items["panel"][1].group_path, ["Folder"]);
    }

    #[test]
    fn dragging_asset_to_another_folder_or_group_keeps_its_recent_card() {
        let mut state = PresetUi::default();
        state.record_recent_item("panel", "Original group", &["Original folder".into()], "stable-asset-id");
        state.record_recent_item("panel", "Other group", &[], "another-asset-id");

        // The browser rebuilds its view after a drag/drop. The asset has the same stable key,
        // but now belongs to a different root group and nested folder.
        let groups = [
            GroupView {
                name: "Destination group".into(),
                root_only: false,
                items: vec![ItemView { key: "stable-asset-id".into(), name: "Moved asset".into(), group_path: vec!["Destination folder".into()] }],
            },
            GroupView {
                name: "Other group".into(),
                root_only: false,
                items: vec![ItemView { key: "another-asset-id".into(), name: "Other asset".into(), group_path: Vec::new() }],
            },
        ];

        super::reconcile_recent_items(state.recent_items.get_mut("panel").unwrap(), &groups);

        let recent = &state.recent_items["panel"];
        assert_eq!(recent.len(), 2, "moving an asset in the browser must not evict its recent miniature");
        assert_eq!(recent[1].group, "Destination group");
        assert_eq!(recent[1].group_path, ["Destination folder"]);
        assert_eq!(recent[1].key, "stable-asset-id");
    }
}
