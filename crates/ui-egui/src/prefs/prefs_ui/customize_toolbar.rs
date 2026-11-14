//! Edit › Toolbar…: the Customize Toolbar dialog (#14616). It shows the stored group layout
//! (#14459) as two lists, Toolbar and Extra Tools, each a column of framed groups whose rows are
//! the tool's icon, name and current letter; Extra Tools rows are dimmed. Restore Defaults puts
//! the default groups back and empties Extra Tools; Clear Tools moves every group, in order, to
//! Extra Tools. Both only change the dialog: Done (the default button, Enter) commits the two
//! lists with `edit.toolbar`, Cancel (Esc, the title bar's ×) leaves the toolbar as it was.
//!
//! The dialog's state is its `toolbar` and `extras` fields (lists of groups of tool names), so
//! `ui.inspect` reads it and `ui.dialog.set` changes it. Dragging rows and groups between the
//! lists is #14618.

use photocraft_engine::prefs::{ToolbarCustomization, ToolbarLayout};

use super::{Map, PhotocraftApp, RichText, Tokens, Value, json};
use crate::state::Tool;

/// The dialog kind (`__prefsui`).
pub(super) const KIND: &str = "customizeToolbar";

/// The reference application's help text above the lists.
pub(super) const HELP: &str = "Drag and drop tools or their groupings in the toolbar list view. Move excess, unused, or low priority tools into the extra list. When enabled, extra tools will appear in their own slot at the bottom of the toolbar.";

/// The fields a dialog showing `layout` starts with.
fn fields(layout: &ToolbarLayout) -> Value {
    json!({"toolbar": layout.toolbar, "extras": layout.extras})
}

/// Opens the dialog on the stored layout.
pub(super) fn open(app: &mut PhotocraftApp) -> Value {
    fields(&app.session.prefs().toolbar.layout())
}

/// Is this dialog Customize Toolbar? Its default button reads Done.
pub fn owns(f: &Map<String, Value>) -> bool {
    f.get("__prefsui").and_then(Value::as_str) == Some(KIND)
}

/// The groups in field `key`, as tool names.
fn groups(f: &Map<String, Value>, key: &str) -> Vec<Vec<String>> {
    f.get(key).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

/// Restore Defaults: the default groups and no Extra Tools (a tool the defaults leave out stays
/// in Extra Tools, as the layout itself places it).
pub(super) fn restore_defaults(f: &mut Map<String, Value>) {
    if let Value::Object(m) = fields(&ToolbarCustomization::default().layout()) {
        f.extend(m);
    }
}

/// Clear Tools: every Toolbar group moves, in order, to the end of Extra Tools.
pub(super) fn clear_tools(f: &mut Map<String, Value>) {
    let mut extras = groups(f, "toolbar");
    extras.extend(groups(f, "extras"));
    f.insert("toolbar".into(), json!(Vec::<Vec<String>>::new()));
    f.insert("extras".into(), json!(extras));
}

/// One list: its title over a scrolling column of framed groups.
fn list(app: &PhotocraftApp, ui: &mut egui::Ui, title: &str, groups: &[Vec<String>], dimmed: bool, salt: &str, height: f32) {
    let t = Tokens::get(ui.ctx());
    ui.vertical(|ui| {
        ui.label(RichText::new(tl!(title)).color(t.text).strong());
        egui::Frame::new().stroke(egui::Stroke::new(1.0, t.field_border)).inner_margin(6.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().id_salt(salt).max_height(height).auto_shrink([false, false]).show(ui, |ui| {
                for group in groups {
                    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.card_border)).corner_radius(3.0).inner_margin(4.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for name in group {
                            let Some(tool) = Tool::from_name(name) else { continue };
                            row(app, ui, tool, dimmed);
                        }
                    });
                    ui.add_space(4.0);
                }
            });
        });
    });
}

/// A row: the tool's icon, its name, and its current letter right-aligned.
fn row(app: &PhotocraftApp, ui: &mut egui::Ui, tool: Tool, dimmed: bool) {
    let t = Tokens::get(ui.ctx());
    let color = if dimmed { t.text_dim } else { t.text };
    let letter = crate::tool_shortcuts::shortcut(app, tool).unwrap_or_default();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        crate::icons::paint(ui, r, crate::icons::tool_icon(tool), 16.0, color);
        // The reference lists the bare names ("Rectangular Marquee"), without " Tool".
        ui.label(RichText::new(tl!(tool.label()).trim_end_matches(" Tool")).color(color));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(letter).color(color));
        });
    });
}

/// The dialog's body: the help text, the two lists side by side, Restore Defaults and Clear Tools.
pub(super) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.add(egui::Label::new(RichText::new(tl!(HELP)).color(t.text)).wrap());
    ui.add_space(8.0);
    let (toolbar, extras) = (groups(f, "toolbar"), groups(f, "extras"));
    let width = ((ui.available_width() - 12.0) / 2.0).max(160.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui(egui::vec2(width, 480.0), |ui| list(app, ui, "Toolbar", &toolbar, false, "customize-toolbar-list", 440.0));
        ui.add_space(12.0);
        ui.allocate_ui(egui::vec2(width, 480.0), |ui| list(app, ui, "Extra Tools", &extras, true, "customize-toolbar-extras", 440.0));
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if crate::widgets::secondary_button(ui, tl!("Restore Defaults"), 0.0).clicked() {
            restore_defaults(f);
        }
        if crate::widgets::secondary_button(ui, tl!("Clear Tools"), 0.0).clicked() {
            clear_tools(f);
        }
    });
}

/// Done: the two lists become the toolbar's layout. The single-tool `hidden` list is cleared, so
/// the lists say everything (a tool it held is already in Extra Tools here). No history state.
pub(super) fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    app.run("edit.toolbar", json!({"toolbar": groups(f, "toolbar"), "extras": groups(f, "extras"), "hidden": []}))
}

#[cfg(test)]
#[path = "../../customize_toolbar_dialog_tests.rs"]
mod customize_toolbar_dialog_tests;
