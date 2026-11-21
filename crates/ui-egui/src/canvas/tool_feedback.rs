//! The selection operation a selection tool's next gesture has, from the options bar's mode and the
//! held modifiers, and the Patch tool's cursor badge (#170).
//!
//! Photoshop marks the selection-tool cursor with a small `+` (add), `−` (subtract) or `×`
//! (intersect) reflecting the effective operation. The badge and the commit read the same rule
//! ([`selection_op`]), so the cursor shows what the gesture will do.

use craft::behavior::CursorBadge;
use craft::behavior::geometry::SelectionOperation;
use craft::keymap::Mods;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Tools whose cursor carries the selection-mode badge.
pub fn is_selection_tool(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::RectMarquee
            | Tool::EllipseMarquee
            | Tool::SingleRowMarquee
            | Tool::SingleColumnMarquee
            | Tool::Lasso
            | Tool::PolygonLasso
            | Tool::MagneticLasso
            | Tool::MagicWand
            | Tool::QuickSelection
            | Tool::ObjectSelection
    )
}

/// The operation `tool`'s gesture will use for the options-bar mode `bar` (0 New, 1 Add,
/// 2 Subtract, 3 Intersect) and the held modifiers `mods`. With nothing selected, ⌥ has nothing to
/// subtract from or intersect with, so it draws a new selection, as in Photoshop; an options-bar
/// Subtract or Intersect is kept (#1106).
pub fn selection_op(tool: Tool, bar: u8, mods: Mods, has_selection: bool) -> SelectionOperation {
    // Every selection tool, the Quick Selection and Object Selection (#12853) included, follows
    // the options bar's mode (New / Add / Subtract / Intersect): ⇧ adds, ⌥ subtracts, ⇧⌥
    // intersects, and ⌥ with nothing selected draws a new one (#1106).
    // The Quick Selection has no Intersect, as in Photoshop, and `select.quick` has no intersect
    // mode (it would add): ⇧⌥ subtracts, as ⌥ alone does (#11837).
    if tool == Tool::QuickSelection {
        let mods = if mods.shift && mods.alt { Mods { shift: false, ..mods } } else { mods };
        return SelectionOperation::resolve(SelectionOperation::ALL[usize::from(bar.min(2))], mods, has_selection);
    }
    SelectionOperation::resolve(SelectionOperation::ALL[usize::from(bar.min(3))], mods, has_selection)
}

/// The options-bar mode that applies to `tool`: the Quick Selection keeps its own New / Add /
/// Subtract mode (#11837); every other selection tool follows the document's.
pub fn bar_mode(app: &PhotocraftApp, tool: Tool) -> u8 {
    if tool == Tool::QuickSelection { app.ui.tool_options.quick_selection_mode } else { app.ui.selection_mode }
}

/// The engine's word for `op`: its selection commands call New "replace", where
/// `SelectionOperation::name` says "new".
pub fn mode_name(op: SelectionOperation) -> &'static str {
    match op {
        SelectionOperation::New => "replace",
        SelectionOperation::Add => "add",
        SelectionOperation::Subtract => "subtract",
        SelectionOperation::Intersect => "intersect",
        _ => "replace",
    }
}

/// Whether the active document has a non-empty selection. Reads the bounds the selection outline
/// caches (same key), since the canvas asks every frame while a Patch lasso is drawn or hovered
/// with ⌥.
pub fn has_selection(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| st.doc.selection.as_ref().is_some_and(|s| !app.known_bounds(u64::MAX - st.doc.id.0, s).is_empty()))
}

/// The badge the Patch or Content-Aware Move cursor shows at `at` (document point), `None` for any
/// other tool. `press` is the running drag's press point and modifiers: a drag keeps what its press
/// decided, as the release does. Over the selection without modifiers a press drags the patch
/// (an arrow for Patch, none for Content-Aware Move); otherwise the tool lassos its area in the
/// Lasso's operation (⇧ adds, ⌥ subtracts, #11024). A ⌘ press is the Move tool's, through the
/// Command stand-in.
pub fn patch_badge(app: &PhotocraftApp, at: [f64; 2], press: Option<([f64; 2], Mods)>, mods: Mods) -> Option<CursorBadge> {
    let tool = app.ui.tool;
    if !matches!(tool, Tool::Patch | Tool::ContentAwareMove) {
        return None;
    }
    let (at, mods) = press.unwrap_or((at, mods));
    if crate::retouch_ui::patch_drags_selection(app, at, mods) {
        return (tool == Tool::Patch).then_some(CursorBadge::Arrow);
    }
    selection_op(Tool::Lasso, app.ui.selection_mode, mods, has_selection(app)).badge()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALT: Mods = Mods { alt: true, ..Mods::NONE };

    /// ⌥'s "is anything selected" reads the bounds the selection outline caches, as the canvas
    /// asks every frame, and scans only a selection that changed since.
    #[test]
    fn alt_reads_the_cached_selection_bounds() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", serde_json::json!({"width": 40, "height": 30})).unwrap();
        app.run("select.rect", serde_json::json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
        assert!(has_selection(&app), "nothing cached: scanned");
        assert_eq!(selection_op(Tool::Lasso, 0, ALT, has_selection(&app)), SelectionOperation::Subtract);
        let st = app.session.active().unwrap();
        let (key, fingerprint) = (u64::MAX - st.doc.id.0, st.doc.selection.as_ref().unwrap().generation());
        // An empty box cached for this very selection is believed, so nothing is scanned.
        app.bounds_cache.insert(key, (fingerprint, photocraft_geom::Rect::EMPTY));
        assert!(!has_selection(&app), "the cached bounds are used");
        assert_eq!(selection_op(Tool::Lasso, 0, ALT, has_selection(&app)), SelectionOperation::New);
        app.run("select.rect", serde_json::json!({"x": 20, "y": 5, "width": 10, "height": 10})).unwrap();
        assert!(has_selection(&app), "a new selection is scanned again");
    }

    /// (tool, options-bar mode, modifiers, anything selected) → the operation the gesture and the
    /// badge share.
    #[test]
    fn selection_op_table() {
        use SelectionOperation::{Add, Intersect, New, Subtract};
        let (shift, both) = (Mods { shift: true, ..Mods::NONE }, Mods { shift: true, alt: true, ..Mods::NONE });
        let rows = [
            (Tool::RectMarquee, 0, Mods::NONE, true, New),
            (Tool::EllipseMarquee, 0, shift, true, Add),
            (Tool::Lasso, 0, ALT, true, Subtract),
            (Tool::PolygonLasso, 0, both, true, Intersect),
            (Tool::MagicWand, 0, ALT, true, Subtract),
            // #1106: with nothing selected ⌥ and ⇧⌥ have nothing to combine with, so they draw a new one.
            (Tool::Lasso, 0, ALT, false, New),
            (Tool::RectMarquee, 0, both, false, New),
            (Tool::RectMarquee, 0, shift, false, Add),
            // The options bar's mode shows with no key held; keys still override it, and a bar
            // Subtract or Intersect is kept with nothing selected.
            (Tool::Lasso, 2, Mods::NONE, true, Subtract),
            (Tool::Lasso, 2, ALT, false, Subtract),
            (Tool::Lasso, 2, shift, true, Add),
            (Tool::RectMarquee, 3, Mods::NONE, false, Intersect),
            (Tool::RectMarquee, 200, Mods::NONE, true, Intersect),
            // The Quick Selection follows its own bar (#11837) and the keys like the other selection
            // tools do theirs.
            (Tool::QuickSelection, 2, Mods::NONE, false, Subtract),
            (Tool::QuickSelection, 0, Mods::NONE, true, New),
            (Tool::QuickSelection, 0, shift, true, Add),
            (Tool::QuickSelection, 1, ALT, true, Subtract),
            (Tool::QuickSelection, 0, both, true, Subtract),
            // Object Selection follows the options bar and the keys like the other selection tools (#12853).
            (Tool::ObjectSelection, 3, Mods::NONE, true, Intersect),
            (Tool::ObjectSelection, 0, Mods::NONE, true, New),
            (Tool::ObjectSelection, 0, shift, true, Add),
            (Tool::ObjectSelection, 0, ALT, true, Subtract),
            (Tool::ObjectSelection, 0, both, true, Intersect),
        ];
        for (tool, bar, mods, selected, op) in rows {
            assert_eq!(selection_op(tool, bar, mods, selected), op, "{tool:?} bar={bar} {mods:?} selected={selected}");
        }
        assert_eq!([New, Add, Subtract, Intersect].map(mode_name), ["replace", "add", "subtract", "intersect"], "the engine's words");
    }
}

#[cfg(test)]
#[path = "../patch_cursor_badge_tests.rs"]
mod patch_cursor_badge_tests;
