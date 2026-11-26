//! #12508: the modifiers held when a Type Mask starts decide how its commit combines with the
//! selection that is already there - ⇧ at the click adds its glyphs, ⌥ at the start of a drag
//! subtracts them. A mask started without modifiers still replaces the selection (#12073), and ⇧ or
//! ⌥ held only at the commit keeps replacing. The cases the issue marks *not measured* keep today's
//! behaviour and are pinned here: ⌥ at a click without a drag, ⇧⌥ at the start, ⇧ at the start of a
//! box drag, and ⌥ at the start of a drag with no selection to subtract from.
//!
//! The expected selection is the engine's own `type.createSelection` with that `operation`, run on
//! the working layer of the same session in the same document: the combining is our own state's
//! arithmetic, not a measured output geometry.

use egui::Modifiers;
use photocraft_algo::selection::mask_from_surface;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing::{ToolEvent, tool_event};
use crate::state::Tool;

use super::mask_replace_selection_tests::{bounds, click, new_app, selection};

/// Replace today's selection by one over the whole document, the state the Alt+drag case is measured
/// with.
fn select_all(app: &mut PhotocraftApp, w: f64, h: f64) {
    app.run("select.rect", json!({"x": 0.0, "y": 0.0, "width": w, "height": h})).unwrap();
    app.sync_views();
}

/// `new_app` with nothing selected: the state a ⌥+drag has no measured rule for.
fn no_selection_app() -> Box<PhotocraftApp> {
    let mut app = new_app();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    assert!(selection(&app).is_none(), "no selection to combine with");
    app
}

/// Start a mask with a click at `at` holding `mods`, and type `s` into it.
fn point_mask(app: &mut PhotocraftApp, at: (f64, f64), s: &str, mods: Modifiers) {
    click(app, at.0, at.1, mods);
    assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask), "the click starts a Type Mask session");
    super::host::type_text(app, s);
}

/// Start a mask with a drag from `from` to `to` holding `mods` at press and release, and type `s`.
fn box_mask(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], s: &str, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
    assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask), "the drag starts a Type Mask session");
    super::host::type_text(app, s);
}

/// Commit the open mask through the tool's own commit.
fn commit(app: &mut PhotocraftApp, mods: Modifiers) {
    super::commit_with(app, mods);
    assert!(app.ui.text_edit.is_none(), "the commit ended the session");
}

/// What the commit of `app`'s open mask does in the engine when it is told `operation` on the very
/// same working layer: the combining the tool's commit is expected to reach by itself.
fn combine_with(app: &mut PhotocraftApp, operation: &str) -> Option<Surface> {
    let ed = app.ui.text_edit.take().expect("an open mask session");
    app.run("type.createSelection", json!({"layer": ed.layer, "removeLayer": true, "operation": operation})).unwrap();
    selection(app)
}

/// The selection's coverage over `area`, as a float mask.
fn coverage(app: &PhotocraftApp, area: Rect) -> Vec<f32> {
    let st = app.session.active().unwrap();
    mask_from_surface(st.doc.selection.as_ref(), area)
}

/// The History entries of the active document.
fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

// ---------------------------------------------------------------- the two measured start modifiers

/// Done when, measured: rectangle selection 600,250 to 700,350, ⇧ down, click at (100,200), ⇧ up,
/// type "AB", commit - the union of rectangle and glyphs.
#[test]
fn shift_at_the_click_that_starts_a_mask_adds_the_glyphs_to_the_selection() {
    let mut oracle_app = new_app();
    point_mask(&mut oracle_app, (100.0, 200.0), "AB", Modifiers::NONE);
    let added = combine_with(&mut oracle_app, "add");

    let mut app = new_app();
    let rect = bounds(&selection(&app)).expect("the rectangle selection");
    point_mask(&mut app, (100.0, 200.0), "AB", Modifiers::SHIFT);
    commit(&mut app, Modifiers::NONE);

    let got = bounds(&selection(&app)).expect("a selection after the commit");
    assert_eq!(selection(&app), added, "the ⇧ start adds: {:?} is not the union {:?}", got, bounds(&added));
    assert_eq!(got, rect.union(&bounds(&added).expect("the added selection has bounds")), "rectangle and glyphs");
    assert!(coverage(&app, rect).iter().all(|v| *v > 0.99), "the rectangle is still selected");
}

/// The same for the Vertical Type Mask tool, which the issue measured alike.
#[test]
fn shift_at_the_click_that_starts_a_vertical_mask_adds_the_glyphs() {
    let mut oracle_app = new_app();
    oracle_app.ui.tool = Tool::VerticalTypeMask;
    point_mask(&mut oracle_app, (100.0, 200.0), "AB", Modifiers::NONE);
    let added = combine_with(&mut oracle_app, "add");

    let mut app = new_app();
    app.ui.tool = Tool::VerticalTypeMask;
    let rect = bounds(&selection(&app)).expect("the rectangle selection");
    point_mask(&mut app, (100.0, 200.0), "AB", Modifiers::SHIFT);
    commit(&mut app, Modifiers::NONE);

    assert_eq!(selection(&app), added, "the vertical mask's ⇧ start adds too");
    assert!(coverage(&app, rect).iter().all(|v| *v > 0.99), "the rectangle is still selected");
}

/// Done when: the start's ⇧ holds "whatever is held at commit" - the commit's own modifiers keep
/// having no say (#12073).
#[test]
fn shift_at_the_start_adds_whatever_is_held_at_the_commit() {
    let mut oracle_app = new_app();
    point_mask(&mut oracle_app, (100.0, 200.0), "AB", Modifiers::NONE);
    let added = combine_with(&mut oracle_app, "add");

    for mods in [Modifiers::SHIFT, Modifiers::ALT, Modifiers { shift: true, alt: true, ..Modifiers::NONE }] {
        let mut app = new_app();
        point_mask(&mut app, (100.0, 200.0), "AB", Modifiers::SHIFT);
        commit(&mut app, mods);
        assert_eq!(selection(&app), added, "start ⇧ with {mods:?} at the commit still adds");
    }
}

/// Done when, measured: full-canvas selection, ⌥ down, drag (450,300) to (700,550) ... - the bounds
/// stay the whole canvas and the selected pixels fall, the glyphs cut out as holes. Here the drag
/// starts the mask's box directly, as the Horizontal Type Mask tool's Alt+drag does.
#[test]
fn alt_at_the_start_of_a_drag_subtracts_the_glyphs_from_the_selection() {
    let mut oracle_app = new_app();
    select_all(&mut oracle_app, 800.0, 400.0);
    box_mask(&mut oracle_app, [100.0, 100.0], [400.0, 300.0], "ABCD", Modifiers::NONE);
    let subtracted = combine_with(&mut oracle_app, "subtract");

    let mut app = new_app();
    select_all(&mut app, 800.0, 400.0);
    let whole = bounds(&selection(&app)).expect("the whole-canvas selection");
    let before: f64 = coverage(&app, whole).iter().map(|v| f64::from(*v)).sum();
    box_mask(&mut app, [100.0, 100.0], [400.0, 300.0], "ABCD", Modifiers::ALT);
    commit(&mut app, Modifiers::NONE);

    let after: f64 = coverage(&app, whole).iter().map(|v| f64::from(*v)).sum();
    assert_eq!(bounds(&selection(&app)), Some(whole), "the bounds stay the whole canvas");
    assert!(after < before - 100.0, "the glyphs are cut out: {before} selected, {after} after the commit");
    assert_eq!(selection(&app), subtracted, "the same combining as the engine's own subtract");
}

/// The Vertical Type Mask tool's Alt+drag goes through the Paragraph Text Size dialog (#12090) and
/// the mask still subtracts, because the dialog creates the box the drag started.
#[test]
fn alt_at_the_start_of_a_vertical_mask_drag_subtracts_the_glyphs() {
    let mut app = new_app();
    app.ui.tool = Tool::VerticalTypeMask;
    select_all(&mut app, 800.0, 400.0);
    let whole = bounds(&selection(&app)).expect("the whole-canvas selection");
    let before: f64 = coverage(&app, whole).iter().map(|v| f64::from(*v)).sum();

    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Move { x: 400.0, y: 300.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: 400.0, y: 300.0 }, Modifiers::ALT);
    let open = app.framework.dialogs.open_dialogs();
    assert_eq!(open.len(), 1, "the Alt+drag opens the Paragraph Text Size dialog");
    let id = open[0].id;
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask), "OK starts the mask session");
    super::host::type_text(&mut app, "ABCD");
    commit(&mut app, Modifiers::NONE);

    let after: f64 = coverage(&app, whole).iter().map(|v| f64::from(*v)).sum();
    assert_eq!(bounds(&selection(&app)), Some(whole), "the bounds stay the whole canvas");
    assert!(after < before - 100.0, "the dialog's mask subtracts too: {before} selected, {after} after the commit");
}

/// Done when: the add is part of the mask's one history step, and so is the subtract.
#[test]
fn the_added_and_subtracted_glyphs_stay_in_the_mask_s_one_history_step() {
    let mut app = new_app();
    let start = steps(&app);
    point_mask(&mut app, (100.0, 200.0), "AB", Modifiers::SHIFT);
    commit(&mut app, Modifiers::NONE);
    assert_eq!(steps(&app), start + 1, "⇧ start: typing and the add are one state");
    assert_eq!(app.session.active().unwrap().history.entries().last().map(String::as_str), Some("Type Mask"));

    select_all(&mut app, 800.0, 400.0);
    let start = steps(&app);
    box_mask(&mut app, [100.0, 100.0], [400.0, 300.0], "ABCD", Modifiers::ALT);
    commit(&mut app, Modifiers::NONE);
    assert_eq!(steps(&app), start + 1, "⌥ start: typing and the subtract are one state");
    assert_eq!(app.session.active().unwrap().history.entries().last().map(String::as_str), Some("Type Mask"));
}

// ---------------------------------------------------------------- the guards the issue names

/// Done when: a mask started without modifiers still replaces the selection.
#[test]
fn a_mask_started_without_modifiers_still_replaces_the_selection() {
    let mut oracle_app = new_app();
    point_mask(&mut oracle_app, (100.0, 200.0), "AB", Modifiers::NONE);
    let replaced = combine_with(&mut oracle_app, "new");

    let mut app = new_app();
    point_mask(&mut app, (100.0, 200.0), "AB", Modifiers::NONE);
    commit(&mut app, Modifiers::NONE);
    assert_eq!(selection(&app), replaced, "the glyphs alone");
    assert!(bounds(&selection(&app)).is_some_and(|r| r.x1 < 600), "the rectangle is gone");
}

/// Not measured: ⌥ at a click without a drag (one run replaced, a second committed nothing) keeps
/// today's replace.
#[test]
fn alt_at_a_click_without_a_drag_still_replaces_the_selection() {
    let mut oracle_app = new_app();
    point_mask(&mut oracle_app, (100.0, 200.0), "AB", Modifiers::NONE);
    let replaced = combine_with(&mut oracle_app, "new");

    let mut app = new_app();
    point_mask(&mut app, (100.0, 200.0), "AB", Modifiers::ALT);
    commit(&mut app, Modifiers::NONE);
    assert_eq!(selection(&app), replaced, "⌥ at a click is not the measured ⌥+drag");
}

/// Not measured: ⇧⌥ at the start (one run replaced; intersect was never seen) keeps today's replace.
#[test]
fn shift_and_alt_at_the_start_still_replace_the_selection() {
    let mut oracle_app = new_app();
    point_mask(&mut oracle_app, (100.0, 200.0), "AB", Modifiers::NONE);
    let replaced = combine_with(&mut oracle_app, "new");

    let mut app = new_app();
    point_mask(&mut app, (100.0, 200.0), "AB", Modifiers { shift: true, alt: true, ..Modifiers::NONE });
    commit(&mut app, Modifiers::NONE);
    assert_eq!(selection(&app), replaced, "no intersect: the issue never saw one");
}

/// Not measured: ⇧ held at the start of a drag (box) rather than a click keeps today's replace.
#[test]
fn shift_at_the_start_of_a_drag_still_replaces_the_selection() {
    let mut oracle_app = new_app();
    select_all(&mut oracle_app, 800.0, 400.0);
    box_mask(&mut oracle_app, [100.0, 100.0], [400.0, 300.0], "ABCD", Modifiers::NONE);
    let replaced = combine_with(&mut oracle_app, "new");

    let mut app = new_app();
    select_all(&mut app, 800.0, 400.0);
    box_mask(&mut app, [100.0, 100.0], [400.0, 300.0], "ABCD", Modifiers::SHIFT);
    commit(&mut app, Modifiers::NONE);
    assert_eq!(selection(&app), replaced, "the measured add is the ⇧ click, not a ⇧ box drag");
}

/// The measured ⌥+drag case is "(with a selection)". With no selection to subtract from, the commit
/// keeps today's replace.
#[test]
fn alt_at_the_start_of_a_drag_without_a_selection_still_replaces_the_selection() {
    let mut oracle_app = no_selection_app();
    box_mask(&mut oracle_app, [100.0, 100.0], [400.0, 300.0], "ABCD", Modifiers::NONE);
    let replaced = combine_with(&mut oracle_app, "new");

    let mut app = no_selection_app();
    box_mask(&mut app, [100.0, 100.0], [400.0, 300.0], "ABCD", Modifiers::ALT);
    commit(&mut app, Modifiers::NONE);
    assert_eq!(selection(&app), replaced, "nothing to subtract from: the glyphs are selected as today");
}

/// The Type tool itself is untouched: a ⇧ click with it makes a type layer, keeps it, and leaves the
/// selection as it was.
#[test]
fn the_type_tool_itself_still_leaves_the_selection_as_it_was() {
    let mut app = new_app();
    app.ui.tool = Tool::Type;
    let rect = bounds(&selection(&app)).expect("the rectangle selection");
    let before = app.session.active().unwrap().doc.layers.len();
    click(&mut app, 100.0, 200.0, Modifiers::SHIFT);
    assert!(app.ui.text_edit.as_ref().is_some_and(|ed| !ed.mask), "a Type tool session, not a mask");
    super::host::type_text(&mut app, "AB");
    commit(&mut app, Modifiers::NONE);
    assert_eq!(bounds(&selection(&app)), Some(rect), "the selection is left as it was");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), before + 1, "and the type layer is kept");
}
