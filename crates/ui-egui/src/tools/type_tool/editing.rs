use craft::text::ui::editor::CharToggle;
use craft::ui::runtime::focus::{self, FocusTarget};

use super::layout::{hit_layer, is_vertical, offset_at, shows_own_layout};
use super::{LayerId, PhotocraftApp, TextEdit, TextLayer, active_doc_id, byte_range, current_text, json, session_key, text_layer};

/// Marks the session's editor to take the keyboard in the pass that first shows it: the framework knows an editor's
/// doc key only once it has been shown, so a request made before that finds nothing.
pub(super) fn focus_editor(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(FOCUS_WANTED), true));
}

const FOCUS_WANTED: &str = "photocraft.type-editor-focus-wanted";

/// Moves the keyboard to the editor just shown when a session start asked for it ([`focus_editor`]).
pub(super) fn take_editor_focus(ctx: &egui::Context) {
    if ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(FOCUS_WANTED))).is_some() {
        let _ = focus::request(ctx, FocusTarget::Doc(super::controls::TYPE_EDITOR));
    }
}

/// Start editing an existing type layer. Like Photoshop, editing shows the text as the type
/// engine lays it out, so a PSD layer is re-rendered first (inside the edit session's history
/// step, so Cancel brings Photoshop's pixels back).
pub(super) fn begin_edit(app: &mut PhotocraftApp, ctx: &egui::Context, id: LayerId, key: &str) -> Result<(), String> {
    let st = app.session.active().ok_or("no document open")?;
    let doc = st.doc.clone();
    if let Some(t) = text_layer(&doc, id)
        && !shows_own_layout(&doc, t)
    {
        app.run("type.edit", json!({"layer": id.0, "coalesce": key}))?;
    }
    focus_editor(ctx);
    Ok(())
}

/// Edit the active type layer from its thumbnail, selecting all text like Photoshop. Reuse the
/// current session when already editing it, so Cancel and undo still cover the whole edit.
pub fn edit_active(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    app.ui.type_insertion_style = None;
    let st = app.session.active().ok_or("no document open")?;
    let id = st.active_layer.ok_or("no active layer")?;
    let n = text_layer(&st.doc, id).ok_or("active layer is not a type layer")?.text.chars().count();
    if app.ui.text_edit.as_ref().is_none_or(|ed| ed.layer != id.0) {
        commit(app);
        let key = session_key(app);
        begin_edit(app, ctx, id, &key)?;
        app.ui.text_edit = Some(TextEdit { layer: id.0, document: active_doc_id(app), caret: n, anchor: 0, session: key, created: false, mask: false });
    } else {
        // The editor drops a composition when the selection moves from outside.
        if let Some(ed) = app.ui.text_edit.as_mut() {
            ed.anchor = 0;
            ed.caret = n;
        }
        focus_editor(ctx);
    }
    let vertical = is_vertical(app, id);
    app.ui.text_edit_return_tool = None;
    app.ui.tool = if vertical { crate::state::Tool::VerticalType } else { crate::state::Tool::Type };
    app.ui.mask_target = false;
    app.ui.vector_mask_target = false;
    Ok(())
}

/// Double-click with the Move tool (Photoshop CC 2019+): edit the type layer under document
/// point (`x`, `y`) with the caret on the glyph there, switching to the Type tool. False when no
/// type is there.
pub fn edit_at(app: &mut PhotocraftApp, ctx: &egui::Context, x: f64, y: f64) -> bool {
    let Some(id) = hit_layer(app, x, y) else { return false };
    let previous_tool = app.ui.tool;
    commit(app);
    if app.session.select_layer(id).is_err() {
        return false;
    }
    let key = session_key(app);
    if begin_edit(app, ctx, id, &key).is_err() {
        return false;
    }
    let off = offset_at(app, id, x, y);
    app.ui.text_edit = Some(TextEdit { layer: id.0, document: active_doc_id(app), caret: off, anchor: off, session: key, created: false, mask: false });
    let vertical = is_vertical(app, id);
    if !previous_tool.is_type() {
        app.ui.text_edit_return_tool = Some(previous_tool);
    }
    app.ui.tool = if vertical { crate::state::Tool::VerticalType } else { crate::state::Tool::Type };
    app.ui.mask_target = false;
    app.ui.vector_mask_target = false;
    true
}

/// Alt+←/→: kern the pair before the caret by `by` (1/1000 em). No pair (caret at a text or line
/// edge): nothing happens, like Photoshop. Not coalesced: one history step per press.
pub(super) fn kern_pair(app: &mut PhotocraftApp, id: LayerId, caret: usize, by: f32) {
    let Some(text) = current_text(app, id) else { return };
    let before = caret.checked_sub(1).and_then(|i| text.chars().nth(i));
    let after = text.chars().nth(caret);
    if before.is_none_or(|c| c == '\n') || after.is_none_or(|c| c == '\n') {
        return;
    }
    crate::type_transform::finish(app);
    let _ = app.run("type.edit", json!({"layer": id.0, "kernPair": {"at": caret, "by": by}}));
}

/// Whether the toggle is on for a character style.
fn char_on(which: CharToggle, st: &photocraft_doc::text::CharStyle) -> bool {
    match which {
        CharToggle::AllCaps => st.caps == photocraft_doc::text::Caps::AllCaps,
        CharToggle::Underline => st.underline,
        CharToggle::FauxBold => st.faux_bold,
        CharToggle::FauxItalic => st.faux_italic,
        CharToggle::Strikethrough => st.strikethrough,
        _ => false,
    }
}

/// The `type.setStyle` parameter that sets the toggle on or off; `None` for a toggle the engine has no parameter for.
fn char_param(which: CharToggle, on: bool) -> Option<(&'static str, serde_json::Value)> {
    Some(match which {
        CharToggle::AllCaps => ("caps", json!(if on { "all" } else { "normal" })),
        CharToggle::Underline => ("underline", json!(on)),
        CharToggle::FauxBold => ("fauxBold", json!(on)),
        CharToggle::FauxItalic => ("fauxItalic", json!(on)),
        CharToggle::Strikethrough => ("strikethrough", json!(on)),
        _ => return None,
    })
}

/// The character styles of the selection, or of the typing style when the text is empty. Empty
/// when nothing is selected in non-empty text.
pub(super) fn selected_styles(t: &TextLayer, a: usize, b: usize) -> Vec<photocraft_doc::text::CharStyle> {
    let runs = t.char_runs();
    if t.text.is_empty() {
        return runs.into_iter().take(1).map(|r| r.style).collect();
    }
    let (ba, bb) = byte_range(&t.text, a, b);
    let mut at = 0usize;
    let mut out = Vec::new();
    for r in runs {
        let (rs, re) = (at, at.saturating_add(r.len));
        at = re;
        if rs < bb && re > ba {
            out.push(r.style);
        }
    }
    out
}

/// The character range the type editor's formatting keys act on: the selection, or the empty
/// text's typing style. `None` for a collapsed caret in non-empty text: Photoshop would style the
/// next characters typed, which the document model can't hold apart from the text, so the keys
/// do nothing there (rather than restyle a word the user didn't select).
fn format_range(app: &PhotocraftApp, ed: &TextEdit) -> Option<(usize, usize)> {
    let t = text_layer(&app.session.active()?.doc, LayerId(ed.layer))?;
    let n = t.text.chars().count();
    let (a, b) = (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n));
    (a < b || n == 0).then_some((a, b))
}

/// One `type.setStyle` step on the edited layer's `range`, kept apart from the typing session's
/// coalesced edit so Undo takes back one key press at a time (as with kerning).
fn set_style(app: &mut PhotocraftApp, ed: &TextEdit, range: (usize, usize), mut p: serde_json::Value) {
    crate::type_transform::finish(app);
    p["layer"] = json!(ed.layer);
    p["range"] = json!([range.0, range.1]);
    if let Err(e) = app.run("type.setStyle", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// ⇧⌘K/U/B/I/: flips a character style over the selection: off when every selected character
/// already has it, else on.
pub(crate) fn toggle_char_style(app: &mut PhotocraftApp, which: CharToggle) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let Some(range) = format_range(app, &ed) else { return };
    let Some(styles) = app.session.active().and_then(|s| text_layer(&s.doc, LayerId(ed.layer))).map(|t| selected_styles(t, range.0, range.1)) else {
        return;
    };
    let on = !styles.iter().all(|st| char_on(which, st));
    let Some((key, value)) = char_param(which, on) else { return };
    set_style(app, &ed, range, json!({ key: value }));
}

/// ⌥↑/↓ leading, ⇧⌥↑/↓ baseline shift and ⌥←/→ tracking: steps every selected character from its
/// own value (`key` is `leadingBy`, `baselineShiftBy` or `trackingBy`).
pub(crate) fn step_char_metric(app: &mut PhotocraftApp, key: &str, by: f32) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let Some(range) = format_range(app, &ed) else { return };
    // Leading steps in the points the Character panel shows for a scaled layer.
    set_style(app, &ed, range, json!({ key: by, "metricsAsShown": key == "leadingBy" }));
}

/// ⌥⇧⌘H: flips Hyphenate for the paragraphs the selection (or the caret) is in: off when all of
/// them hyphenate, else on.
pub(crate) fn toggle_hyphenate(app: &mut PhotocraftApp) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let Some(t) = app.session.active().and_then(|s| text_layer(&s.doc, LayerId(ed.layer))) else { return };
    let n = t.text.chars().count();
    let (a, b) = (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n));
    let (ba, bb) = byte_range(&t.text, a, b);
    let len = t.text.len();
    let mut at = 0usize;
    let mut all = true;
    let mut any = false;
    for r in t.paragraph_runs() {
        let (ps, pe) = (at, at.saturating_add(r.len));
        at = pe;
        let touches = if ba == bb { ps <= ba && (ba < pe || pe >= len) } else { ps < bb && pe > ba };
        if touches {
            any = true;
            all &= r.style.hyphenate;
        }
    }
    let on = !(any && all);
    set_style(app, &ed, (a, b), json!({ "hyphenate": on }));
}

/// End the editing session. A new layer left empty is deleted; a new layer is named after its text.
pub fn commit(app: &mut PhotocraftApp) {
    commit_with(app, egui::Modifiers::NONE);
}

/// [`commit`] from a click, button or key that has modifiers held. They no longer change the
/// result, since a Type Mask always replaces the selection (#12073); the parameter stays so its
/// callers don't change.
pub fn commit_with(app: &mut PhotocraftApp, _mods: egui::Modifiers) {
    app.ui.type_insertion_style = None;
    // A box drag in progress keeps the box it reached, as when every move was applied.
    crate::type_box_preview::flush(app);
    crate::type_font_preview::clear(app);
    let current = app.session.active_index();
    let Some(home) = app.ui.text_edit.as_ref().map(|ed| edit_home(app, ed)) else {
        return commit_here(app);
    };
    match home {
        Some(home) if Some(home) != current => {
            // An edit left in a background document commits there, in its own history (#6483).
            app.session.set_active(home);
            commit_here(app);
            if let Some(i) = current {
                app.session.set_active(i);
            }
        }
        Some(_) => commit_here(app),
        // Its document closed: there is nothing to commit into, and the layer id may name a
        // layer of another document.
        None => {
            crate::type_transform::reset(app);
            app.ui.text_edit = None;
            app.ui.text_edit_return_tool = None;
        }
    }
}

/// Index of the document an edit belongs to; `None` once it is closed.
pub(super) fn edit_home(app: &PhotocraftApp, ed: &TextEdit) -> Option<usize> {
    match ed.document {
        Some(d) => app.session.documents().iter().position(|st| st.doc.id.0 == d),
        None => app.session.active_index(),
    }
}

/// [`commit_with`] in the active document, which is the edit's own.
fn commit_here(app: &mut PhotocraftApp) {
    if app.ui.tool.is_type() {
        crate::type_transform::finish(app);
    }
    crate::type_transform::reset(app);
    let Some(ed) = app.ui.text_edit.take() else { return };
    if let Some(tool) = app.ui.text_edit_return_tool.take()
        && app.ui.tool.is_type()
    {
        app.ui.tool = tool;
    }
    let Some(text) = current_text(app, LayerId(ed.layer)) else { return };
    if ed.mask {
        // Type Mask: the text's glyphs replace the selection, with ⇧ or ⌥ held too, as in the
        // reference application (#12073), and the working layer goes, all in the session's one
        // history step. No text: nothing changes.
        if !text.trim().is_empty()
            && let Err(e) = app.run("type.createSelection", json!({"layer": ed.layer, "removeLayer": true, "operation": "new", "coalesce": ed.session}))
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        if current_text(app, LayerId(ed.layer)).is_some() {
            undo_session(app, &ed);
        }
        return;
    }
    // #12041: an Indexed Color or Bitmap document has no type layer to keep. The session's working
    // layer is still edited and shown while typing, as the reference application shows it, and the
    // commit makes what a Type Mask tool's commit makes: the glyphs become the selection and the
    // layer goes, in the typing session's one history step - which keeps the "New Type Layer" name
    // it was opened with, as Photoshop's state there is named.
    // `photocraft_engine::type_cmds::commits_as_selection` holds the ticket's scope - the two modes
    // and the measured horizontal point text - and why the check belongs to the commit and not to
    // the `type.create` command. Three cases are not measured in these modes and keep today's
    // behaviour: an edit of a layer the session did not create (`ed.created`), paragraph (box) and
    // vertical text, whose editability a coverage mask could not give back, and a commit with no
    // text in it, which is the empty-layer clean-up below.
    let session_layer = LayerId(ed.layer);
    let converts = app
        .session
        .active()
        .is_some_and(|st| text_layer(&st.doc, session_layer).is_some_and(|t| photocraft_engine::type_cmds::commits_as_selection(st.doc.mode, t)));
    let leaves_a_selection = ed.created && !text.trim().is_empty() && converts;
    if leaves_a_selection {
        if let Err(e) = app.run("type.createSelection", json!({"layer": ed.layer, "removeLayer": true, "operation": "new", "coalesce": ed.session})) {
            // The refused step left the document alone: keep today's type layer, so what was typed
            // is not lost with a selection that was never made.
            app.ui.status = e;
            app.ui.status_error = true;
            return;
        }
        photocraft_engine::type_cmds::name_selection_commit(&mut app.session);
        return;
    }
    if text.trim().is_empty() && ed.created {
        let _ = app.run("layer.delete", json!({"layer": ed.layer}));
    } else if ed.created {
        let name = photocraft_engine::type_cmds::layer_name(&text);
        let _ = app.run("type.edit", json!({"layer": ed.layer, "name": name, "coalesce": ed.session}));
    }
}

/// Commit the edit once its layer is no longer the one worked on (#6483): another layer became
/// active (a Layers panel menu, the canvas layer picker, automation) or another document did (a
/// tab, File ▸ New/Open). Photoshop commits then; left open, the session took single-key
/// shortcuts as typing and typed into whichever document was active.
pub fn end_if_left(app: &mut PhotocraftApp) {
    let Some(ed) = app.ui.text_edit.as_ref() else { return };
    let layer = LayerId(ed.layer);
    if edit_home(app, ed).is_none_or(|h| Some(h) != app.session.active_index()) {
        return commit(app);
    }
    let Some(st) = app.session.active() else { return };
    if st.active_layer == Some(layer) {
        return;
    }
    // The selection just made stands: ending an empty Type Mask undoes its session, and undo
    // restores the selection from before it.
    let chosen = (st.active_layer, st.selected_layers.clone(), st.layer_anchor);
    commit(app);
    let moved = app.session.active().is_some_and(|st| (st.active_layer, &st.selected_layers, st.layer_anchor) != (chosen.0, &chosen.1, chosen.2));
    if moved {
        let _ = app.session.restore_layer_selection(chosen.0, &chosen.1, chosen.2);
    }
}

/// Cancel the editing session: undo it back to where it started (removes a new layer).
pub fn cancel(app: &mut PhotocraftApp) {
    app.ui.type_insertion_style = None;
    // The session is undone below, which took the dragged box with it before too.
    crate::type_box_preview::discard(app);
    crate::type_transform::reset(app);
    let Some(ed) = app.ui.text_edit.take() else { return };
    if let Some(tool) = app.ui.text_edit_return_tool.take()
        && app.ui.tool.is_type()
    {
        app.ui.tool = tool;
    }
    undo_session(app, &ed);
}

/// Undo the session's history step, if it is still the latest.
fn undo_session(app: &mut PhotocraftApp, ed: &TextEdit) {
    let coalesced = app.session.active().is_some_and(|s| s.coalesce.as_deref() == Some(ed.session.as_str()));
    if coalesced {
        let _ = app.run("edit.undo", json!({}));
    }
}

/// #12041: what the Type tool's commit does in a document whose colour mode has no type layers to
/// keep. The engine side - the mode check, the conversion it reuses and the step's name - is pinned
/// in `photocraft-engine`'s `cmd::type_indexed_mode_tests`; this is the tool's own commit, which is
/// where the difference is measured.
#[cfg(test)]
mod indexed_commit_tests {
    use super::commit;
    use crate::PhotocraftApp;
    use crate::state::Tool;
    use crate::tools::type_tool::host::type_text;

    use crate::canvas::testing::{ToolEvent, tool_event};
    use egui::Modifiers;
    use photocraft_doc::{ColorMode, LayerContent, LayerId, TextLayer};
    use serde_json::json;

    /// A new transparent document in `mode`: `rgb` keeps today's behaviour, `indexed` and `bitmap`
    /// are the two the issue is about.
    fn app(mode: &str) -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 200, "background": "transparent"})).unwrap();
        match mode {
            "indexed" => {
                app.run("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
                assert_eq!(doc_mode(&app), ColorMode::Indexed, "the document is Indexed Color");
            }
            "bitmap" => {
                app.run("image.mode.grayscale", json!({})).unwrap();
                app.run("image.mode.bitmap", json!({"method": "threshold", "frequency": 12, "shape": "diamond"})).unwrap();
                assert_eq!(doc_mode(&app), ColorMode::Bitmap, "the document is Bitmap");
            }
            _ => assert_eq!(doc_mode(&app), ColorMode::Rgb, "the document is RGB"),
        }
        app.sync_views();
        app.ui.tool = Tool::Type;
        app
    }

    /// Opens a session with the tool in hand: a click, or a drag from `from` to `to` for a paragraph box.
    // PLAN: tools-w21's shared `type_new` helper replaces this setup.
    fn open(app: &mut PhotocraftApp, from: (f64, f64), to: (f64, f64)) {
        tool_event(app, ToolEvent::Down { x: from.0, y: from.1, pressure: 1.0 }, Modifiers::NONE);
        if from != to {
            tool_event(app, ToolEvent::Move { x: to.0, y: to.1, pressure: 1.0 }, Modifiers::NONE);
        }
        tool_event(app, ToolEvent::Up { x: to.0, y: to.1 }, Modifiers::NONE);
    }

    fn doc_mode(app: &PhotocraftApp) -> ColorMode {
        app.session.active().unwrap().doc.mode
    }

    fn text_of(app: &PhotocraftApp, id: LayerId) -> Option<TextLayer> {
        let doc = app.session.active().unwrap().doc.clone();
        super::text_layer(&doc, id).cloned()
    }

    fn text_layers(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().doc.walk().into_iter().filter(|(_, _, l)| matches!(l.content, LayerContent::Text(_))).count()
    }

    fn layers(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().doc.walk().len()
    }

    fn states(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().history.entries().len()
    }

    fn last_state(app: &PhotocraftApp) -> Option<String> {
        app.session.active().unwrap().history.entries().last().cloned()
    }

    fn selection(app: &PhotocraftApp) -> Option<photocraft_geom::Rect> {
        app.session.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds())
    }

    /// The reference application, in an Indexed Color and in a Bitmap document: clicking and typing
    /// works and the text is shown while editing, and the commit leaves no type layer but a
    /// selection in the shape of the glyphs, as the one history state named "New Type Layer".
    #[test]
    fn committing_in_indexed_color_or_bitmap_leaves_the_glyphs_selected() {
        // The bounds the same text covers where it does become a layer.
        let mut rgb = app("rgb");
        open(&mut rgb, (80.0, 120.0), (80.0, 120.0));
        type_text(&mut rgb, "Hi");
        commit(&mut rgb);
        let glyphs = rgb.session.active().unwrap().doc.layers.last().unwrap().surface().unwrap().content_bounds();
        assert_eq!(text_layers(&rgb), 1, "an RGB document keeps its type layer");
        assert!(selection(&rgb).is_none(), "and selects nothing");
        assert_eq!(last_state(&rgb).as_deref(), Some("New Type Layer"), "as the session's one state");

        for mode in ["indexed", "bitmap"] {
            let mut app = app(mode);
            let (layers_before, states_before) = (layers(&app), states(&app));

            // Typing: the working layer is there, and shown, as measured.
            open(&mut app, (80.0, 120.0), (80.0, 120.0));
            type_text(&mut app, "Hi");
            assert!(app.ui.text_edit.is_some(), "{mode}: the session is open");
            assert_eq!(text_layers(&app), 1, "{mode}: the text is edited on a layer");

            commit(&mut app);
            assert!(app.ui.text_edit.is_none(), "{mode}: the commit ends the session");
            assert_eq!(text_layers(&app), 0, "{mode}: committing leaves no type layer");
            assert_eq!(layers(&app), layers_before, "{mode}: the working layer went with it");
            assert_eq!(selection(&app), Some(glyphs), "{mode}: the typed glyphs are the selection");
            assert_eq!(states(&app), states_before + 1, "{mode}: one state for the session");
            assert_eq!(last_state(&app).as_deref(), Some("New Type Layer"), "{mode}: and its name");

            app.run("edit.undo", json!({})).unwrap();
            assert!(selection(&app).is_none(), "{mode}: one undo takes the selection");
            assert_eq!(layers(&app), layers_before, "{mode}: and the whole session");
        }
    }

    /// #12041 measures the horizontal point text you click and type. Paragraph (box) text and
    /// vertical type are on its *Not measured* list, whose rule is that they keep today's
    /// behaviour - and today's behaviour is the right one here anyway: a coverage mask cannot bring
    /// back a box that reflows or a column that runs down the page, so their editable layer stays.
    #[test]
    fn paragraph_and_vertical_text_keep_their_type_layer_in_indexed_and_bitmap() {
        use photocraft_doc::text::{Orientation, TextShape};

        for mode in ["indexed", "bitmap"] {
            // A dragged paragraph box.
            let mut boxed = app(mode);
            open(&mut boxed, (40.0, 60.0), (320.0, 180.0));
            type_text(&mut boxed, "Hi");
            let box_layer = LayerId(boxed.ui.text_edit.as_ref().unwrap().layer);
            assert!(matches!(text_of(&boxed, box_layer).map(|t| t.shape), Some(TextShape::Box { .. })), "{mode}: the setup made paragraph text");
            commit(&mut boxed);
            assert_eq!(text_layers(&boxed), 1, "{mode}: paragraph text keeps its editable layer");
            assert!(selection(&boxed).is_none(), "{mode}: and is not turned into a selection");

            // Vertical point text.
            let mut column_app = app(mode);
            column_app.ui.tool = Tool::VerticalType;
            open(&mut column_app, (80.0, 120.0), (80.0, 120.0));
            type_text(&mut column_app, "Hi");
            let column = LayerId(column_app.ui.text_edit.as_ref().unwrap().layer);
            assert!(text_of(&column_app, column).is_some_and(|t| t.orientation == Orientation::Vertical), "{mode}: the setup made vertical text");
            commit(&mut column_app);
            assert_eq!(text_layers(&column_app), 1, "{mode}: vertical text keeps its editable layer");
            assert!(selection(&column_app).is_none(), "{mode}: and is not turned into a selection");
        }
    }
}
