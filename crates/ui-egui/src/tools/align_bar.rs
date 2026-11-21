//! The Move tool's Align and Distribute controls (Photoshop shows them whenever the Move tool is
//! active; they enable once enough layers are selected). The framework's `AlignBar` lays out the
//! buttons and the More menu; each fired `AlignAction` runs the matching `layer.align.*` /
//! `layer.distribute.*` command through the app's command sources ([`run_fired`]). The gear after
//! the bar holds the Move tool's three hover and group toggles (#11381).

use craft::behavior::IconId;
use craft::ui::bars::{AlignAction, AlignBar};
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Entry, Props as _, Show as _, Trigger};
use craft::ui::menu::Menu;
use craft::ui::runtime::access::PopupKind;
use craft::ui::runtime::commands::{CommandRef, CommandSet as _, CommandState};
use serde_json::json;

use crate::PhotocraftApp;

/// Every align and distribute action, once.
fn all() -> impl Iterator<Item = AlignAction> {
    AlignAction::ALIGN.into_iter().chain(AlignAction::DISTRIBUTE)
}

/// The engine command an action runs.
pub(crate) fn command_id(action: AlignAction) -> Option<&'static str> {
    match action {
        AlignAction::AlignLeftEdges => Some("layer.align.leftEdges"),
        AlignAction::AlignHorizontalCenters => Some("layer.align.horizontalCenters"),
        AlignAction::AlignRightEdges => Some("layer.align.rightEdges"),
        AlignAction::AlignTopEdges => Some("layer.align.topEdges"),
        AlignAction::AlignVerticalCenters => Some("layer.align.verticalCenters"),
        AlignAction::AlignBottomEdges => Some("layer.align.bottomEdges"),
        AlignAction::DistributeTopEdges => Some("layer.distribute.topEdges"),
        AlignAction::DistributeVerticalCenters => Some("layer.distribute.verticalCenters"),
        AlignAction::DistributeBottomEdges => Some("layer.distribute.bottomEdges"),
        AlignAction::DistributeLeftEdges => Some("layer.distribute.leftEdges"),
        AlignAction::DistributeHorizontalCenters => Some("layer.distribute.horizontalCenters"),
        AlignAction::DistributeRightEdges => Some("layer.distribute.rightEdges"),
        AlignAction::DistributeHorizontally => Some("layer.distribute.horizontally"),
        AlignAction::DistributeVertically => Some("layer.distribute.vertically"),
        // `AlignAction` is `#[non_exhaustive]`; `all()` yields only the actions listed above, and a later one has no engine command here.
        _ => None,
    }
}

/// The enabled state of every action: the session's, as the Layer menu reads it.
pub(crate) fn command_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    all().map(|action| (CommandRef::new(action.id()), CommandState::new().enabled(command_id(action).is_some_and(|id| app.session.is_enabled(id))))).collect()
}

/// [`command_states`], derived from what the rules read.
pub(crate) fn source_states(app: &PhotocraftApp, name: &'static str) -> std::sync::Arc<crate::app::framework::SourceStates> {
    crate::app::framework::derived_states(app, name, &crate::menus::facts::RuleFacts::of(app), || command_states(app))
}

/// Runs a fired `AlignAction`. False when `command` is not one of them. The context is the
/// command-source shape's; these commands need none.
pub(crate) fn run_fired(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
    let Some(action) = AlignAction::from_ref(command) else { return false };
    let Some(id) = command_id(action) else { return false };
    if let Err(e) = app.run(id, json!({})) {
        app.ui.status = e;
    }
    true
}

/// The bar, then the gear.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    AlignBar::new().show(ui);
    gear(app, ui);
}

/// A row of the gear menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GearOption {
    HoverBounds,
    ExpandGroup,
    LayersHoverBounds,
}

/// The gear after the bar: a menu of the Move tool's three check items, all on by default and
/// saved with the tool options.
fn gear(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let label = tl!("Options").to_string();
    let button = IconButton::new(IconId::new("settings")).label(label.clone()).popup(PopupKind::Menu).show(ui);
    let o = &app.ui.tool_options;
    let entries = [
        Entry::new(GearOption::HoverBounds, tl!("Show layer bounds on hover").to_string()).checked(o.move_show_hover_bounds),
        Entry::new(GearOption::ExpandGroup, tl!("Expand layer group on click").to_string()).checked(o.move_expand_group_on_click),
        Entry::new(GearOption::LayersHoverBounds, tl!("Show hover bounds from Layers").to_string()).checked(o.move_show_layers_hover_bounds),
    ];
    let o = match Menu::new(label).opened_by(&button, Trigger::Press).pick(ui, &entries).picked {
        Some(picked) => picked,
        None => return,
    };
    let options = &mut app.ui.tool_options;
    match o {
        GearOption::HoverBounds => options.move_show_hover_bounds = !options.move_show_hover_bounds,
        GearOption::ExpandGroup => options.move_expand_group_on_click = !options.move_expand_group_on_click,
        GearOption::LayersHoverBounds => options.move_show_layers_hover_bounds = !options.move_show_layers_hover_bounds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_align_action_runs_a_live_command() {
        let specs = photocraft_engine::command_specs();
        let mut count = 0;
        for action in all() {
            let id = command_id(action).expect("an engine command");
            assert!(specs.iter().any(|c| c.id == id), "{id} is not a command");
            count += 1;
        }
        assert_eq!(count, 14);
    }
}
