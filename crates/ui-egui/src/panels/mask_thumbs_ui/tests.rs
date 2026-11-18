//! #153: mask thumbnails, link chains and the red X in Layers rows. The thumbnail click, drop and
//! target rows are `panels/layers_events/tests.rs`'s; the trials here drive the same events for the
//! rules that sit with the mask thumbnails (links, targets and swatches, Properties).

use craft::behavior::SelectionChange;
use craft::layers::ui::panel::{LayerSelect, LayersEvent};
use craft::layers::ui::thumbs::{LayerThumb, ThumbContents};
use craft::layers::ui::tree::{LayerPart, LayerTree};
use craft::ui::primitives::textures::{ThumbRequest, ThumbSource};
use egui::{Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::LayerId;
use serde_json::json;

use super::super::layers_events::apply_layers_events;
use super::super::layers_events::tests::{canvas_px, dotted, session};
use super::super::layers_tree::{PhotoLayers, PhotoThumbs};
use super::{MaskKind, vector_thumb_image};
use crate::PhotocraftApp;

fn app_of(session: photocraft_engine::Session) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    app.background_jobs = false;
    app.sync_views();
    app
}

fn harness(session: photocraft_engine::Session, ppp: f32, dock_width: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_pixels_per_point(ppp).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), session, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"theme": "promedium", "dockWidth": dock_width}));
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    h
}

fn click_with(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, modifiers: Modifiers) {
    h.hover_at(at);
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(1);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

fn layer(app: &PhotocraftApp, id: u64) -> photocraft_doc::Layer {
    app.session.active().unwrap().doc.layer(LayerId(id)).unwrap().clone()
}

fn name(h: &Harness<'_, PhotocraftApp>, id: u64) -> String {
    layer(h.state(), id).name
}

fn mask_rect(h: &Harness<'_, PhotocraftApp>, id: u64, kind: MaskKind) -> Rect {
    let part = if kind == MaskKind::Pixel { LayerPart::LayerMask } else { LayerPart::VectorMask };
    crate::layers_panel_nodes::thumb(h, &name(h, id), part)
}

fn mask_view(app: &PhotocraftApp) -> serde_json::Value {
    photocraft_engine::inspect::document(app.session.active().unwrap())["layerMaskView"].clone()
}

/// A click on a layer's thumbnail (`Some(part)`) or elsewhere on its row, as the panel reports it.
fn click_row(app: &mut PhotocraftApp, ctx: &egui::Context, id: u64, part: Option<LayerPart>) {
    let select = LayerSelect::new(SelectionChange::Set(vec![LayerId(id)]), part);
    apply_layers_events(app, ctx, vec![LayersEvent::Select(select)]);
}

#[test]
fn the_chain_toggles_linking_and_it_survives_psd() {
    let (s, masked, _) = session();
    let mut app = app_of(s);
    let ctx = app.ctx().clone();
    for part in [LayerPart::LayerMask, LayerPart::VectorMask] {
        apply_layers_events(&mut app, &ctx, vec![LayersEvent::mask_link(LayerId(masked), part, false)]);
    }
    let l = layer(&app, masked);
    assert!(!l.mask.unwrap().linked && !l.vector_mask.unwrap().linked);
    let doc = (*app.session.active().unwrap().doc).clone();
    let bytes = photocraft_io::export(&doc, "m.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("m.psd", &bytes).unwrap().document;
    let l = back.walk().into_iter().map(|(_, _, l)| l.clone()).find(|l| l.name == "Masked").unwrap();
    assert!(!l.mask.unwrap().linked && !l.vector_mask.unwrap().linked, "unlinked state round-trips PSD");
    // Clicking the (now empty) chain slot links again.
    apply_layers_events(&mut app, &ctx, vec![LayersEvent::mask_link(LayerId(masked), LayerPart::LayerMask, true)]);
    assert!(layer(&app, masked).mask.unwrap().linked);
}

#[test]
fn vector_thumbnail_is_white_inside_grey_outside_at_the_document_aspect() {
    let (s, masked, _) = session();
    let doc = s.active().unwrap().doc.clone();
    let vm = doc.layer(LayerId(masked)).unwrap().vector_mask.clone().unwrap();
    let img = vector_thumb_image(&doc, &vm, 64);
    let at = |x: usize, y: usize| img.pixels[y * 64 + x];
    // 200×100 doc in 64 px: 0.32 scale, rows 16..48. The mask covers x 20..120, y 10..90.
    assert_eq!(at(20, 32), egui::Color32::from_gray(255), "inside reveals");
    assert_eq!(at(55, 32), egui::Color32::from_gray(128), "outside hides");
    assert_eq!(at(20, 4), egui::Color32::TRANSPARENT, "letterbox");
}

#[test]
fn channels_list_the_layer_mask_and_keep_the_aspect() {
    let (s, masked, _) = session();
    let mut app = app_of(s);
    let names = |app: &PhotocraftApp| crate::channels_panel::rows(app).iter().map(|row| row.name.to_string()).collect::<Vec<_>>();
    assert_eq!(names(&app), ["RGB", "Red", "Green", "Blue", "Masked Mask"], "the mask is a temporary row below the colour channels");
    // A layer without a mask has no mask row.
    app.run("layer.layerMask.delete", json!({"layer": masked})).unwrap();
    assert!(names(&app).iter().all(|n| !n.ends_with("Mask")));
}

#[test]
fn panels_never_panic_on_odd_documents() {
    // Rows and thumbnails work for a 1×1 and a very wide document, and for masks with nothing in them.
    for (w, hgt) in [(1, 1), (2000, 3)] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": w, "height": hgt})).unwrap();
        let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        s.execute("layer.vectorMask.revealAll", json!({})).unwrap();
        let app = app_of(s);
        let st = app.session.active().unwrap();
        let tree = PhotoLayers::new(&app, st, &app.ui.layer_filter, false, None);
        let info = tree.info(&LayerId(id));
        assert!(!info.name.is_empty(), "{w}×{hgt}");
        let thumbs = PhotoThumbs::new(&app, &st.doc);
        for part in [LayerPart::Pixels, LayerPart::LayerMask, LayerPart::VectorMask] {
            let key = LayerThumb::of_part(LayerId(id), part, ThumbContents::EntireDocument);
            let request = ThumbRequest::for_cell(28.0, 1.0);
            let _ = thumbs.revision(&key, &request);
            let _ = thumbs.render(&key, &request);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// #196: the vector mask as the target, the mask colours, Properties.

/// Clicking the vector mask targets it for the path tools: Path Selection edits the mask's path, the
/// Pen adds a subpath to it, and the other thumbnails take the target back.
#[test]
fn clicking_the_vector_mask_targets_it_for_the_path_tools() {
    let (s, masked, _) = session();
    let mut app = app_of(s);
    let ctx = app.ctx().clone();
    click_row(&mut app, &ctx, masked, Some(LayerPart::VectorMask));
    assert!(app.ui.vector_mask_target && !app.ui.mask_target);
    assert_eq!(app.ui.selected_path.as_deref(), Some("layer"), "the Paths panel selects its path");
    assert_eq!(crate::canvas::paint_target(&app), json!("pixels"));
    // Path Selection moves the vector mask.
    crate::vector_ui::path_selection_finish(&mut app, [30.0, 30.0], [35.0, 32.0]);
    let k = layer(&app, masked).vector_mask.unwrap().path.subpaths[0].knots[0].anchor;
    assert_eq!((k.x, k.y), (25.0, 12.0));
    // The Pen adds a subpath to it.
    let owner = crate::vector_ui::capture_pen_owner(&app);
    app.ui.pen = Some(crate::vector_ui::PenPath::corners(&[[150.0, 10.0], [190.0, 10.0], [190.0, 40.0]], owner));
    crate::vector_ui::pen_commit(&mut app, true);
    assert_eq!(layer(&app, masked).vector_mask.unwrap().path.subpaths.len(), 2);
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "not a work path");
    // The pixel mask and the layer thumbnail take the target back.
    click_row(&mut app, &ctx, masked, Some(LayerPart::LayerMask));
    assert!(app.ui.mask_target && !app.ui.vector_mask_target);
    click_row(&mut app, &ctx, masked, Some(LayerPart::VectorMask));
    click_row(&mut app, &ctx, masked, Some(LayerPart::Pixels));
    assert!(!app.ui.mask_target && !app.ui.vector_mask_target);
    // Deleting the vector mask drops the target.
    click_row(&mut app, &ctx, masked, Some(LayerPart::VectorMask));
    assert!(app.ui.vector_mask_target);
    app.run("layer.vectorMask.delete", json!({"layer": masked})).unwrap();
    app.sync_mask_targets();
    assert!(!app.ui.vector_mask_target);
}

#[test]
fn a_targeted_vector_mask_flips_its_path_not_the_layer_pixels() {
    let (s, masked, _) = session();
    let mut app = app_of(s);
    let ctx = app.ctx().clone();
    click_row(&mut app, &ctx, masked, Some(LayerPart::VectorMask));
    assert!(app.ui.vector_mask_target);
    let before = layer(&app, masked);
    app.run("edit.transform.flipHorizontal", json!({})).unwrap();
    let after = layer(&app, masked);
    let path = after.vector_mask.as_ref().unwrap().path.clone();
    let k = &path.subpaths[0].knots;
    assert_eq!((k[0].anchor.x, k[0].anchor.y), (120.0, 10.0), "mirrored about the path's bounds (x = 140)");
    assert_eq!(after.surface().unwrap().content_bounds(), before.surface().unwrap().content_bounds(), "the pixels are untouched");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&app, masked).vector_mask.unwrap().path, before.vector_mask.unwrap().path, "undo restores the path");
}

/// #12283: the panel's vector-mask target must not override an alpha channel or Quick Mask (the
/// engine routes those), and an explicit target in the params always wins.
#[test]
fn a_targeted_vector_mask_yields_to_alpha_quick_mask_and_explicit_targets() {
    let (s, masked, _) = session();
    let mut app = app_of(s);
    let ctx = app.ctx().clone();
    click_row(&mut app, &ctx, masked, Some(LayerPart::VectorMask));
    assert!(app.ui.vector_mask_target && app.vector_mask_targeted());
    let target = |app: &PhotocraftApp, p: serde_json::Value| app.with_mask_target("edit.transform.flipHorizontal", p);
    assert_eq!(target(&app, json!({}))["target"], "vectorMask");
    assert_eq!(target(&app, json!({"target": "pixels"}))["target"], "pixels", "an explicit target wins");
    let before = layer(&app, masked).vector_mask.unwrap().path;

    // Photoshop targets a new alpha channel on its own; the panel still says vector mask.
    app.run("channel.new", json!({"name": "Alpha", "fill": "white"})).unwrap();
    assert!(app.ui.vector_mask_target, "the panel keeps its vector-mask flag");
    assert_eq!(app.session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Alpha(0));
    assert!(target(&app, json!({})).get("target").is_none(), "the alpha channel wins");
    app.run("edit.transform.flipHorizontal", json!({})).unwrap();
    assert_eq!(layer(&app, masked).vector_mask.unwrap().path, before, "the vector mask stays put");

    // Quick Mask mode wins too, even with the stale panel flag.
    app.run("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    assert!(app.ui.vector_mask_target && !app.vector_mask_targeted());
    assert!(target(&app, json!({})).get("target").is_none(), "the Quick Mask wins");
    app.run("edit.transform.flipHorizontal", json!({})).unwrap();
    assert_eq!(layer(&app, masked).vector_mask.unwrap().path, before, "the vector mask stays put");
}

/// #1765: `\` shows the active layer's mask as a rubylith and hides it again, as in Photoshop;
/// the mask stays the paint target meanwhile. On a layer without a mask it does nothing.
#[test]
fn backslash_toggles_the_rubylith() {
    let (s, masked, shape) = dotted();
    let mut h = harness(s, 1.0, 290.0);
    let doc = h.state().session.active().unwrap().doc.clone();
    h.key_press(egui::Key::Backslash);
    h.run_steps(2);
    assert_eq!(mask_view(h.state()), json!({"layer": masked, "mode": "overlay"}), "\\ shows the overlay");
    assert_eq!(crate::canvas::paint_target(h.state()), json!("mask"), "painting paints the mask");
    let px = canvas_px(h.state()).unwrap();
    assert_eq!(px[50 * 200 + 50], egui::Color32::from_rgba_premultiplied(128, 0, 0, 128), "50% red over hidden areas");
    // Layer Mask Display Options: opaque green over what the mask hides instead.
    h.state_mut().run("channel.options", json!({"channel": "mask", "color": "#00ff00", "opacity": 100})).unwrap();
    assert_eq!(canvas_px(h.state()).unwrap()[50 * 200 + 50], egui::Color32::from_rgb(0, 255, 0));
    h.key_press(egui::Key::Backslash);
    h.run_steps(2);
    assert_eq!(mask_view(h.state()), serde_json::Value::Null, "\\ again hides it");
    assert!(std::sync::Arc::ptr_eq(&doc, &h.state().session.active().unwrap().doc), "a view toggle, not an edit");
    // A layer without a layer mask: nothing happens.
    h.state_mut().run("layer.select", json!({"layer": shape})).unwrap();
    h.run_steps(2);
    h.key_press(egui::Key::Backslash);
    h.run_steps(2);
    assert_eq!(mask_view(h.state()), serde_json::Value::Null);
}

/// #780: clicking the mask thumbnail targets the mask, and ⌘I (Image › Adjustments › Invert)
/// then inverts the mask instead of the layer; on an adjustment layer the menu item is live.
#[test]
fn command_i_inverts_the_targeted_mask() {
    let (s, masked, _) = dotted();
    let mut h = harness(s, 1.0, 290.0);
    let pixels = layer(h.state(), masked).content;
    let r = mask_rect(&h, masked, MaskKind::Pixel);
    click_with(&mut h, r.center(), Modifiers::NONE);
    assert!(h.state().ui.mask_target);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::I);
    h.run_steps(2);
    let l = layer(h.state(), masked);
    let mask = &l.mask.as_ref().unwrap().surface;
    assert_eq!((mask.sample_channel(50, 50, 0), mask.sample_channel(150, 10, 0)), (1.0, 0.0), "the mask inverted");
    assert!(l.content == pixels, "the layer's pixels are untouched");
    // An adjustment layer's mask: with no pixels of its own, Invert edits the mask whether the
    // panel targets the mask or the icon thumbnail (#4298).
    let app = h.state_mut();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = false;
    assert!(crate::menus::is_enabled(app, "image.adjustments.invert"));
    app.ui.mask_target = true;
    assert!(crate::menus::is_enabled(app, "image.adjustments.invert"));
    app.run("image.adjustments.invert", json!({})).unwrap();
    let st = app.session.active().unwrap();
    let mask = &st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface;
    assert_eq!(mask.sample_channel(10, 10, 0), 0.0, "reveal all → hide all");
}

/// #2420: Selecting a layer mask in the Layers panel toggles the color swatches
/// to the mask's colors (white/black by default), and deselecting toggles back.
#[test]
fn selecting_mask_thumbnail_toggles_swatches_to_mask_colors_and_back() {
    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
    const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    let (s, masked, _) = dotted();
    let mut app = app_of(s);
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    let ctx = app.ctx().clone();

    // Initial state: layer pixels targeted, red foreground, blue background.
    assert!(!app.ui.mask_target);
    assert_eq!(app.session.tools.foreground, RED);
    assert_eq!(app.session.tools.background, BLUE);

    // 1. Click the mask thumbnail -> targets the mask, swatches toggle to mask colors.
    click_row(&mut app, &ctx, masked, Some(LayerPart::LayerMask));
    assert!(app.ui.mask_target, "mask thumbnail clicked: mask is targeted");
    assert_eq!(app.session.tools.foreground, WHITE);
    assert_eq!(app.session.tools.background, BLACK);

    // 2. Click the layer thumbnail -> deselects the mask, swatches toggle back to pixel colors.
    click_row(&mut app, &ctx, masked, Some(LayerPart::Pixels));
    assert!(!app.ui.mask_target, "layer thumbnail clicked: pixels are targeted");
    assert_eq!(app.session.tools.foreground, RED);
    assert_eq!(app.session.tools.background, BLUE);

    // 3. Click the mask thumbnail again -> swatches toggle back to mask colors.
    click_row(&mut app, &ctx, masked, Some(LayerPart::LayerMask));
    assert!(app.ui.mask_target);
    assert_eq!(app.session.tools.foreground, WHITE);
    assert_eq!(app.session.tools.background, BLACK);

    // 4. Changing mask colors preserves them when switching away and back.
    app.run("tools.setColors", json!({"foreground": "#808080"})).unwrap();
    let grey = app.session.tools.foreground;
    assert_eq!(grey, [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
    click_row(&mut app, &ctx, masked, Some(LayerPart::Pixels));
    assert_eq!(app.session.tools.foreground, RED, "pixel colors restored");
    click_row(&mut app, &ctx, masked, Some(LayerPart::LayerMask));
    assert_eq!(app.session.tools.foreground, grey, "mask colors restored");

    // 5. Creating a new adjustment layer targets its mask and toggles to mask colors.
    click_row(&mut app, &ctx, masked, Some(LayerPart::Pixels));
    assert_eq!(app.session.tools.foreground, RED);
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    assert!(app.ui.mask_target, "new adjustment layer targets its mask");
    assert_eq!(app.session.tools.foreground, grey);
}

/// #3369: a new adjustment or fill layer targets its mask, but Properties shows its settings until
/// the mask thumbnail is clicked, as in Photoshop.
#[test]
fn a_new_adjustment_or_fill_layer_shows_its_settings_until_the_mask_thumbnail_is_clicked() {
    for command in ["layer.newAdjustmentLayer.curves", "layer.newFillLayer.solidColor"] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let mut app = app_of(s);
        let ctx = app.ctx().clone();
        app.run(command, json!({})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap().0;
        assert!(app.ui.mask_target, "{command}: the new mask is targeted");
        assert_eq!(crate::canvas::paint_target(&app), json!("mask"), "{command}: painting edits the mask");
        assert_eq!(app.ui.properties_layer, Some(id), "{command}: Properties shows the layer, not its mask");
        // Clicking the row (not a thumbnail) keeps the settings.
        click_row(&mut app, &ctx, id, None);
        assert!(app.ui.mask_target);
        assert_eq!(app.ui.properties_layer, Some(id), "{command}: a row click keeps the settings");
        // The mask thumbnail switches Properties to the mask.
        click_row(&mut app, &ctx, id, Some(LayerPart::LayerMask));
        assert!(app.ui.mask_target);
        assert_eq!(app.ui.properties_layer, None, "{command}: the mask thumbnail shows the mask controls");
    }
}

/// The group (by its place in the docking layout) that holds panel `id` is minimized.
fn group_minimized(app: &PhotocraftApp, id: &str) -> bool {
    let layout = &app.ui.docking.layout;
    let place = layout.place(&id.into()).expect("the panel is docked");
    layout.group(&place).expect("its group").minimized
}

// #4028: with Adjustments showing in a minimized Properties group, a new adjustment layer brings
// Properties to the front and expands it, as in Photoshop, so its settings can be edited at once.
#[test]
fn a_new_adjustment_layer_brings_properties_to_the_front() {
    use craft::behavior::dock::Action;
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let mut h = harness(s, 1.0, 290.0);
    let _ = crate::panel_docking::apply(h.state_mut(), vec![Action::set_minimized("properties".into(), true)]);
    h.run_steps(2);
    assert!(group_minimized(h.state(), "properties"), "the group starts minimized");
    h.state_mut().run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    h.run_steps(6);
    assert!(crate::panel_docking::is_front(h.state(), "properties"), "Properties is the group's front tab");
    assert!(!group_minimized(h.state(), "properties"), "the group is expanded");
    assert!(h.query_all_by_label("Reset to defaults").next().is_some(), "the Curves settings show");
}
