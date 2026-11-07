use super::*;
use egui_kittest::{Harness, kittest::Queryable};

/// Build a settled dock and status bar around a document with the requested layer count.
/// GPU rendering is optional so geometry and interaction checks can run without snapshots.
fn harness(kind: theme::ThemeKind, count: usize, scale: f32, render: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.ui.theme = kind;
    app.ui.panels.color = false;
    app.ui.panels.properties = false;
    app.ui.panels.rail = false;
    app.run("file.new", json!({"width": 160, "height": 120})).unwrap();
    for i in 1..count {
        app.run("layer.new.layer", json!({"name": format!("Layer {i}")})).unwrap();
    }
    let builder = Harness::builder().with_size(vec2(900.0, 400.0)).with_pixels_per_point(scale);
    let builder = if render { builder.wgpu() } else { builder };
    let mut ready = false;
    let mut h = builder.build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), kind);
                ready = true;
                return;
            }
            crate::i18n::with_language(crate::i18n::Lang::EN, || {
                crate::panels::status_bar(app, ui);
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("resize-test-status-top"), ui.available_rect_before_wrap().bottom()));
                crate::panels::dock_area(app, ui);
            });
        },
        app,
    );
    h.run_steps(5);
    h
}

/// Check all seven Layers actions against the dock and status bar bounds.
fn assert_footer_inside_panel(h: &Harness<'_, PhotocraftApp>) {
    assert_tab_footer_inside_panel(h, "layers", 7);
}

/// Require visible hit targets to stay in the panel, and full buttons to fit when space allows.
fn assert_tab_footer_inside_panel(h: &Harness<'_, PhotocraftApp>, tab: &str, count: usize) {
    let panel = crate::panel_docking::last_bodies(&h.ctx).into_iter().find(|(id, _)| id == tab).unwrap().1;
    let buttons: Vec<_> = h.ctx.viewport(|v| {
        v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(26.0, 26.0)).map(|w| (w.rect, w.interact_rect)).collect()
    });
    assert_eq!(buttons.len(), count, "all {tab} actions remain present: {buttons:?}");
    let status_top = h.ctx.data(|d| d.get_temp::<f32>(egui::Id::unique("resize-test-status-top"))).unwrap();
    let visible = panel.intersect(Rect::from_min_max(panel.min, pos2(panel.right(), status_top)));
    assert!(buttons.iter().filter(|(_, hit)| hit.is_positive()).all(|(_, hit)| visible.contains_rect(*hit)), "footer {buttons:?} must stay within {visible:?}");
    if visible.height() >= widgets::FOOTER_BAR + 1.0 {
        assert!(buttons.iter().all(|(rect, _)| visible.contains_rect(*rect)), "the full action targets must fit in {visible:?}: {buttons:?}");
    }
}

/// Long channel and path lists must leave their footers inside the dock at every tested height.
#[test]
fn channels_and_paths_keep_footers_inside_short_docks() {
    for kind in theme::ThemeKind::ALL {
        for (tab, count) in [("channels", 4), ("paths", 7)] {
            let mut h = harness(kind, 1, 1.0, false);
            for i in 0..20 {
                let (cmd, params) = if tab == "channels" {
                    ("channel.new", json!({"name": format!("Alpha {i}")}))
                } else {
                    ("path.set", json!({"name": format!("Path {i}"), "path": {"subpaths": []}}))
                };
                h.state_mut().run(cmd, params).unwrap();
            }
            h.state_mut().run("window.panel.activate", json!({"panel": tab})).unwrap();
            for height in [180.0, 145.0, 125.0, 400.0] {
                h.set_size(vec2(900.0, height));
                h.run_steps(4);
                assert_tab_footer_inside_panel(&h, tab, count);
            }
        }
    }
}

/// Resizing in both directions must preserve footer bounds across themes and display scales.
#[test]
fn layers_footer_stays_inside_dock_while_resizing_short_and_tall() {
    for kind in theme::ThemeKind::ALL {
        for scale in [1.0, 1.25] {
            for count in [1, 20] {
                let mut h = harness(kind, count, scale, false);
                for height in [400.0, 230.0, 200.0, 180.0, 165.0, 400.0] {
                    h.set_size(vec2(900.0, height));
                    h.run_steps(4);
                    assert_footer_inside_panel(&h);
                }
            }
        }
    }
}

/// A footer button in a shortened dock must still dispatch its document-changing command.
#[test]
fn short_layers_footer_keeps_its_create_group_action() {
    let mut h = harness(theme::ThemeKind::ProMedium, 20, 1.0, false);
    h.set_size(vec2(900.0, 180.0));
    h.run_steps(4);
    assert_footer_inside_panel(&h);
    let before = h.state().session.active().unwrap().doc.layer_count();
    h.get_by_label("Create a new group").click();
    h.run_steps(3);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.layer_count(), before + 1);
    assert!(st.doc.layer(st.active_layer.unwrap()).unwrap().is_group());
}

/// Wheel scrolling must move layer rows while leaving the footer stationary and contained.
#[test]
fn short_layers_list_scrolls_without_moving_its_footer() {
    let mut h = harness(theme::ThemeKind::ProMedium, 20, 1.0, false);
    h.set_size(vec2(900.0, 230.0));
    h.run_steps(4);
    let footer = h.get_by_label("Create a new group").rect();
    let before = h.get_by_label("Layer 19").rect();
    h.hover_at(before.center());
    h.run_steps(1);
    h.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -100.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(20);
    assert!(h.get_by_label("Layer 19").rect().top() < before.top() - 20.0, "the layer list scrolled");
    assert_eq!(h.get_by_label("Create a new group").rect(), footer, "the action bar stays fixed");
    assert_footer_inside_panel(&h);
}

/// Even sub-button-height panels must clip hit targets and preserve the parent's bounds.
#[test]
fn tiny_layers_panel_clips_actions_and_does_not_expand_its_parent() {
    for height in [0.0, 8.0, 16.0, 20.0] {
        for document in [false, true] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            if document {
                app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
            }
            let body = Rect::from_min_size(pos2(20.0, 20.0), vec2(300.0, height));
            let bounds = body + widgets::body_margin(true);
            let mut ready = false;
            let mut h = Harness::builder().with_size(vec2(360.0, 160.0)).build_ui_state(
                move |ui, app: &mut PhotocraftApp| {
                    if !ready {
                        PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::ProMedium);
                        ready = true;
                        return;
                    }
                    let mut panel = ui.new_child(egui::UiBuilder::new().max_rect(body));
                    panel.set_clip_rect(bounds);
                    layers(app, &mut panel);
                    assert!(panel.min_rect().bottom() <= body.bottom(), "fixed controls cannot enlarge the frame");
                },
                app,
            );
            h.run_steps(4);
            let buttons: Vec<_> = h.ctx.viewport(|v| {
                v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(26.0, 26.0)).map(|w| w.interact_rect).collect()
            });
            assert_eq!(buttons.len(), if document { 7 } else { 0 });
            assert!(buttons.iter().filter(|r| r.is_positive()).all(|r| bounds.contains_rect(*r)), "{buttons:?} in {bounds:?}");
        }
    }
}

/// Save short and tall dock snapshots for each theme when a fixture directory is requested.
#[test]
fn layers_resize_visual_fixtures() {
    let Some(directory) = std::env::var_os("PHOTOCRAFT_LAYERS_RESIZE_FIXTURES").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&directory).unwrap();
    for kind in theme::ThemeKind::ALL {
        let mut h = harness(kind, 20, 1.0, true);
        for height in [400.0, 180.0] {
            h.set_size(vec2(900.0, height));
            h.run_steps(4);
            assert_footer_inside_panel(&h);
            h.render().unwrap().save(directory.join(format!("layers-{}-{height}.png", kind.id()))).unwrap();
        }
    }
}
