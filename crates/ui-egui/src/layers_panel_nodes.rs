//! The Layers panel's rows and their named parts in a kittest harness, for tests that drive the
//! real panel (craft-layers' `LayersPanel`): a row is named after its layer, its parts after the
//! panel's words (`Visibility {row}`, `Unlock {row}`, `Collapse effects {layer}`, …). Thumbnails
//! and the fx badge have no node of their own: craft-layers' testing helpers read their rects
//! from the row's node.

use craft::layers::ui::tree::{LayerPart, LayerTree};
use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;

use crate::PhotocraftApp;

/// The rect of the row of the layer, effect or Smart Filter named `name`.
pub(crate) fn row<S>(h: &Harness<'_, S>, name: &str) -> Rect {
    h.query_all_by_label(name)
        .find(|n| matches!(n.accesskit_node().role(), Role::TreeItem | Role::ListItem | Role::ListBoxOption))
        .unwrap_or_else(|| panic!("no Layers panel row named {name:?}"))
        .rect()
}

/// Whether the panel drew a row named `name` (a closed group's layers and rows out of view have
/// no node).
pub(crate) fn row_drawn<S>(h: &Harness<'_, S>, name: &str) -> bool {
    h.query_all_by_label(name).any(|n| n.accesskit_node().role() == Role::TreeItem)
}

/// The active document's layer named `name`.
pub(crate) fn layer_id(app: &PhotocraftApp, name: &str) -> LayerId {
    let st = app.session.active().expect("a document");
    st.doc.walk().into_iter().find(|(_, _, l)| l.name == name).map(|(_, _, l)| l.id).unwrap_or_else(|| panic!("no layer named {name:?}"))
}

/// The rect of the thumbnail that shows `part` of the layer named `name` on the row named
/// `row_name`, as the row drew it last pass; `None` when the row shows no such thumbnail. A Smart
/// Filter mask's thumbnail is on the layer's Smart Filters row; every other one on the layer's row.
pub(crate) fn thumb_on(h: &Harness<'_, PhotocraftApp>, name: &str, row_name: &str, part: LayerPart) -> Option<Rect> {
    let app = h.state();
    let st = app.session.active().expect("a document");
    let filter = &app.ui.layer_filter;
    let info = crate::panels::layers_tree::PhotoLayers::new(app, st, filter, false, None).info(&layer_id(app, name));
    let node = h.get_by_role_and_label(Role::TreeItem, row_name).accesskit_node().locate().0;
    craft::layers::testing::thumb_rect(&h.ctx, node, &info, part)
}

/// The rect of the thumbnail that shows `part` of the layer named `name`.
pub(crate) fn thumb(h: &Harness<'_, PhotocraftApp>, name: &str, part: LayerPart) -> Rect {
    thumb_on(h, name, name, part).unwrap_or_else(|| panic!("{name:?} shows no {part:?} thumbnail"))
}

/// A point on the name of the row of the layer named `name`, past its thumbnails: a press there
/// takes the row itself, where one on a thumbnail is the thumbnail's.
pub(crate) fn name_point(h: &Harness<'_, PhotocraftApp>, name: &str) -> Pos2 {
    let at = row(h, name);
    let parts = [LayerPart::Pixels, LayerPart::LayerMask, LayerPart::VectorMask, LayerPart::FrameShape, LayerPart::FrameContent];
    let past = parts.into_iter().filter_map(|part| thumb_on(h, name, name, part)).map(|thumb| thumb.right()).fold(at.left(), f32::max);
    Pos2::new(past + 16.0, at.center().y)
}

/// A click at `at` with `mods` held: hover, press, release, as a hand does over separate passes.
pub(crate) fn click<S>(h: &mut Harness<'_, S>, at: Pos2, mods: Modifiers) {
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: mods });
        h.step();
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

/// A drag from `from` to `to` with `mods` held: press, a move past the drag threshold, the move
/// to `to`, release there.
pub(crate) fn drag<S>(h: &mut Harness<'_, S>, from: Pos2, to: Pos2, mods: Modifiers) {
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: mods });
    h.step();
    for at in [from + egui::vec2(0.0, 8.0), to, to] {
        h.event(Event::PointerMoved(at));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: mods });
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}
