//! The Modifier Keys panel: latching Shift, ⌘ and ⌥ for touch and pen users. Code that needs the held modifiers reads
//! `craft::ui::runtime::pointer::frame(ctx).mods`, which includes the latched keys.

use craft::ui::contract::Bind;
use craft::ui::patterns::panels::ModifierKeysPanel;

use crate::PhotocraftApp;

/// The Modifier Keys panel body: its buttons latch into `ShellUi::latched`, which `frame_input` hands to the
/// framework as `FrameInput::latched`.
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ModifierKeysPanel::new(Bind::bound(&mut app.ui.shell.latched)).show(ui);
}
