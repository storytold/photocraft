//! Blur Gallery glue: the bounds a dialog opens with, and the engine's pin payload per kind.

use craft::behavior::kurbo::Vec2;
use craft::imaging::ui::{BlurKind, BlurPin, BlurPins, IrisShape, TiltBand};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog};
use egui::{Event, Modifiers, PointerButton, Shape, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::blur_pins::{COMMAND, IRIS, SPIN, TILT, initial, kind_of, pins_of, write_pins};

fn app_with(selection: bool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    if selection {
        app.run("select.rect", json!({"x": 100, "y": 50, "width": 200, "height": 100})).unwrap();
    }
    app
}

#[test]
fn blur_gallery_overlay_draws_and_drags_on_the_canvas() {
    let snapshots = std::env::var_os("PHOTOCRAFT_BLUR_PINS_SNAPSHOTS");
    let builder = Harness::builder().with_size(vec2(1500.0, 950.0)).with_pixels_per_point(1.25);
    let builder = if snapshots.is_some() { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#20547f"})).unwrap();
        app.background_jobs = false;
        app
    });
    h.run_steps(4);
    // Keep the handles clear of the dialog, which opens in the middle of the window.
    let pan = h.state().last_canvas_rect.width() * 0.23;
    let view = &mut h.state_mut().ui.views[0];
    view.zoom = 1.0;
    view.center = [200.0 + pan, 150.0];
    view.clear_pending();
    h.run_steps(2);
    let ctx = h.ctx.clone();

    for command in [SPIN, IRIS, TILT, COMMAND] {
        crate::menus::invoke(h.state_mut(), &ctx, command, json!({})).unwrap();
        h.run_steps(5);
        let id = h.state().framework.dialogs.stack().top().expect("the filter dialog is open");
        let kind = kind_of(command).unwrap();
        let bounds = [0.0, 0.0, 400.0, 300.0];
        let fields = |app: &PhotocraftApp| app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields().clone();
        let before = pins_of(&fields(h.state()), kind, bounds);
        let pin = before.pins.first().expect("the dialog seeds its first pin");
        let centre = before.to_document(pin);
        let view = crate::canvas::testing::view(h.state()).expect("the primary canvas is visible");
        let screen_centre = view.to_screen(centre);

        if let Some(dir) = &snapshots {
            let path = std::path::Path::new(dir).join(format!("{command}.png"));
            h.render().unwrap().save(path).unwrap();
        }
        let (from, to) = if let Some(iris) = pin.iris {
            let left = view.to_screen(centre - Vec2::new(iris.radius_x * before.short_side(), 0.0));
            assert!(
                h.output().shapes.iter().any(|shape| {
                    shape.clip_rect.contains(left)
                        && matches!(&shape.shape, Shape::Path(path) if path.points.len() > 16 && path.points.iter().any(|point| point.distance(left) < 1.0))
                }),
                "{command}: the ellipse is visible at the resize handle"
            );
            (left, left - vec2(24.0, 0.0))
        } else if let Some(band) = pin.tilt {
            let edge = view.to_screen(centre + Vec2::new(-45.0, band.focus * before.short_side()));
            assert!(
                h.output().shapes.iter().any(|shape| {
                    shape.clip_rect.contains(edge)
                        && matches!(&shape.shape, Shape::Path(path) if path.points.len() == 2 && path.visual_bounding_rect().width() > 150.0
                            && path.visual_bounding_rect().contains(edge))
                }),
                "Tilt-Shift opens with the focus band visible"
            );
            (edge, edge + vec2(0.0, 16.0))
        } else {
            (screen_centre, screen_centre - vec2(24.0, 0.0))
        };
        assert!(h.query_by_label_contains("Pin 1, Blur").is_some(), "{command}: the pin is exposed to assistive technology");

        h.hover_at(from);
        h.run_steps(2);
        h.drag_at(from);
        h.run_steps(2);
        h.hover_at(from.lerp(to, 0.5));
        h.run_steps(1);
        h.hover_at(to);
        h.run_steps(1);
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);

        let changed = fields(h.state());
        let after = pins_of(&changed, kind, bounds);
        let moved = &after.pins[0];
        if let Some(iris) = pin.iris {
            let radius = moved.iris.unwrap().radius_x;
            assert!(radius > iris.radius_x, "{command}: dragging the left handle grows the radius");
            assert_eq!(after.to_document(moved), centre, "{command}: resizing keeps the pin centred");
            assert_eq!(changed.number("radiusX"), Some(radius), "{command}: Radius X shows the new radius");
        } else if let Some(band) = pin.tilt {
            assert!(changed.number("focus").unwrap() > band.focus, "dragging the focus line widens the band");
        } else {
            assert!(after.to_document(moved).x < centre.x, "{command}: dragging moves the centre pin");
            assert_eq!(changed.number("centerX"), Some(moved.x), "{command}: Center X shows the new position");
        }
        if command == SPIN
            && let Some(dir) = &snapshots
        {
            h.render().unwrap().save(std::path::Path::new(dir).join("spin-blur-resized.png")).unwrap();
        }

        h.state_mut().framework.dialogs.cancel_by_id(&ctx, id);
        h.run_steps(3);
        assert!(h.query_by_label_contains("Pin 1, Blur").is_none(), "{command}: closing the dialog removes its pin");
    }
}

#[test]
fn it_opens_with_one_selected_pin_in_the_middle_of_the_bounds() {
    for (selection, want) in [(false, [0.0, 0.0, 400.0, 300.0]), (true, [100.0, 50.0, 300.0, 150.0])] {
        let mut app = app_with(selection);
        crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        assert!(app.framework.dialogs.get::<HostedParamDialog>().is_some(), "the dialog is hosted");
        let (kind, bounds, start) = initial(&mut app, COMMAND).unwrap();
        assert_eq!((kind, bounds), (BlurKind::Field, want));
        let pins = BlurPins::new(kind, bounds, start);
        assert_eq!((pins.pins.len(), pins.selected, pins.bounds), (1, Some(0), want));
    }
    assert!(initial(&mut app_with(false), "filter.gaussianBlur").is_none());
}

/// The pins of each kind, written as the engine's command keys and read back.
#[test]
fn each_kind_writes_the_engines_pin_payload() {
    let bounds = [0.0, 0.0, 400.0, 300.0];
    let mut iris = BlurPin::new(0.5, 0.5, 20.0);
    iris.iris = Some(IrisShape::new(0.2, 0.1, 30.0, 40.0, 0.5));
    // Spin has neither roundness nor feather, so its pin carries the Spin defaults for both.
    let spin_shape = IrisShape::spin();
    let mut spin = BlurPin::new(0.5, 0.5, 20.0);
    spin.iris = Some(IrisShape::new(0.2, 0.1, 30.0, spin_shape.roundness, spin_shape.feather));
    let mut tilt = BlurPin::new(0.4, 0.6, 20.0);
    tilt.tilt = Some(TiltBand::new(20.0, 0.2, 0.3));
    let table = [
        (COMMAND, BlurPin::new(0.5, 0.5, 20.0), vec!["blur", "x", "y"]),
        (IRIS, iris, vec!["angle", "blur", "feather", "radiusX", "radiusY", "roundness", "x", "y"]),
        (SPIN, spin, vec!["angle", "blurAngle", "radiusX", "radiusY", "x", "y"]),
    ];
    for (command, start, keys) in table {
        let kind = kind_of(command).unwrap();
        let pins = BlurPins::new(kind, bounds, start);
        let mut fields = DialogFields::new();
        write_pins(&pins, |k, v| fields.set(k, v));
        let Some(FieldValue::Json(list)) = fields.get("pins") else { panic!("{command}: pins are Json") };
        let mut got: Vec<&str> = list[0].as_object().unwrap().keys().map(String::as_str).collect();
        got.sort_unstable();
        assert_eq!(got, keys, "{command}");
        for top in ["centerX", "radiusX"] {
            assert!(!fields.contains(top), "{command}: no top-level {top}");
        }
        assert_eq!(pins_of(&fields, kind, bounds), pins, "{command}: round trip");
    }
    let pins = BlurPins::new(BlurKind::TiltShift, bounds, tilt);
    let mut fields = DialogFields::new();
    write_pins(&pins, |k, v| fields.set(k, v));
    for key in ["centerX", "centerY", "angle", "focus", "transition", "blur"] {
        assert!(fields.number(key).is_some(), "Tilt-Shift {key}");
    }
    assert!(!fields.contains("pins"));
    assert_eq!(pins_of(&fields, BlurKind::TiltShift, bounds), pins);
    // The untagged read of an array of objects is a list of Json: the same pins.
    let mut fields = DialogFields::new();
    fields.set("pins", FieldValue::List(vec![FieldValue::Json(json!({"x": 0.1, "y": 0.2, "blur": 7.0}))]));
    assert_eq!(pins_of(&fields, BlurKind::Field, bounds).pins, vec![BlurPin::new(0.1, 0.2, 7.0)]);
}
