//! Pointer, keyboard, colour precision and history regressions for shape appearance controls.
use super::*;
use crate::shape_options::set_tool_paint;
use crate::shape_paint_ui::color_of;
use craft::draw::geometry::ShapeGeometry;
use craft::draw::ui::shape::VectorMode;
use craft::keymap::Mods;
use craft::theme::Srgba;
use egui::{Event, Key, Modifiers};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::ShapeLayer;
use photocraft_engine::presets::PresetsExt;

fn app(mode: &str, depth: u32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 100, "mode": mode, "depth": depth})).unwrap();
    app.ui.tool = Tool::Rectangle;
    app.sync_views();
    app
}

fn shape(app: &PhotocraftApp) -> &ShapeLayer {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("expected editable shape") };
    shape
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn assert_saved_path(saved: &Path, original: &Path) {
    assert_eq!((saved.fill_rule, saved.inverted), (original.fill_rule, original.inverted));
    assert_eq!(saved.subpaths.len(), original.subpaths.len());
    for (saved, original) in saved.subpaths.iter().zip(&original.subpaths) {
        assert_eq!((saved.closed, saved.op, saved.knots.len()), (original.closed, original.op, original.knots.len()));
        for (saved, original) in saved.knots.iter().zip(&original.knots) {
            assert_eq!(saved.smooth, original.smooth);
            for (saved, original) in [(saved.anchor, original.anchor), (saved.in_ctrl, original.in_ctrl), (saved.out_ctrl, original.out_ctrl)] {
                // Native manifests use JSON f64 coordinates, whose existing round trip may
                // differ by one ulp for generated ellipse/polygon control points.
                assert!((saved.x - original.x).abs() <= 1e-10 && (saved.y - original.y).abs() <= 1e-10, "{saved:?} differs from {original:?}");
            }
        }
    }
}

/// The drag box of `tool`'s kind from `from` to `to`, no modifiers.
fn geometry(tool: Tool, from: [f64; 2], to: [f64; 2]) -> ShapeGeometry {
    let rect = craft::behavior::kurbo::Rect::new(from[0], from[1], to[0], to[1]);
    match tool {
        Tool::Rectangle => ShapeGeometry::Rect { rect, radii: [0.0; 4] },
        Tool::EllipseShape => ShapeGeometry::Ellipse { rect },
        Tool::Triangle => ShapeGeometry::Polygon { rect, sides: 3, star_ratio: 1.0 },
        other => panic!("no drag box for {other:?}"),
    }
}

/// Commits a drag of `tool` from `from` to `to` as the canvas does on release.
fn draw(app: &mut PhotocraftApp, tool: Tool, from: [f64; 2], to: [f64; 2]) {
    shapes::finish_drag(app, shapes::kind_of(tool), &geometry(tool, from, to), Mods::default());
}

fn harness(app: Box<PhotocraftApp>, properties: bool) -> Harness<'static, Box<PhotocraftApp>> {
    // kittest's default step: 0.25 s a frame.
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(0.25).with_context(ctx).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::screen_picker::tick(app, ui.ctx());
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            if properties {
                let id = app.session.active().unwrap().active_layer.unwrap();
                ui.set_max_width(320.0);
                shape_properties(app, ui, id);
            } else {
                let tool = app.ui.tool;
                ui.horizontal(|ui| options_bar(app, ui, tool));
            }
            // The Color Picker the chip opens, as the app draws it.
            crate::dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(3);
    h
}

fn stroke_width(h: &mut Harness<'_, Box<PhotocraftApp>>, width: &str) {
    h.get_all_by_role(egui::accesskit::Role::SpinButton).next().expect("the stroke width").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(width.into()));
    h.key_press(Key::Enter);
    h.run_steps(3);
}

/// The Fill or Stroke chip named `label` (a colour well; the popover's kind switch carries the same name).
fn chip<'h>(h: &'h Harness<'_, Box<PhotocraftApp>>, label: &'h str) -> egui_kittest::Node<'h> {
    h.get_by_role_and_label(egui::accesskit::Role::ColorWell, label)
}

/// The Stroke style button, which opens the Stroke Options popover.
fn stroke_style_button<'h>(h: &'h Harness<'_, Box<PhotocraftApp>>) -> egui_kittest::Node<'h> {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Stroke style")
}

/// Opens the chip `label` and picks its kind button `kind` ("No Color", "Solid", "Gradient").
fn pick_kind(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str, kind: &str) {
    chip(h, label).click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::RadioButton, kind).click();
    h.run_steps(2);
}

/// Clicks the popover's swatch `name`: a list item labelled "<name>, <hex>" in the Swatches list. The list is the Solid
/// paint's, and a group's rows show once its header is open (the group named by the swatch's first word).
fn click_swatch(h: &mut Harness<'_, Box<PhotocraftApp>>, name: &str) {
    let prefix = format!("{name}, #");
    let found = |h: &Harness<'_, Box<PhotocraftApp>>| {
        h.query_all_by_role(egui::accesskit::Role::ListItem).any(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.starts_with(&prefix)))
    };
    if !found(h) {
        // The swatches are the Solid paint's: a chip with no paint offers none until its kind is Solid.
        let group = name.split(' ').next().unwrap_or(name);
        if h.query_by_role_and_label(egui::accesskit::Role::Button, group).is_none() {
            h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Solid").click();
            h.run_steps(2);
        }
        if !found(h) {
            h.get_by_role_and_label(egui::accesskit::Role::Button, group).click();
            h.run_steps(2);
        }
    }
    h.query_all_by_role(egui::accesskit::Role::ListItem)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.starts_with(&prefix)))
        .unwrap_or_else(|| panic!("the swatch {name}"))
        .click();
    h.run_steps(2);
}

/// Opens the chip `label`, its Color Picker… button, sets `color` (the colour it opened with when
/// `None`) and presses OK.
fn pick_in_full_picker(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str, color: Option<Srgba>) {
    chip(h, label).click();
    h.run_steps(3);
    h.get_by_label("Color Picker…").click();
    h.run_steps(3);
    assert!(h.state().dialog_open(), "the Color Picker dialog");
    let id = h.state().framework.dialogs.stack().top().expect("the Color Picker dialog");
    if let Some(color) = color {
        crate::color_picker_ui::testing::set_color(h.state_mut(), id, &crate::color_picker_ui::rgb_hex(color));
        h.run_steps(2);
    }
    let ctx = h.ctx.clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    h.run_steps(2);
}

/// #6191: passing through zero hides the stroke without discarding its paint or style.
#[test]
fn zero_width_preserves_selected_shape_stroke_and_restores_it() {
    for depth in [8, 16, 32] {
        for (properties, tool) in [(true, Tool::Rectangle), (false, Tool::Rectangle), (false, Tool::PathSelection), (false, Tool::DirectSelection)] {
            let mut app = app("rgb", depth);
            app.ui.tool = tool;
            app.run(
                "shape.create",
                json!({"rect": [20, 20, 80, 60], "fill": null,
                    "stroke": {"width": 4, "color": [0.123456, 0.345678, 0.567891, 0.618237],
                        "align": "inside", "cap": "round", "join": "bevel", "miterLimit": 7,
                        "dashes": [3, 2], "dashOffset": 1.25, "opacity": 63}}),
            )
            .unwrap();
            let original = shape(&app).clone();
            let before = history(&app);
            let mut h = harness(app, properties);
            stroke_width(&mut h, "0");
            let zero = shape(h.state()).clone();
            let mut expected = original.stroke.clone().unwrap();
            expected.width = 0.0;
            assert_eq!(zero.stroke, Some(expected), "{tool:?}, Properties={properties}, depth={depth}: zero width retains the stroke");
            assert_eq!(zero.path, original.path);
            assert!(zero.cache.as_ref().unwrap().content_bounds().is_empty(), "a zero-width stroke paints nothing");
            assert_eq!(history(h.state()), before + 1);

            // A save at zero must retain the dormant stroke too.
            let st = h.state().session.active().unwrap();
            let bytes = photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap();
            let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Shape(saved) = &loaded.layer(st.active_layer.unwrap()).unwrap().content else { panic!("saved shape") };
            assert_eq!(saved.stroke, zero.stroke);

            stroke_width(&mut h, "4");
            assert_eq!(shape(h.state()), &original, "restoring width restores the complete style and rendered pixels");
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(shape(h.state()), &zero);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(shape(h.state()), &original);
            h.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(shape(h.state()), &zero);
            h.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(shape(h.state()), &original);
        }
    }
}

#[test]
fn zero_width_style_can_be_inspected_removed_or_reenabled_explicitly() {
    for properties in [true, false] {
        let mut app = app("rgb", 8);
        app.run("shape.create", json!({"rect": [20, 20, 80, 60], "stroke": {"width": 4, "color": "#e73581", "dashes": [3, 2]}})).unwrap();
        let paint = shape(&app).stroke.as_ref().unwrap().paint.clone();
        let mut h = harness(app, properties);
        stroke_width(&mut h, "0");
        let zero = shape(h.state()).clone();

        pick_kind(&mut h, "Stroke", "No Color");
        assert!(shape(h.state()).stroke.is_none(), "No Color still removes the stroke");
        h.key_press(Key::Escape);
        h.run_steps(2);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(shape(h.state()), &zero);
        h.run_steps(2);

        stroke_style_button(&h).click();
        h.run_steps(3);
        h.get_by_role_and_label(egui::accesskit::Role::ListItem, "Dotted").click();
        h.run_steps(3);
        let stroke = shape(h.state()).stroke.as_ref().unwrap();
        assert_eq!(stroke.width, 3.0, "a deliberate style choice enables the dormant stroke");
        assert_eq!(stroke.paint, paint);
        assert_eq!(stroke.dashes, [0.0, 2.0]);
    }
}

#[test]
fn a_pasted_hex_colour_updates_the_selected_shape_and_undoes_once() {
    for label in ["Fill color", "Stroke"] {
        let mut app = app("rgb", 32);
        app.run("tools.setColors", json!({"foreground": [0.2, 0.3, 0.4, 0.6], "background": [0.5, 0.6, 0.7, 0.8]})).unwrap();
        app.ui.tool_options.stroke_width = 3.0;
        draw(&mut app, Tool::Rectangle, [20.0, 20.0], [100.0, 80.0]);
        let original = shape(&app).clone();
        let before = history(&app);
        let mut h = harness(app, false);
        let alpha = if label == "Fill color" { 0.6 } else { 0.8 };
        let want = Srgba::new(18.0 / 255.0, 171.0 / 255.0, 239.0 / 255.0, alpha);
        pick_in_full_picker(&mut h, label, Some(want));
        let edited = shape(h.state());
        let (paint, untouched) = if label == "Fill color" {
            (edited.fill.as_ref().and_then(color_of), edited.stroke == original.stroke)
        } else {
            (edited.stroke.as_ref().and_then(|s| color_of(&s.paint)), edited.fill == original.fill)
        };
        assert_eq!(paint, Some(want.to_array()));
        assert!(untouched, "the other paint stays unchanged");
        assert_eq!(history(h.state()), before + 1);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(shape(h.state()), &original);
    }
}

#[test]
fn no_colour_and_solid_choices_are_explicit_live_tool_defaults() {
    let mut h = harness(app("rgb", 8), false);
    pick_kind(&mut h, "Fill color", "Gradient");
    h.key_press(Key::Escape);
    h.run_steps(2);
    pick_kind(&mut h, "Fill color", "No Color");
    assert!(!h.state().ui.tool_options.shape_fill);
    assert!(h.state().ui.tool_options.shape_fill_gradient.is_some(), "No Color keeps the gradient for when the paint comes back");
    h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Solid").click();
    h.run_steps(2);
    assert!(h.state().ui.tool_options.shape_fill);
    assert!(h.state().ui.tool_options.shape_fill_gradient.is_none(), "Solid drops the gradient");
    h.key_press(Key::Escape);
    h.run_steps(2);
    pick_kind(&mut h, "Stroke", "Solid");
    assert_eq!(h.state().ui.tool_options.stroke_width, 3.0, "choosing unchanged white enables a stroke");
    h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "No Color").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.stroke_width, 0.0);
    assert_eq!(history(h.state()), 0, "tool defaults do not edit the document");
}

#[test]
fn float_paints_match_preview_creation_native_roundtrip_and_document_depth() {
    let foreground = [0.123456, 0.345678, 0.567891, 0.372819];
    let background = [0.891234, 0.234567, 0.456789, 0.618237];
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 16), ("cmyk", 8), ("lab", 16)] {
        for tool in [Tool::Rectangle, Tool::Triangle, Tool::EllipseShape] {
            let mut app = app(mode, depth);
            set_tool_paint(&mut app, false, PaintChange::Color(foreground)).unwrap();
            set_tool_paint(&mut app, true, PaintChange::Color(background)).unwrap();
            app.ui.tool_options.shape_stroke.cap = photocraft_doc::LineCap::Round;
            app.ui.tool_options.shape_stroke.dashes = vec![0.0, 2.0];
            let drag = geometry(tool, [20.0, 20.0], [100.0, 80.0]);
            let preview = shapes::shape_preview(&app, shapes::kind_of(tool), &drag).unwrap();
            draw(&mut app, tool, [20.0, 20.0], [100.0, 80.0]);
            let committed = shape(&app).clone();
            assert_eq!(preview.fill, committed.fill);
            assert_eq!(preview.stroke, committed.stroke);
            assert_eq!(preview.path, committed.path);
            assert_eq!(committed.fill, Some(solid_paint(foreground)));
            assert_eq!(committed.stroke.as_ref().unwrap().paint, solid_paint(background));
            let doc = &app.session.active().unwrap().doc;
            let rendered = photocraft_vector::render_shape(&preview, doc.pixel_format(), doc.bounds());
            assert_eq!(committed.cache.as_ref().unwrap(), &rendered, "{tool:?} {mode}/{depth}");
            let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
            let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
            let id = app.session.active().unwrap().active_layer.unwrap();
            let LayerContent::Shape(saved) = &loaded.layer(id).unwrap().content else { panic!("saved editable shape") };
            assert_eq!((&saved.fill, &saved.stroke), (&committed.fill, &committed.stroke));
            assert_saved_path(&saved.path, &committed.path);
            app.run("edit.undo", json!({})).unwrap();
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(shape(&app), &committed);
        }
    }
}

#[test]
fn open_pen_uses_selected_stroke_color_and_width() {
    let mut app = app("rgb", 32);
    let foreground = [0.9, 0.1, 0.2, 1.0];
    let stroke = [0.1, 0.2, 0.9, 0.7];
    app.run("tools.setColors", json!({"foreground": foreground})).unwrap();
    set_tool_paint(&mut app, true, PaintChange::Color(stroke)).unwrap();
    app.ui.tool_options.stroke_width = 4.0;
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    app.ui.pen = Some(PenPath::corners(&[[20.0, 20.0], [80.0, 60.0]], super::capture_pen_owner(&app)));
    pen_commit(&mut app, false);
    let drawn = shape(&app).stroke.as_ref().unwrap();
    assert_eq!(drawn.paint, solid_paint(stroke));
    assert_eq!(drawn.width, 4.0);
}

#[test]
fn transparent_and_absent_paints_remain_distinct_and_open_pen_keeps_foreground() {
    let mut app = app("rgb", 32);
    let transparent = [0.124578, 0.564321, 0.765432, 0.0];
    set_tool_paint(&mut app, false, PaintChange::Color(transparent)).unwrap();
    set_tool_paint(&mut app, true, PaintChange::Color(transparent)).unwrap();
    let drag = geometry(Tool::Rectangle, [20.0, 20.0], [80.0, 70.0]);
    let preview = shapes::shape_preview(&app, shapes::kind_of(Tool::Rectangle), &drag).unwrap();
    assert_eq!(preview.fill, Some(solid_paint(transparent)));
    assert_eq!(preview.stroke.as_ref().unwrap().paint, solid_paint(transparent));
    draw(&mut app, Tool::Rectangle, [20.0, 20.0], [80.0, 70.0]);
    assert_eq!(shape(&app).fill, preview.fill);
    assert_eq!(shape(&app).stroke, preview.stroke);
    assert_eq!(shape(&app).cache.as_ref().unwrap().pixel(40, 40), vec![0.0; 4]);
    set_tool_paint(&mut app, false, PaintChange::None).unwrap();
    set_tool_paint(&mut app, true, PaintChange::None).unwrap();
    draw(&mut app, Tool::Triangle, [20.0, 20.0], [80.0, 70.0]);
    assert!(shape(&app).fill.is_none() && shape(&app).stroke.is_none());
    let foreground = [0.836521, 0.125468, 0.745632, 0.438271];
    app.run("tools.setColors", json!({"foreground": foreground, "background": [0.0, 1.0, 0.0, 1.0]})).unwrap();
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    app.ui.pen = Some(PenPath::corners(&[[20.0, 20.0], [80.0, 60.0]], super::capture_pen_owner(&app)));
    pen_commit(&mut app, false);
    assert!(shape(&app).fill.is_none());
    assert!(!shape(&app).path.subpaths[0].closed);
    assert_eq!(shape(&app).stroke.as_ref().unwrap().paint, solid_paint(foreground));
}

#[test]
fn selected_shape_style_is_available_when_absent_and_dismissal_does_not_enable_it() {
    let mut app = app("rgb", 8);
    app.run("shape.create", json!({"rect": [20,20,80,60], "stroke": null})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let before = history(&app);
    let mut h = harness(app, true);
    stroke_style_button(&h).click();
    h.run_steps(3);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc, doc);
    stroke_style_button(&h).click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::ListItem, "Dotted").click();
    h.run_steps(3);
    assert_eq!(shape(h.state()).stroke.as_ref().unwrap().width, 3.0);
    assert_eq!(shape(h.state()).stroke.as_ref().unwrap().dashes, [0.0, 2.0]);
    assert_eq!(history(h.state()), before + 1);
    h.key_press(Key::Escape);
    h.run_steps(2);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(shape(h.state()).stroke.is_none());
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(shape(h.state()).stroke.as_ref().unwrap().dashes, [0.0, 2.0]);
}

#[test]
fn selected_shape_swatch_clicks_are_one_undo_step_each_and_keep_tool_defaults() {
    let mut app = app("rgb", 32);
    app.run("shape.create", json!({"rect": [20,20,80,60], "fill": [0.2, 0.4, 0.6, 0.5], "stroke": {"width": 5, "color": "#ee3355"}})).unwrap();
    let original = shape(&app).clone();
    let defaults = (app.session.tools.foreground, app.session.tools.background, app.ui.tool_options.clone());
    let before = history(&app);
    let mut h = harness(app, true);
    chip(&h, "Fill").click();
    h.run_steps(4);
    assert_eq!(history(h.state()), before, "opening changes nothing");
    click_swatch(&mut h, "RGB Green");
    assert_eq!(shape(h.state()).fill, Some(solid_paint([0.0, 1.0, 0.0, 0.5])), "the fill's alpha stays");
    click_swatch(&mut h, "RGB Blue");
    assert_eq!(history(h.state()), before + 2);
    assert_eq!(shape(h.state()).stroke, original.stroke);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!((h.state().session.tools.foreground, h.state().session.tools.background, h.state().ui.tool_options.clone()), defaults);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(shape(h.state()).fill, Some(solid_paint([0.0, 1.0, 0.0, 0.5])));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(shape(h.state()).fill, original.fill);
}

#[test]
fn selected_shape_gradient_and_full_picker_edit_the_layer() {
    let mut app = app("rgb", 8);
    app.run("shape.create", json!({"rect": [20,20,80,60], "fill": "#3270c0", "stroke": null})).unwrap();
    let before = history(&app);
    let mut h = harness(app, true);
    pick_kind(&mut h, "Fill", "Gradient");
    assert!(matches!(shape(h.state()).fill, Some(photocraft_doc::Fill::Gradient { .. })), "Gradient fills the shape");
    assert_eq!(history(h.state()), before + 1);
    h.key_press(Key::Escape);
    h.run_steps(2);
    // The stroke is off: the Color Picker turns it on in the picked colour, as one edit.
    pick_in_full_picker(&mut h, "Stroke", Some(Srgba::rgb(0.0, 1.0, 0.0)));
    let stroke = shape(h.state()).stroke.clone().expect("the stroke is on");
    assert_eq!((stroke.paint, stroke.width), (solid_paint([0.0, 1.0, 0.0, 1.0]), 3.0));
    assert_eq!(history(h.state()), before + 2);
    // The fill's picker from the gradient: OK on the colour it opened with still makes it solid.
    pick_in_full_picker(&mut h, "Fill", None);
    assert_eq!(shape(h.state()).fill, Some(solid_paint(h.state().session.tools.foreground)));
}

fn test_gradient() -> photocraft_doc::Fill {
    photocraft_doc::Fill::Gradient {
        stops: vec![(0.0, photocraft_doc::Color::rgba(0.123456, 0.5, 0.75, 1.0)), (1.0, photocraft_doc::Color::rgba(0.9, 0.2, 0.1, 0.6))],
        angle: 30.0,
        scale: 0.5,
        style: photocraft_doc::GradientStyle::Radial,
        reverse: true,
        opacity_stops: vec![(0.0, 1.0), (1.0, 0.25)],
        midpoints: vec![0.3],
        offset: (0.125, -0.25),
        dither: true,
        align: false,
    }
}

#[test]
fn open_pen_preserves_selected_stroke_gradient() {
    let mut app = app("rgb", 32);
    set_tool_paint(&mut app, true, PaintChange::Gradient(test_gradient())).unwrap();
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    app.ui.pen = Some(PenPath::corners(&[[20.0, 20.0], [80.0, 60.0]], super::capture_pen_owner(&app)));
    pen_commit(&mut app, false);
    assert_eq!(shape(&app).stroke.as_ref().unwrap().paint, test_gradient());
}

/// A gradient picked for the next shape's stroke and fill (the swatches' Gradient tab) reaches
/// the drag preview and the committed shape exactly, every gradient field kept; a colour picked
/// afterwards replaces it.
#[test]
fn tool_gradient_paints_reach_the_preview_and_the_next_shape() {
    for (mode, depth) in [("rgb", 8), ("rgb", 32), ("gray", 16)] {
        let mut app = app(mode, depth);
        set_tool_paint(&mut app, true, PaintChange::Gradient(test_gradient())).unwrap();
        set_tool_paint(&mut app, false, PaintChange::Gradient(test_gradient())).unwrap();
        assert_eq!(app.ui.tool_options.stroke_width, 3.0, "a gradient enables the stroke");
        assert!(app.ui.tool_options.shape_fill);
        let drag = geometry(Tool::Rectangle, [20.0, 20.0], [100.0, 80.0]);
        let preview = shapes::shape_preview(&app, shapes::kind_of(Tool::Rectangle), &drag).unwrap();
        draw(&mut app, Tool::Rectangle, [20.0, 20.0], [100.0, 80.0]);
        let committed = shape(&app).clone();
        assert_eq!(preview.fill, committed.fill);
        assert_eq!(preview.stroke, committed.stroke);
        assert_eq!(committed.stroke.as_ref().unwrap().paint, test_gradient(), "{mode}/{depth}");
        assert_eq!(committed.fill, Some(test_gradient()));
        let doc = &app.session.active().unwrap().doc;
        let rendered = photocraft_vector::render_shape(&preview, doc.pixel_format(), doc.bounds());
        assert_eq!(committed.cache.as_ref().unwrap(), &rendered, "{mode}/{depth}");
        set_tool_paint(&mut app, true, PaintChange::Color([0.0, 1.0, 0.0, 1.0])).unwrap();
        assert!(app.ui.tool_options.shape_stroke_gradient.is_none());
        draw(&mut app, Tool::Rectangle, [20.0, 20.0], [100.0, 80.0]);
        assert_eq!(shape(&app).stroke.as_ref().unwrap().paint, solid_paint([0.0, 1.0, 0.0, 1.0]));
        assert_eq!(shape(&app).fill, Some(test_gradient()), "the fill keeps its gradient");
    }
}

/// Options bar › Stroke › Gradient: the Rectangle tool's next stroke takes the current gradient,
/// a preset's colours and a style, without editing the document; the next shape draws with it.
#[test]
fn stroke_swatch_gradient_tab_sets_the_next_shapes_stroke() {
    let mut h = harness(app("rgb", 8), false);
    pick_kind(&mut h, "Stroke", "Gradient");
    let stroke_gradient = |h: &Harness<'_, Box<PhotocraftApp>>| match h.state().ui.tool_options.shape_stroke_gradient.clone() {
        Some(photocraft_doc::Fill::Gradient { stops, style, angle, .. }) => (stops, style, angle),
        other => panic!("a stroke gradient, not {other:?}"),
    };
    let (_, style, angle) = stroke_gradient(&h);
    assert_eq!((style, angle), (photocraft_doc::GradientStyle::Linear, 90.0));
    assert_eq!(h.state().ui.tool_options.stroke_width, 3.0, "choosing a gradient enables the stroke");
    h.get_by_label("Blue 01").click();
    h.run_steps(2);
    let presets = h.state().session.presets().gradients.clone();
    let blue = presets.iter().flat_map(|g| &g.items).find(|p| p.name == "Blue 01").unwrap();
    let (fg, bg) = (h.state().session.tools.foreground, h.state().session.tools.background);
    let blue_stops = blue.fill_stops(fg, bg).0;
    assert_eq!(stroke_gradient(&h).0, blue_stops);
    h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Radial").click();
    h.run_steps(2);
    let (stops, style, _) = stroke_gradient(&h);
    assert_eq!((stops, style), (blue_stops, photocraft_doc::GradientStyle::Radial), "the style keeps the colours");
    assert_eq!(history(h.state()), 0, "tool defaults do not edit the document");
    draw(h.state_mut(), Tool::Rectangle, [20.0, 20.0], [100.0, 80.0]);
    assert_eq!(shape(h.state()).stroke.as_ref().unwrap().paint, tool_stroke_gradient(&h), "the next shape's stroke is the tool gradient");
    h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Solid").click();
    h.run_steps(2);
    assert!(h.state().ui.tool_options.shape_stroke_gradient.is_none(), "Solid goes back to the colour");
}

/// Properties › Stroke › Gradient gives the selected shape a gradient stroke in one undoable
/// step, keeping its width; a style change keeps its colours.
#[test]
fn properties_stroke_swatch_gives_the_selected_shape_a_gradient_stroke() {
    let mut app = app("rgb", 16);
    app.run("shape.create", json!({"rect": [20, 20, 60, 40], "stroke": {"width": 4, "color": [1.0, 0.0, 0.0, 1.0]}})).unwrap();
    let before = history(&app);
    let mut h = harness(app, true);
    pick_kind(&mut h, "Stroke", "Gradient");
    let stroke = shape(h.state()).stroke.clone().unwrap();
    let photocraft_doc::Fill::Gradient { stops, .. } = &stroke.paint else { panic!("a gradient stroke, not {:?}", stroke.paint) };
    assert_eq!(stroke.width, 4.0);
    assert_eq!(history(h.state()), before + 1);
    h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Reflected").click();
    h.run_steps(3);
    let photocraft_doc::Fill::Gradient { stops: after, style, .. } = shape(h.state()).stroke.clone().unwrap().paint else { panic!("still a gradient stroke") };
    assert_eq!((&after, style), (stops, photocraft_doc::GradientStyle::Reflected));
    while history(h.state()) > before {
        h.state_mut().run("edit.undo", json!({})).unwrap();
    }
    assert_eq!(shape(h.state()).stroke.as_ref().unwrap().paint, solid_paint([1.0, 0.0, 0.0, 1.0]));
}

fn tool_stroke_gradient(h: &Harness<'_, Box<PhotocraftApp>>) -> photocraft_doc::Fill {
    h.state().ui.tool_options.shape_stroke_gradient.clone().expect("a stroke gradient")
}

fn gradient_stops(f: &photocraft_doc::Fill) -> photocraft_engine::presets::gradients::FillStops {
    match f {
        photocraft_doc::Fill::Gradient { stops, opacity_stops, .. } => (stops.clone(), opacity_stops.clone()),
        other => panic!("a gradient, not {other:?}"),
    }
}

/// A swatch edit that only changes the gradient's settings (Reverse) keeps a CMYK document's
/// stops as they are: before, every stop went out as sRGB RGBA and came back in the RGB model.
#[test]
fn gradient_tab_edit_keeps_cmyk_stops_in_their_model() {
    let cmyk = |c: [f32; 4]| photocraft_doc::Color { mode: photocraft_doc::ColorMode::Cmyk, c, alpha: 1.0 };
    let stops = vec![(0.0, cmyk([0.1, 0.2, 0.3, 0.9])), (1.0, cmyk([0.75, 0.0, 0.5, 0.0]))];
    for (properties, label) in [(true, "Stroke"), (false, "Stroke")] {
        let mut app = app("cmyk", 8);
        let stroke = json!({"width": 4, "color": {"gradient": {"stops": [[0.0, stops[0].1], [1.0, stops[1].1]]}}});
        app.run("shape.create", json!({"rect": [20, 20, 60, 40], "stroke": stroke})).unwrap();
        assert_eq!(gradient_stops(&shape(&app).stroke.as_ref().unwrap().paint).0, stops);
        let mut h = harness(app, properties);
        chip(&h, label).click();
        h.run_steps(3);
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Reverse").click();
        h.run_steps(3);
        let paint = shape(h.state()).stroke.clone().unwrap().paint;
        assert!(matches!(paint, photocraft_doc::Fill::Gradient { reverse: true, .. }), "Properties={properties}: Reverse applied");
        assert_eq!(gradient_stops(&paint).0, stops, "Properties={properties}: the stops keep their CMYK model and values");
    }
}

/// Options bar with a strokeless shape selected and the stroke swatch's gradient chosen: giving
/// the shape a stroke through the width field or the stroke type takes that gradient, as the next
/// shape does, not a solid background colour.
#[test]
fn width_and_stroke_type_give_a_strokeless_shape_the_tool_stroke_gradient() {
    for via_width in [true, false] {
        let mut app = app("rgb", 8);
        app.run("shape.create", json!({"rect": [20, 20, 80, 60], "fill": "#336699", "stroke": null})).unwrap();
        set_tool_paint(&mut app, true, PaintChange::Gradient(test_gradient())).unwrap();
        let mut h = harness(app, false);
        if via_width {
            stroke_width(&mut h, "5");
        } else {
            stroke_style_button(&h).click();
            h.run_steps(3);
            h.get_by_role_and_label(egui::accesskit::Role::ListItem, "Dotted").click();
            h.run_steps(3);
        }
        let stroke = shape(h.state()).stroke.clone().expect("a stroke");
        assert_eq!(stroke.paint, test_gradient(), "via_width={via_width}");
        assert_eq!(stroke.width, if via_width { 5.0 } else { 3.0 });
    }
}

/// An arrow-key step at 0 px on a strokeless shape must not add an invisible stroke.
#[test]
fn arrow_down_at_zero_width_keeps_a_strokeless_shape_strokeless() {
    for properties in [true, false] {
        let mut app = app("rgb", 8);
        app.run("shape.create", json!({"rect": [20, 20, 80, 60], "fill": "#336699", "stroke": null})).unwrap();
        assert!(shape(&app).stroke.is_none());
        let before = history(&app);
        let mut h = harness(app, properties);
        h.get_all_by_role(egui::accesskit::Role::SpinButton).next().expect("the stroke width").click();
        h.run();
        h.key_press(Key::ArrowDown);
        h.run_steps(3);
        assert!(shape(h.state()).stroke.is_none(), "Properties={properties}: Down at 0 px added a stroke");
        assert_eq!(history(h.state()), before, "Properties={properties}: no history step");
        // The field has the keys: Up still enables a 1 px stroke.
        h.key_press(Key::ArrowUp);
        h.run_steps(3);
        assert_eq!(shape(h.state()).stroke.as_ref().map(|s| s.width), Some(1.0), "Properties={properties}");
    }
}

/// A swatch the selected shape refuses (a job holds its document) doesn't join the Recently Used
/// Colors, in the Properties panel or the options bar; once the shape takes it, it does.
#[test]
fn a_refused_popover_swatch_is_not_a_recently_used_colour() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    for (properties, popover) in [(true, "Fill"), (false, "Fill color")] {
        let mut app = app("rgb", 8);
        app.run("shape.create", json!({"rect": [20,20,80,60], "fill": "#336699"})).unwrap();
        let original = shape(&app).clone();
        let gate = Arc::new(AtomicBool::new(false));
        let open = gate.clone();
        let started = app
            .session
            .start_job(
                "test.hold",
                json!({}),
                "Hold",
                true,
                move |ctx| {
                    while !open.load(Ordering::Relaxed) {
                        ctx.check()?;
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Ok(())
                },
                |_, ()| Ok(json!({})),
            )
            .unwrap();
        let photocraft_engine::jobs::Started::Job(job) = started else { panic!("the job ran inline") };
        let mut h = harness(app, properties);
        chip(&h, popover).click();
        h.run_steps(3);
        click_swatch(&mut h, "RGB Green");
        assert_eq!(shape(h.state()).fill, original.fill, "properties={properties}: the shape refused it");
        assert!(crate::paint_picker::recent_colors(&h.state().session).is_empty(), "properties={properties}: a refused colour isn't recent");
        h.key_press(Key::Escape);
        h.run_steps(2);
        gate.store(true, Ordering::Relaxed);
        h.state_mut().session.cancel_job(job);
        h.state_mut().session.join_cancelled_jobs();
        h.run_steps(2);
        chip(&h, popover).click();
        h.run_steps(3);
        click_swatch(&mut h, "RGB Green");
        assert_eq!(shape(h.state()).fill, Some(solid_paint([0.0, 1.0, 0.0, 1.0])), "properties={properties}");
        let recent: Vec<[f32; 3]> = crate::paint_picker::recent_colors(&h.state().session).into_iter().collect();
        assert_eq!(recent, [[0.0, 1.0, 0.0]], "properties={properties}");
    }
}

/// CodeRabbit on #9456: in Path mode the Radius field sets the next outline's radius only; it
/// must not edit a shape layer that happens to be active.
#[test]
fn path_mode_radius_leaves_the_active_shape_layer_alone() {
    let mut app = app("rgb", 8);
    app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 60, 40], "radii": 10})).unwrap();
    let radii = |app: &PhotocraftApp| match &shape(app).live {
        Some(photocraft_doc::LiveShape::Rect { radii, .. }) => *radii,
        other => panic!("expected a live rectangle, got {other:?}"),
    };
    assert_eq!(radii(&app), [10.0; 4]);
    app.ui.tool_options.corner_radius = 3.0;
    app.ui.tool_options.shape_mode = VectorMode::Path;
    let before = history(&app);
    let mut h = harness(app, false);
    h.get_all_by_role(egui::accesskit::Role::SpinButton).next().expect("the radius").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("20".into()));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.corner_radius, 20.0, "the next outline's radius");
    assert_eq!(radii(h.state()), [10.0; 4], "the shape layer keeps its radius");
    assert_eq!(history(h.state()), before, "no edit");
}
