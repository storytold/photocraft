//! What the Layers panel's events do, driven as the panel reports them: the lock column, thumbnail
//! clicks, footer and drop events, the row menu's values, and activations. Each trial sends
//! `LayersEvent`s through `apply_layers_events`, so the commands, targets and prompts run as they do
//! from the panel.

use craft::behavior::{EditEvent, Mixed, SelectionChange};
use craft::keymap::Mods;
use craft::layers::ui::panel::{
    ActivateOn, LayerColumn, LayerDrop, LayerMove, LayerPosition, LayerSelect, LayerToggle, LayersEvent, LayersFooterAction, LayersFooterButton, MaskMove,
    MenuTarget, RowTransfer, SubRowMove,
};
use craft::layers::ui::tree::{LayerPart, LayerRowKey, LayerTree, SubRowGroup};
use craft::ui::list::ThumbSize;
use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::Session;
use serde_json::{Value, json};

use super::super::layers_tree::{self, PhotoLayers};
use super::{apply_layers_events, menu_pick};
use crate::PhotocraftApp;

type Event = LayersEvent<LayerId>;

// ------------------------------------------------------------------------------------ fixtures

/// A 200×100 document: "Masked" (pixel + vector mask) and a "Badge" shape layer; "Masked" is active.
/// Returns the session and both layer ids.
pub(crate) fn session() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let masked = s.execute("layer.new.layer", json!({"name": "Masked"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({"layer": masked})).unwrap();
    let path = json!({"subpaths": [{"knots": [[20, 10], [120, 10], [120, 90], [20, 90]]}]});
    s.execute("layer.vectorMask.add", json!({"layer": masked, "path": path})).unwrap();
    let shape =
        s.execute("shape.create", json!({"kind": "rect", "rect": [140, 20, 40, 40], "fill": "#cc3333", "name": "Badge"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": masked})).unwrap();
    (s, masked, shape)
}

/// [`session`] with a black dot painted into the layer mask at (50, 50).
pub(crate) fn dotted() -> (Session, u64, u64) {
    let (mut s, masked, shape) = session();
    s.execute("paint.stroke", json!({"points": [[50, 50]], "size": 20, "hardness": 1.0, "color": "#000000", "target": "mask"})).unwrap();
    (s, masked, shape)
}

/// Smart objects "Smart A" (Gaussian Blur, then Offset) and "Smart B" (no filters), and the pixel
/// layer "Pixels" on top (active): the session and the ids of A, B and Pixels.
fn smart_session() -> (Session, u64, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let smart = |s: &mut Session, name: &str| {
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        let id = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.setProps", json!({"layer": id, "name": name})).unwrap();
        id
    };
    let a = smart(&mut s, "Smart A");
    s.execute("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
    s.execute("filter.other.offset", json!({"horizontal": 2})).unwrap();
    let b = smart(&mut s, "Smart B");
    let c = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": c, "name": "Pixels"})).unwrap();
    (s, a, b, c)
}

fn app_of(s: Session) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.background_jobs = false;
    // The app's first frame binds the Paths panel selection to the open document.
    app.sync_views();
    app
}

/// Sends each event in its own pass on the app's context, so a later event reads what the earlier ones did.
fn send(app: &mut PhotocraftApp, events: Vec<Event>) {
    let ctx = app.ctx().clone();
    for event in events {
        apply_layers_events(app, &ctx, vec![event]);
    }
}

fn layer(app: &PhotocraftApp, id: u64) -> photocraft_doc::Layer {
    app.session.active().unwrap().doc.layer(LayerId(id)).unwrap().clone()
}

fn active(app: &PhotocraftApp) -> Option<LayerId> {
    app.session.active().unwrap().active_layer
}

fn mods(shift: bool, alt: bool, cmd: bool) -> Mods {
    Mods { shift, alt, cmd, ..Mods::NONE }
}

/// A one-step change of `after`.
fn set<T>(after: T) -> EditEvent<T> {
    EditEvent::set(Mixed::Mixed, after, "test".into(), None)
}

fn select(ids: &[u64], part: Option<LayerPart>) -> Event {
    LayersEvent::Select(LayerSelect::new(SelectionChange::Set(ids.iter().map(|&id| LayerId(id)).collect()), part))
}

/// A click on a Smart Filter row of `layer`: it picks the layer and the row.
fn select_filter(app: &PhotocraftApp, layer: u64, command: &str) -> Event {
    let row = LayerRowKey::Filters(LayerId(layer), Some(filter_row(app, layer, command)));
    LayersEvent::Select(LayerSelect::new(SelectionChange::Set(vec![LayerId(layer)]), None).on_row(&row))
}

fn thumb(layer: u64, part: LayerPart, mods: Mods) -> Event {
    LayersEvent::thumb_click(LayerId(layer), part, mods)
}

fn dropped(drop: LayerDrop<LayerId>) -> Event {
    LayersEvent::Drop(set(drop))
}

fn filter_commands(app: &PhotocraftApp, layer_id: u64) -> Vec<String> {
    match &layer(app, layer_id).content {
        LayerContent::Smart(sm) => sm.smart_filters.iter().map(|f| f.command.clone()).collect(),
        _ => Vec::new(),
    }
}

/// The row a smart filter shows in: the stack lists the last filter first, so the filter at document
/// index `i` of `n` is row `n - 1 - i` (A's two filters: Gaussian Blur is index 0, row 1; Offset is
/// index 1, row 0).
fn filter_row(app: &PhotocraftApp, layer_id: u64, command: &str) -> usize {
    let commands = filter_commands(app, layer_id);
    let index = commands.iter().position(|c| c == command).expect("the filter is recorded");
    commands.len() - 1 - index
}

const BLUR: &str = "filter.blur.gaussianBlur";
const OFFSET: &str = "filter.other.offset";

fn mask_view(app: &PhotocraftApp) -> Value {
    photocraft_engine::inspect::document(app.session.active().unwrap())["layerMaskView"].clone()
}

pub(crate) fn canvas_px(app: &PhotocraftApp) -> Option<Vec<egui::Color32>> {
    let st = app.session.active().unwrap();
    let look = crate::channel_view::Look::of(&app.session);
    crate::channel_view::render(&st.doc, &st.channel_view, photocraft_geom::Rect::new(0, 0, 200, 100), 1, look, st.active_layer)
}

fn selection_bounds(app: &PhotocraftApp) -> (i32, i32, u32, u32) {
    let b = app.session.active().unwrap().doc.selection.clone().expect("a selection").content_bounds();
    (b.x0, b.y0, b.width(), b.height())
}

// ------------------------------------------------------------------------------------ lock column

/// #76: the Lock column on the Background makes it a normal layer in one click (so the Eraser is
/// not shut out), and on a layer with a lock one click clears every lock.
#[test]
fn lock_column_converts_the_background_and_unlocks_in_one_click() {
    use crate::canvas::{ToolEvent, tool_event};
    let mut app = app_of(Session::new());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("tools.setColors", json!({"background": "#ff0000"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0}})).unwrap();
    let lock = |app: &mut PhotocraftApp| {
        let id = app.session.active().unwrap().doc.layers[0].id;
        let toggle = LayerToggle::new(vec![LayerRowKey::Layer(id)], LayerColumn::Lock, set(true));
        apply_layers_events(app, &ctx, vec![LayersEvent::Toggle(toggle)]);
    };
    lock(&mut app);
    let l = &app.session.active().unwrap().doc.layers[0];
    assert_eq!(l.name, "Layer 0", "the Background became a normal layer");
    assert!(!l.locks.any());
    app.ui.tool = crate::state::Tool::Eraser;
    let m = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 40.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap().rgba(50, 40)[3], 0.0, "the Eraser erased to transparency");
    // A layer with only transparency locked, then one with every lock: one click clears them.
    for locks in [json!({"transparency": true}), json!({"all": true})] {
        app.run("layer.setProps", json!({"locks": locks})).unwrap();
        assert!(app.session.active().unwrap().doc.layers[0].locks.any());
        lock(&mut app);
        assert!(!app.session.active().unwrap().doc.layers[0].locks.any(), "{locks}");
    }
}

// ------------------------------------------------------------------------------------ thumbnails

/// One table of thumbnail clicks and selects: each row sends its events, one pass each, and checks
/// the commands' outcome and the targets.
#[test]
fn thumb_clicks_pick_commands_and_targets() {
    type Setup = fn() -> (Session, u64, u64);
    type Case = (&'static str, Setup, fn(u64, u64) -> Vec<Event>, fn(&PhotocraftApp, u64, u64));
    const BLACK: egui::Color32 = egui::Color32::BLACK;
    /// "Masked" (a) and "Badge" (b) with a rectangle selected, so the ⌘-click operations have something to combine with.
    fn with_selection(command: &'static str, params: Value) -> (Session, u64, u64) {
        let (mut s, a, b) = session();
        s.execute(command, params).unwrap();
        (s, a, b)
    }
    /// An adjustment layer with both masks over "Masked": the table's `a` is the adjustment, `b` "Masked".
    fn adjustment(command: &str) -> (Session, u64, u64) {
        let (mut s, masked, _) = session();
        s.execute(command, json!({})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap().0;
        let path = json!({"subpaths": [{"knots": [[20, 10], [120, 10], [120, 90], [20, 90]]}]});
        s.execute("layer.vectorMask.add", json!({"layer": id, "path": path})).unwrap();
        (s, id, masked)
    }
    let cases: Vec<Case> = vec![
        (
            "⇧-click disables a mask",
            session,
            |a, _| vec![thumb(a, LayerPart::LayerMask, Mods::SHIFT), thumb(a, LayerPart::VectorMask, Mods::SHIFT)],
            |app, a, _| {
                let l = layer(app, a);
                assert!(!l.mask.unwrap().enabled && !l.vector_mask.unwrap().enabled);
            },
        ),
        (
            "⌥-click views the mask in gray",
            dotted,
            |a, _| vec![thumb(a, LayerPart::LayerMask, Mods::ALT)],
            |app, a, _| {
                assert_eq!(mask_view(app), json!({"layer": a, "mode": "gray"}));
                let px = canvas_px(app).expect("the canvas shows the mask");
                assert_eq!(px[50 * 200 + 50], BLACK, "hidden reads black");
                assert_eq!(px[10 * 200 + 150], egui::Color32::WHITE, "revealed reads white");
            },
        ),
        (
            "⌥-click again returns to the composite",
            dotted,
            |a, _| vec![thumb(a, LayerPart::LayerMask, Mods::ALT), thumb(a, LayerPart::LayerMask, Mods::ALT)],
            |app, _, _| {
                assert_eq!(mask_view(app), Value::Null);
                assert!(canvas_px(app).is_none());
            },
        ),
        (
            "a plain click on the layer thumbnail leaves the mask view and targets the pixels",
            dotted,
            |a, _| vec![thumb(a, LayerPart::LayerMask, Mods::ALT), select(&[a], Some(LayerPart::Pixels))],
            |app, _, _| {
                assert_eq!(mask_view(app), Value::Null);
                assert!(!app.ui.mask_target);
            },
        ),
        (
            "⇧⌥-click shows a rubylith and keeps the mask enabled",
            dotted,
            |a, _| vec![thumb(a, LayerPart::LayerMask, mods(true, true, false))],
            |app, a, _| {
                assert_eq!(mask_view(app), json!({"layer": a, "mode": "overlay"}));
                let px = canvas_px(app).unwrap();
                assert_eq!(px[50 * 200 + 50], egui::Color32::from_rgba_premultiplied(128, 0, 0, 128), "50% red over hidden areas");
                assert_eq!(px[10 * 200 + 150], egui::Color32::TRANSPARENT, "the composite shows through elsewhere");
                assert!(layer(app, a).mask.unwrap().enabled);
            },
        ),
        (
            "⌘-click a vector mask loads its path as a selection and keeps the target",
            session,
            |a, _| vec![thumb(a, LayerPart::VectorMask, Mods::CMD)],
            |app, _, _| {
                assert_eq!(selection_bounds(app), (20, 10, 100, 80));
                assert!(!app.ui.vector_mask_target, "⌘-click doesn't retarget");
            },
        ),
        (
            "⌘⇧-click adds the path to the selection",
            || with_selection("select.rect", json!({"x": 150, "y": 50, "width": 4, "height": 4})),
            |a, _| vec![thumb(a, LayerPart::VectorMask, mods(true, false, true))],
            |app, _, _| {
                let sel = app.session.active().unwrap().doc.selection.clone().unwrap();
                assert!(sel.sample_channel(60, 50, 0) > 0.99 && sel.sample_channel(151, 51, 0) > 0.99, "added");
            },
        ),
        (
            "⌘⌥-click subtracts the path",
            || with_selection("select.all", json!({})),
            |a, _| vec![thumb(a, LayerPart::VectorMask, mods(false, true, true))],
            |app, _, _| {
                let sel = app.session.active().unwrap().doc.selection.clone().unwrap();
                assert!(sel.sample_channel(60, 50, 0) < 0.01 && sel.sample_channel(150, 50, 0) > 0.99, "subtracted");
            },
        ),
        (
            "⌘⇧⌥-click intersects the path",
            || with_selection("select.rect", json!({"x": 100, "y": 0, "width": 100, "height": 100})),
            |a, _| vec![thumb(a, LayerPart::VectorMask, mods(true, true, true))],
            |app, _, _| assert_eq!(selection_bounds(app), (100, 10, 20, 80)),
        ),
        (
            "mask view gestures without a mask do nothing",
            session,
            |a, b| vec![thumb(a, LayerPart::VectorMask, Mods::ALT), thumb(b, LayerPart::LayerMask, Mods::ALT), thumb(b, LayerPart::LayerMask, Mods::SHIFT)],
            |app, _, b| {
                assert_eq!(mask_view(app), Value::Null);
                assert!(layer(app, b).mask.is_none());
                assert!(!app.ui.status_error, "{}", app.ui.status);
            },
        ),
        (
            "the mask thumbnail targets the layer mask",
            session,
            |a, _| vec![select(&[a], Some(LayerPart::LayerMask))],
            |app, _, _| assert!(app.ui.mask_target && !app.ui.vector_mask_target),
        ),
        (
            "the vector mask thumbnail targets the vector mask and its path",
            session,
            |a, _| vec![select(&[a], Some(LayerPart::VectorMask))],
            |app, _, _| {
                assert!(app.ui.vector_mask_target && !app.ui.mask_target);
                assert_eq!(app.ui.selected_path.as_deref(), Some("layer"), "the Paths panel selects its path");
            },
        ),
        (
            "the layer thumbnail takes the target back from a mask",
            session,
            |a, _| vec![select(&[a], Some(LayerPart::VectorMask)), select(&[a], Some(LayerPart::LayerMask)), select(&[a], Some(LayerPart::Pixels))],
            |app, _, _| assert!(!app.ui.mask_target && !app.ui.vector_mask_target),
        ),
        (
            "selecting an adjustment layer from another layer targets its mask",
            || adjustment("layer.newAdjustmentLayer.curves"),
            |a, b| vec![select(&[b], None), select(&[a], None)],
            |app, a, _| {
                assert_eq!(active(app), Some(LayerId(a)));
                assert!(app.ui.mask_target && !app.ui.vector_mask_target);
                assert_eq!(crate::canvas::paint_target(app), json!("mask"));
            },
        ),
        (
            "selecting a masked pixel layer by its row targets its contents",
            || adjustment("layer.newAdjustmentLayer.hueSaturation"),
            |a, b| vec![select(&[a], None), select(&[b], None)],
            |app, _, b| {
                assert_eq!(active(app), Some(LayerId(b)));
                assert!(!app.ui.mask_target && !app.ui.vector_mask_target);
            },
        ),
        (
            "the filter mask thumbnail targets the filter mask",
            || {
                let (s, a, b, _) = smart_session();
                (s, a, b)
            },
            |a, _| vec![select(&[a], Some(LayerPart::FilterMask))],
            |app, _, _| assert!(app.filter_mask_targeted()),
        ),
        (
            "a plain click on the filter mask thumbnail picks the smart object and targets it",
            || {
                let (s, a, b, _) = smart_session();
                (s, a, b)
            },
            |a, _| vec![thumb(a, LayerPart::FilterMask, Mods::NONE)],
            |app, a, _| {
                assert_eq!(active(app), Some(LayerId(a)));
                assert!(app.filter_mask_targeted());
            },
        ),
    ];
    for (case, setup, events, after) in cases {
        let (s, a, b) = setup();
        let mut app = app_of(s);
        send(&mut app, events(a, b));
        eprintln!("{case}");
        after(&app, a, b);
    }
}

// ------------------------------------------------------------------------------------ footer and drops

/// Rules with bug history the footer tests do not cover: Delete with a Smart Filter picked (#4751),
/// the smart object's own row clearing the pick, and filters moved or copied by a drop.
#[test]
fn footer_and_drop_events_follow_the_targets() {
    let fresh = || {
        let (s, a, b, c) = smart_session();
        (app_of(s), a, b, c)
    };
    let picked = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        crate::smart_ui::selected(&app.smart_filter_picks, &st.doc, st.active_layer)
    };

    // A click on a filter row picks the filter and makes its smart object active (the layer index is
    // the row counted from the other end).
    let (mut app, a, _, c) = fresh();
    assert_eq!(active(&app), Some(LayerId(c)));
    let ev = select_filter(&app, a, BLUR);
    send(&mut app, vec![ev]);
    assert_eq!(active(&app), Some(LayerId(a)), "the smart object becomes active");
    assert_eq!(picked(&app), Some((a, Some(0))), "Gaussian Blur is index 0");
    // Another layer: the filter is no longer picked.
    send(&mut app, vec![select(&[c], None)]);
    assert_eq!(picked(&app), None);

    // #4751: Delete with a filter picked removes that filter, not the smart object, and the pick does not
    // carry over to the filter now at its index.
    let (mut app, a, _, _) = fresh();
    let ev = select_filter(&app, a, BLUR);
    send(&mut app, vec![ev]);
    send(&mut app, vec![LayersEvent::Footer(LayersFooterAction::delete(true))]);
    assert_eq!(filter_commands(&app, a), [OFFSET]);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 4, "no layer was deleted");
    assert_eq!(picked(&app), None);
    send(&mut app, vec![LayersEvent::Footer(LayersFooterAction::delete(true))]);
    assert_eq!(filter_commands(&app, a), [OFFSET], "the second Delete takes no filter");

    // The smart object's own row leaves its filters: nothing is picked.
    let (mut app, a, _, _) = fresh();
    let ev = select_filter(&app, a, OFFSET);
    send(&mut app, vec![ev, select(&[a], None)]);
    assert_eq!(picked(&app), None);
    assert_eq!(filter_commands(&app, a).len(), 2, "the filters stay");

    // Filters dropped on the footer's Delete: a filter's row removes it, the Smart Filters row clears them all.
    let (mut app, a, _, _) = fresh();
    let row = LayerRowKey::Filters(LayerId(a), Some(filter_row(&app, a, OFFSET)));
    send(&mut app, vec![LayersEvent::footer_drop(LayersFooterButton::Delete, vec![row])]);
    assert_eq!(filter_commands(&app, a), [BLUR]);
    send(&mut app, vec![LayersEvent::footer_drop(LayersFooterButton::Delete, vec![LayerRowKey::Filters(LayerId(a), None)])]);
    assert!(filter_commands(&app, a).is_empty());

    // A filter dropped on another smart object moves there; with ⌥ it is copied; on a pixel layer nothing happens.
    type Transfer = (&'static str, &'static str, bool, bool, &'static [&'static str], &'static [&'static str]);
    let transfers: [Transfer; 4] = [
        ("onto another smart object: moves", OFFSET, false, false, &[BLUR], &[OFFSET]),
        ("⌥ onto another smart object: copies", BLUR, true, false, &[BLUR, OFFSET], &[BLUR]),
        ("onto a pixel layer: nothing", BLUR, false, true, &[BLUR, OFFSET], &[]),
        ("onto a pixel layer with ⌥: nothing", BLUR, true, true, &[BLUR, OFFSET], &[]),
    ];
    for (case, filter, copy, onto_pixels, on_a, on_b) in transfers {
        let (mut app, a, b, c) = fresh();
        let steps = app.session.active().unwrap().history.past_len();
        let to = if onto_pixels { c } else { b };
        let row = LayerRowKey::Filters(LayerId(a), Some(filter_row(&app, a, filter)));
        send(&mut app, vec![dropped(LayerDrop::Transfer(RowTransfer::new(vec![row], LayerId(to), copy)))]);
        assert_eq!(filter_commands(&app, a), on_a.to_vec(), "{case}: Smart A");
        assert_eq!(filter_commands(&app, b), on_b.to_vec(), "{case}: Smart B");
        let changed = !on_b.is_empty();
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + usize::from(changed), "{case}: one undo step");
    }

    // A drop within the stack: Gaussian Blur (row 1) onto row 0 lands above Offset's row, so the document order
    // is [Offset, Blur] and the moved filter (now index 1) stays picked.
    let (mut app, a, _, _) = fresh();
    let from = filter_row(&app, a, BLUR);
    assert_eq!(from, 1);
    let steps = app.session.active().unwrap().history.past_len();
    send(&mut app, vec![dropped(LayerDrop::SubRows(SubRowMove::new(LayerId(a), SubRowGroup::SmartFilters, from, 0, false)))]);
    assert_eq!(filter_commands(&app, a), [OFFSET, BLUR]);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(picked(&app), Some((a, Some(1))), "the moved filter stays picked");

    // The report's case: a Camera Raw Filter ⌥-dropped on another smart object keeps its settings, and
    // activating the copy opens Camera Raw on that smart object.
    let (mut app, a, b, _) = fresh();
    send(&mut app, vec![select(&[a], None)]);
    app.run("filter.cameraRaw", json!({"exposure": 1.0, "contrast": 20})).unwrap();
    let raw = |app: &PhotocraftApp, id: u64| match &layer(app, id).content {
        LayerContent::Smart(sm) => sm.smart_filters.iter().find(|f| f.command == "filter.cameraRaw").map(|f| f.params.clone()),
        _ => None,
    };
    let row = LayerRowKey::Filters(LayerId(a), Some(filter_row(&app, a, "filter.cameraRaw")));
    send(&mut app, vec![dropped(LayerDrop::Transfer(RowTransfer::new(vec![row], LayerId(b), true)))]);
    assert_eq!(raw(&app, b), raw(&app, a), "copied with its settings");
    assert_eq!(raw(&app, b).unwrap()["exposure"], 1.0);
    send(&mut app, vec![LayersEvent::activate(LayerRowKey::Filters(LayerId(b), Some(0)), ActivateOn::Row)]);
    assert!(app.dialog_open(), "Camera Raw opened");
    assert_eq!(active(&app), Some(LayerId(b)), "on the copy's smart object");
}

/// Layer masks moved or copied onto a group (#10485) and refused onto a masked row, and a layer dropped into a frame.
#[test]
fn mask_and_frame_drops_follow_the_targets() {
    // #10485: a layer mask dropped on a group moves there, with ⌥ it is copied.
    for copy in [false, true] {
        let (mut s, masked, shape) = session();
        let group = s.execute("layer.groupLayers", json!({"layer": shape, "name": "Grp"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.select", json!({"layer": masked})).unwrap();
        let mut app = app_of(s);
        let mv = MaskMove::new(LayerId(masked), LayerPart::LayerMask, LayerId(group), copy);
        send(&mut app, vec![dropped(LayerDrop::Mask(mv))]);
        assert!(layer(&app, group).mask.is_some(), "copy: {copy}: the group has the mask");
        assert_eq!(layer(&app, masked).mask.is_some(), copy, "copy: {copy}: the original stays only with ⌥");
    }

    // The panel offers no drop of a layer mask on a row that has one, or on its own row.
    let (mut s, masked, shape) = session();
    s.execute("layer.layerMask.hideAll", json!({"layer": shape})).unwrap();
    let app = app_of(s);
    let st = app.session.active().unwrap();
    let tree = PhotoLayers::new(&app, st, &app.ui.layer_filter, false, None);
    assert!(!tree.accepts_mask_drop(&LayerId(masked), LayerPart::LayerMask, &LayerId(shape)), "onto a masked row");
    assert!(!tree.accepts_mask_drop(&LayerId(masked), LayerPart::LayerMask, &LayerId(masked)), "onto its own row");

    // A layer dropped into a frame becomes its content.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let pic = s.execute("layer.new.layer", json!({"name": "pic"})).unwrap()["layer"].as_u64().unwrap();
    // A frame takes a layer with pixels: an empty one has nothing to convert.
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let frame = s.execute("layer.new.frame", json!({"x": 4, "y": 4, "width": 40, "height": 30})).unwrap()["layer"].as_u64().unwrap();
    let mut app = app_of(s);
    let mv = LayerMove::new(vec![LayerId(pic)], LayerId(frame), LayerPosition::Into, false);
    send(&mut app, vec![dropped(LayerDrop::Layers(mv))]);
    let content = layer(&app, frame);
    assert_eq!(content.children().map(<[_]>::len), Some(1), "the frame holds the one layer");
}

// ------------------------------------------------------------------------------------ menu picks

/// The row menu's `layersPanel:*` values run their rules.
#[test]
fn menu_pick_values_run_their_rules() {
    let (s, masked, _) = session();
    let mut app = app_of(s);
    let ctx = app.ctx().clone();
    let on = Some(LayerRowKey::Layer(LayerId(masked)));
    let pick = |app: &mut PhotocraftApp, row: &Option<LayerRowKey<LayerId>>, command: &str| {
        let mut actions = Vec::new();
        menu_pick(app, &ctx, row, command, &mut actions);
        actions
    };

    // A thumbnail size is set and saved: restoring from the saved options gives it back.
    let value = format!("{}{}", layers_tree::THUMB_SIZE, crate::layers_panel_ui::size_key(ThumbSize::Large));
    assert!(pick(&mut app, &None, &value).is_empty());
    assert_eq!(app.ui.layers_settings.options.thumbnail_size, ThumbSize::Large);
    app.ui.layers_settings.options.thumbnail_size = ThumbSize::Small;
    crate::layers_panel_ui::restore_options(&mut app);
    assert_eq!(app.ui.layers_settings.options.thumbnail_size, ThumbSize::Large, "the choice was saved");

    // A clip value runs `prefs.set` only when the preference differs.
    let now = app.session.prefs().interface.clip_thumbnails_to_layer_bounds;
    assert!(pick(&mut app, &None, &format!("{}{now}", layers_tree::CLIP_THUMBS)).is_empty(), "the same value does nothing");
    let actions = pick(&mut app, &None, &format!("{}{}", layers_tree::CLIP_THUMBS, !now));
    assert_eq!(actions, [("prefs.set".to_string(), json!({"path": "interface.clipThumbnailsToLayerBounds", "value": !now}))]);

    // Select Pixels and Load Mask build their params from the row's layer.
    assert_eq!(pick(&mut app, &on, layers_tree::SELECT_PIXELS), [("select.loadSelection".to_string(), layers_tree::select_pixels(LayerId(masked)))]);
    let actions = pick(&mut app, &on, &format!("{}add", layers_tree::LOAD_MASK));
    assert_eq!(actions, [("select.loadSelection".to_string(), json!({"channel": "mask", "layer": masked, "operation": "add"}))]);

    // Rename asks the panel to edit the row's name.
    assert!(pick(&mut app, &on, layers_tree::RENAME).is_empty());
    assert_eq!(app.layers_rename, Some(LayerId(masked)));

    // Any other value is a command that runs as the menu item does; with no row there is nothing to act on.
    assert_eq!(pick(&mut app, &on, "layer.duplicate"), [("layer.duplicate".to_string(), Value::Null)]);
    assert!(pick(&mut app, &None, "layer.duplicate").is_empty());
    assert!(pick(&mut app, &None, layers_tree::SELECT_PIXELS).is_empty());

    // As an event from the panel, Select Pixels loads the layer's pixels as the selection.
    assert!(app.session.active().unwrap().doc.selection.is_none());
    send(&mut app, vec![LayersEvent::menu(MenuTarget::new(on), layers_tree::SELECT_PIXELS)]);
    assert!(app.session.active().unwrap().doc.selection.is_some());
}

// ------------------------------------------------------------------------------------ activation

/// An activation opens the right editor: a filter row's Blending Options part its blending dialog, the
/// row its own editor (also on a smart object that is not the active layer), and the effects disclosure
/// is a view change only.
#[test]
fn activate_and_fx_events_open_the_right_editor() {
    let top = |app: &PhotocraftApp| app.framework.dialogs.open_dialogs().pop().expect("a dialog opened");
    let (s, a, _, c) = smart_session();
    let mut app = app_of(s);
    let ctx = app.ctx().clone();
    assert_eq!(active(&app), Some(LayerId(c)));

    // Blending Options opens the blending dialog and picks the smart object.
    let blur_row = LayerRowKey::Filters(LayerId(a), Some(filter_row(&app, a, BLUR)));
    send(&mut app, vec![LayersEvent::activate(blur_row.clone(), ActivateOn::Options)]);
    assert_eq!(active(&app), Some(LayerId(a)), "the smart object becomes active");
    let dialog = top(&app);
    assert!(crate::smart_filter_dialog::owns(dialog.key.as_deref().unwrap_or_default()));
    crate::dialogs::testing::cancel(&mut app, dialog.id).unwrap();

    // The row opens the filter's editor on the recorded values; OK edits it in place.
    send(&mut app, vec![select(&[c], None)]);
    send(&mut app, vec![LayersEvent::activate(blur_row, ActivateOn::Row)]);
    assert_eq!(active(&app), Some(LayerId(a)), "an inactive smart object is selected before its editor opens");
    let dialog = top(&app);
    assert_eq!(dialog.fields.as_ref().unwrap()["radius"].as_f64(), Some(1.0), "the recorded radius");
    app.framework.dialogs.set_json_field(&ctx, dialog.id, "radius", json!(3.0)).unwrap();
    crate::dialogs::testing::confirm(&mut app, dialog.id).unwrap();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(filter_commands(&app, a).len(), 2, "edited in place, not added");
    let LayerContent::Smart(sm) = &layer(&app, a).content else { panic!("smart object") };
    assert_eq!(sm.smart_filters[0].params["radius"], json!(3.0));

    // Liquify: the workspace opens with the recorded strokes and a new one is kept on commit.
    let stroke = json!({"tool": "forwardWarp", "size": 8, "points": [[4.0, 8.0, 1.0], [8.0, 8.0, 1.0]]});
    app.run("layer.select", json!({"layer": a})).unwrap();
    app.run("filter.liquify", json!({"strokes": [stroke], "meshSize": 3.0})).unwrap();
    app.run("layer.select", json!({"layer": c})).unwrap();
    let row = LayerRowKey::Filters(LayerId(a), Some(filter_row(&app, a, photocraft_engine::distort_cmds::LIQUIFY)));
    send(&mut app, vec![LayersEvent::activate(row, ActivateOn::Row)]);
    assert!(app.liquify.is_some(), "the Liquify workspace opens");
    crate::liquify_ui::record_stroke(&mut app, &[[6.0, 4.0, 1.0], [10.0, 4.0, 1.0]]);
    let ctx = app.ctx().clone();
    crate::control::dialogs::workspace_ui(&mut app, &ctx, "filter.liquify", &json!({"commit": true})).unwrap();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let LayerContent::Smart(sm) = &layer(&app, a).content else { panic!("smart object") };
    let liquify = sm.smart_filters.iter().find(|f| f.command == photocraft_engine::distort_cmds::LIQUIFY).unwrap();
    assert_eq!(liquify.params["strokes"].as_array().map(Vec::len), Some(2), "the new stroke was kept");

    // The effects disclosure hides the effects and Smart Filters stacks together, as a view change only.
    let (s, a, _, c) = smart_session();
    let mut app = app_of(s);
    app.run("layer.layerStyle.dropShadow", json!({"layer": a})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let (revision, steps) = (app.session.active().unwrap().revision, app.session.active().unwrap().history.past_len());
    send(&mut app, vec![LayersEvent::fx_open(LayerId(a), false, false)]);
    let st = app.session.active().unwrap();
    assert!(st.fx_collapsed.contains(&LayerId(a)));
    assert!(std::sync::Arc::ptr_eq(&doc, &st.doc), "no pixel changed");
    assert_eq!((st.revision, st.history.past_len()), (revision, steps), "no history step");
    assert_eq!(st.active_layer, Some(LayerId(c)), "the selection is the same");
    send(&mut app, vec![LayersEvent::fx_open(LayerId(a), true, false)]);
    assert!(app.session.active().unwrap().fx_collapsed.is_empty());
}

// ------------------------------------------------------------------------------------ text edit

/// #5584: a select commits an open text edit first, whether it picks the layer being edited or another.
#[test]
fn a_select_commits_an_open_text_edit_before_changing_the_selection() {
    for same in [true, false] {
        for thumbnail in [true, false] {
            let mut app = app_of(Session::new());
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            let text = LayerId(app.run("type.create", json!({"text": "Before", "size": 24, "x": 20, "y": 80})).unwrap()["layer"].as_u64().unwrap());
            let other = LayerId(app.run("layer.new.layer", json!({"name": "Other pixels"})).unwrap()["layer"].as_u64().unwrap());
            app.run("layer.select", json!({"layer": text.0})).unwrap();
            let ctx = app.ctx().clone();
            crate::type_tool::edit_active(&mut app, &ctx).unwrap();
            let session = app.ui.text_edit.as_ref().unwrap().session.clone();
            app.run("type.edit", json!({"layer": text.0, "text": "Preserved edit", "coalesce": session})).unwrap();
            let target = if same { text } else { other };
            send(&mut app, vec![select(&[target.0], thumbnail.then_some(LayerPart::Pixels))]);
            let case = format!("same={same}, thumbnail={thumbnail}");
            assert!(app.ui.text_edit.is_none(), "{case}: the edit committed");
            assert_eq!(active(&app), Some(target), "{case}");
            let LayerContent::Text(t) = &app.session.active().unwrap().doc.layer(text).unwrap().content else { panic!("type must remain editable") };
            assert_eq!(t.text, "Preserved edit", "{case}: the edit was kept");
        }
    }
}
