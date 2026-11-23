//! The Path operations, Path alignment and Path arrangement buttons of the vector tools' bars.

use craft::behavior::IconId;
use craft::ui::buttons::{IconButton, PopupMark};
use craft::ui::contract::{Bind, Entry, PopupKind, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::display::{Divider, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::menu::Menu;
use craft::ui::pickers::Select;
use craft::ui::popover::Popover;
use craft::ui::runtime::env::Label;

use super::PhotocraftApp;
use super::pen::{PATH_OPS, merge_target, tool_in_shape_mode, tool_path_op};

/// Path alignment's Align entries: label and icon.
pub(crate) const PATH_ALIGN_ENTRIES: [(&str, &str); 6] = [
    ("Align Left Edges", "align_edge_left"),
    ("Align Horizontal Centers", "align-edge-hcenter"),
    ("Align Right Edges", "align-edge-right"),
    ("Align Top Edges", "align-edge-top"),
    ("Align Vertical Centers", "align-edge-vcenter"),
    ("Align Bottom Edges", "align-edge-bottom"),
];

/// Path alignment's Distribute entries.
pub(crate) const PATH_DISTRIBUTE_ENTRIES: [(&str, &str); 6] = [
    ("Distribute Left Edges", "distribute-left"),
    ("Distribute Horizontal Centers", "distribute-hcenter"),
    ("Distribute Right Edges", "distribute-right"),
    ("Distribute Top Edges", "distribute-top"),
    ("Distribute Vertical Centers", "distribute-vcenter"),
    ("Distribute Bottom Edges", "distribute-bottom"),
];

/// Path alignment's Distribute Spacing entries.
pub(crate) const PATH_SPACING_ENTRIES: [(&str, &str); 2] =
    [("Distribute Horizontally", "distribute-horizontal"), ("Distribute Vertically", "distribute-vertical")];

/// Path arrangement's entries, front to back.
pub(crate) const PATH_ARRANGE_ENTRIES: [&str; 4] = ["Bring Shape To Front", "Bring Shape Forward", "Send Shape Backward", "Send Shape To Back"];

/// A label from a translated string.
fn label(text: &str) -> Label {
    Label::from(text.to_string())
}

/// Whether the alignment popover is open, kept by the framework per viewer.
#[derive(Clone, Copy, Default)]
struct AlignOpen(bool);

/// What the operations menu picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum OpsPick {
    Op(&'static str),
    Merge,
}

/// What the Path Operations menu's rows mean: Align To's two choices, in `path_align_to` order.
const ALIGN_TO: [&str; 2] = ["Selection", "Canvas"];

/// The Path Operations button of the Pen, Freeform Pen, Curvature Pen, Path Selection and Direct
/// Selection, Photoshop's: the operation of the selected path components, or with none selected how the next
/// path combines with the shape or path it is drawn into; and Merge Shape Components. Its icon
/// shows the selected components' operation, else the tool's.
pub(super) fn path_operations_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // New Layer makes a shape layer: Path mode, Path Selection and Direct Selection have none.
    let shape = tool_in_shape_mode(app);
    let current = crate::path_components::selected_op(app)
        .or_else(|| tool_path_op(app))
        .map_or(if shape { "newLayer" } else { "combine" }, |op| if op == "newLayer" && !shape { "combine" } else { op });
    // PLAN: icon ids assumed registered under the PATH_OPS glyph names.
    let icon = PATH_OPS.iter().find(|(v, ..)| *v == current).map_or("path-op-combine", |(.., i)| *i);
    let merge = merge_target(app).filter(|(.., n)| *n > 1);
    let mut entries: Vec<Entry<OpsPick>> = PATH_OPS
        .iter()
        .map(|(value, name, glyph)| {
            Entry::new(OpsPick::Op(value), label(tl!(name))).icon(IconId::new(glyph)).checked(*value == current).enabled(shape || *value != "newLayer")
        })
        .collect();
    entries.push(Entry::separator());
    entries.push(Entry::new(OpsPick::Merge, label(tl!("Merge Shape Components"))).icon(IconId::new("path-op-merge")).enabled(merge.is_some()));
    let button = IconButton::new(IconId::new(icon)).label(label(tl!("Path operations"))).popup(PopupKind::Menu).popup_mark(PopupMark::Menu).show(ui).response;
    let picked = Menu::new(label(tl!("Path operations"))).opened_by(&button, Trigger::Click).pick(ui, &entries).picked;
    let result = match picked {
        Some(OpsPick::Op(op)) => crate::path_components::set_op(app, op),
        Some(OpsPick::Merge) => match merge {
            Some((command, params, _)) => app.run(command, params).map(|_| ()),
            None => Ok(()),
        },
        None => Ok(()),
    };
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The Path alignment button of the Shape tools. Every entry is greyed: the engine has no
/// command that aligns or distributes the components of a path yet. The popover stays open while
/// Align To is used.
pub(super) fn path_alignment_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let align_to: Vec<Entry<usize>> = ALIGN_TO.iter().enumerate().map(|(i, name)| Entry::new(i, label(tl!(name)))).collect();
    let choice = &mut app.ui.path_align_to;
    Row::new().show(ui, |ui, cx| {
        let open = &mut cx.memory.shown::<AlignOpen>(cx.id).0;
        let button = IconButton::new(IconId::new("align_edge_left"))
            .label(label(tl!("Path alignment")))
            .popup(PopupKind::Dialog)
            .popup_mark(PopupMark::Menu)
            .expanded(*open)
            .show(ui)
            .response;
        if craft::ui::contract::clicked(&button) {
            *open = !*open;
        }
        Popover::new(label(tl!("Path alignment")), Bind::bound(open)).opened_by(&button).show(ui, |ui, _| {
            for (heading, rows) in [
                (tl!("Align:"), &PATH_ALIGN_ENTRIES[..]),
                (tl!("Distribute:"), &PATH_DISTRIBUTE_ENTRIES[..]),
                (tl!("Distribute Spacing:"), &PATH_SPACING_ENTRIES[..]),
            ] {
                Text::new(label(heading)).show(ui);
                for (name, _glyph) in rows {
                    craft::ui::buttons::Button::new(label(tl!(name))).enabled(false).show(ui);
                }
                Divider::horizontal().show(ui);
            }
            Field::new(label(tl!("Align To:"))).show(ui, |ui, _| {
                Select::new(label(tl!("Align To:")), Bind::bound(&mut *choice), &align_to).show(ui);
            });
        });
    });
}

/// The Path arrangement button of the Shape tools. Its entries are greyed: the engine
/// has no command that reorders the components of a path yet.
pub(super) fn path_arrangement_menu(_app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let entries: Vec<Entry<usize>> = PATH_ARRANGE_ENTRIES.iter().enumerate().map(|(i, name)| Entry::new(i, label(tl!(name))).enabled(false)).collect();
    let button =
        IconButton::new(IconId::new("layers")).label(label(tl!("Path arrangement"))).popup(PopupKind::Menu).popup_mark(PopupMark::Menu).show(ui).response;
    let _ = Menu::new(label(tl!("Path arrangement"))).opened_by(&button, Trigger::Click).pick(ui, &entries);
}
