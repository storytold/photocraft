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
use super::pen::{PATH_OPS, drawn_path, merge_target, tool_in_shape_mode, tool_path_op};
use super::{Path, json};

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

/// `path.alignComponents` `edge` of each [`PATH_ALIGN_ENTRIES`] entry, in the same order (the
/// Distribute entries use them too, with `path.distributeComponents`).
const PATH_ALIGN_EDGES: [&str; 6] = ["leftEdges", "horizontalCenters", "rightEdges", "topEdges", "verticalCenters", "bottomEdges"];

/// `path.distributeComponents` `edge` of each [`PATH_SPACING_ENTRIES`] entry, in the same order.
const PATH_SPACING_EDGES: [&str; 2] = ["horizontally", "vertically"];

/// The components of `path` that have an anchor: the ones alignment can move.
fn alignable_components(path: &Path) -> usize {
    path.components().into_iter().filter(|c| path.subpaths.get(c.clone()).is_some_and(|s| s.iter().any(|sp| !sp.knots.is_empty()))).count()
}

/// The Path alignment button of the Shape tools. Its entries move the components of the path the
/// tool draws into ([`drawn_path`]: the selected shape layer in Shape mode), one History step each
/// (#13456). The Shape tools show no component selection, so they act on every component, as
/// Merge Shape Components does: Align needs two of them (one to align to the canvas), Distribute
/// and Distribute Spacing three. The popover stays open after a click and while Align To is used.
pub(super) fn path_alignment_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let align_to: Vec<Entry<usize>> = ALIGN_TO.iter().enumerate().map(|(i, name)| Entry::new(i, label(tl!(name)))).collect();
    let target = drawn_path(app).map(|(params, path)| (params, alignable_components(path)));
    let count = target.as_ref().map_or(0, |(_, n)| *n);
    let mut run = None;
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
            let canvas = *choice == 1;
            let (align, distribute) = (count >= if canvas { 1 } else { 2 }, count >= 3);
            for (heading, rows, command, edges, enabled, to) in [
                (tl!("Align:"), &PATH_ALIGN_ENTRIES[..], "path.alignComponents", &PATH_ALIGN_EDGES[..], align, Some(canvas)),
                (tl!("Distribute:"), &PATH_DISTRIBUTE_ENTRIES[..], "path.distributeComponents", &PATH_ALIGN_EDGES[..], distribute, None),
                (tl!("Distribute Spacing:"), &PATH_SPACING_ENTRIES[..], "path.distributeComponents", &PATH_SPACING_EDGES[..], distribute, None),
            ] {
                Text::new(label(heading)).show(ui);
                for ((name, _glyph), edge) in rows.iter().zip(edges) {
                    if craft::ui::contract::clicked(&craft::ui::buttons::Button::new(label(tl!(name))).enabled(enabled).show(ui)) {
                        run = Some((command, *edge, to));
                    }
                }
                Divider::horizontal().show(ui);
            }
            Field::new(label(tl!("Align To:"))).show(ui, |ui, _| {
                Select::new(label(tl!("Align To:")), Bind::bound(&mut *choice), &align_to).show(ui);
            });
        });
    });
    if let Some((command, edge, to)) = run
        && let Some((mut params, _)) = target
    {
        params["edge"] = json!(edge);
        if let Some(canvas) = to {
            params["to"] = json!(if canvas { "canvas" } else { "selection" });
        }
        if let Err(e) = app.run(command, params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// The Path arrangement button of the Shape tools. Its entries are greyed: the engine
/// has no command that reorders the components of a path yet.
pub(super) fn path_arrangement_menu(_app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let entries: Vec<Entry<usize>> = PATH_ARRANGE_ENTRIES.iter().enumerate().map(|(i, name)| Entry::new(i, label(tl!(name))).enabled(false)).collect();
    let button =
        IconButton::new(IconId::new("layers")).label(label(tl!("Path arrangement"))).popup(PopupKind::Menu).popup_mark(PopupMark::Menu).show(ui).response;
    let _ = Menu::new(label(tl!("Path arrangement"))).opened_by(&button, Trigger::Click).pick(ui, &entries);
}
