//! Opt-in audit fixtures use a copied panel layout, never the user's preferences.
use super::tests::{compact_pair, harness, layout, press, release};
use super::*;
use egui::{Event, Modifiers};
use egui_kittest::Harness;
const PROBLEM_LAYOUT: &str = include_str!("../../../tests/fixtures/panel-repair-workspace.json");

fn strip(h: &Harness<'static, PhotocraftApp>, panel: &str) -> Strip {
    strips(h.state(), &h.ctx, layout(h.state())).into_iter().find(|s| s.panels().any(|p| p == panel)).unwrap()
}
fn columns(h: &Harness<'static, PhotocraftApp>) -> Vec<(String, Rect, Side)> {
    h.ctx.data(|d| d.get_temp(key("columns"))).unwrap()
}

fn layers_column(h: &Harness<'static, PhotocraftApp>) -> (String, Rect, Side) {
    let anchor = compat::columns(layout(h.state()))
        .into_iter()
        .find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|id| id == "layers")))
        .unwrap()
        .1
        .groups
        .first()
        .unwrap()
        .panels
        .first()
        .unwrap();
    columns(h).into_iter().find(|(id, _, _)| id == anchor).unwrap()
}

#[test]
fn attached_compact_has_one_input_owner_and_uses_the_fitted_rect() {
    let mut app = compact_pair();
    app.run("window.panel.edge", json!({"panel":"properties","edge":"rightInner"})).unwrap();
    let h = harness(app);
    let s = strip(&h, "properties");
    let c = columns(&h).into_iter().find(|(id, _, _)| id == "properties").unwrap().1;
    assert_eq!(s.rect.x_range(), c.x_range());
    let hits = DockArea::<String>::hit_rects(&h.ctx, area_id());
    assert!(
        !hits
            .iter()
            .any(|(hit, _)| matches!(hit, Hit::Icon(id) | Hit::IconGrip(id) | Hit::Divider(craft_ui::docking::Divider::Column(id)) if id == "properties"))
    );
    for (id, cell) in s.cells() {
        let response = h.ctx.read_response(key("icon").with(id)).unwrap();
        assert_eq!(response.rect, cell.shrink2(vec2(INSET, 1.5)));
    }
}

#[test]
fn attached_resize_reflows_neighbours_during_drag_and_escape_restores_every_rect() {
    for edge in ["left", "rightInner", "rightOuter"] {
        let mut app = compact_pair();
        app.run("window.panel.edge", json!({"panel":"properties","edge":edge})).unwrap();
        let mut h = harness(app);
        let before = crate::dock::snapshot(h.state());
        let initial = columns(&h);
        let s = strip(&h, "properties");
        let from = pos2(if s.left { s.rect.right() - 1.0 } else { s.rect.left() + 1.0 }, s.rect.top() + 100.0);
        let direction = if s.left { 1.0 } else { -1.0 };
        press(&mut h, from, from + vec2(direction * 20.0, 0.0));
        for distance in [30.0, 50.0, 80.0] {
            h.event(Event::PointerMoved(from + vec2(direction * distance, 0.0)));
            h.run_steps(1);
            assert_eq!(crate::dock::snapshot(h.state()), before, "preview cannot persist");
            let drawn = columns(&h);
            let actual = strip(&h, "properties");
            assert!(actual.rect.width() > s.rect.width() + 15.0, "{edge}");
            assert_eq!(actual.rect.x_range(), drawn.iter().find(|(id, _, _)| id == "properties").unwrap().1.x_range());
            for (i, (_, a, _)) in drawn.iter().enumerate() {
                for (_, b, _) in drawn.iter().skip(i + 1) {
                    assert!(a.intersect(*b).width() <= 0.01);
                }
            }
        }
        h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        assert_eq!(columns(&h), initial);
        release(&mut h, from);
        assert_eq!(crate::dock::snapshot(h.state()), before);
    }
}

#[test]
fn mouse_drop_reaches_both_layers_boundaries_from_compact_and_full_sources() {
    for full in [false, true] {
        for after in [false, true] {
            let mut app = compact_pair();
            if full {
                app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
            }
            let mut h = harness(app);
            let source = if full {
                DockArea::<String>::hit_rects(&h.ctx, area_id())
                    .into_iter()
                    .find(|(hit, _)| matches!(hit,Hit::DragBar(id) if id=="properties"))
                    .unwrap()
                    .1
                    .center()
            } else {
                let s = strip(&h, "properties");
                pos2(s.rect.center().x, s.rect.top() + INSET + HEADER + GRIP * 0.5)
            };
            let (_, r, side) = layers_column(&h);
            let left = (side == Side::Start) != crate::direction::is_rtl();
            let x = if left != after { r.left() } else { r.right() };
            let to = pos2(x, r.top() + 150.0);
            press(&mut h, source, to);
            release(&mut h, to);
            let cols = compat::columns(layout(h.state()));
            let source = cols.iter().position(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|id| id == "properties"))).unwrap();
            let layers = cols.iter().position(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|id| id == "layers"))).unwrap();
            assert_eq!(source > layers, after, "full={full} after={after}");
            assert_eq!(cols[source].1.iconic, !full);
        }
    }
}

#[test]
fn header_click_controls_with_pointer_drift_never_detach() {
    let mut h = harness(compact_pair());
    let s = strip(&h, "properties");
    let arrow = pos2(s.rect.right() - 31.0, s.rect.top() + INSET + 10.0);
    press(&mut h, arrow, arrow + vec2(6.0, 0.0));
    assert!(h.ctx.data(|d| d.get_temp::<Drag>(key("drag"))).is_none());
    assert_eq!(h.ctx.read_response(key("expand").with("properties")).unwrap().sense, Sense::click());
    release(&mut h, arrow + vec2(6.0, 0.0));
}

#[test]
fn repeated_icon_click_closes_the_flyout_without_detach_or_mouse_focus_residue() {
    let mut h = harness(compact_pair());
    let s = strip(&h, "properties");
    let cell = s.cells().into_iter().find(|(id, _)| id == "properties").unwrap().1;
    let before = layout(h.state()).clone();
    super::tests::click(&mut h, cell.center());
    assert_eq!(h.state().ui.docking_flyout.as_deref(), Some("properties"));
    super::tests::click(&mut h, cell.center());
    h.event(Event::PointerMoved(pos2(500.0, 500.0)));
    h.run_steps(3);
    assert!(h.state().ui.docking_flyout.is_none());
    assert_eq!(layout(h.state()), &before);
    let rect = cell.shrink2(vec2(INSET, 1.5));
    let accent = crate::theme::Tokens::get(&h.ctx).accent;
    assert!(!h.output().shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Rect(r) if r.rect==rect && r.stroke.color==accent)));
}

#[test]
fn width_authority_transfers_only_when_crossing_float_dock() {
    let mut app = compact_pair();
    app.run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":123})).unwrap();
    app.run("window.panel.edge", json!({"panel":"properties","edge":"rightInner"})).unwrap();
    let column = compat::columns(layout(&app)).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|id| id == "properties"))).unwrap().1;
    assert_eq!(column.iconic_width, Some(123.0));
    app.ui.panel_presentation.widths.insert("properties".into(), 49.6);
    app.run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":142})).unwrap();
    app.run("window.panel.edge", json!({"panel":"properties","edge":"floating"})).unwrap();
    assert_eq!(app.ui.panel_presentation.widths.get("properties"), Some(&142.0));
    let saved = crate::dock::snapshot(&app);
    crate::dock::apply(&mut app, &saved);
    assert_eq!(app.ui.panel_presentation.widths.get("properties"), Some(&142.0));
    app.run("window.panel.group", json!({"panel":"properties","operation":"close-window"})).unwrap();
    app.run("window.panel.activate", json!({"panel":"properties"})).unwrap();
    assert_eq!(app.ui.panel_presentation.widths.get("properties"), Some(&142.0));
}

#[test]
fn joining_an_attached_compact_column_keeps_the_receiving_width() {
    let mut app = compact_pair();
    app.run("window.panel.edge", json!({"panel":"properties","edge":"left"})).unwrap();
    app.run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":133})).unwrap();
    app.run("window.panel.edge", json!({"panel":"navigator","edge":"floating","collapsed":true,"individual":true})).unwrap();
    app.run("window.panel.group", json!({"panel":"navigator","operation":"strip-width","width":72})).unwrap();
    app.run("window.panel.move", json!({"panel":"navigator","anchor":"properties","zone":"bottom"})).unwrap();
    let column = compat::columns(layout(&app)).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|id| id == "navigator"))).unwrap().1;
    assert_eq!(column.iconic_width, Some(133.0));
}

#[test]
fn legacy_default_width_narrows_once_and_personal_widths_survive_restore() {
    let mut app = compact_pair();
    app.run("window.panel.edge", json!({"panel":"properties","edge":"left"})).unwrap();
    app.run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":44})).unwrap();
    let mut saved = crate::dock::snapshot(&app);
    saved["panelPresentation"].as_object_mut().unwrap().remove("metrics_revision");
    saved["panelPresentation"]["widths"]["color"] = json!(49.6);
    crate::dock::apply(&mut app, &saved);
    let column = compat::columns(layout(&app)).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|id| id == "properties"))).unwrap().1;
    assert_eq!(column.iconic_width, None);
    assert_eq!(app.ui.panel_presentation.widths["color"], 49.6);
    app.run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":44})).unwrap();
    let saved = crate::dock::snapshot(&app);
    crate::dock::apply(&mut app, &saved);
    let column = compat::columns(layout(&app)).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|id| id == "properties"))).unwrap().1;
    assert_eq!(column.iconic_width, Some(44.0));
}

#[test]
fn attached_group_redocks_in_one_drag_and_landing_ghost_expires() {
    let mut app = compact_pair();
    app.run("window.panel.edge", json!({"panel":"properties","edge":"left"})).unwrap();
    let mut h = harness(app);
    let s = strip(&h, "properties");
    let from = pos2(s.rect.center().x, s.rect.top() + INSET + HEADER + GRIP * 0.5);
    let r = layers_column(&h).1;
    let to = pos2(r.left(), r.top() + 140.0);
    press(&mut h, from, to);
    assert!(strip(&h, "properties").attached);
    release(&mut h, to);
    assert!(!strip(&h, "properties").left);
    h.run_steps(24);
    assert!(h.ctx.data(|d| d.get_temp::<Flight>(key("flight"))).is_none());
    assert!(h.ctx.data(|d| d.get_temp::<Drag>(key("drag"))).is_none());
}

#[test]
fn lock_and_focus_loss_cancel_resize_without_committing() {
    for lock in [false, true] {
        let mut app = compact_pair();
        app.run("window.panel.edge", json!({"panel":"properties","edge":"left"})).unwrap();
        let mut h = harness(app);
        let initial = layout(h.state()).clone();
        let s = strip(&h, "properties");
        let from = pos2(s.rect.right() - 1.0, s.rect.top() + 100.0);
        press(&mut h, from, from + vec2(60.0, 0.0));
        if lock {
            h.state_mut().session.prefs.edit(|p| p.workspace_locked = true);
        } else {
            h.input_mut().focused = false;
        }
        h.run_steps(2);
        assert!(h.ctx.data(|d| d.get_temp::<Resize>(key("resize"))).is_none());
        release(&mut h, from + vec2(60.0, 0.0));
        assert_eq!(layout(h.state()), &initial);
    }
}

#[test]
fn empty_header_has_no_mouse_outline_but_keeps_keyboard_focus() {
    let mut app = compact_pair();
    app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
    let mut h = harness(app);
    let hit = Hit::DragBar("properties".to_owned());
    let rect = DockArea::<String>::hit_rects(&h.ctx, area_id()).into_iter().find(|(h, _)| h == &hit).unwrap().1;
    let accent = crate::theme::Tokens::get(&h.ctx).accent.gamma_multiply(0.5);
    let has_outline = |h: &Harness<'static, PhotocraftApp>| {
        h.output().shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Rect(r) if r.rect == rect.shrink(1.0) && r.stroke.color==accent))
    };
    h.event(Event::PointerMoved(rect.center()));
    h.run_steps(2);
    assert!(!has_outline(&h));
    h.ctx.memory_mut(|m| m.request_focus(DockArea::<String>::hit_id(area_id(), &hit)));
    h.event(Event::Key { key: egui::Key::F6, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(has_outline(&h));
}

#[test]
fn floating_strip_occludes_the_attached_drop_boundary_behind_it() {
    let mut h = harness(compact_pair());
    let r = layers_column(&h).1;
    h.state_mut().run("window.panel.edge", json!({"panel":"properties","edge":"floating","x":r.left()-10.0,"y":r.top()+100.0})).unwrap();
    h.run_steps(4);
    let s = strip(&h, "properties");
    let point = pos2(r.left(), s.rect.top() + INSET + HEADER + GRIP + 10.0);
    let target = landing(&h.ctx, "navigator", false, point).unwrap();
    assert!(matches!(target.target, Target::At { placement: Placement::Split(_), .. }));
}

#[test]
fn compact_resize_keeps_keyboard_and_accessibility_value_control() {
    let mut app = compact_pair();
    app.run("window.panel.edge", json!({"panel":"properties","edge":"left"})).unwrap();
    let mut h = harness(app);
    let initial = strip(&h, "properties");
    let id = key("resize").with((&initial.first, false));
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(1); // Establish the focused handle's arrow-key navigation filter.
    h.event(Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!((strip(&h, "properties").rect.width() - initial.rect.width() - 8.0).abs() < 0.1);
    assert!(h.ctx.read_response(id).unwrap().has_focus());
    let x = strip(&h, "properties").rect.right() - 0.5;
    let stroke = h
        .output()
        .shapes
        .iter()
        .rev()
        .find_map(|s| match &s.shape {
            egui::Shape::LineSegment { points, stroke } if (points[0].x - x).abs() < 0.01 && points[0].x == points[1].x => Some(*stroke),
            _ => None,
        })
        .unwrap();
    assert_eq!(stroke.color, crate::theme::Tokens::get(&h.ctx).accent, "the final seam cannot paint over keyboard focus");
}

#[test]
fn floating_tools_outline_keeps_one_physical_pixel_at_all_scales() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_pixels_per_point(scale).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ProMedium);
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("window.tools.float", json!({"x":600,"y":100})).unwrap();
            app
        });
        h.run_steps(6);
        let rect = h.ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique("hidden-panels-hover").with("tools"))).unwrap();
        let border = crate::theme::Tokens::get(&h.ctx).card_border;
        let stroke = h
            .output()
            .shapes
            .iter()
            .rev()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.rect == rect && r.stroke.color == border => Some(r.stroke),
                _ => None,
            })
            .unwrap();
        assert!((stroke.width * scale - 1.0).abs() < 0.01, "Tools border scale {scale}");
    }
}

#[test]
fn shared_column_resize_has_no_hover_fill_and_keeps_a_thin_keyboard_indicator() {
    let mut h = harness(compact_pair());
    let (anchor, rect, _) = layers_column(&h);
    let hit = Hit::Divider(craft_ui::docking::Divider::Column(anchor));
    let id = DockArea::<String>::hit_id(area_id(), &hit);
    h.event(Event::PointerMoved(pos2(rect.left(), rect.center().y)));
    h.run_steps(3);
    let accent = crate::theme::Tokens::get(&h.ctx).accent;
    assert!(
        !h.output().shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Rect(r) if r.fill==accent && r.rect.height()>100.0)),
        "hover cannot add a stock divider fill"
    );
    h.event(Event::PointerMoved(pos2(500.0, 500.0)));
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.run_steps(2);
    assert!(h.ctx.read_response(id).unwrap().has_focus());
    let pixel = h.ctx.pixels_per_point().recip();
    let x = (rect.left() / pixel).round() * pixel + pixel * 0.5;
    let stroke = h
        .output()
        .shapes
        .iter()
        .rev()
        .find_map(|s| match &s.shape {
            egui::Shape::LineSegment { points, stroke } if (points[0].x - x).abs() < 0.01 && points[0].x == points[1].x => Some(*stroke),
            _ => None,
        })
        .unwrap();
    assert_eq!(stroke.color, accent);
    assert_eq!(stroke.width, pixel);
}

#[test]
fn ordinary_layer_selection_keeps_properties_closed_but_new_adjustment_and_fill_reveal_it() {
    let mut app = compact_pair();
    app.run("file.new", json!({"width":8,"height":8})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap().0;
    app.run("window.panel.close", json!({"panel":"properties"})).unwrap();
    let before = crate::dock::snapshot(&app);
    for _ in 0..2 {
        app.run("layer.select", json!({"layer":layer})).unwrap();
        assert_eq!(crate::dock::snapshot(&app), before);
        assert!(!layout(&app).contains(&"properties".to_owned()));
    }
    for command in ["layer.newAdjustmentLayer.levels", "layer.newFillLayer.solidColor"] {
        app.run(command, json!({})).unwrap();
        assert!(app.ui.panels.properties);
        assert!(layout(&app).contains(&"properties".to_owned()));
        app.run("window.panel.close", json!({"panel":"properties"})).unwrap();
    }
}

#[test]
fn removing_side_dock_gaps_keeps_floating_stack_row_dividers_draggable() {
    let mut app = compact_pair();
    app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
    let mut h = harness(app);
    let before = layout(h.state()).clone();
    let divider = DockArea::<String>::hit_rects(&h.ctx, area_id())
        .into_iter()
        .find(|(hit, _)| matches!(hit,Hit::Divider(craft_ui::docking::Divider::WindowSplit(id,_)) if id=="properties"))
        .unwrap()
        .1;
    let from = divider.center();
    press(&mut h, from, from + vec2(0.0, 40.0));
    release(&mut h, from + vec2(0.0, 40.0));
    assert_ne!(layout(h.state()), &before);
    assert!(layout(h.state()).floating.iter().any(|w| w.contains("properties") && w.contains("actions")));
}

#[test]
fn audited_problem_layout_fits_narrow_viewports_and_restores_without_overlap() {
    let saved: Value = serde_json::from_str(PROBLEM_LAYOUT).unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::dock::apply(&mut app, &saved["panelLayout"]);
    let mut h = harness(app);
    for width in [1280.0, 760.0, 420.0, 1280.0] {
        h.set_size(vec2(width, 540.0));
        h.run_steps(6);
        for s in strips(h.state(), &h.ctx, layout(h.state())) {
            assert!(h.ctx.content_rect().contains_rect(s.rect), "{} at width {width}: {:?}", s.first, s.rect);
        }
        let fitted = columns(&h);
        for (i, (_, a, _)) in fitted.iter().enumerate() {
            assert!(h.ctx.content_rect().contains_rect(*a));
            for (_, b, _) in fitted.iter().skip(i + 1) {
                assert!(a.intersect(*b).width() <= 0.01);
            }
        }
    }
    let snapshot = crate::dock::snapshot(h.state());
    crate::dock::apply(h.state_mut(), &snapshot);
    h.run_steps(6);
    assert_eq!(crate::dock::snapshot(h.state()), snapshot);
}

#[test]
fn repair_audit_captures() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_REPAIR_CAPTURES").map(std::path::PathBuf::from) else { return };
    let saved: Value = std::env::var_os("PHOTOCRAFT_REPAIR_LAYOUT")
        .map(|fixture| serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap())
        .unwrap_or_else(|| serde_json::from_str(PROBLEM_LAYOUT).unwrap());
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    for scale in [1.0, 1.25, 1.5, 2.0] {
        for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::ProLight] {
            let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).with_max_steps(64).wgpu().build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                crate::dock::apply(&mut app, &saved["panelLayout"]);
                if let Some(workspace) = saved.get("workspace").and_then(Value::as_object) {
                    for (key, value) in workspace {
                        app.run("prefs.set", json!({"path":format!("workspace.{key}"),"value":value})).unwrap();
                    }
                }
                app.run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
                app.run("prefs.set", json!({"path":"interface.language","value":"en"})).unwrap();
                app
            });
            h.run_steps(8);
            assert_eq!(h.state().ui.theme, theme, "capture must use the requested theme");
            let prefix = format!("{theme:?}-{scale}");
            let image = h.render().unwrap();
            let border = crate::theme::Tokens::get(&h.ctx).card_border.to_array();
            for (_, rect, side) in columns(&h) {
                let left = (side == Side::Start) != crate::direction::is_rtl();
                let edge = if left { rect.right() } else { rect.left() };
                let x = (edge * scale).round() as u32 - u32::from(left);
                let y = (rect.center().y * scale).round() as u32;
                assert_eq!(image.get_pixel(x, y).0, border, "one-pixel seam at {theme:?}/{scale}");
                // Check the column interior. In ProLight the canvas itself equals the
                // border colour, so an outside pixel cannot distinguish a second seam.
                let inside = if left { x - 1 } else { x + 1 };
                assert_ne!(image.get_pixel(inside, y).0, border, "no second seam inside the column");
            }
            image.save(root.join(format!("{prefix}-restored.png"))).unwrap();
            let layout = h.state().ui.docking.as_ref().unwrap().layout.clone();
            let drawn = strips(h.state(), &h.ctx, &layout);
            std::fs::write(
                root.join(format!("{prefix}-rects.txt")),
                drawn.iter().map(|s| format!("{} {:?} attached={}\n", s.first, s.rect, s.attached)).collect::<String>(),
            )
            .unwrap();
            for (id, rect, side) in columns(&h) {
                let left = (side == Side::Start) != crate::direction::is_rtl();
                let edge = if left { rect.right() } else { rect.left() };
                h.event(Event::PointerMoved(pos2(edge, rect.center().y)));
                h.run_steps(3);
                h.render().unwrap().save(root.join(format!("{prefix}-hover-seam-{id}.png"))).unwrap();
            }
            h.event(Event::PointerMoved(pos2(500.0, 700.0)));
            h.run_steps(3);
            h.render().unwrap().save(root.join(format!("{prefix}-hover-away.png"))).unwrap();
            if let Some(s) = drawn.iter().find(|s| s.attached) {
                let p = pos2(s.rect.left() + 1.0, s.rect.center().y);
                h.event(egui::Event::PointerMoved(p));
                h.run_steps(3);
                h.render().unwrap().save(root.join(format!("{prefix}-hover.png"))).unwrap();
                // Save actual intermediate frames, including the old neighbour seam's location.
                let from = pos2(if s.left { s.rect.right() - 1.0 } else { s.rect.left() + 1.0 }, s.rect.top() + 100.0);
                let direction = if s.left { 1.0 } else { -1.0 };
                let before = crate::dock::snapshot(h.state());
                press(&mut h, from, from + vec2(direction * 15.0, 0.0));
                for distance in [30.0, 60.0, 90.0] {
                    h.event(Event::PointerMoved(from + vec2(direction * distance, 0.0)));
                    h.run_steps(1);
                    assert_eq!(crate::dock::snapshot(h.state()), before);
                    h.render().unwrap().save(root.join(format!("{prefix}-resize-{distance}.png"))).unwrap();
                }
                release(&mut h, from + vec2(direction * 90.0, 0.0));
                h.render().unwrap().save(root.join(format!("{prefix}-resize-released.png"))).unwrap();
            }
            // The three compact populations that expose outline/header clipping.
            for count in [1, 2, 5] {
                crate::dock::apply(h.state_mut(), &saved["panelLayout"]);
                h.state_mut().run("window.panel.edge", json!({"panel":"color","edge":"floating","individual":true,"collapsed":true,"x":300,"y":160})).unwrap();
                for panel in ["navigator", "channels", "actions", "brushes"].iter().take(count - 1) {
                    h.state_mut().run("window.panel.move", json!({"panel":panel,"anchor":"color","zone":"bottom"})).unwrap();
                }
                h.state_mut().run("window.panel.group", json!({"panel":"color","operation":"collapse"})).unwrap();
                h.run_steps(6);
                let image = h.render().unwrap();
                let floating = strip(&h, "color");
                let border = crate::theme::Tokens::get(&h.ctx).card_border.to_array();
                let x = (floating.rect.left() * scale).round() as u32;
                let y = ((floating.rect.top() + INSET + HEADER * 0.5) * scale).round() as u32;
                assert_eq!(image.get_pixel(x, y).0, border, "closed left header outline at {theme:?}/{scale}");
                image.save(root.join(format!("{prefix}-floating-{count}.png"))).unwrap();
            }
            crate::dock::apply(h.state_mut(), &saved["panelLayout"]);
            h.state_mut().run("window.panel.edge", json!({"panel":"color","edge":"floating","individual":true,"collapsed":false,"x":300,"y":160})).unwrap();
            h.run_steps(6);
            let empty = DockArea::<String>::hit_rects(&h.ctx, area_id()).into_iter().find(|(hit, _)| matches!(hit,Hit::DragBar(id) if id=="color")).unwrap().1;
            h.event(Event::PointerMoved(empty.center()));
            h.run_steps(2);
            h.render().unwrap().save(root.join(format!("{prefix}-empty-header-hover.png"))).unwrap();
            for edge in ["left", "rightInner"] {
                crate::dock::apply(h.state_mut(), &saved["panelLayout"]);
                h.state_mut().run("window.panel.edge", json!({"panel":"color","edge":edge,"individual":true,"collapsed":true})).unwrap();
                h.run_steps(6);
                let image = h.render().unwrap();
                if edge == "left" {
                    let s = strip(&h, "color");
                    let x = (s.rect.left() * scale).round() as u32;
                    let y = (s.rect.center().y * scale).round() as u32;
                    assert_eq!(image.get_pixel(x, y).0, border, "Tools/strip seam snaps to one pixel");
                    assert_ne!(image.get_pixel(x + 1, y).0, border, "no second Tools/strip border");
                }
                image.save(root.join(format!("{prefix}-adjacent-{edge}.png"))).unwrap();
            }
            crate::dock::apply(h.state_mut(), &saved["panelLayout"]);
            h.state_mut().run("file.new", json!({"width":64,"height":64,"name":"Panel repair fixture"})).unwrap();
            h.run_steps(6);
            h.render().unwrap().save(root.join(format!("{prefix}-document.png"))).unwrap();
            h.set_size(vec2(760.0, 540.0));
            h.run_steps(6);
            h.render().unwrap().save(root.join(format!("{prefix}-narrow.png"))).unwrap();
            let clean = crate::dock::snapshot(&PhotocraftApp::new(photocraft_engine::Session::new(), Default::default()));
            h.set_size(vec2(1280.0, 800.0));
            crate::dock::apply(h.state_mut(), &clean);
            h.run_steps(6);
            h.render().unwrap().save(root.join(format!("{prefix}-clean.png"))).unwrap();
            if theme == crate::theme::ThemeKind::ProMedium && scale == 1.25 {
                crate::dock::apply(h.state_mut(), &saved["panelLayout"]);
                h.state_mut().run("prefs.set", json!({"path":"interface.language","value":"ru"})).unwrap();
                h.run_steps(8);
                h.render().unwrap().save(root.join("problem-ru-125.png")).unwrap();
                h.state_mut().run("window.panel.group", json!({"panel":"actions","operation":"strip-width","width":120})).unwrap();
                h.run_steps(6);
                h.render().unwrap().save(root.join("problem-ru-125-labels.png")).unwrap();
            }
        }
    }
}

#[test]
fn expanded_column_seam_resizes_without_painting_the_document() {
    for edge in ["left", "rightInner"] {
        let mut app = compact_pair();
        app.run("file.new", json!({"width":640,"height":480})).unwrap();
        app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
        app.run("window.panel.edge", json!({"panel":"properties","edge":edge})).unwrap();
        let mut h = harness(app);
        let before = columns(&h).into_iter().find(|(id, _, _)| id == "properties").unwrap().1;
        let left = edge == "left";
        let from = pos2(if left { before.right() } else { before.left() }, before.center().y);
        let to = from + vec2(if left { 60.0 } else { -60.0 }, 0.0);
        press(&mut h, from, to);
        h.event(Event::PointerMoved(to + vec2(if left { 10.0 } else { -10.0 }, 0.0)));
        h.run_steps(2);
        release(&mut h, to);
        let after = columns(&h).into_iter().find(|(id, _, _)| id == "properties").unwrap().1;
        assert!(after.width() > before.width() + 30.0, "{edge}: divider must own the seam");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 0, "{edge}: no canvas stroke");
    }
}
