//! The Channels panel (tab of the Layers card): composite, colour, layer-mask, alpha / spot and
//! Quick Mask rows over the framework's `ChannelsPanel`. Every action is an engine command
//! (`channel.*`, `select.loadSelection`, `select.saveSelection`, …); the targeted channel and the
//! eyes live in the engine's `DocState::channel_view`, so agents see the same state through
//! `inspect`. The panel reports `ChannelsEvent`s; [`apply`] is the one map from an event to its command.

use egui::RichText;
use photocraft_engine::channel_cmds::{self, ChannelTarget};
use serde_json::{Value, json};

// PLAN: assumed paths of the framework items (imaging channels, keymap, ui contract and runtime).
use craft::imaging::ui::channels::{self as channels, ChannelKind, ChannelRow, ChannelsCommands, ChannelsEvent, ChannelsFooterButton, ChannelsPanel};
use craft::keymap::Mods;
use craft::ui::contract::{CommandSet, SweepStep};
use craft::ui::menu::PanelRow;
use craft::ui::primitives::textures::{ThumbnailOptions, Thumbnails};
use craft::ui::runtime::commands::{CommandRef, CommandState};

use crate::PhotocraftApp;
use crate::app::thumbs::ChannelThumbs;
use crate::theme::Tokens;

/// One row of the panel: its key, its thumbnail's key and the key of its events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Row {
    Composite,
    Color(usize),
    Alpha(usize),
    QuickMask,
    /// The selected layer's mask: a temporary channel, listed while the layer is selected.
    LayerMask,
}

impl Row {
    /// The channel reference the engine commands take.
    fn reference(self) -> Value {
        match self {
            Row::Composite => json!("composite"),
            Row::Color(k) => json!({ "color": k }),
            Row::Alpha(i) => json!(i),
            Row::QuickMask => json!("quickMask"),
            Row::LayerMask => json!("mask"),
        }
    }

    /// The `channel.target` reference a click records: a colour row by the mode's colour name,
    /// which a recorded action can save as Photoshop's component select (an index can't be).
    fn target_reference(self, mode: photocraft_doc::ColorMode) -> Value {
        let names = channel_cmds::color_names(mode);
        match self {
            Row::QuickMask => json!("composite"),
            Row::Color(k) if names.len() > 1 => names.get(k).map_or_else(|| self.reference(), |name| json!({ "color": name.to_ascii_lowercase() })),
            _ => self.reference(),
        }
    }
}

struct RowPresentationContext<'a> {
    doc: &'a photocraft_doc::Document,
    view: &'a photocraft_engine::channel_cmds::ChannelView,
    mode: photocraft_doc::ColorMode,
    colors: usize,
    gray_view: bool,
    quick: bool,
    mask_targeted: bool,
    masked: Option<&'a photocraft_doc::Layer>,
}

struct RowPresentation {
    name: String,
    visible: bool,
    selected: bool,
}

/// What clicking `row`'s eye asks for.
fn eye_action(row: Row, visible: bool, gray_view: bool, layer_id: Option<u64>) -> (String, Value) {
    let view_cmd = photocraft_engine::mask_view_cmds::ID.to_string();
    match row {
        // The mask's eye: the overlay on, or the mask view off.
        Row::LayerMask => (view_cmd, json!({ "layer": layer_id, "mode": if visible { "off" } else { "overlay" } })),
        // Showing the composite again over a gray mask view keeps the mask as an overlay.
        Row::Composite if gray_view => (view_cmd, json!({ "layer": layer_id, "mode": "overlay" })),
        _ => ("channel.setVisible".into(), json!({ "channel": row.reference(), "visible": !visible })),
    }
}

fn row_presentation(row: Row, context: &RowPresentationContext<'_>, mask_visible: bool) -> RowPresentation {
    let (name, visible, selected) = match row {
        Row::Composite => (
            channel_cmds::composite_name(context.mode).to_string(),
            context.view.visible_colors(context.colors) == context.colors && !context.gray_view,
            context.view.target == ChannelTarget::Composite && !context.quick && !context.mask_targeted,
        ),
        Row::Color(k) => (
            channel_cmds::color_names(context.mode)[k].to_string(),
            context.view.color_visible(k) && !context.gray_view,
            (context.view.target == ChannelTarget::Composite && !context.quick && !context.mask_targeted)
                || context.view.color_selection(context.colors).is_some_and(|c| c.contains(k)),
        ),
        Row::Alpha(i) => (context.doc.channels[i].name.clone(), context.view.alpha_shown(i), context.view.target == ChannelTarget::Alpha(i)),
        Row::QuickMask => ("Quick Mask".to_string(), !context.view.quick_mask_hidden, context.view.target == ChannelTarget::Composite),
        Row::LayerMask => (context.masked.map_or_else(String::new, mask_row_name), mask_visible, context.mask_targeted),
    };
    RowPresentation { name, visible, selected }
}

/// The framework's Color Indicates setting for the document's.
fn indicates(setting: photocraft_doc::ColorIndicates) -> channels::ColorIndicates {
    match setting {
        photocraft_doc::ColorIndicates::MaskedAreas => channels::ColorIndicates::MaskedAreas,
        photocraft_doc::ColorIndicates::SelectedAreas => channels::ColorIndicates::SelectedAreas,
    }
}

/// The kind the framework panel shows `row` as.
fn row_kind(row: Row, context: &RowPresentationContext<'_>) -> ChannelKind {
    match row {
        Row::Composite => ChannelKind::Composite,
        Row::Color(_) => ChannelKind::Color,
        Row::Alpha(i) => {
            let channel = &context.doc.channels[i];
            match channel.spot {
                Some((ink, _)) => {
                    let c = ink.to_rgb().map(|v| v.clamp(0.0, 1.0));
                    ChannelKind::Spot(craft::theme::Srgba::rgb(c[0], c[1], c[2]))
                }
                None => ChannelKind::Alpha(indicates(channel.indicates)),
            }
        }
        Row::QuickMask => ChannelKind::QuickMask(context.doc.quick_mask.as_ref().map_or_else(Default::default, |mask| indicates(mask.indicates))),
        Row::LayerMask => ChannelKind::LayerMask(context.masked.and_then(|layer| layer.mask.as_ref()).is_none_or(|mask| mask.enabled)),
    }
}

/// The ⌥-click mask view of the active document: gray shows the mask alone (#196).
fn gray_view(st: &photocraft_engine::DocState) -> bool {
    photocraft_engine::mask_view_cmds::current(st).map(|v| v.mode) == Some(photocraft_engine::mask_view_cmds::MaskViewMode::Gray)
}

/// The rows of the active document in display order: Composite (not in Multichannel), the colour channels when
/// there is more than one, the active layer's mask, the alpha and spot channels, Quick Mask. Empty without a document.
pub(crate) fn rows(app: &PhotocraftApp) -> Vec<ChannelRow<Row>> {
    let Some(st) = app.session.active() else { return Vec::new() };
    let doc = &st.doc;
    let view = &st.channel_view;
    let mask_view = photocraft_engine::mask_view_cmds::current(st).map(|v| v.mode);
    let mode = doc.pixel_format().mode;
    let colors = mode.color_channels();
    let quick = doc.quick_mask.is_some();
    let masked = st.active_layer.and_then(|id| doc.layer(id)).filter(|l| l.mask.is_some());
    let mask_targeted = mask_row_targeted(masked.is_some(), app.ui.mask_target, view.target, quick);
    let context = RowPresentationContext { doc, view, mode, colors, gray_view: gray_view(st), quick, mask_targeted, masked };
    // Multichannel images are their ink channels only (no composite or colour rows).
    let multichannel = doc.mode == photocraft_doc::ColorMode::Multichannel;
    let mut keys = if multichannel { Vec::new() } else { vec![Row::Composite] };
    if colors > 1 && !multichannel {
        keys.extend((0..colors).map(Row::Color));
    }
    if masked.is_some() {
        keys.push(Row::LayerMask);
    }
    keys.extend((0..doc.channels.len()).map(Row::Alpha));
    if quick {
        keys.push(Row::QuickMask);
    }
    keys.into_iter()
        .map(|key| {
            let RowPresentation { name, visible, selected } = row_presentation(key, &context, mask_view.is_some());
            // Built-in names are translated; alpha and spot names and "{layer} Mask" stay verbatim.
            let name = match key {
                Row::Composite | Row::Color(_) | Row::QuickMask => tl!(&name).to_owned(),
                Row::Alpha(_) | Row::LayerMask => name,
            };
            let mut row = ChannelRow::new(key, row_kind(key, &context), name);
            row.visible = visible;
            row.targeted = selected;
            row
        })
        .collect()
}

/// The layer whose mask the temporary mask row shows.
fn masked_layer(app: &PhotocraftApp) -> Option<u64> {
    let st = app.session.active()?;
    st.active_layer.and_then(|id| st.doc.layer(id)).filter(|l| l.mask.is_some()).map(|l| l.id.0)
}

/// Where the Rename Channel… command leaves the row (an `Option<Row>`) whose rename field the next pass opens.
fn rename_request() -> egui::Id {
    egui::Id::unique("photocraft.channels.rename")
}

/// The Channels panel body.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    if app.session.active().is_none() {
        let t = Tokens::get(&ctx);
        ui.add_space(6.0);
        ui.label(RichText::new(tl!("No document")).color(t.text_faint));
        return;
    }
    let rows = rows(app);
    let thumbnails = Thumbnails::<Row>::in_context(&ctx, "photocraft.channels", ThumbnailOptions::new());
    let rename = ctx.data_mut(|d| d.remove_temp::<Option<Row>>(rename_request())).flatten();
    let events = {
        let Some(st) = app.session.active() else { return };
        let source = ChannelThumbs::new(app, st);
        let mut panel = ChannelsPanel::new(&rows).thumbnails(&thumbnails, &source);
        if let Some(key) = rename {
            panel = panel.start_rename(key);
        }
        panel.show(ui).events
    };
    for event in events {
        apply(app, &ctx, event);
    }
}

/// Runs the engine command of `event`. `ctx` serves the commands that ask first (`menus::invoke`).
// PLAN: the sheet's `apply(app, event)` has no context; `menus::invoke` needs one, so it is a parameter.
pub(crate) fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, event: ChannelsEvent<Row>) {
    match event {
        ChannelsEvent::Target { key, extend, .. } => target(app, key, extend),
        ChannelsEvent::Visibility(step) => {
            let edit = match step {
                SweepStep::Flip(e) | SweepStep::ShowOnly(e) | SweepStep::Restore(e) | SweepStep::UndoShowOnly(e) => e,
                _ => return,
            };
            apply_visibility(app, edit.key, edit.on);
        }
        ChannelsEvent::LoadSelection { key, operation, .. } => {
            let params = match key {
                Row::LayerMask => json!({ "channel": "mask", "layer": masked_layer(app), "operation": operation.name() }),
                _ => json!({ "channel": key.reference(), "operation": operation.name() }),
            };
            let _ = app.run("select.loadSelection", params);
        }
        ChannelsEvent::Rename { key: Row::Alpha(index), text, .. } => {
            let _ = app.run("channel.rename", json!({ "channel": index, "name": text }));
        }
        ChannelsEvent::Options(key) => open_options(app, key),
        ChannelsEvent::Reorder(edit) => reorder(app, edit),
        ChannelsEvent::Footer { button, rows, mods, .. } => apply_footer(app, ctx, button, &rows, mods),
        ChannelsEvent::FooterDrop { button, rows, .. } => apply_footer_drop(app, button, &rows),
        ChannelsEvent::Command { command, key, .. } => run_command(app, ctx, command, key),
        _ => {}
    }
}

/// An eye set to `on`: the engine command takes the value the eye had before.
// PLAN: `ChannelsEvent` variants are `#[non_exhaustive]`, so tests cannot build them; the event arms delegate to these functions.
fn apply_visibility(app: &mut PhotocraftApp, key: Row, on: bool) {
    let Some(st) = app.session.active() else { return };
    let (cmd, params) = eye_action(key, !on, gray_view(st), masked_layer(app));
    let _ = app.run(&cmd, params);
}

/// A footer click. `rows` is the targeted row's key (empty without rows); Alt on New asks for the channel's options first.
fn apply_footer(app: &mut PhotocraftApp, ctx: &egui::Context, button: ChannelsFooterButton, rows: &[Row], mods: Mods) {
    match button {
        ChannelsFooterButton::New if mods.alt => {
            let asked = crate::menus::invoke(app, ctx, "channel.new", json!({}));
            report(app, asked)
        }
        ChannelsFooterButton::New => {
            let _ = app.run("channel.new", json!({}));
        }
        ChannelsFooterButton::SaveSelection => {
            let _ = app.run("select.saveSelection", json!({}));
        }
        ChannelsFooterButton::Delete | ChannelsFooterButton::LoadSelection => apply_footer_drop(app, button, rows),
        _ => {}
    }
}

/// A row dropped on a footer button (or the button acting on the targeted row): the first of `rows` is acted on.
fn apply_footer_drop(app: &mut PhotocraftApp, button: ChannelsFooterButton, rows: &[Row]) {
    if let Some(&key) = rows.first() {
        run_drop(app, key, button);
    }
}

fn report<T>(app: &mut PhotocraftApp, result: Result<T, String>) {
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// A click on a row targets its channel; the mask row targets the layer mask, the composite and colour rows the pixels.
fn target(app: &mut PhotocraftApp, key: Row, extend: bool) {
    let Some(mode) = app.session.active().map(|st| st.doc.pixel_format().mode) else { return };
    if key == Row::LayerMask {
        let _ = app.run("channel.target", json!({ "channel": "mask" }));
        app.ui.mask_target = true;
    } else {
        if matches!(key, Row::Composite | Row::Color(_)) {
            app.ui.mask_target = false;
        }
        let _ = app.run("channel.target", json!({ "channel": key.target_reference(mode), "extend": extend && matches!(key, Row::Color(_)) }));
    }
    app.sync_mask_targets();
}

fn open_options(app: &mut PhotocraftApp, key: Row) {
    let _ = crate::channel_options_ui::open(app, &key.reference());
}

/// An alpha or spot row moved. `to` of the event counts the rows that remain once the moved row is out, over every
/// row of the panel; `channel.move` counts alpha and spot channels only, so it is the number of moving rows among
/// the first `to` remaining rows.
fn reorder(app: &mut PhotocraftApp, edit: craft::behavior::value::EditEvent<craft::ui::contract::ReorderMove<Row>>) {
    use craft::behavior::value::EditEvent;
    let mv = match edit {
        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => after,
        _ => return,
    };
    let Row::Alpha(from) = mv.key else { return };
    let rows = rows(app);
    let to = rows.iter().filter(|r| !mv.keys.contains(&r.key)).take(mv.to).filter(|r| r.kind.moves()).count();
    if to != from {
        let _ = app.run("channel.move", json!({ "channel": from, "to": to }));
    }
}

/// What dropping `row` (named `name`) on a footer button does, as in Photoshop; `None` where
/// Photoshop does nothing either (colour channels can't be deleted here). `mask_layer` is the
/// layer whose mask the temporary mask row shows.
fn drop_action(row: Row, name: &str, mask_layer: Option<u64>, on: ChannelsFooterButton) -> Option<(String, Value)> {
    match (on, row) {
        (ChannelsFooterButton::Delete, Row::Alpha(i)) => Some(("channel.delete".into(), json!({ "channel": i }))),
        // `PhotocraftApp::run` asks "Apply mask to layer before removing?", like dropping the mask's Layers thumbnail.
        (ChannelsFooterButton::Delete, Row::LayerMask) => Some(crate::mask_trash_prompt::drop_action(mask_layer?, crate::mask_thumbs_ui::MaskKind::Pixel)),
        (ChannelsFooterButton::Delete, _) => None,
        // The engine would name a mask's copy after the composite; Photoshop says "Layer 1 Mask copy".
        (ChannelsFooterButton::New, Row::LayerMask) => Some(("channel.duplicate".into(), json!({ "channel": "mask", "name": format!("{name} copy") }))),
        (ChannelsFooterButton::New, _) => Some(("channel.duplicate".into(), json!({ "channel": row.reference() }))),
        (ChannelsFooterButton::LoadSelection, Row::LayerMask) => Some(("select.loadSelection".into(), json!({ "channel": "mask", "layer": mask_layer? }))),
        (ChannelsFooterButton::LoadSelection, _) => Some(("select.loadSelection".into(), json!({ "channel": row.reference() }))),
        _ => None,
    }
}

/// Runs what `button` does to `key`: a footer click on the targeted row, or the row dropped on it.
fn run_drop(app: &mut PhotocraftApp, key: Row, button: ChannelsFooterButton) {
    let name = rows(app).into_iter().find(|r| r.key == key).map(|r| r.name).unwrap_or_default();
    if let Some((cmd, params)) = drop_action(key, &name, masked_layer(app), button) {
        let _ = app.run(&cmd, params);
    }
}

/// Runs a command of the set on `key`, the row it was opened on (the targeted row for the panel menu).
fn run_command(app: &mut PhotocraftApp, ctx: &egui::Context, command: ChannelsCommands, key: Option<Row>) {
    // New Channel… and Merge Channels… ask first.
    match command {
        ChannelsCommands::NewChannel => {
            let asked = crate::menus::invoke(app, ctx, "channel.new", json!({}));
            return report(app, asked);
        }
        ChannelsCommands::MergeChannels => {
            let asked = crate::menus::invoke(app, ctx, "channel.merge", json!({}));
            return report(app, asked);
        }
        ChannelsCommands::NewSpotChannel => {
            let _ = crate::channel_options_ui::open_new_spot(app);
            return;
        }
        ChannelsCommands::SplitChannels => {
            let _ = app.run("channel.split", json!({}));
            return;
        }
        ChannelsCommands::SaveSelection => {
            let _ = app.run("select.saveSelection", json!({}));
            return;
        }
        ChannelsCommands::ExitQuickMask => {
            let _ = app.run("select.editInQuickMaskMode", json!({ "on": false }));
            return;
        }
        _ => {}
    }
    let Some(key) = key else { return };
    if let Some(preset) = command.overlay_color() {
        let [r, g, b] = preset.rgb();
        let _ = app.run("channel.options", json!({ "channel": key.reference(), "color": format!("#{r:02x}{g:02x}{b:02x}") }));
        return;
    }
    if let Some(opacity) = command.overlay_opacity() {
        let _ = app.run("channel.options", json!({ "channel": key.reference(), "opacity": opacity }));
        return;
    }
    match command {
        ChannelsCommands::Duplicate => run_drop(app, key, ChannelsFooterButton::New),
        ChannelsCommands::Delete => run_drop(app, key, ChannelsFooterButton::Delete),
        ChannelsCommands::LoadSelection => run_drop(app, key, ChannelsFooterButton::LoadSelection),
        ChannelsCommands::ChannelOptions => open_options(app, key),
        ChannelsCommands::MergeSpot => {
            if let Row::Alpha(i) = key {
                let _ = app.run("channel.mergeSpot", json!({ "channel": i }));
            }
        }
        ChannelsCommands::ConvertToAlpha => {
            if let Row::Alpha(i) = key {
                let _ = app.run("channel.options", json!({ "channel": i, "indicates": "masked" }));
            }
        }
        ChannelsCommands::ColorIndicatesMasked => {
            let _ = app.run("channel.options", json!({ "channel": key.reference(), "indicates": "masked" }));
        }
        ChannelsCommands::ColorIndicatesSelected => {
            let _ = app.run("channel.options", json!({ "channel": key.reference(), "indicates": "selected" }));
        }
        ChannelsCommands::ToggleLayerMask => {
            let _ = app.run("layer.layerMask.enabled", json!({}));
        }
        ChannelsCommands::DeleteLayerMask => {
            let _ = app.run("layer.layerMask.delete", json!({}));
        }
        // Moving runs through the reorder event.
        _ => {}
    }
}

/// Whether the selected layer's mask row is the one shown targeted: the layer has a mask, and it
/// (not the composite) is what painting edits, outside Quick Mask.
fn mask_row_targeted(has_mask: bool, mask_target: bool, target: ChannelTarget, quick: bool) -> bool {
    has_mask && (mask_target || target == ChannelTarget::LayerMask) && matches!(target, ChannelTarget::Composite | ChannelTarget::LayerMask) && !quick
}

/// The layer mask row's name, "<layer> Mask", as Photoshop lists it.
fn mask_row_name(layer: &photocraft_doc::Layer) -> String {
    format!("{} Mask", layer.name)
}

/// The Channels entries of the panel menu: Photoshop's fixed list.
pub(crate) fn panel_rows(_app: &PhotocraftApp) -> Vec<PanelRow> {
    channels::panel_rows()
}

/// The state of each command of the set: the framework's rule over the rows, with Merge Channels… waiting for
/// enough grayscale documents of one size.
pub(crate) fn command_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    let merge = crate::menus::is_enabled(app, "channel.merge");
    let rows = rows(app);
    // A composite of several colour channels, or several targeted rows, have nothing to duplicate; the gray composite
    // is its one channel.
    let composite_of_several = channels::targeted_row(&rows).is_some_and(|row| row.kind == ChannelKind::Composite)
        && app.session.active().is_some_and(|st| st.doc.pixel_format().mode.color_channels() > 1);
    let several_targeted = rows.iter().filter(|row| row.targeted).count() > 1;
    channels::states(&rows)
        .into_iter()
        .map(|(command, state)| {
            let state = match command {
                ChannelsCommands::MergeChannels => CommandState::new().enabled(merge),
                ChannelsCommands::Duplicate if composite_of_several || several_targeted => CommandState::new().enabled(false),
                _ => state,
            };
            (CommandRef::from(command), state)
        })
        .collect()
}

/// What [`command_states`] reads: the rules (the active document and its channel view among them) and the mask target.
#[derive(craft::ui::contract::MemoProps)]
struct StatesFacts<'a> {
    rules: crate::menus::facts::RuleFacts<'a>,
    mask_target: bool,
}

/// [`command_states`], derived from [`StatesFacts`].
pub(crate) fn source_states(app: &PhotocraftApp, name: &'static str) -> std::sync::Arc<crate::app::framework::SourceStates> {
    let facts = StatesFacts { rules: crate::menus::facts::RuleFacts::of(app), mask_target: app.ui.mask_target };
    crate::app::framework::derived_states(app, name, &facts, || command_states(app))
}

/// Runs a fired command of the Channels set; false for another set's command.
pub(crate) fn run_fired(app: &mut PhotocraftApp, ctx: &egui::Context, command: &CommandRef) -> bool {
    let Some(command) = ChannelsCommands::from_ref(command) else { return false };
    // The panel opens the rename field itself, so the command has no event: leave the request for the next pass.
    if command == ChannelsCommands::RenameChannel {
        let rows = rows(app);
        if let Some(row) = channels::targeted_row(&rows).filter(|r| r.kind.renames()) {
            ctx.data_mut(|d| d.insert_temp(rename_request(), Some(row.key)));
        }
        return true;
    }
    let mods = craft::ui::runtime::pointer::frame(ctx).mods;
    if let Some(event) = command.event(&rows(app), mods) {
        apply(app, ctx, event);
    }
    true
}

#[cfg(test)]
mod tests;
