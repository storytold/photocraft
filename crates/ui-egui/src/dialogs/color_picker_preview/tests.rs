use craft::ui::dialog::DialogId;

use super::*;
use photocraft_doc::{LayerContent, TextLayer};
use serde_json::json;

thread_local! {
    /// How often this thread built a preview copy (`display_doc` runs the command).
    pub(super) static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn app_at(depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200, "depth": depth})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    app
}

fn new_type_layer(app: &mut PhotocraftApp, text: &str) -> LayerId {
    LayerId(app.run("type.create", json!({"text": text, "color": "#000000"})).unwrap()["layer"].as_u64().unwrap())
}

fn text_in(doc: &Document, id: LayerId) -> TextLayer {
    match &doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.clone(),
        _ => panic!("not a type layer"),
    }
}

/// The colour of character `ci` (a char index) in `doc`'s type layer `id`.
fn rgb_at(doc: &Document, id: LayerId, ci: usize) -> [u8; 4] {
    let t = text_in(doc, id);
    let b = t.text.char_indices().nth(ci).map_or(t.text.len(), |(b, _)| b);
    let mut at = 0;
    for r in t.char_runs() {
        if b < at + r.len {
            return r.style.color.to_rgba8();
        }
        at += r.len;
    }
    [0; 4]
}

/// Open the text Color Picker and move it to `color`, as dragging in its field does.
fn pick(app: &mut PhotocraftApp, color: &str) -> DialogId {
    crate::type_tool::open_color_picker(app, [0.0; 3]);
    let dialog = crate::color_picker_ui::top(app).expect("the text Color Picker opens");
    set_color(app, color);
    dialog
}

/// Moves the top Color Picker to `color`.
fn set_color(app: &mut PhotocraftApp, color: &str) {
    let picker = crate::color_picker_ui::top(app).expect("a Color Picker is open");
    crate::color_picker_ui::testing::set_color(app, picker, color);
}

/// The Color Picker on top, which a test that opened it through an editor entry point did not get the id of.
fn top_picker(app: &PhotocraftApp) -> DialogId {
    crate::color_picker_ui::top(app).expect("a Color Picker is open")
}

/// What the session holds: document, revision, dirty flag and history length.
fn committed(app: &PhotocraftApp) -> (Arc<Document>, u64, bool, usize) {
    let st = app.session.active().unwrap();
    (st.doc.clone(), st.revision, st.is_dirty(), st.history.entries().len())
}

#[test]
fn the_canvas_shows_the_picked_colour_before_ok_without_touching_the_session() {
    for depth in [8, 16, 32] {
        let mut app = app_at(depth);
        let id = new_type_layer(&mut app, "Hello");
        let (doc, revision, dirty, history) = committed(&app);
        pick(&mut app, "#e51c33");

        let (shown, key) = crate::canvas::display_doc(&mut app, 0);
        assert_ne!(key & TAG, 0, "{depth}-bit: the canvas draws the live preview, keyed apart from the document");
        assert_eq!(rgb_at(&shown, id, 0), [229, 28, 51, 255], "{depth}-bit: the canvas shows the picked colour");
        assert_eq!(rgb_at(&shown, id, 4), [229, 28, 51, 255]);
        let after = committed(&app);
        assert!(Arc::ptr_eq(&after.0, &doc), "{depth}-bit: the document is not edited");
        assert_eq!((after.1, after.2, after.3), (revision, dirty, history), "{depth}-bit: no revision, dirty flag or history step");
        assert_eq!(rgb_at(&app.session.active().unwrap().doc, id, 0), [0, 0, 0, 255]);
    }
}

#[test]
fn cancel_leaves_the_text_and_history_as_they_were_and_stops_the_preview() {
    let mut app = app_at(8);
    let id = new_type_layer(&mut app, "Hello");
    let (doc, revision, dirty, history) = committed(&app);
    let dialog = pick(&mut app, "#00ff00");
    assert!(display_doc(&mut app, 0).is_some());

    crate::dialogs::testing::cancel(&mut app, dialog).unwrap();
    assert!(display_doc(&mut app, 0).is_none(), "Cancel: the canvas draws the document again");
    let after = committed(&app);
    assert!(Arc::ptr_eq(&after.0, &doc));
    assert_eq!((after.1, after.2, after.3), (revision, dirty, history));
    assert_eq!(rgb_at(&after.0, id, 0), [0, 0, 0, 255]);
}

#[test]
fn ok_commits_exactly_what_the_preview_showed_in_one_history_step() {
    let mut app = app_at(8);
    let id = new_type_layer(&mut app, "Hello");
    let history = committed(&app).3;
    let dialog = pick(&mut app, "#ff8800");
    let (shown, _) = display_doc(&mut app, 0).unwrap();

    crate::dialogs::testing::confirm(&mut app, dialog).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    assert_eq!(text_in(&doc, id).char_runs(), text_in(&shown, id).char_runs(), "the preview is the committed edit");
    assert_eq!(committed(&app).3, history + 1);
    assert!(display_doc(&mut app, 0).is_none(), "no preview once the picker has closed");
}

#[test]
fn the_preview_follows_every_colour_and_stops_at_the_opening_one() {
    let mut app = app_at(8);
    let id = new_type_layer(&mut app, "Hello");
    pick(&mut app, "#ff0000");
    let (red, red_key) = display_doc(&mut app, 0).unwrap();
    assert_eq!(rgb_at(&red, id, 0), [255, 0, 0, 255]);
    let (again, again_key) = display_doc(&mut app, 0).unwrap();
    assert!(Arc::ptr_eq(&red, &again) && red_key == again_key, "an unchanged colour reuses its preview");

    set_color(&mut app, "#0000ff");
    let (blue, blue_key) = display_doc(&mut app, 0).unwrap();
    assert_ne!(blue_key, red_key);
    assert_eq!(rgb_at(&blue, id, 0), [0, 0, 255, 255]);

    // Back to the colour the picker opened on: nothing to preview, the document shows.
    set_color(&mut app, "#000000");
    assert!(display_doc(&mut app, 0).is_none());
}

#[test]
fn only_the_selected_characters_are_previewed_in_the_new_colour() {
    let mut app = app_at(8);
    let id = new_type_layer(&mut app, "Hello world");
    app.ui.text_edit = Some(crate::state::TextEdit {
        layer: id.0,
        document: None,
        caret: 11,
        anchor: 6,
        session: "type-color-preview-test".into(),
        created: false,
        mask: false,
    });
    let history = committed(&app).3;
    pick(&mut app, "#00ff00");
    let (shown, _) = display_doc(&mut app, 0).unwrap();
    assert_eq!(rgb_at(&shown, id, 5), [0, 0, 0, 255], "outside the selection");
    assert_eq!(rgb_at(&shown, id, 6), [0, 255, 0, 255], "inside the selection");
    assert_eq!(rgb_at(&shown, id, 10), [0, 255, 0, 255]);
    assert_eq!(committed(&app).3, history, "the typing session's history is untouched");
}

#[test]
fn no_preview_for_other_pickers_other_tabs_or_other_commands() {
    let mut app = app_at(8);
    let id = new_type_layer(&mut app, "Hello");

    // The foreground Color Picker previews nothing on the canvas.
    let fg = crate::color_picker_ui::open(&mut app, "foreground");
    set_color(&mut app, "#ff0000");
    assert!(display_doc(&mut app, 0).is_none());
    crate::dialogs::testing::cancel(&mut app, fg).unwrap();

    // A picker that runs a command this module does not preview shows nothing either.
    let other = crate::color_picker_ui::open_for_command(&mut app, "Color Picker (Preset Stop)", [0.0; 3], "gradient.presets.stop", json!({"layer": id.0}));
    set_color(&mut app, "#ff0000");
    assert!(display_doc(&mut app, 0).is_none(), "a preset's stop edits no pixels");
    crate::dialogs::testing::cancel(&mut app, other).unwrap();

    // Another document's tab never shows this picker's colour.
    pick(&mut app, "#ff0000");
    assert!(display_doc(&mut app, 1).is_none());
    assert!(display_doc(&mut app, 0).is_some(), "the text picker itself previews on its own tab");
    assert_eq!(rgb_at(&app.session.active().unwrap().doc, id, 0), [0, 0, 0, 255]);
}

#[test]
fn damage_between_previews_is_the_text_layer_only() {
    let mut app = app_at(8);
    let id = new_type_layer(&mut app, "Hello");
    let (doc_id, revision) = {
        let st = app.session.active().unwrap();
        (st.doc.id, st.revision)
    };
    let bounds = app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().tile_bounds();
    pick(&mut app, "#ff0000");
    let (_, red) = display_doc(&mut app, 0).unwrap();
    set_color(&mut app, "#0000ff");
    let (_, blue) = display_doc(&mut app, 0).unwrap();

    let between = damage(&app, doc_id, revision, red, blue).expect("both previews are ours");
    assert_eq!(between, bounds);
    assert_eq!(damage(&app, doc_id, revision, 0, red), Some(bounds), "document to preview");
    assert_eq!(damage(&app, doc_id, revision, blue, 0), Some(bounds), "preview back to the document (Cancel)");
    assert!(damage(&app, doc_id, revision, red, red).is_none());
    assert!(damage(&app, doc_id, revision + 1, red, blue).is_none(), "another revision isn't ours");
    assert!(damage(&app, doc_id, revision, red, 12_345 | TAG).is_none(), "an unknown key isn't ours");

    // The canvas asks through `damage_since`, with the display's key folded into what it cached.
    let doc = app.session.active().unwrap().doc.clone();
    let dk = crate::canvas::canvas_display(&app, &doc, None).1;
    let since = |seen: u64, now: u64| crate::canvas::damage_since(&app, 0, (revision, seen ^ dk), (revision, now), dk, (revision, None));
    assert_eq!(since(red, blue), Some(bounds), "the canvas refreshes only the text layer between colours");
    assert_eq!(since(blue, 0), Some(bounds), "and on Cancel");
}

#[test]
fn free_transform_keeps_its_preview_while_the_text_colour_picker_is_open() {
    let mut app = app_at(8);
    new_type_layer(&mut app, "Hello");
    let ctx = app.ctx().clone();
    crate::transform_tool::begin(&mut app, &ctx).expect("Free Transform opens on the type layer");
    pick(&mut app, "#ff0000");

    // The transform's own preview shows the layer where the box puts it; a copy with the old
    // placement under it would show the text twice.
    let (_, key) = crate::canvas::display_doc(&mut app, 0);
    let session = app.transform_preview.as_ref().expect("the transform preview").session;
    assert_eq!(key, (1 << 40) + session, "the canvas keeps drawing the Free Transform preview");
    assert!(display_doc(&mut app, 0).is_none());
}

#[test]
fn ok_counts_the_previewed_canvas_as_the_document_so_only_the_layer_is_redrawn() {
    for gradient in [false, true] {
        let mut app = app_at(8);
        let ctx = app.ctx().clone();
        let dialog = if gradient {
            let id = new_gradient_fill(&mut app);
            crate::gradient_ui::edit_stop_color(&mut app, id, 0);
            set_color(&mut app, "#ff0000");
            top_picker(&app)
        } else {
            new_type_layer(&mut app, "Hello");
            pick(&mut app, "#ff0000")
        };
        // The canvas draws the preview, then OK commits it.
        drop(crate::canvas::CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap());
        let doc_id = app.session.active().unwrap().doc.id;
        let cached = |app: &PhotocraftApp| app.raster_logs[&crate::canvas::cache_key(doc_id, None)].composite_seen().unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        let dk = crate::canvas::canvas_display(&app, &doc, None).1;
        assert_ne!(cached(&app).1, dk, "gradient={gradient}: the canvas holds the preview");

        crate::dialogs::testing::confirm(&mut app, dialog).unwrap();
        let revision = app.session.active().unwrap().revision;
        let seen = cached(&app);
        assert_eq!(seen.1, dk, "gradient={gradient}: OK's edit is what the canvas already shows");
        // So the commit's own damage is all the canvas refreshes, not the whole document.
        let edit = photocraft_geom::Rect::new(1, 2, 3, 4);
        let damage = crate::canvas::damage_since(&app, 0, seen, (revision, 0), dk, (revision - 1, Some(edit)));
        assert_eq!(damage, Some(edit), "gradient={gradient}");
    }
}

#[test]
fn ok_after_a_colour_the_canvas_never_drew_redraws_as_before() {
    let mut app = app_at(8);
    let ctx = app.ctx().clone();
    new_type_layer(&mut app, "Hello");
    let dialog = pick(&mut app, "#ff0000");
    drop(crate::canvas::CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap());
    let doc = app.session.active().unwrap().doc.clone();
    let dk = crate::canvas::canvas_display(&app, &doc, None).1;
    // The colour changes and OK lands before the canvas drew it: the shown copy is not the edit.
    set_color(&mut app, "#0000ff");
    crate::dialogs::testing::confirm(&mut app, dialog).unwrap();
    let seen = app.raster_logs[&crate::canvas::cache_key(doc.id, None)].composite_seen().unwrap();
    assert_ne!(seen.1, dk, "a stale red preview is never counted as the blue document");
}

#[test]
fn a_refused_preview_is_not_rebuilt_every_frame() {
    let mut app = app_at(8);
    new_type_layer(&mut app, "Hello");
    crate::color_picker_ui::open_for_command(&mut app, "Color Picker (Text Color)", [0.0; 3], "type.setStyle", json!({"layer": 999_999}));
    set_color(&mut app, "#ff0000");

    BUILDS.with(|b| b.set(0));
    assert!(display_doc(&mut app, 0).is_none(), "the command refuses a missing layer");
    assert!(display_doc(&mut app, 0).is_none());
    assert_eq!(crate::canvas::display_doc(&mut app, 0).1, 0, "the canvas draws the document");
    assert_eq!(BUILDS.with(|b| b.get()), 1, "the refusal is remembered for this colour");

    // Another colour is another preview: it is tried once.
    set_color(&mut app, "#00ff00");
    assert!(display_doc(&mut app, 0).is_none());
    assert!(display_doc(&mut app, 0).is_none());
    assert_eq!(BUILDS.with(|b| b.get()), 2);
}

#[test]
fn every_selected_type_layer_is_previewed_and_refreshed() {
    let mut app = app_at(8);
    let a = new_type_layer(&mut app, "Hello");
    let b = new_type_layer(&mut app, "World");
    app.run("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    let (doc_id, revision, bounds) = {
        let st = app.session.active().unwrap();
        let area = |id: LayerId| st.doc.layer(id).unwrap().surface().unwrap().tile_bounds();
        (st.doc.id, st.revision, area(a).union(&area(b)))
    };
    pick(&mut app, "#ff0000");
    let (shown, red) = display_doc(&mut app, 0).unwrap();
    assert_eq!(rgb_at(&shown, a, 0), [255, 0, 0, 255]);
    assert_eq!(rgb_at(&shown, b, 0), [255, 0, 0, 255]);
    // The recoloured copies may re-render with wider tiles; the damage must cover both layers.
    let d = damage(&app, doc_id, revision, 0, red).expect("both layers are ours");
    assert_eq!(d.union(&bounds), d, "covers both layers' bounds: {d:?} vs {bounds:?}");
}

/// Colour stop `i` (by location) of gradient fill layer `id`, as 8-bit RGB.
fn stop_rgb(doc: &Document, id: LayerId, i: usize) -> [u8; 3] {
    let LayerContent::Fill(photocraft_doc::Fill::Gradient { stops, .. }) = &doc.layer(id).unwrap().content else { panic!("not a gradient fill") };
    let mut sorted = stops.clone();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    sorted[i].1.to_rgb().map(|v| (v * 255.0).round() as u8)
}

fn new_gradient_fill(app: &mut PhotocraftApp) -> LayerId {
    LayerId(app.run("layer.newFillLayer.gradient", json!({"from": "#000000", "to": "#ffffff"})).unwrap()["layer"].as_u64().unwrap())
}

#[test]
fn a_gradient_stop_colour_is_previewed_before_ok_and_cancel_changes_nothing() {
    for depth in [8, 16, 32] {
        let mut app = app_at(depth);
        let id = new_gradient_fill(&mut app);
        let (doc, revision, dirty, history) = committed(&app);
        crate::gradient_ui::edit_stop_color(&mut app, id, 0);
        let dialog = top_picker(&app);
        assert!(crate::color_picker_ui::target(&app, dialog).is_some_and(owns), "{depth}-bit: the stop picker is previewed");
        set_color(&mut app, "#ff0000");

        let (shown, key) = crate::canvas::display_doc(&mut app, 0);
        assert_ne!(key & TAG, 0, "{depth}-bit: the canvas draws the live preview");
        assert_eq!(stop_rgb(&shown, id, 0), [255, 0, 0], "{depth}-bit: the canvas shows the picked stop colour");
        assert_eq!(stop_rgb(&shown, id, 1), [255, 255, 255], "{depth}-bit: the other stop is unchanged");
        let after = committed(&app);
        assert!(Arc::ptr_eq(&after.0, &doc));
        assert_eq!((after.1, after.2, after.3), (revision, dirty, history), "{depth}-bit: the session is untouched");

        crate::dialogs::testing::cancel(&mut app, dialog).unwrap();
        assert!(display_doc(&mut app, 0).is_none(), "{depth}-bit: Cancel shows the document again");
        assert_eq!(crate::canvas::display_doc(&mut app, 0).1, 0, "{depth}-bit: the canvas draws the document");
        assert_eq!(stop_rgb(&app.session.active().unwrap().doc, id, 0), [0, 0, 0]);
    }
}

#[test]
fn ok_commits_the_previewed_gradient_stop_in_one_history_step() {
    let mut app = app_at(8);
    let id = new_gradient_fill(&mut app);
    let history = committed(&app).3;
    crate::gradient_ui::edit_stop_color(&mut app, id, 1);
    let dialog = top_picker(&app);
    set_color(&mut app, "#00ff00");
    let (shown, _) = display_doc(&mut app, 0).unwrap();

    crate::dialogs::testing::confirm(&mut app, dialog).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    assert_eq!(stop_rgb(&doc, id, 1), stop_rgb(&shown, id, 1), "the preview is the committed edit");
    assert_eq!(stop_rgb(&doc, id, 1), [0, 255, 0]);
    assert_eq!(committed(&app).3, history + 1);
}

#[test]
fn the_preview_never_shows_on_a_document_switched_to_while_the_picker_is_open() {
    let mut app = app_at(8);
    let id = new_type_layer(&mut app, "Hello");
    pick(&mut app, "#ff0000");
    assert!(display_doc(&mut app, 0).is_some());

    // A second document becomes the active tab, with a type layer that may share the id.
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let other = new_type_layer(&mut app, "Other");
    let idx = app.session.active_index().unwrap();
    assert_ne!(idx, 0);
    assert!(display_doc(&mut app, idx).is_none(), "the picker belongs to the first document");
    assert!(display_doc(&mut app, 0).is_none(), "an inactive tab shows no dialog preview");
    assert_eq!(rgb_at(&app.session.active().unwrap().doc, other, 0), [0, 0, 0, 255]);

    // Back on the document it was opened on, the preview returns.
    assert!(app.session.set_active(0));
    let (shown, _) = display_doc(&mut app, 0).expect("preview on its own document");
    assert_eq!(rgb_at(&shown, id, 0), [255, 0, 0, 255]);
}

#[test]
fn no_preview_when_the_params_name_no_layer() {
    let mut app = app_at(8);
    let id = new_type_layer(&mut app, "Hello");
    new_type_layer(&mut app, "Top");
    // OK would recolour the selected layers of the real session, so params that name no layer preview nothing.
    for params in [json!({}), json!({"range": [0, 3]}), json!({"layers": []}), json!({"layer": "1"})] {
        let picker = crate::color_picker_ui::open_for_command(&mut app, "Color Picker (Text Color)", [0.0; 3], "type.setStyle", params.clone());
        set_color(&mut app, "#ff0000");
        assert!(display_doc(&mut app, 0).is_none(), "{params}: no preview of a guessed layer");
        assert_eq!(crate::canvas::display_doc(&mut app, 0).1, 0, "{params}");
        crate::dialogs::testing::cancel(&mut app, picker).unwrap();
    }
    // A named layer is previewed.
    crate::color_picker_ui::open_for_command(&mut app, "Color Picker (Text Color)", [0.0; 3], "type.setStyle", json!({"layer": id.0}));
    set_color(&mut app, "#ff0000");
    let (shown, _) = display_doc(&mut app, 0).expect("a named layer is previewed");
    assert_eq!(rgb_at(&shown, id, 0), [255, 0, 0, 255]);
}
