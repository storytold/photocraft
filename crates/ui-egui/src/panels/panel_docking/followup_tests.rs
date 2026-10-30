//! Regressions for the user's twelve follow-up observations (local evidence 01..12).
use super::tests::{compact_pair, harness, layout, press, release};
use super::*;
use egui::Event;
use egui_kittest::Harness;

fn single() -> PhotocraftApp {
    let mut app = compact_pair();
    app.run("window.panel.close", json!({"panel":"actions"})).unwrap();
    app
}
fn scaled(app: PhotocraftApp, scale: f32) -> Harness<'static, PhotocraftApp> {
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            crate::panels::dock_area(app, ui);
        },
        app,
    );
    h.run_steps(6);
    h
}
fn strip(h: &Harness<'static, PhotocraftApp>, id: &str) -> Strip {
    strips(h.state(), &h.ctx, layout(h.state())).into_iter().find(|s| s.panels().any(|p| p == id)).unwrap()
}
fn main_icon(h: &Harness<'static, PhotocraftApp>, s: &Strip) -> Rect {
    let cell = s.cells().first().unwrap().1;
    h.output()
        .shapes
        .iter()
        .find_map(|v| match &v.shape {
            egui::Shape::Mesh(m) => {
                let r = m.calc_bounds();
                ((r.width() - 20.0).abs() < 0.1 && (r.height() - 20.0).abs() < 0.1 && cell.contains(r.center())).then_some(r)
            }
            egui::Shape::Rect(r) if r.brush.is_some() && cell.contains(r.rect.center()) && (r.rect.width() - 20.0).abs() < 0.1 => Some(r.rect),
            _ => None,
        })
        .unwrap()
}
#[test]
fn squeezing_to_minimum_keeps_icon_anchor_and_tight_header_at_all_scales() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let mut h = scaled(single(), scale);
        let mut anchor: Option<f32> = None;
        for width in [130.0, 70.0, 42.0] {
            h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":width})).unwrap();
            h.run_steps(5);
            let s = strip(&h, "properties");
            let glyph = main_icon(&h, &s);
            let offset = (glyph.center().x - s.rect.left()) * scale;
            if let Some(before) = anchor {
                assert!((offset - before).abs() < 0.02, "{scale}: icon must not jump at minimum");
            } else {
                anchor = Some(offset);
            }
            assert!(s.rect.width() * scale + 0.01 >= COMPACT_COLUMN_WIDTH * scale);
            let arrow = h.ctx.read_response(key("expand").with("properties")).unwrap();
            let close = h.ctx.read_response(key("close").with("properties")).unwrap();
            assert_eq!(arrow.rect.top(), s.rect.top());
            assert_eq!(close.rect.top(), s.rect.top());
            assert_eq!(close.rect.right(), s.rect.right());
            if width == 42.0 {
                assert!((arrow.rect.left() - s.rect.left()).abs() < 0.01);
                let cell = h.ctx.read_response(key("icon").with("properties")).unwrap().rect;
                assert!(((s.rect.bottom() - cell.bottom()) - (cell.left() - s.rect.left())).abs() * scale <= 1.01, "balanced bottom/side button padding");
            }
        }
    }
}
#[test]
fn expanded_header_has_no_extra_top_gutter_and_even_button_margins() {
    let mut app = single();
    app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
    let h = harness(app);
    let w = &layout(h.state()).floating.iter().find(|w| w.contains(&"properties".to_owned())).unwrap();
    let close = h.ctx.read_response(area_id().with(("full-close", "properties"))).unwrap();
    assert!((close.rect.top() - w.rect[1] - 3.0).abs() < 0.01, "no separate resize gutter above the header");
    assert!((w.rect[0] + w.rect[2] - close.rect.right() - 3.0).abs() < 0.01, "right margin equals top/bottom");
}
#[test]
fn full_source_joining_a_compact_receiver_keeps_every_panel_compact() {
    for attached in [false, true] {
        let mut app = compact_pair();
        app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
        app.run(
            "window.panel.edge",
            json!({"panel":"color","edge":if attached {"rightInner"} else {"floating"},"individual":true,"collapsed":true,"x":600,"y":200}),
        )
        .unwrap();
        let to: Target<String> = Target::At {
            anchor: "color".into(),
            placement: if attached { Placement::Group { below: true, height: None } } else { Placement::Split(Zone::Bottom) },
        };
        app.run("window.panel.group", json!({"panel":"properties","operation":"drop","whole":true,"to":to})).unwrap();
        for id in ["color", "properties", "actions"] {
            assert!(compat::compact(&app.ui.docking.as_ref().unwrap().layout, &app.ui.panel_presentation, id), "{attached}: {id}");
        }
    }
}
#[test]
fn compact_source_can_join_full_sides_or_explicit_tab_slots() {
    for zone in ["left", "right", "tab"] {
        let mut app = single();
        app.run("window.panel.edge", json!({"panel":"color","edge":"floating","individual":true,"collapsed":false,"x":550,"y":200})).unwrap();
        let mut h = harness(app);
        let r = super::super::last_bodies(&h.ctx).into_iter().find(|(id, _)| id == "color").unwrap().1;
        let to = match zone {
            "top" => pos2(r.center().x, r.top() + 2.0),
            "bottom" => pos2(r.center().x, r.bottom() - 2.0),
            "left" => pos2(r.left() + 2.0, r.center().y),
            "right" => pos2(r.right() - 2.0, r.center().y),
            "tab" => {
                DockArea::<String>::hit_rects(&h.ctx, area_id()).into_iter().find(|(hit, _)| matches!(hit,Hit::Tab(id) if id=="color")).unwrap().1.left_center()
                    + vec2(2.0, 0.0)
            }
            _ => r.center(),
        };
        let from = strip(&h, "properties").cells().first().unwrap().1.center();
        press(&mut h, from, to);
        release(&mut h, to);
        let w = layout(h.state()).floating.iter().find(|w| w.contains(&"color".to_owned())).unwrap();
        assert!(w.contains(&"properties".to_owned()), "{zone}: compact must reach full destination");
        assert_eq!(h.state().ui.panel_presentation.compact.contains(&"properties".to_owned()), zone != "tab");
    }
}
#[test]
fn moving_a_floating_compact_window_to_empty_space_has_no_insertion_animation() {
    let mut h = harness(single());
    let s = strip(&h, "properties");
    let from = pos2(s.rect.center().x, s.rect.top() + HEADER + GRIP * 0.5);
    let to = pos2(650.0, 320.0);
    press(&mut h, from, to);
    release(&mut h, to);
    assert!(h.ctx.data(|d| d.get_temp::<Flight>(key("flight"))).is_none());
}
#[test]
fn a_column_boundary_has_one_canonical_marker_through_its_drop_band() {
    let h = harness(single());
    let cols = h.ctx.data(|d| d.get_temp::<Vec<(String, Rect, Side)>>(key("columns"))).unwrap();
    let anchor = compat::columns(layout(h.state()))
        .into_iter()
        .find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == "layers")))
        .unwrap()
        .1
        .groups
        .first()
        .unwrap()
        .panels
        .first()
        .unwrap();
    let (id, r, side) = cols.iter().find(|(id, _, _)| id == anchor).unwrap();
    let x = if *side == Side::End { r.left() } else { r.right() };
    let mut target = None;
    for offset in [-12.0, -6.0, 0.0, 6.0, 12.0] {
        let t = landing(&h.ctx, "properties", true, pos2(x + offset, r.center().y)).unwrap();
        assert!(matches!(&t.target,Target::At {anchor,..} if anchor==id));
        let actual = (serde_json::to_value(&t.target).unwrap(), t.marker);
        if let Some(ref expected) = target {
            assert_eq!(&actual, expected);
        } else {
            target = Some(actual);
        }
    }
}
#[test]
fn horizontal_docks_restore_their_normal_floating_size_and_visible_position() {
    for edge in ["top", "bottom"] {
        let mut app = single();
        app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
        app.run("window.panel.edge", json!({"panel":"properties","edge":edge})).unwrap();
        let mut h = harness(app);
        let body = super::super::last_bodies(&h.ctx).into_iter().find(|(id, _)| id == "properties").unwrap().1;
        assert!(body.width() > 900.0, "{edge}: horizontal dock should span the workspace");
        let saved = crate::dock::snapshot(h.state());
        let mut restored = single();
        crate::dock::apply(&mut restored, &saved);
        assert_eq!(restored.ui.panel_presentation.floating_sizes.get("properties"), Some(&[320.0, 400.0]));
        restored.run("window.panel.group", json!({"panel":"properties","operation":"collapse"})).unwrap();
        let collapsed = harness(restored);
        assert!(!floats(layout(collapsed.state()), "properties"), "horizontal collapse stays attached");
        let bar =
            collapsed.ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects"))).unwrap().into_iter().find(|(p, _)| p == "properties").unwrap().1;
        assert!((bar.height() - super::super::contract::FULL_HEADER_HEIGHT).abs() < 1.1, "horizontal collapse shows its tab bar");
        let source = DockArea::<String>::hit_rects(&h.ctx, area_id())
            .into_iter()
            .find(|(hit, _)| matches!(hit,Hit::DragBar(id) if id=="properties"))
            .unwrap()
            .1
            .center();
        let to = pos2(450.0, if edge == "top" { 650.0 } else { 150.0 });
        press(&mut h, source, to);
        release(&mut h, to);
        let w = layout(h.state()).floating.iter().find(|w| w.contains(&"properties".to_owned())).unwrap();
        assert_eq!([w.rect[2], w.rect[3]], [320.0, 400.0]);
        assert!(Rect::from_min_size(pos2(w.rect[0], w.rect[1]), vec2(w.rect[2], w.rect[3])).contains(to), "the restored window must stay under the grab");
        h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"collapse"})).unwrap();
        h.run_steps(4);
        let s = strip(&h, "properties");
        let bounds = h.ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap();
        assert!(bounds.contains_rect(s.rect));
    }
}
#[test]
fn mouse_reaches_top_and_bottom_from_compact_and_expanded_sources() {
    for expanded in [false, true] {
        for bottom in [false, true] {
            let mut app = single();
            if expanded {
                app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
            }
            let mut h = harness(app);
            let bounds = h.ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap();
            let from = if expanded {
                DockArea::<String>::hit_rects(&h.ctx, area_id())
                    .into_iter()
                    .find(|(hit, _)| matches!(hit,Hit::DragBar(id) if id=="properties"))
                    .unwrap()
                    .1
                    .center()
            } else {
                let s = strip(&h, "properties");
                pos2(s.rect.center().x, s.rect.top() + HEADER + GRIP * 0.5)
            };
            let to = pos2(bounds.center().x, if bottom { bounds.bottom() - 2.0 } else { bounds.top() + 2.0 });
            press(&mut h, from, to);
            release(&mut h, to);
            assert!(!layout(h.state()).floating.iter().any(|w| w.contains(&"properties".to_owned())));
            let frame = h.ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects"))).unwrap().into_iter().find(|(p, _)| p == "properties").unwrap().1;
            assert!(frame.width() > 900.0);
            if !expanded {
                assert!((frame.height() - super::super::contract::FULL_HEADER_HEIGHT).abs() < 1.1);
            }
        }
    }
}
#[test]
fn control_drag_of_an_expanded_source_bypasses_all_dock_targets() {
    let mut app = single();
    app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
    let mut h = harness(app);
    let root = layout(h.state()).root.clone();
    let from =
        DockArea::<String>::hit_rects(&h.ctx, area_id()).into_iter().find(|(hit, _)| matches!(hit,Hit::DragBar(id) if id=="properties")).unwrap().1.center();
    let to = super::super::last_bodies(&h.ctx).into_iter().find(|(id, _)| id == "layers").unwrap().1.center();
    let mods = egui::Modifiers { ctrl: true, ..Default::default() };
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: mods });
    h.run_steps(1);
    h.event(Event::PointerMoved(to));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: mods });
    h.run_steps(3);
    assert_eq!(layout(h.state()).root, root);
    assert!(layout(h.state()).floating.iter().any(|w| w.contains(&"properties".to_owned())));
}
pub(super) fn full_app(ctx: &egui::Context) -> PhotocraftApp {
    PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ProMedium);
    let mut app = single();
    app.run("prefs.set", json!({"path":"interface.theme","value":crate::theme::ThemeKind::ProMedium.id()})).unwrap();
    app
}
#[test]
fn floating_tools_resize_both_axes_and_restore_through_workspace() {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(|cc| {
        let mut app = full_app(&cc.egui_ctx);
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        app.run("window.tools.float", json!({"x":650,"y":120,"width":60,"height":500})).unwrap();
        app
    });
    h.run_steps(8);
    let frame = egui::Id::unique("tools-palette").with("floating");
    let at = h.ctx.read_response(frame.with("resize-right")).unwrap().rect.center();
    press(&mut h, at, at + vec2(50.0, 0.0));
    release(&mut h, at + vec2(50.0, 0.0));
    let size = h.state().ui.panel_presentation.tools_size.unwrap();
    assert!((size[0] - 110.0).abs() < 1.0);
    assert_eq!(size[1], 500.0);
    let at = h.ctx.read_response(frame.with("resize-bottom")).unwrap().rect.center();
    press(&mut h, at, at - vec2(0.0, 180.0));
    release(&mut h, at - vec2(0.0, 180.0));
    let size = h.state().ui.panel_presentation.tools_size.unwrap();
    assert!((size[1] - 320.0).abs() < 1.0);
    assert!((size[0] - 110.0).abs() < 1.0);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    let saved = crate::dock::snapshot(h.state());
    let mut restored = single();
    crate::dock::apply(&mut restored, &saved);
    assert_eq!(restored.ui.panel_presentation.tools_size, Some(size));
    let before = crate::dock::snapshot(h.state());
    assert!(h.state_mut().run("window.tools.float", json!({"x":10,"width":12})).is_err());
    assert_eq!(crate::dock::snapshot(h.state()), before);
}
#[test]
fn capture_followup_twelve_scenarios() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_FOLLOWUP_CAPTURES").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).wgpu().build_eframe(|cc| full_app(&cc.egui_ctx));
        h.run_steps(8);
        let save = |h: &mut Harness<'static, PhotocraftApp>, name: &str| h.render().unwrap().save(root.join(format!("{scale}-{name}.png"))).unwrap();
        for width in [130.0, 70.0, 42.0] {
            h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":width})).unwrap();
            h.run_steps(5);
            save(&mut h, &format!("width-{width}"));
        }
        for name in ["expand", "close"] {
            let at = h.ctx.read_response(key(name).with("properties")).unwrap().rect.center();
            h.event(Event::PointerMoved(at));
            h.run_steps(2);
            save(&mut h, &format!("hover-{name}"));
        }
        h.event(Event::PointerMoved(pos2(450.0, 400.0)));
        h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
        h.run_steps(6);
        save(&mut h, "expanded");
        h.state_mut().run("window.panel.edge", json!({"panel":"properties","edge":"top"})).unwrap();
        h.run_steps(6);
        save(&mut h, "top-docked");
        let source = DockArea::<String>::hit_rects(&h.ctx, area_id())
            .into_iter()
            .find(|(hit, _)| matches!(hit,Hit::DragBar(id) if id=="properties"))
            .unwrap()
            .1
            .center();
        press(&mut h, source, pos2(450.0, 650.0));
        release(&mut h, pos2(450.0, 650.0));
        save(&mut h, "detached-from-top");
        h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"collapse"})).unwrap();
        h.run_steps(6);
        save(&mut h, "collapsed-after-top");
        h.state_mut().run("window.tools.float", json!({"x":650,"y":120,"width":110,"height":500})).unwrap();
        h.run_steps(6);
        save(&mut h, "tools-large");
        h.state_mut().run("window.tools.float", json!({"x":650,"y":120,"width":42,"height":240})).unwrap();
        h.run_steps(6);
        save(&mut h, "tools-small");
        h.state_mut().run("window.panel.edge", json!({"panel":"color","edge":"floating","individual":true,"collapsed":false,"x":750,"y":180})).unwrap();
        h.state_mut().run("window.panel.group",json!({"panel":"color","operation":"drop","whole":true,"to":Target::At {anchor:"properties".to_owned(),placement:Placement::Split(Zone::Bottom)}})).unwrap();
        h.run_steps(6);
        save(&mut h, "full-joins-compact");
    }
}

#[test]
fn compact_header_fills_preserve_the_window_silhouette() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let mut h = scaled(single(), scale);
        for width in [42.0, 130.0] {
            h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":width})).unwrap();
            h.run_steps(4);
            for hover in [None, Some("expand"), Some("close")] {
                let at = hover.map(|name| h.ctx.read_response(key(name).with("properties")).unwrap().rect.center()).unwrap_or(pos2(500.0, 500.0));
                h.event(Event::PointerMoved(at));
                h.run_steps(2);
                let strip = strip(&h, "properties");
                let t = crate::theme::Tokens::get(&h.ctx);
                let outer = egui::CornerRadius::from(t.radius_sm);
                for shape in &h.output().shapes {
                    if let egui::Shape::Rect(r) = &shape.shape
                        && r.fill.a() > 0
                        && r.rect.top() == strip.rect.top()
                    {
                        if r.rect.left() == strip.rect.left() {
                            assert!(r.corner_radius.nw >= outer.nw, "square fill over the top-left corner: {r:?}");
                        }
                        if r.rect.right() == strip.rect.right() {
                            assert!(r.corner_radius.ne >= outer.ne, "square fill over the top-right corner: {r:?}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn capture_compact_corner_silhouette() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_CORNER_CAPTURES").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::ProLight] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).wgpu().build_eframe(|cc| {
                let mut app = full_app(&cc.egui_ctx);
                app.run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
                app
            });
            h.run_steps(8);
            assert_eq!(h.state().ui.theme, theme);
            for width in [42.0, 130.0] {
                h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":width})).unwrap();
                h.run_steps(4);
                for hover in [None, Some("expand"), Some("close")] {
                    let at = hover.map(|name| h.ctx.read_response(key(name).with("properties")).unwrap().rect.center()).unwrap_or(pos2(500.0, 500.0));
                    h.event(Event::PointerMoved(at));
                    h.run_steps(2);
                    let strip = strip(&h, "properties");
                    let t = crate::theme::Tokens::get(&h.ctx);
                    let image = h.render().unwrap();
                    if t.radius_sm * scale >= 3.0 {
                        let x = (strip.rect.left() * scale).round() as u32;
                        let y = (strip.rect.top() * scale).round() as u32;
                        let right = (strip.rect.right() * scale).round() as u32 - 1;
                        for (inside, outside) in [(x, x - 2), (right, right + 2)] {
                            assert_eq!(
                                image.get_pixel(inside, y),
                                image.get_pixel(outside, y - 2),
                                "corner outside silhouette: {theme:?} {scale} {width} {hover:?}"
                            );
                        }
                    }
                    image.save(root.join(format!("{}-{scale}-{width}-{}.png", theme.id(), hover.unwrap_or("idle")))).unwrap();
                }
            }
        }
    }
}

#[test]
fn unreadable_shared_layout_is_kept_even_without_a_new_version_tag() {
    let mut app = single();
    let mut saved = crate::dock::snapshot(&app);
    saved["docking"] = json!({"version":1,"layout":{"kind":"future-framework-tree","nodes":["properties","layers"]},"future":{"opaque":"keep me"}});
    let previous_status = app.ui.status.clone();
    crate::dock::apply(&mut app, &saved);
    assert_ne!(app.ui.status, previous_status);
    assert!(app.ui.docking.is_none());
    assert!(app.ui.docking_kept.is_some());
    assert!(!app.ui.status.is_empty());
    let mut h = harness(app);
    h.run_steps(5);
    let after = crate::dock::snapshot(h.state());
    for key in ["docking", "dockingReturns", "dockingCollapsed", "panelPresentation", "toolbarFloating"] {
        assert_eq!(after.get(key), saved.get(key), "{key} must survive fallback rendering");
    }
}
#[test]
fn one_bad_presentation_field_does_not_reset_the_other_view_preferences() {
    let mut app = single();
    app.run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":90})).unwrap();
    app.ui.panel_presentation.tools_size = Some([74.0, 300.0]);
    let mut saved = crate::dock::snapshot(&app);
    saved["panelPresentation"]["tools_size"] = json!({"future_sizing":"unsupported"});
    crate::dock::apply(&mut app, &saved);
    assert!(app.ui.panel_presentation.compact.contains(&"properties".to_owned()));
    assert_eq!(app.ui.panel_presentation.widths.get("properties"), Some(&90.0));
    assert!(app.ui.panel_presentation.migrated);
    assert!(app.ui.panel_presentation.tools_size.is_none());
}
#[test]
fn open_windows_keep_identity_and_preferences_across_host_ui_changes() {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(|cc| {
        let mut app = full_app(&cc.egui_ctx);
        app.run("window.tools.float", json!({"x":650,"y":120,"width":74,"height":300})).unwrap();
        app
    });
    h.run_steps(8);
    let original = h.state().ui.docking.clone();
    let view = h.state().ui.panel_presentation.clone();
    let tools = h.state().ui.toolbar_floating;
    for (theme, scale, size, chrome) in [
        (crate::theme::ThemeKind::StudioLight, 1.25, vec2(760.0, 580.0), false),
        (crate::theme::ThemeKind::ProLight, 2.0, vec2(420.0, 540.0), true),
        (crate::theme::ThemeKind::Pro, 1.5, vec2(1280.0, 800.0), false),
        (crate::theme::ThemeKind::ProMedium, 1.0, vec2(1280.0, 800.0), true),
    ] {
        h.state_mut().run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
        h.state_mut().ui.panels.menu_bar = chrome;
        h.state_mut().ui.panels.rail = chrome;
        h.set_pixels_per_point(scale);
        h.set_size(size);
        h.run_steps(8);
        assert_eq!(h.state().ui.docking, original, "layout identity changed with the host UI");
        assert_eq!(h.state().ui.panel_presentation, view);
        assert_eq!(h.state().ui.toolbar_floating, tools);
    }
    let saved = crate::dock::snapshot(h.state());
    let mut restored = single();
    crate::dock::apply(&mut restored, &saved);
    assert_eq!(restored.ui.docking, original);
    assert_eq!(restored.ui.panel_presentation, view);
    assert_eq!(restored.ui.toolbar_floating, tools);
}
