use super::*;
use crate::control::{ControlRequest, Outcome};
use egui::{Rect as ScreenRect, pos2, vec2};

/// Keep the fixture in memory so command, preference and rendering-shape checks require neither disk state nor a GPU.
fn app() -> (PhotocraftApp, egui::Context) {
    (PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()), egui::Context::default())
}

/// Keep a frame's shapes for assertions while explicitly discarding updates that no renderer will apply.
fn shape_frame(ctx: &egui::Context, input: egui::RawInput, draw: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
    let mut frame = ctx.run_ui(input, draw);
    frame.textures_delta.clear();
    frame
}

/// Exercise the production control handler and require an immediate reply, exposing rejected patches and partial mutations.
fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, p: Value) -> Value {
    let (req, _) = ControlRequest::new(method, p);
    match crate::control::handle(app, ctx, &req) {
        Outcome::Done(v) => v,
        _ => panic!("expected immediate {method} reply"),
    }
}

/// Paint the production snapshot in isolation for geometry checks; full-canvas tests separately cover frame ordering.
fn paint_view(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, idx: usize) {
    let shown = snapshot(app, idx);
    paint(app, painter, xf, shown.as_deref());
}

/// Use the registered menu transport rather than calling the mutator directly, covering command dispatch as well as state.
fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, action: &str) -> Value {
    call(app, ctx, "ui.menu.invoke", json!({"id": format!("{PREFIX}{action}")}))
}

/// Offset the viewport and camera so a guide accidentally anchored at the screen origin cannot pass the geometry checks.
fn xf() -> ViewXform {
    ViewXform {
        rect: ScreenRect::from_min_size(pos2(50.0, 90.0), vec2(900.0, 600.0)),
        zoom: 1.0,
        center: [300.0, 200.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    }
}

/// Older settings start off; damaged guide fields fall back independently without erasing unrelated Show/Grid choices.
#[test]
fn composition_preferences_default_restore_and_malformed_fields() {
    assert_eq!(serde_json::from_value::<crate::view_cmds::ViewOptions>(json!({})).unwrap().composition_guides, CompositionGuides::default());
    for bad in [Value::Null, json!([]), json!("bad"), json!({"enabled": "yes", "kind": "grid", "orientation": 8})] {
        assert_eq!(serde_json::from_value::<CompositionGuides>(bad).unwrap(), CompositionGuides::default());
    }
    let (mut a, ctx) = app();
    a.session.prefs.edit(|p| p.view_extras = json!({"compositionGuides":{"enabled":true,"kind":"goldenSpiral","orientation":7},"show":{"notes":false}}));
    crate::view_cmds::restore(&mut a);
    let wanted = CompositionGuides { enabled: true, kind: GuideKind::GoldenSpiral, orientation: 7 };
    assert_eq!(a.ui.view.composition_guides, wanted);
    assert!(!a.ui.view.show.notes);
    crate::view_cmds::persist(&mut a, &ctx);
    let saved = a.session.prefs().view_extras.clone();
    assert_eq!(saved["compositionGuides"], json!(wanted));
    a.ui.view.composition_guides = Default::default();
    crate::view_cmds::restore(&mut a);
    assert_eq!(a.ui.view.composition_guides, wanted);
    a.session.prefs.edit(|p| p.view_extras = json!({"compositionGuides":{"enabled":true,"kind":null,"orientation":-1},"grid":true}));
    crate::view_cmds::restore(&mut a);
    assert_eq!(a.ui.view.composition_guides, CompositionGuides { enabled: true, ..Default::default() });
    assert!(a.ui.extras.grid);
    let malformed = CompositionGuides::from_saved(&json!({"enabled":true,"kind":{"goldenSpiral":null},"orientation":6}));
    assert_eq!(
        malformed,
        CompositionGuides { enabled: true, kind: GuideKind::Thirds, orientation: 6 },
        "object-shaped kinds fall back without discarding other valid saved fields"
    );
}

/// Mixed tool and layout patches reject invalid fields atomically, and valid guide settings remain inspectable without a document.
#[test]
fn composition_control_is_strict_atomic_inspectable_and_works_without_document() {
    let (mut a, ctx) = app();
    let patch = json!({"compositionGuides":{"enabled":true,"kind":"goldenSpiral","orientation":7}});
    assert_eq!(call(&mut a, &ctx, "ui.set", patch)["ok"], true);
    let wanted = a.ui.view.composition_guides;
    assert_eq!(call(&mut a, &ctx, "ui.inspect", json!({}))["result"]["view"]["composition_guides"], json!(wanted));
    for bad in [
        json!(null),
        json!([]),
        json!(true),
        json!({"enabled":1}),
        json!({"kind":"grid"}),
        json!({"kind":null}),
        json!({"kind":{"thirds":null}}),
        json!({"kind":{"goldenRatio":null}}),
        json!({"kind":{"goldenSpiral":null}}),
        json!({"orientation":-1}),
        json!({"orientation":8}),
        json!({"orientation":1.5}),
        json!({"orientation":"2"}),
        json!({"orientation":1e100}),
        json!({"extra":true}),
        json!({"enabled":false,"orientation":null}),
    ] {
        let tool = a.ui.tool;
        let reply = call(&mut a, &ctx, "ui.set", json!({"compositionGuides":bad,"tool":"pencil"}));
        assert_eq!(reply["ok"], false, "{reply}");
        assert_eq!(a.ui.view.composition_guides, wanted);
        assert_eq!(a.ui.tool, tool);
    }
    let reply = call(&mut a, &ctx, "ui.set", json!({"compositionGuides":{"enabled":false},"cropOverlay":"bad"}));
    assert_eq!(reply["ok"], false);
    assert_eq!(a.ui.view.composition_guides, wanted);
    let task_bar = json!(a.ui.panels.task_bar);
    let reply = call(&mut a, &ctx, "ui.set", json!({"compositionGuides":{"orientation":8},"panels":{"task_bar":{"pinned":true,"at":[15.0,25.0]}}}));
    assert_eq!(reply["ok"], false);
    assert_eq!(json!(a.ui.panels.task_bar), task_bar, "invalid guides must not apply the independently merged Crop task-bar patch");
    assert_eq!(a.ui.view.composition_guides, wanted);
    let side = a.ui.panels.toolbar_side;
    let reply = call(&mut a, &ctx, "ui.set", json!({"compositionGuides":{"enabled":false},"panels":{"toolbar_side":"diagonal"}}));
    assert_eq!(reply["ok"], false);
    assert_eq!(a.ui.panels.toolbar_side, side);
    assert_eq!(a.ui.view.composition_guides, wanted, "a refused toolbar side must not hide the guides");
    let reply = call(&mut a, &ctx, "ui.set", json!({"compositionGuides":{"enabled":false},"panels":{"toolbar_side":"right"}}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(a.ui.panels.toolbar_side, crate::state::ToolbarSide::Right);
    assert_eq!(a.ui.view.composition_guides, CompositionGuides { enabled: false, ..wanted });
}

/// View commands preserve the document/history, active-layer marker, Move reveal pick and separately persisted toolbar history.
#[test]
fn composition_menu_enablement_and_history_are_view_only() {
    let (mut a, ctx) = app();
    assert!(!a.ui.view.composition_guides.enabled);
    assert_eq!(enabled(&a, "view.compositionGuides.rotate"), Some(false));
    a.run("file.new", json!({"width":64,"height":48})).unwrap();
    a.ui.remember_group_tool(Tool::PerspectiveCrop);
    a.ui.remember_group_tool(Tool::Eyedropper);
    a.ui.tool = Tool::Move;
    crate::tool_memory::persist(&mut a, &ctx);
    let tools = a.ui.group_tools.clone();
    let saved_tools = a.session.prefs().group_tools.clone();
    let marker = a.session.active_layer_changes();
    let active_layer = a.session.active().unwrap().active_layer.unwrap();
    a.move_pick = Some((active_layer, marker));
    let pick = a.move_pick;
    let original = a.session.active().unwrap().doc.clone();
    let revision = a.session.active().unwrap().revision;
    let document = photocraft_engine::inspect::document(a.session.active().unwrap());
    let undo = a.session.active().unwrap().history.can_undo();
    let redo = a.session.active().unwrap().history.can_redo();
    for action in ["show", "goldenRatio", "goldenSpiral", "rotate", "flipHorizontal", "flipVertical", "thirds", "show"] {
        let response = menu(&mut a, &ctx, action);
        assert_eq!(response["ok"], true, "{action}: {response}");
        assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &original));
        assert_eq!(a.session.active().unwrap().revision, revision);
        assert_eq!(a.session.active().unwrap().history.can_undo(), undo);
        assert_eq!(a.session.active().unwrap().history.can_redo(), redo);
        assert_eq!(photocraft_engine::inspect::document(a.session.active().unwrap()), document);
        crate::view_cmds::persist(&mut a, &ctx);
        assert_eq!(a.session.active_layer_changes(), marker, "a guide choice must not invalidate upstream Move selection tracking");
        assert_eq!(a.move_pick, pick);
        assert_eq!(a.ui.group_tools, tools);
        assert_eq!(a.session.prefs().group_tools, saved_tools, "the View preference bucket must not overwrite toolbar history");
    }
    let wanted = a.ui.view.composition_guides;
    for p in [json!({"on":"yes"}), json!({"on":null}), json!({"kind":"grid"})] {
        assert!(invoke(&mut a, "view.compositionGuides.show", &p).is_err());
        assert_eq!(a.ui.view.composition_guides, wanted);
    }
    assert!(invoke(&mut a, "view.compositionGuides.rotate", &json!({})).is_err());
    let items = crate::menus::menu_items(&a);
    for action in ACTIONS {
        let id = format!("{PREFIX}{action}");
        let item = items.iter().find(|i| i.id == id).unwrap();
        assert_eq!(item.path, ["View", "Composition Guides"]);
        assert!(item.shortcut.is_none());
    }
}

/// Prevent the submenu falling below the normal scroll fold or losing its path while preserving existing explicit menu anchors.
#[test]
fn composition_menu_is_beside_extras_and_preserves_submenu_and_existing_placement() {
    let (a, _) = app();
    let items = crate::menus::menu_items(&a);
    let extras = items.iter().position(|i| i.id == "view.extras").unwrap();
    let show = items.iter().position(|i| i.path == ["View", "Show"]).unwrap();
    let guides: Vec<_> = items.iter().enumerate().filter(|(_, i)| handles(&i.id)).collect();
    assert_eq!(guides.len(), 7);
    assert!(guides.iter().all(|(n, _)| *n > extras && *n < show), "composition guides should be beside Extras and Show, not buried at the end of View");
    assert!(guides.iter().all(|(_, i)| i.path == ["View", "Composition Guides"]));
    let flip = items.iter().position(|i| i.id == "view.flipHorizontal").unwrap();
    assert_eq!(items[flip + 1].id, "view.resetView", "existing explicit placements must keep working");
}

/// Tool/document changes retain one global choice; Crop owns overlays even when its own guides are hidden.
#[test]
fn composition_arbitrates_crop_extras_tools_and_documents() {
    let (mut a, ctx) = app();
    menu(&mut a, &ctx, "goldenSpiral");
    let crop_before = (a.ui.tool_options.crop_overlay, a.ui.tool_options.crop_overlay_show, a.ui.tool_options.crop_overlay_orientation);
    for tool in [Tool::Brush, Tool::Move, Tool::Eyedropper, Tool::Crop, Tool::PerspectiveCrop, Tool::Brush] {
        crate::tool_memory::select(&mut a, tool);
        assert_eq!(a.ui.view.composition_guides.visible(&a), !matches!(tool, Tool::Crop | Tool::PerspectiveCrop));
        a.ui.view.extras = false;
        assert!(!a.ui.view.composition_guides.visible(&a));
        a.ui.view.extras = true;
    }
    assert_eq!((a.ui.tool_options.crop_overlay, a.ui.tool_options.crop_overlay_show, a.ui.tool_options.crop_overlay_orientation), crop_before);
    let wanted = a.ui.view.composition_guides;
    for (w, h) in [(300, 200), (200, 300), (80, 80), (1, 1)] {
        a.run("file.new", json!({"width":w,"height":h})).unwrap();
        assert_eq!(a.ui.view.composition_guides, wanted);
    }
    for i in 0..4 {
        assert!(a.session.set_active(i));
        assert_eq!(a.ui.view.composition_guides, wanted);
    }
}

/// Negative origins, skinny/tiny canvases and transformed cameras expose screen-space and aspect-ratio shortcuts; invalid geometry stays harmless.
#[test]
fn composition_geometry_tracks_canvas_through_camera_and_resize() {
    for (w, h) in [(1, 1), (300, 200), (200, 300), (200, 200), (1, 10000)] {
        let b = Rect::new(-25, 40, w - 25, h + 40);
        for zoom in [0.01, 0.5, 1.0, 8.0] {
            for rotation in [0.0, 33.0, 90.0, -145.0] {
                for flip in [false, true] {
                    for aspect in [0.5, 1.0, 2.0] {
                        let xf = ViewXform { zoom, rotation, flip, aspect, center: [-17.0, 85.0], ..xf() };
                        let g = CompositionGuides { enabled: true, ..Default::default() };
                        let lines = screen_guides(g, b, &xf);
                        assert_eq!(lines.len(), 4);
                        for (line, f) in [(0, 1.0 / 3.0), (2, 2.0 / 3.0)] {
                            for (pt, y) in [(0, 40.0), (1, (h + 40) as f32)] {
                                assert!(lines[line][pt].distance(xf.to_screen(-25.0 + f * w as f32, y)) < 0.01);
                            }
                        }
                        let resized = ViewXform { rect: ScreenRect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 280.0)), ..xf };
                        assert_ne!(screen_guides(g, b, &resized), lines);
                    }
                }
            }
        }
        let golden = screen_guides(CompositionGuides { kind: GuideKind::GoldenRatio, ..Default::default() }, b, &xf());
        assert!(golden[0][0].distance(xf().to_screen(-25.0 + 0.381_966 * w as f32, 40.0)) < 0.01);
    }
    assert!(screen_guides(Default::default(), Rect::EMPTY, &xf()).is_empty());
    assert!(screen_guides(Default::default(), Rect::new(0, 0, 10, 10), &ViewXform { zoom: f32::INFINITY, ..xf() }).is_empty());
}

/// Distinguish mirrored winding from quarter turns and verify that menu operations preserve the remembered pattern orientation.
#[test]
fn composition_all_eight_spiral_orientations_and_menu_operations() {
    let (mut a, ctx) = app();
    menu(&mut a, &ctx, "goldenSpiral");
    let mut shapes = Vec::new();
    for orientation in 0..8 {
        let g = CompositionGuides { enabled: true, kind: GuideKind::GoldenSpiral, orientation };
        let lines = screen_guides(g, Rect::new(0, 0, 300, 200), &xf());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].len() > 100 && lines[0].len() < 200);
        assert!(shapes.iter().all(|s| *s != lines));
        shapes.push(lines);
        a.ui.view.composition_guides = g;
        for action in ["rotate", "flipHorizontal", "flipVertical"] {
            a.ui.view.composition_guides = g;
            assert_eq!(menu(&mut a, &ctx, action)["ok"], true);
            let o = a.ui.view.composition_guides.orientation;
            for p in [[0.1, 0.3], [0.7, 0.9], [0.5, 0.5]] {
                let [x, y] = orient(p, orientation);
                let expected = match action {
                    "rotate" => [1.0 - y, x],
                    "flipHorizontal" => [1.0 - x, y],
                    _ => [x, 1.0 - y],
                };
                let got = orient(p, o);
                assert!(got.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6), "{orientation} {action}");
            }
        }
    }
}

/// Visible artboards each receive their own shapes; hidden boards and pasteboard do not, across every supported theme.
#[test]
fn composition_paints_each_visible_artboard_and_never_the_pasteboard() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Artboard, Layer};
    use photocraft_geom::Size;
    let (mut a, ctx) = app();
    a.ui.tool = Tool::Brush;
    a.ui.view.composition_guides.enabled = true;
    let mut doc = Document::new("Boards", Size::new(800, 600), ColorMode::Rgb, SampleType::U8);
    for (r, visible) in [(Rect::new(0, 0, 300, 200), true), (Rect::new(400, 100, 600, 400), true), (Rect::new(650, 0, 800, 200), false)] {
        let mut layer = Layer::group("Board", Vec::new());
        if let LayerContent::Group(group) = &mut layer.content {
            group.artboard = Some(Artboard::new(r));
        }
        layer.visible = visible;
        doc.layers.push(layer);
    }
    let bounds: Vec<_> = doc.artboards().iter().take(2).map(|b| b.2.rect).collect();
    for theme in crate::theme::ThemeKind::ALL {
        PhotocraftApp::setup_context(&ctx, theme);
        for tool in [Tool::Brush, Tool::Move, Tool::Crop, Tool::PerspectiveCrop] {
            a.ui.tool = tool;
            let output = shape_frame(&ctx, Default::default(), |ui| {
                paint(&a, ui.painter(), &xf(), Some(&doc));
            });
            let lines: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Path(p) => Some(p),
                    _ => None,
                })
                .collect();
            if matches!(tool, Tool::Crop | Tool::PerspectiveCrop) {
                assert!(lines.is_empty());
            } else {
                assert_eq!(lines.len(), 16);
                let expected: Vec<_> =
                    bounds.iter().flat_map(|&b| screen_guides(a.ui.view.composition_guides, b, &xf())).flat_map(|l| [l.clone(), l]).collect();
                assert_eq!(lines.iter().map(|p| p.points.clone()).collect::<Vec<_>>(), expected);
            }
        }
    }
}

/// O and Shift+O continue to change Crop settings without changing the independent canvas-guide preference.
#[test]
fn composition_crop_shortcuts_keep_their_own_pattern_and_orientation() {
    use egui::{Key, Modifiers};
    let (mut a, _) = app();
    a.run("file.new", json!({"width":200,"height":120})).unwrap();
    a.ui.tool = Tool::Crop;
    crate::crop_ui::ensure_frame(&mut a);
    a.ui.view.composition_guides = CompositionGuides { enabled: true, kind: GuideKind::GoldenRatio, orientation: 7 };
    let composition = a.ui.view.composition_guides;
    let press = |app: &mut PhotocraftApp, modifiers| {
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.events.push(egui::Event::Key { key: Key::O, physical_key: None, pressed: true, repeat: false, modifiers }));
        assert!(crate::crop_overlay::keys(app, &ctx));
    };
    press(&mut a, Modifiers::NONE);
    assert_eq!(a.ui.tool_options.crop_overlay, CropOverlay::Grid);
    a.ui.tool_options.crop_overlay = CropOverlay::GoldenSpiral;
    press(&mut a, Modifiers::SHIFT);
    assert_eq!(a.ui.tool_options.crop_overlay_orientation, 1);
    assert_eq!(a.ui.view.composition_guides, composition);
    for show in crate::crop_overlay::OverlayShow::ALL {
        a.ui.tool_options.crop_overlay_show = show;
        assert!(!composition.visible(&a));
    }
}

/// A finite f64 such as -1e100 overflows f32; reject it before a valid guide patch can reveal Extras or change the tool.
#[test]
fn composition_mixed_brush_failure_applies_nothing() {
    let (mut a, ctx) = app();
    a.ui.view.extras = false;
    let guides = a.ui.view.composition_guides;
    let tool = a.ui.tool;
    let brush = a.session.tools.brush.clone();
    let response = call(&mut a, &ctx, "ui.set", json!({"compositionGuides":{"enabled":true},"brushSize":-1e100,"tool":"pencil"}));
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(a.ui.view.composition_guides, guides, "rejected brush must not apply guides");
    assert!(!a.ui.view.extras);
    assert_eq!(a.ui.tool, tool);
    assert_eq!(a.session.tools.brush, brush);
}

/// Both command transports treat null as omitted parameters while rejecting nonobjects and unknown fields without mutation.
#[test]
fn composition_commands_accept_null_as_no_params_and_reject_nonobjects() {
    let (mut a, ctx) = app();
    for method in ["engine.execute", "ui.menu.invoke"] {
        for action in ["thirds", "goldenRatio", "goldenSpiral", "rotate", "flipHorizontal", "flipVertical", "show"] {
            let key = if method == "engine.execute" { "command" } else { "id" };
            let p = json!({key:format!("{PREFIX}{action}"),"params":null});
            let response = call(&mut a, &ctx, method, p);
            assert_eq!(response["ok"], true, "{method} {action}: {response}");
        }
        let guides = a.ui.view.composition_guides;
        for p in [json!(true), json!(1), json!([]), json!("bad")] {
            let key = if method == "engine.execute" { "command" } else { "id" };
            let response = call(&mut a, &ctx, method, json!({key:format!("{PREFIX}thirds"),"params":p}));
            assert_eq!(response["ok"], false, "{response}");
            assert_eq!(a.ui.view.composition_guides, guides);
        }
    }
}

/// An appearance-only merge must keep hidden Extras hidden even when composition Show is already enabled.
#[test]
fn composition_empty_and_orientation_patches_preserve_hidden_extras() {
    let (mut a, ctx) = app();
    a.ui.view.composition_guides.enabled = true;
    a.ui.view.extras = false;
    for p in [json!({}), json!({"orientation":3})] {
        assert_eq!(call(&mut a, &ctx, "ui.set", json!({"compositionGuides":p}))["ok"], true);
        assert!(!a.ui.view.extras);
    }
    assert_eq!(call(&mut a, &ctx, "ui.set", json!({"compositionGuides":{"enabled":true}}))["ok"], true);
    assert!(a.ui.view.extras, "explicitly showing restores Extras");
}

/// Rotate and both flips must not reveal unrelated Extras; choosing a pattern or turning Show on must still reveal them.
#[test]
fn composition_menu_orientation_preserves_hidden_extras_and_show_restores() {
    let (mut a, ctx) = app();
    for enabled in [false, true] {
        a.ui.view.composition_guides = CompositionGuides { enabled, kind: GuideKind::GoldenSpiral, orientation: 0 };
        a.ui.view.extras = false;
        for (action, orientation) in [("rotate", 1), ("flipHorizontal", 5), ("flipVertical", 3)] {
            assert_eq!(menu(&mut a, &ctx, action)["ok"], true);
            assert_eq!(a.ui.view.composition_guides, CompositionGuides { enabled, kind: GuideKind::GoldenSpiral, orientation });
            assert!(!a.ui.view.extras, "{action} must not reveal unrelated Extras");
        }
    }
    for action in ["thirds", "goldenRatio", "goldenSpiral"] {
        a.ui.view.extras = false;
        assert_eq!(menu(&mut a, &ctx, action)["ok"], true);
        assert!(a.ui.view.extras && a.ui.view.composition_guides.enabled, "choosing {action} shows guides");
    }
    a.ui.view.extras = false;
    assert_eq!(menu(&mut a, &ctx, "show")["ok"], true);
    assert!(!a.ui.view.extras && !a.ui.view.composition_guides.enabled, "hiding preserves hidden Extras");
    assert_eq!(menu(&mut a, &ctx, "show")["ok"], true);
    assert!(a.ui.view.extras && a.ui.view.composition_guides.enabled, "showing restores Extras");
}

/// Strengthen representable-range validation without changing the existing clamping of ordinary finite brush sizes.
#[test]
fn composition_mixed_brush_updates_preserve_accepted_representable_inputs() {
    let (mut a, ctx) = app();
    for size in [-1.0, 0.0, 0.5, 42.5] {
        let reply = call(&mut a, &ctx, "ui.set", json!({"compositionGuides":{"enabled":true},"brushSize":size}));
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(a.session.tools.brush.size, size as f32);
        assert!(a.ui.view.composition_guides.enabled);
    }
}

/// A held Move drag changes only the displayed artboard: guides follow that preview and never leak onto the old pasteboard.
#[test]
fn composition_artboard_guides_follow_live_move_without_committing_history() {
    use crate::canvas::{ToolEvent, tool_event};
    let (mut a, ctx) = app();
    a.run("file.new", json!({"width":300,"height":200,"background":"transparent"})).unwrap();
    a.run("layer.new.artboard", json!({"rect":[0,0,120,80],"name":"Moving board","background":"white"})).unwrap();
    a.sync_views();
    a.ui.tool = Tool::Move;
    a.ui.tool_options.move_auto_select = false;
    a.ui.extras.snap = false;
    a.ui.view.show.smart_guides = false;
    a.ui.view.composition_guides.enabled = true;
    let original = a.session.active().unwrap().doc.clone();
    let before = original.artboards()[0].2.rect;
    let revision = a.session.active().unwrap().revision;
    let steps = a.session.active().unwrap().history.past_len();
    let paint_paths = |a: &mut PhotocraftApp| {
        shape_frame(&ctx, Default::default(), |ui| paint_view(a, ui.painter(), &xf(), 0))
            .shapes
            .into_iter()
            .filter_map(|s| match s.shape {
                egui::Shape::Path(p) => Some(p.points),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let expected = |bounds| screen_guides(a.ui.view.composition_guides, bounds, &xf()).into_iter().flat_map(|l| [l.clone(), l]).collect::<Vec<_>>();
    let stationary = expected(before);
    assert_eq!(paint_paths(&mut a), stationary, "ordinary artboards retain their committed bounds");
    tool_event(&mut a, ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut a, ToolEvent::Move { x: 70.0, y: 60.0, pressure: 1.0 }, egui::Modifiers::NONE);
    let shown = crate::canvas::display_doc(&mut a, 0).0;
    let moved = shown.artboards()[0].2.rect;
    assert_eq!(moved, before.translate(30, 20), "fixture must actually move the artboard preview");
    let live = screen_guides(a.ui.view.composition_guides, moved, &xf()).into_iter().flat_map(|l| [l.clone(), l]).collect::<Vec<_>>();
    assert_ne!(live, stationary);
    assert_eq!(paint_paths(&mut a), live, "guides follow the displayed board before release");
    assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &original));
    assert_eq!(a.session.active().unwrap().revision, revision);
    assert_eq!(a.session.active().unwrap().history.past_len(), steps);
    a.ui.view.extras = false;
    assert!(paint_paths(&mut a).is_empty(), "Extras still suppresses a live preview's guides");
}

/// Configuring guides without a document and a stale canvas index must remain harmless, even with Show enabled.
#[test]
fn composition_preview_paint_without_document_or_with_stale_index_draws_nothing() {
    let (mut a, ctx) = app();
    a.ui.view.composition_guides.enabled = true;
    for has_document in [false, true] {
        if has_document {
            a.run("file.new", json!({"width":8,"height":8})).unwrap();
        }
        let idx = if has_document { usize::MAX } else { 0 };
        let output = shape_frame(&ctx, Default::default(), |ui| paint_view(&mut a, ui.painter(), &xf(), idx));
        assert!(output.shapes.iter().all(|s| !matches!(s.shape, egui::Shape::Path(_))));
    }
}

/// Crop owns both untouched and changed frames: its current Always default must not bring back full-canvas aids before the first change.
#[test]
fn composition_crop_ownership_preserves_untouched_frame_and_always_default() {
    use crate::crop_overlay::{OverlayShow, current_guides};
    let (mut a, ctx) = app();
    a.run("file.new", json!({"width":200,"height":120})).unwrap();
    a.sync_views();
    a.ui.tool = Tool::Crop;
    crate::crop_ui::ensure_frame(&mut a);
    assert_eq!(a.ui.tool_options.crop_overlay_show, OverlayShow::Always);
    assert!(a.crop.default_frame, "fixture is the untouched default Crop frame");
    let g = CompositionGuides { enabled: true, kind: GuideKind::GoldenSpiral, orientation: 5 };
    a.ui.view.composition_guides = g;
    let original = a.session.active().unwrap().doc.clone();
    let steps = a.session.active().unwrap().history.past_len();
    let paths = |a: &mut PhotocraftApp| {
        shape_frame(&ctx, Default::default(), |ui| paint_view(a, ui.painter(), &xf(), 0))
            .shapes
            .into_iter()
            .filter(|s| matches!(s.shape, egui::Shape::Path(_)))
            .count()
    };
    assert!(current_guides(&a, 200.0, 120.0).is_empty(), "upstream leaves the untouched frame free of Crop guides");
    assert_eq!(paths(&mut a), 0, "composition guides must not fill in for an untouched Crop frame");
    a.ui.crop_rect = Some([10.0, 10.0, 170.0, 90.0]);
    a.crop.default_frame = false;
    for (show, crop_visible) in [(OverlayShow::Always, true), (OverlayShow::Never, false), (OverlayShow::Auto, false)] {
        a.ui.tool_options.crop_overlay_show = show;
        assert_eq!(!current_guides(&a, 160.0, 80.0).is_empty(), crop_visible);
        assert_eq!(paths(&mut a), 0, "Crop keeps exclusive ownership even when its own guide is hidden");
        assert_eq!(a.ui.view.composition_guides, g);
        assert_eq!(a.ui.tool_options.crop_overlay, CropOverlay::Thirds);
    }
    a.ui.tool = Tool::Brush;
    assert!(current_guides(&a, 200.0, 120.0).is_empty());
    assert_eq!(paths(&mut a), 0, "a frame not yet reconciled on leaving Crop still owns Enter/Escape");
    crate::crop_ui::dismiss(&mut a);
    assert_eq!(paths(&mut a), 2, "clearing Crop ownership restores the two strokes of the remembered spiral");
    assert_eq!(a.ui.view.composition_guides, g);
    assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &original));
    assert_eq!(a.session.active().unwrap().history.past_len(), steps);
}

/// Count actual production guide paths, including both outline strokes, without drawing unrelated tool overlays.
fn composition_path_count(app: &mut PhotocraftApp, ctx: &egui::Context) -> usize {
    let idx = app.session.active_index().unwrap();
    shape_frame(ctx, Default::default(), |ui| paint_view(app, ui.painter(), &xf(), idx))
        .shapes
        .into_iter()
        .filter(|s| matches!(s.shape, egui::Shape::Path(_)))
        .count()
}

/// Make a real default or pointer-drawn Crop frame, with predictable document coordinates and no snapping.
fn composition_crop_fixture(drawn: bool) -> (PhotocraftApp, egui::Context) {
    use crate::canvas::ToolEvent;
    let (mut a, ctx) = app();
    a.run("file.new", json!({"width":200,"height":100,"background":"white"})).unwrap();
    a.sync_views();
    a.ui.tool = Tool::Crop;
    a.ui.tool_options.crop_shield.classic_mode = true;
    a.ui.tool_options.crop_ratio.clear();
    a.ui.extras.snap = false;
    a.ui.view.show.smart_guides = false;
    a.ui.view.composition_guides = CompositionGuides { enabled: true, ..Default::default() };
    if drawn {
        for event in
            [ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, ToolEvent::Move { x: 150.0, y: 80.0, pressure: 1.0 }, ToolEvent::Up { x: 150.0, y: 80.0 }]
        {
            assert!(crate::crop_ui::pointer(&mut a, event, egui::Modifiers::NONE));
        }
        assert_eq!(a.ui.crop_rect, Some([10.0, 10.0, 150.0, 80.0]));
        assert!(!a.crop.default_frame);
    } else {
        crate::crop_ui::ensure_frame(&mut a);
        assert!(a.crop.default_frame);
    }
    (a, ctx)
}

/// Pen erasure keeps a drawn frame and its real Enter/Escape handling; guide suspension must follow that ownership.
#[test]
fn composition_retained_crop_frame_owns_eraser_until_commit_or_cancel() {
    for key in [egui::Key::Enter, egui::Key::Escape] {
        let (mut a, ctx) = composition_crop_fixture(true);
        let original = a.session.active().unwrap().doc.clone();
        let steps = a.session.active().unwrap().history.past_len();
        let revision = a.session.active().unwrap().revision;
        let guides = a.ui.view.composition_guides;
        let pattern = (a.ui.tool_options.crop_overlay, a.ui.tool_options.crop_overlay_orientation);
        a.stylus.feed.set(Some(crate::stylus::PenSample { eraser: true, ..Default::default() }));
        assert!(crate::stylus::Stylus::sync_eraser_tool(&mut a));
        assert_eq!(a.stylus.tool_before_eraser, Some(Tool::Crop));
        assert_eq!(a.ui.tool, Tool::Eraser);
        crate::crop_ui::ensure_frame(&mut a);
        assert_eq!(a.ui.crop_rect, Some([10.0, 10.0, 150.0, 80.0]), "the fixture must retain Crop ownership");
        for show in crate::crop_overlay::OverlayShow::ALL {
            a.ui.tool_options.crop_overlay_show = show;
            assert_eq!(composition_path_count(&mut a, &ctx), 0, "{key:?}/{show:?}: a retained Crop frame owns the canvas");
        }
        // Late drawing must also reject a snapshot captured before ownership changed this frame.
        let direct = shape_frame(&ctx, Default::default(), |ui| paint(&a, ui.painter(), &xf(), Some(&original)));
        assert!(direct.shapes.iter().all(|s| !matches!(s.shape, egui::Shape::Path(_))));
        assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &original));
        assert_eq!((a.session.active().unwrap().history.past_len(), a.session.active().unwrap().revision), (steps, revision));
        let keys = egui::Context::default();
        keys.input_mut(|i| i.events.push(egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }));
        crate::shortcuts::handle(&mut a, &keys);
        assert!(a.ui.crop_rect.is_none(), "{key:?} must reach the retained frame through the production shortcut handler");
        if key == egui::Key::Enter {
            assert_eq!(a.session.active().unwrap().doc.size, photocraft_doc::Size::new(140, 70));
            assert_eq!(a.session.active().unwrap().history.past_len(), steps + 1, "only the intentional Crop adds history");
        } else {
            assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &original));
            assert_eq!((a.session.active().unwrap().history.past_len(), a.session.active().unwrap().revision), (steps, revision));
        }
        assert_eq!(a.ui.tool, Tool::Eraser);
        assert_eq!(composition_path_count(&mut a, &ctx), 8, "a cleared frame restores Thirds even while the pen still remembers Crop");
        assert_eq!(a.ui.view.composition_guides, guides);
        assert_eq!((a.ui.tool_options.crop_overlay, a.ui.tool_options.crop_overlay_orientation), pattern);
        a.ui.view.extras = false;
        assert_eq!(composition_path_count(&mut a, &ctx), 0, "ownership release must still respect Extras");
        a.ui.view.extras = true;
        a.ui.view.composition_guides.enabled = false;
        assert_eq!(composition_path_count(&mut a, &ctx), 0, "ownership release must not enable guides");
    }
}

/// A held Crop gesture retains its frame across a tool switch; cancelling it restores guides without changing the document.
#[test]
fn composition_retained_crop_drag_suspends_until_cancelled() {
    use crate::canvas::ToolEvent;
    let (mut a, ctx) = composition_crop_fixture(true);
    let original = a.session.active().unwrap().doc.clone();
    let steps = a.session.active().unwrap().history.past_len();
    assert!(crate::crop_ui::pointer(&mut a, ToolEvent::Down { x: 60.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE));
    assert!(a.crop.drag.is_some());
    crate::tool_memory::select(&mut a, Tool::Move);
    crate::crop_ui::ensure_frame(&mut a);
    assert!(a.ui.crop_rect.is_some());
    assert_eq!(composition_path_count(&mut a, &ctx), 0);
    crate::crop_ui::dismiss(&mut a);
    assert!(a.crop.drag.is_none());
    assert_eq!(composition_path_count(&mut a, &ctx), 8);
    assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &original));
    assert_eq!(a.session.active().unwrap().history.past_len(), steps);
}

/// The pen's remembered tool alone never hides guides: an untouched frame is dropped, and another document cancels a drawn frame.
#[test]
fn composition_dropped_crop_frame_restores_guides_after_tool_or_document_change() {
    for drawn in [false, true] {
        let (mut a, ctx) = composition_crop_fixture(drawn);
        let guides = a.ui.view.composition_guides;
        let original = a.session.active().unwrap().doc.clone();
        let steps = a.session.active().unwrap().history.past_len();
        a.stylus.feed.set(Some(crate::stylus::PenSample { eraser: true, ..Default::default() }));
        assert!(crate::stylus::Stylus::sync_eraser_tool(&mut a));
        assert_eq!(a.ui.tool, Tool::Eraser);
        if drawn {
            a.run("file.new", json!({"width":80,"height":120})).unwrap();
            a.sync_views();
        } else {
            crate::crop_ui::ensure_frame(&mut a);
        }
        assert!(a.ui.crop_rect.is_none());
        assert_eq!(a.stylus.tool_before_eraser, Some(Tool::Crop));
        assert_eq!(composition_path_count(&mut a, &ctx), 8, "drawn={drawn}: clearing ownership must restore guides");
        assert_eq!(a.ui.view.composition_guides, guides);
        assert!(std::sync::Arc::ptr_eq(&a.session.documents()[0].doc, &original));
        assert_eq!(a.session.documents()[0].history.past_len(), steps);
    }
}

/// The same production frame that drops untouched Classic Crop ownership shows guides, while a drawn frame still suspends them.
#[test]
fn composition_pen_flip_rechecks_crop_ownership_in_the_painted_frame() {
    for drawn in [false, true] {
        let (mut a, ctx) = composition_crop_fixture(drawn);
        ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::MIN);
        a.ui.extras.rulers = false;
        a.ui.extras.grid = false;
        a.ui.view.show.canvas_guides = false;
        a.session.prefs.edit(|p| {
            p.general.animated_zoom = false;
            p.tools.overscroll = true;
        });
        let camera = ViewXform { center: [100.0, 50.0], ..xf() };
        let view = crate::state::View { center: camera.center, fit_pending: false, doc_size: [200, 100], ..Default::default() };
        let original = a.session.active().unwrap().doc.clone();
        let steps = a.session.active().unwrap().history.past_len();
        let expected = screen_guides(a.ui.view.composition_guides, original.bounds(), &camera);
        assert_eq!(a.ui.tool, Tool::Crop);
        assert!(a.ui.crop_rect.is_some());
        a.stylus.feed.set(Some(crate::stylus::PenSample { eraser: true, ..Default::default() }));
        let out = shape_frame(
            &ctx,
            egui::RawInput {
                screen_rect: Some(ScreenRect::from_min_size(Pos2::ZERO, vec2(1024.0, 768.0))),
                max_texture_side: Some(2048),
                ..Default::default()
            },
            |ui| {
                let _ = crate::canvas::canvas_view(&mut a, ui, 0, camera.rect, view.clone(), true);
            },
        );
        assert_eq!(a.ui.tool, Tool::Eraser, "the canvas must process the pen flip during this frame");
        assert_eq!(a.ui.crop_rect.is_some(), drawn, "only the drawn crop keeps frame ownership");
        let guides = out.shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Path(p) if expected.contains(&p.points))).count();
        assert_eq!(guides, if drawn { 0 } else { 8 }, "the overlay must use this frame's Crop ownership");
        assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &original));
        assert_eq!(a.session.active().unwrap().history.past_len(), steps);
    }
}

/// A dormant Perspective Crop quad after an explicit tool switch neither draws nor owns keys; it must not hide guides indefinitely.
#[test]
fn composition_dormant_perspective_quad_does_not_claim_another_tools_guides() {
    use crate::canvas::ToolEvent;
    let (mut a, ctx) = app();
    a.run("file.new", json!({"width":200,"height":100,"background":"white"})).unwrap();
    a.sync_views();
    a.ui.tool = Tool::PerspectiveCrop;
    for event in
        [ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, ToolEvent::Move { x: 150.0, y: 80.0, pressure: 1.0 }, ToolEvent::Up { x: 150.0, y: 80.0 }]
    {
        assert!(crate::perspective_crop_ui::pointer(&mut a, event, egui::Modifiers::NONE));
    }
    let quad = a.ui.perspective_crop;
    assert!(quad.is_some());
    crate::tool_memory::select(&mut a, Tool::Move);
    assert_eq!(a.ui.perspective_crop, quad);
    assert!(crate::perspective_crop_ui::pending(&a));
    let original = a.session.active().unwrap().doc.clone();
    let steps = a.session.active().unwrap().history.past_len();
    ctx.input_mut(|i| {
        i.events.push(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE })
    });
    crate::shortcuts::handle(&mut a, &ctx);
    a.ui.view.composition_guides.enabled = true;
    assert_eq!(composition_path_count(&mut a, &ctx), 8);
    assert_eq!(a.ui.perspective_crop, quad);
    assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &original));
    assert_eq!(a.session.active().unwrap().history.past_len(), steps);
}

/// The full production canvas must keep all guide strokes above an opaque transform preview, and retain them in secondary views.
#[test]
fn composition_canvas_orders_guides_above_opaque_transform_and_keeps_secondary_views() {
    for primary in [true, false] {
        let (mut a, ctx) = app();
        a.run("file.new", json!({"width":64,"height":48,"background":"transparent"})).unwrap();
        a.run("layer.new.layer", json!({"name":"Opaque preview"})).unwrap();
        a.run("edit.fill", json!({"color":"#ffffff"})).unwrap();
        a.sync_views();
        a.ui.tool = Tool::Move;
        a.ui.extras.rulers = false;
        a.ui.extras.grid = false;
        a.ui.view.show.canvas_guides = false;
        a.ui.view.show.layer_edges = false;
        a.ui.view.composition_guides.enabled = true;
        a.session.prefs.edit(|p| {
            p.general.animated_zoom = false;
            p.tools.overscroll = true;
        });
        let original = a.session.active().unwrap().doc.clone();
        let steps = a.session.active().unwrap().history.past_len();
        let camera = ViewXform { center: [32.0, 24.0], ..xf() };
        let view = crate::state::View { center: camera.center, fit_pending: false, doc_size: [64, 48], ..Default::default() };
        let expected = screen_guides(a.ui.view.composition_guides, original.bounds(), &camera);
        let out = shape_frame(
            &ctx,
            egui::RawInput {
                screen_rect: Some(ScreenRect::from_min_size(Pos2::ZERO, vec2(1024.0, 768.0))),
                max_texture_side: Some(2048),
                ..Default::default()
            },
            |ui| {
                crate::transform_tool::begin(&mut a, ui.ctx()).unwrap();
                assert_eq!(a.transform_preview.as_ref().unwrap().opacity, 1.0, "counterexample needs opaque moving content");
                let _ = crate::canvas::canvas_view(&mut a, ui, 0, camera.rect, view.clone(), primary);
            },
        );
        let guides: Vec<_> = out
            .shapes
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match &s.shape {
                egui::Shape::Path(p) if expected.contains(&p.points) => Some(i),
                _ => None,
            })
            .collect();
        assert_eq!(guides.len(), 8, "both outline strokes of all four thirds must survive, primary={primary}");
        if primary {
            let content = out
                .shapes
                .iter()
                .enumerate()
                .filter_map(|(i, s)| match &s.shape {
                    egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() && m.vertices.len() > 4 => Some(i),
                    _ => None,
                })
                .max()
                .expect("full canvas must actually draw the transform's subdivided textured mesh");
            assert!(guides.iter().all(|i| *i > content), "composition guides must paint above opaque content previews");
        }
        assert!(a.ui.transform.is_some(), "drawing guides must not commit or cancel the transform");
        assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &original));
        assert_eq!(a.session.active().unwrap().history.past_len(), steps);
    }
}

/// Held Move and release frames must use the same artboard snapshot for pixels and guides, then catch up together next frame.
#[test]
fn composition_canvas_artboard_input_keeps_guides_on_painted_snapshot() {
    let (mut a, ctx) = app();
    a.run("file.new", json!({"width":300,"height":200,"background":"transparent"})).unwrap();
    a.run("layer.new.artboard", json!({"rect":[0,0,120,80],"name":"Moving board","background":"white"})).unwrap();
    a.sync_views();
    a.ui.tool = Tool::Move;
    a.ui.tool_options.move_auto_select = false;
    a.ui.tool_options.move_show_transform = false;
    a.ui.extras.snap = false;
    a.ui.extras.rulers = false;
    a.ui.extras.grid = false;
    a.ui.view.show.smart_guides = false;
    a.ui.view.show.canvas_guides = false;
    a.ui.view.show.layer_edges = false;
    a.ui.view.composition_guides.enabled = true;
    a.session.prefs.edit(|p| {
        p.general.animated_zoom = false;
        p.tools.overscroll = true;
    });
    let camera = ViewXform { center: [150.0, 100.0], ..xf() };
    let view = crate::state::View { center: camera.center, fit_pending: false, doc_size: [300, 200], ..Default::default() };
    let original = a.session.active().unwrap().doc.clone();
    let steps = a.session.active().unwrap().history.past_len();
    let at = |x, y| camera.to_screen(x, y);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let frames = [
        vec![],
        vec![egui::Event::PointerMoved(at(40.0, 40.0)), button(at(40.0, 40.0), true)],
        vec![egui::Event::PointerMoved(at(70.0, 60.0))],
        vec![egui::Event::PointerMoved(at(100.0, 80.0))],
        vec![egui::Event::PointerMoved(at(130.0, 100.0)), button(at(130.0, 100.0), false)],
        vec![],
    ];
    let mut advanced_while_held = 0;
    for (frame, events) in frames.into_iter().enumerate() {
        let shown_before = crate::canvas::display_doc(&mut a, 0).0;
        let before = shown_before.artboards()[0].2.rect;
        let expected = screen_guides(a.ui.view.composition_guides, before, &camera);
        let out = shape_frame(
            &ctx,
            egui::RawInput {
                screen_rect: Some(ScreenRect::from_min_size(Pos2::ZERO, vec2(1024.0, 768.0))),
                max_texture_side: Some(2048),
                time: Some(frame as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                let _ = crate::canvas::canvas_view(&mut a, ui, 0, camera.rect, view.clone(), true);
            },
        );
        let guide_count = out.shapes.iter().filter(|s| matches!(&s.shape,egui::Shape::Path(p) if expected.contains(&p.points))).count();
        assert_eq!(guide_count, 8, "frame {frame}: guides must match the artboard image submitted before this frame's input");
        let after = crate::canvas::display_doc(&mut a, 0).0.artboards()[0].2.rect;
        if frame == 2 || frame == 3 {
            assert_ne!(after, before, "fixture must actually advance a held artboard Move on frame {frame}");
            assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &original));
            assert_eq!(a.session.active().unwrap().history.past_len(), steps);
            advanced_while_held += 1;
        }
        if frame == 4 {
            assert!(a.drag.is_none(), "release must actually commit the Move");
            assert_eq!(a.session.active().unwrap().history.past_len(), steps + 1, "only the intended Move may enter history");
        }
        if frame == 5 {
            assert_eq!(after, before, "idle image and guides must catch up together");
        }
    }
    assert_eq!(advanced_while_held, 2);
}

/// Even a rotation below 0.05° tilts the CPU texture edges. Guide endpoints must divide those actual edges.
#[test]
fn composition_cpu_image_and_guides_share_a_small_nonzero_rotation() {
    let (mut a, ctx) = app();
    a.run("file.new", json!({"width":256,"height":128,"background":"white"})).unwrap();
    a.sync_views();
    a.ui.tool = Tool::Move;
    a.ui.extras.rulers = false;
    a.ui.extras.grid = false;
    a.ui.view.show.canvas_guides = false;
    a.ui.view.composition_guides.enabled = true;
    a.session.prefs.edit(|p| p.general.animated_zoom = false);
    let view = crate::state::View { zoom: 2.0, center: [128.0, 64.0], rotation: 0.04, fit_pending: false, doc_size: [256, 128], ..Default::default() };
    a.ui.views[0] = view.clone();
    assert!(a.gpu.is_none());
    let texture = crate::canvas::ensure_texture(&mut a, &ctx, 0, None).unwrap().0;
    let rect = ScreenRect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0));
    let out = shape_frame(&ctx, egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ui| {
        let _ = crate::canvas::canvas_view(&mut a, ui, 0, rect, view.clone(), true);
    });
    let image = out
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh),
            _ => None,
        })
        .expect("the CPU image must be drawn");
    let at_uv = |u, v| image.vertices.iter().find(|vertex| vertex.uv == pos2(u, v)).unwrap().pos;
    let (tl, tr, bl, br) = (at_uv(0.0, 0.0), at_uv(1.0, 0.0), at_uv(0.0, 1.0), at_uv(1.0, 1.0));
    assert!(tr.y - tl.y > 0.25, "the CPU image's top edge must follow the small rotation");
    let between = |a: Pos2, b: Pos2, fraction| a + (b - a) * fraction;
    for f in [1.0 / 3.0, 2.0 / 3.0] {
        let segments = [[between(tl, tr, f), between(bl, br, f)], [between(tl, bl, f), between(tr, br, f)]];
        for [start, end] in segments {
            let strokes = out.shapes.iter().filter(
                |s| matches!(&s.shape, egui::Shape::Path(p) if matches!(p.points.as_slice(), [a, b] if a.distance(start) < 0.01 && b.distance(end) < 0.01)),
            );
            assert_eq!(strokes.count(), 2, "both guide strokes must divide the rotated texture edges");
        }
    }
}

/// The guide strokes divide the submitted image during zoom interpolation and during the frame that pans a live tool's camera.
#[test]
fn composition_live_tools_share_the_image_camera_during_zoom_and_pan() {
    for (tool, pan) in [(Tool::Brush, false), (Tool::MixerBrush, false), (Tool::Brush, true), (Tool::MixerBrush, true)] {
        let (mut a, ctx) = app();
        a.run("file.new", json!({"width":64,"height":48,"background":"white"})).unwrap();
        a.sync_views();
        a.ui.tool = tool;
        a.ui.extras.rulers = false;
        a.ui.extras.grid = false;
        a.ui.view.show.canvas_guides = false;
        a.ui.view.show.layer_edges = false;
        a.ui.view.composition_guides.enabled = true;
        a.session.prefs.edit(|p| {
            p.general.animated_zoom = !pan;
            p.general.zoom_with_scroll_wheel = false;
            p.tools.overscroll = true;
        });
        let center = [32.0, 24.0];
        let view = &mut a.ui.views[0];
        view.fit_pending = false;
        view.zoom = 2.0;
        view.center = center;
        view.doc_size = [64, 48];
        if !pan {
            view.zoom_animating = true;
            view.display_camera = Some((1.0, center));
            view.zoom_transition = Some(crate::state::ZoomTransition { from: (1.0, center), target: (2.0, center), started: 0.0 });
        }
        let view = view.clone();
        let doc = a.session.active().unwrap().doc.clone();
        let steps = a.session.active().unwrap().history.past_len();
        let texture = crate::canvas::ensure_texture(&mut a, &ctx, 0, None).unwrap().0;
        let rect = ScreenRect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0));
        let frame = |a: &mut PhotocraftApp, time, events| {
            shape_frame(&ctx, egui::RawInput { screen_rect: Some(rect), time: Some(time), events, ..Default::default() }, |ui| {
                let _ = crate::canvas::canvas_view(a, ui, 0, rect, view.clone(), true);
            })
        };
        let events = if pan {
            let _ = frame(&mut a, 0.0, vec![egui::Event::PointerMoved(rect.center())]);
            vec![egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(24.0, 0.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            }]
        } else {
            Vec::new()
        };
        let out = frame(&mut a, 0.1, events);
        let (image_order, image) = out
            .shapes
            .iter()
            .enumerate()
            .find_map(|(i, s)| match &s.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some((i, mesh)),
                _ => None,
            })
            .expect("the deferred image must be drawn");
        let mut bounds = ScreenRect::NOTHING;
        for vertex in &image.vertices {
            bounds.extend_with(vertex.pos);
        }
        if pan {
            let center_after = a.ui.views[0].center;
            assert_ne!(center_after, center, "the wheel input must pan the camera");
            let wanted_x = rect.center().x + (center[0] - center_after[0]) * view.zoom;
            assert!((bounds.center().x - wanted_x).abs() < 1.0, "{tool:?}: this frame's image must use the new center");
        } else {
            let zoom = a.ui.views[0].display_camera.unwrap().0;
            assert!(zoom > 1.0 && zoom < 2.0);
            assert!((bounds.width() - 64.0 * zoom).abs() < 0.01, "{tool:?}: the image uses the interpolated camera");
        }
        for fraction in [1.0 / 3.0, 2.0 / 3.0] {
            let x = bounds.left() + fraction * bounds.width();
            let y = bounds.top() + fraction * bounds.height();
            for [start, end] in [[pos2(x, bounds.top()), pos2(x, bounds.bottom())], [pos2(bounds.left(), y), pos2(bounds.right(), y)]] {
                let matching = out.shapes.iter().skip(image_order + 1).filter(
                    |s| matches!(&s.shape, egui::Shape::Path(p) if matches!(p.points.as_slice(), [a, b] if a.distance(start) < 0.01 && b.distance(end) < 0.01)),
                );
                assert_eq!(matching.count(), 2, "{tool:?}: each guide must divide the submitted image and draw above it");
            }
        }
        assert!(std::sync::Arc::ptr_eq(&a.session.active().unwrap().doc, &doc));
        assert_eq!(a.session.active().unwrap().history.past_len(), steps);
    }
}
