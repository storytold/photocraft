use super::*;

/// A textured layer (a checker of two colours over a gradient) with a 20 px square selected.
fn app_with_selection() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 200, "height": 160, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..160 {
                for x in 0..200 {
                    let v = ((x / 6 + y / 6) % 2) as f32 * 0.5 + x as f32 / 400.0;
                    s.write_pixel(x, y, &[v, 0.3, 1.0 - v, 1.0]);
                }
            }
            // A red blemish to remove.
            for y in 72..88 {
                for x in 92..108 {
                    s.write_pixel(x, y, &[1.0, 0.0, 0.0, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 90, "y": 70, "width": 20, "height": 20})).unwrap();
    app.sync_views();
    app
}

fn px(app: &PhotocraftApp, layer: LayerId, x: i32, y: i32) -> Vec<f32> {
    app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(x, y)
}

fn ws(app: &PhotocraftApp) -> &CafWorkspace {
    app.distort.caf.as_ref().unwrap()
}

#[test]
fn settings_dropdowns_are_named_for_their_visible_labels() {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 900.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
        .build_ui_state(|ui, app: &mut PhotocraftApp| show(app, ui.ctx()), app);
    h.run_steps(2);
    // The Indicates dropdown lives inside a collapsed "Sampling Area Overlay" header.
    h.get_by_role_and_label(Role::DisclosureTriangle, "Sampling Area Overlay").click();
    h.run_steps(3);
    let labels: Vec<String> = h.query_all_by_role(Role::ComboBox).filter_map(|n| n.accesskit_node().label()).collect();
    for label in ["Indicates", "Color Adaptation", "Rotation Adaptation", "Output To"] {
        assert!(labels.iter().any(|have| have == label), "{label} names its dropdown: {labels:?}");
    }
    for wrong in ["Contents:", "Color", "Rotation", "Source"] {
        assert!(!labels.iter().any(|have| have == wrong), "{wrong} names no control here: {labels:?}");
    }
}

/// Photoshop opens the workspace with the Sampling Brush subtracting at a tenth of the
/// window's side, Auto sampling, output to a new layer.
#[test]
fn opens_like_photoshop() {
    let mut app = app_with_selection();
    app.session.prefs.edit(|p| {
        p.dialogs.remove(REMEMBERED);
    });
    open(&mut app).unwrap();
    let d = ws(&app).describe();
    assert_eq!(d["window"], json!([0, 0, 200, 160]), "a 200 px window, clipped to the 200 × 160 canvas");
    assert_eq!((d["tool"].as_str(), d["brushMode"].as_str(), d["brushSize"].as_f64()), (Some("brush"), Some("subtract"), Some(20.0)));
    assert_eq!((d["settings"]["sampling"].as_str(), d["settings"]["output"].as_str()), (Some("auto"), Some("new")));
    assert_eq!(d["fillArea"], json!(400));
    assert!(d["sampledProxyPixels"].as_u64().unwrap() > 1000);
}

#[test]
fn brush_strokes_change_the_sampling_area_and_undo() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    control(&mut app, &json!({"sampling": "rectangular"})).unwrap();
    let all = ws(&app).describe()["sampledProxyPixels"].as_u64().unwrap();
    assert_eq!(all, 200 * 160 - 400, "Rectangular: the whole window but the fill area");
    // Subtracting (the default): a 10 px stroke takes about 10 × 60 px away.
    let d = control(&mut app, &json!({"stroke": {"points": [[20.0, 20.0], [80.0, 20.0]], "size": 10}})).unwrap();
    let less = d["sampledProxyPixels"].as_u64().unwrap();
    assert!((all - less).abs_diff(10 * 60 + 79) < 120, "{all} → {less}");
    // Undo brings it back; redo takes it again.
    assert_eq!(control(&mut app, &json!({"undo": true})).unwrap()["sampledProxyPixels"].as_u64(), Some(all));
    assert_eq!(control(&mut app, &json!({"redo": true})).unwrap()["sampledProxyPixels"].as_u64(), Some(less));
    // Custom starts empty and the brush adds; Alt reverses it.
    let d = control(&mut app, &json!({"sampling": "custom"})).unwrap();
    assert_eq!((d["sampledProxyPixels"].as_u64(), d["brushMode"].as_str(), d["strokes"].as_u64()), (Some(0), Some("add"), Some(0)));
    let d = control(&mut app, &json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8}})).unwrap();
    assert!(d["sampledProxyPixels"].as_u64().unwrap() > 300);
    let d = control(&mut app, &json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8, "alt": true}})).unwrap();
    assert_eq!(d["sampledProxyPixels"].as_u64(), Some(0));
    // Undoing the change of option restores Rectangular with its stroke.
    control(&mut app, &json!({"undo": true})).unwrap();
    control(&mut app, &json!({"undo": true})).unwrap();
    let d = control(&mut app, &json!({"undo": true})).unwrap();
    assert_eq!((d["settings"]["sampling"].as_str(), d["strokes"].as_u64(), d["brushMode"].as_str()), (Some("rectangular"), Some(1), Some("subtract")));
}

#[test]
fn the_lasso_edits_the_fill_area_and_the_window_follows() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let d = control(&mut app, &json!({"lasso": {"op": "add", "points": [[130.0, 70.0], [150.0, 70.0], [150.0, 90.0], [130.0, 90.0]]}})).unwrap();
    assert_eq!(d["fillArea"], json!(800));
    let d = control(&mut app, &json!({"expand": 2})).unwrap();
    assert!(d["fillArea"].as_u64().unwrap() > 900);
    let d = control(&mut app, &json!({"undo": true, "lassoMode": "subtract"})).unwrap();
    assert_eq!((d["fillArea"].as_u64(), d["lassoMode"].as_str()), (Some(800), Some("subtract")));
    // Subtracting everything is refused: the fill area can't be empty.
    let d = control(&mut app, &json!({"lasso": {"points": [[0.0, 0.0], [200.0, 0.0], [200.0, 160.0], [0.0, 160.0]]}})).unwrap();
    assert_eq!(d["fillArea"], json!(800));
    assert!(d["error"].as_str().is_some_and(|e| e.contains("empty")), "{d}");
    // The reset button goes back to the selection.
    let d = control(&mut app, &json!({"reset": "lasso"})).unwrap();
    assert_eq!(d["fillArea"], json!(400));
}

/// A sampling area the Sampling Brush erased entirely previews as an error naming the
/// problem, not as the black fill it would make; undoing the stroke brings the preview back.
#[test]
fn an_erased_sampling_area_previews_as_an_error() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    control(&mut app, &json!({"sampling": "rectangular", "waitPreview": true})).unwrap();
    assert!(ws(&app).describe()["previewError"].is_null());
    // Subtracting (the default) with a brush wider than the 200 × 160 window.
    let e = control(&mut app, &json!({"stroke": {"points": [[100.0, 80.0]], "size": 600}, "waitPreview": true})).unwrap_err();
    assert!(e.contains("nothing to sample from"), "{e}");
    let d = ws(&app).describe();
    assert_eq!(d["sampledProxyPixels"].as_u64(), Some(0), "{d}");
    assert!(d["previewError"].as_str().is_some_and(|e| e.contains("nothing to sample from")), "{d}");
    let d = control(&mut app, &json!({"undo": true, "waitPreview": true})).unwrap();
    assert!(d["previewError"].is_null(), "{d}");
}

/// OK whose fill fails keeps the workspace open with the error in its panel, from the button
/// and from the control channel, so the work isn't thrown away; once the fill can run, OK
/// fills and closes.
#[test]
fn a_failed_ok_keeps_the_workspace_open() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    app.session.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
    commit(&mut app);
    assert!(app.distort.caf.is_some(), "OK keeps the workspace open when the fill fails");
    assert!(ws(&app).error.as_deref().is_some_and(|e| e.contains("locked")), "{:?}", ws(&app).error);
    assert!(app.ui.status_error);
    assert!(control(&mut app, &json!({"commit": true})).unwrap_err().contains("locked"));
    assert!(app.distort.caf.is_some(), "the control channel's OK too");
    app.session.execute("layer.setProps", json!({"locks": {"pixels": false}})).unwrap();
    let steps = app.session.active().unwrap().history.past_len().max(steps);
    commit(&mut app);
    assert!(app.distort.caf.is_none(), "OK closes once the fill runs");
    assert!(app.session.active().unwrap().history.past_len() > steps, "and fills");
}

/// Undo or Redo of a sampling stroke changes what OK would fill, so it clears the error a
/// failed OK left in the panel, as a new stroke does; otherwise the stale error would stay
/// drawn over the recovered preview.
#[test]
fn undo_and_redo_of_a_stroke_clear_a_failed_oks_error() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    control(&mut app, &json!({"stroke": {"points": [[20.0, 20.0], [60.0, 20.0]], "size": 10}})).unwrap();
    app.session.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
    commit(&mut app);
    assert!(ws(&app).error.is_some(), "the failed OK reports in the panel");
    control(&mut app, &json!({"undo": true})).unwrap();
    assert!(ws(&app).error.is_none(), "Undo of the stroke clears it");
    commit(&mut app);
    assert!(ws(&app).error.is_some());
    control(&mut app, &json!({"redo": true})).unwrap();
    assert!(ws(&app).error.is_none(), "Redo of the stroke clears it too");
}

#[test]
fn apply_fills_and_stays_open_ok_closes_cancel_keeps_what_was_applied() {
    let mut app = app_with_selection();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    open(&mut app).unwrap();
    // The preview is what Apply writes.
    control(&mut app, &json!({"output": "current", "colorAdaptation": "none", "waitPreview": true})).unwrap();
    let (rect, img) = ws(&app).preview.pending_image.clone().unwrap();
    control(&mut app, &json!({"apply": true})).unwrap();
    assert!(app.distort.caf.is_some(), "Apply keeps the workspace open");
    let after = px(&app, layer, 100, 80);
    assert!(after[0] < 0.9 || after[1] > 0.1, "the blemish is filled: {after:?}");
    let i = ((80 - rect.y0) * rect.width() as i32 + (100 - rect.x0)) as usize;
    let shown = img.pixels[i];
    assert!((f32::from(shown.r()) / 255.0 - after[0]).abs() < 0.01 && (f32::from(shown.b()) / 255.0 - after[2]).abs() < 0.01, "preview {shown:?} vs {after:?}");
    let steps = app.session.active().unwrap().history.past_len();
    cancel(&mut app);
    assert!(app.distort.caf.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "Cancel keeps the applied fill");
    // OK with New Layer: a new layer holds the fill; the original stays.
    app.session.execute("edit.undo", json!({})).unwrap();
    open(&mut app).unwrap();
    control(&mut app, &json!({"output": "new", "commit": true})).unwrap();
    assert!(app.distort.caf.is_none());
    assert_eq!(px(&app, layer, 100, 80)[0], 1.0, "the original layer keeps its blemish");
    let top = app.session.active().unwrap().active_layer.unwrap();
    assert_ne!(top, layer);
    assert!(px(&app, top, 100, 80)[3] > 0.99, "the new layer holds the fill");
}

/// #5092: the workspace left open does not follow the user to another document, and closing
/// the document it was opened on closes it.
#[test]
fn the_workspace_is_dropped_when_another_document_becomes_active() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    crate::distort_ui::drop_foreign(&mut app);
    assert!(app.distort.caf.is_some());
    app.session.execute("file.new", json!({"width": 32, "height": 32, "depth": 8})).unwrap();
    crate::distort_ui::drop_foreign(&mut app);
    assert!(app.distort.caf.is_none());
    assert!(app.session.set_active(0));
    open(&mut app).unwrap();
    crate::distort_ui::drop_foreign(&mut app);
    assert!(app.distort.caf.is_some());
    assert!(app.session.close(0).is_some());
    crate::distort_ui::drop_foreign(&mut app);
    assert!(app.distort.caf.is_none());
}

#[test]
fn settings_are_remembered_and_the_menu_opens_the_workspace() {
    let ctx = egui::Context::default();
    let mut app = app_with_selection();
    let r = crate::menus::invoke(&mut app, &ctx, "edit.contentAwareFill", json!({})).unwrap();
    assert!(r["contentAwareFill"]["window"].is_array(), "{r}");
    let set = json!({"opacity": 30, "color": [200, 40, 40], "excluded": true, "colorAdaptation": "high", "rotationAdaptation": "low", "mirror": true, "output": "duplicate"});
    crate::menus::invoke(&mut app, &ctx, "edit.contentAwareFill", json!({"ui": set})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "edit.contentAwareFill", json!({"ui": {"cancel": true}})).unwrap();
    open(&mut app).unwrap();
    let mut want = CafOpts::default();
    want.apply(&set).unwrap();
    assert_eq!(ws(&app).opts, want);
    // Bad values are errors; the engine command with params still runs directly.
    assert!(control(&mut app, &json!({"sampling": "everywhere"})).is_err());
    assert!(control(&mut app, &json!({"tool": "pencil"})).is_err());
    cancel(&mut app);
    let r = crate::menus::invoke(&mut app, &ctx, "edit.contentAwareFill", json!({"colorAdaptation": "none"})).unwrap();
    assert_eq!(r["filled"], json!(400));
    assert!(app.distort.caf.is_none());
}

/// The engine's sampling area over the plan's rectangle (what the fill samples) and its fill
/// area.
fn engine_source(app: &PhotocraftApp) -> (Rect, Vec<bool>, Vec<bool>) {
    let w = ws(app);
    let st = app.session.active().unwrap();
    let plan = photocraft_engine::edit_menu_cmds::content_aware_plan(&st.doc, w.layer, &plan_params(w, true), CMD).unwrap();
    (plan.rect, plan.source, plan.hole)
}

/// Each stroke is painted into the proxy once, over its own pixels: 400 strokes of 50 points
/// and the undo and redo that rebuild the proxy stay cheap (they used to test every proxy
/// pixel against every segment), and the overlay is exactly what the engine samples (at one
/// proxy px per document px).
#[test]
fn strokes_are_painted_once_and_the_overlay_is_the_engines_sampling_area() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    control(&mut app, &json!({"sampling": "rectangular"})).unwrap();
    let start = std::time::Instant::now();
    for i in 0..400u32 {
        // Lines across the document, most subtracting, every fifth adding (Alt).
        let y = 2.0 + f64::from(i % 40) * 4.0;
        let pts: Vec<[f64; 2]> = (0..50).map(|j| [f64::from(j) * 4.0 + f64::from(i % 3), y + f64::from(j % 2)]).collect();
        control(&mut app, &json!({"stroke": {"points": pts, "size": 6 + (i % 4) * 3, "alt": i % 5 == 0}})).unwrap();
    }
    let painted = start.elapsed();
    let start = std::time::Instant::now();
    control(&mut app, &json!({"undo": true})).unwrap();
    control(&mut app, &json!({"redo": true})).unwrap();
    let rebuilt = start.elapsed();
    assert!(painted.as_secs_f64() < 10.0 && rebuilt.as_secs_f64() < 10.0, "400 strokes {painted:?}, undo + redo {rebuilt:?}");
    let w = ws(&app);
    assert_eq!((w.k, w.strokes.len()), (1, 400));
    let (rect, source, hole) = engine_source(&app);
    assert_eq!(rect, w.canvas, "Rectangular's window is the whole 200 × 160 canvas");
    let differ = (0..source.len()).filter(|&i| (w.mask[i] && !w.hole_px[i]) != (source[i] && !hole[i])).count();
    assert_eq!(differ, 0, "proxy pixels that differ from the engine's sampling area");
    assert!(w.describe()["sampledProxyPixels"].as_u64().unwrap() > 1000);
}

/// `edit.contentAwareFill {"ui": …}` addresses the workspace: with it closed that is an
/// error, not a fill with the default settings (an agent polling the state after an Esc
/// used to add a fill per poll).
#[test]
fn ui_params_with_the_workspace_closed_are_an_error_not_a_fill() {
    let ctx = egui::Context::default();
    let mut app = app_with_selection();
    let steps = app.session.active().unwrap().history.past_len();
    for ui in [json!({}), json!({"cancel": true}), json!({"sampling": "rectangular"})] {
        let r = crate::menus::invoke(&mut app, &ctx, "edit.contentAwareFill", json!({"ui": ui}));
        assert!(r.as_ref().is_err_and(|e| e.contains("not open")), "{r:?}");
    }
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "nothing was filled");
    assert!(app.distort.caf.is_none());
}

/// The press is the first point of a stroke (and of a lasso), and the pointer's moves before
/// egui's drag threshold are its next ones: the drag, which starts a few pixels later, goes
/// on with the same stroke rather than beginning another over it.
#[test]
fn a_stroke_keeps_its_points_from_before_the_drag_threshold() {
    use egui::{Modifiers, PointerButton};
    for tool in ["brush", "lasso"] {
        let mut app = app_with_selection();
        open(&mut app).unwrap();
        control(&mut app, &json!({"tool": tool, "sampling": "rectangular"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1600.0, 1000.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                show(app, &ctx);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        // The document view sits between the tool strip and the Preview panel, under the
        // title and the options bar; its centre shows the document's centre.
        let view = ERect::from_min_max(pos2(LEFT_W, 30.0 + BAR_H), pos2(1600.0 - PANEL_W - PREVIEW_W, 1000.0));
        let c = view.center();
        let at = |dx: f32, dy: f32| pos2(c.x + dx, c.y + dy);
        let press = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>, down: bool, p: Pos2| {
            h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
            h.run_steps(1);
        };
        h.event(egui::Event::PointerMoved(at(0.0, 0.0)));
        h.run_steps(1);
        press(&mut h, true, at(0.0, 0.0));
        // Within egui's click distance (6 px): no drag yet, but these are the stroke's points.
        // Then past it: the drag starts. (Not collinear: the lasso's polygon needs an area.)
        for (dx, dy) in [(2.0, 1.0), (4.0, 2.0), (30.0, 20.0), (60.0, -10.0), (90.0, 40.0)] {
            h.event(egui::Event::PointerMoved(at(dx, dy)));
            h.run_steps(1);
        }
        press(&mut h, false, at(90.0, 40.0));
        h.run_steps(2);
        let w = h.state().distort.caf.as_ref().unwrap();
        let points: Vec<[f64; 2]> = match tool {
            "brush" => {
                assert_eq!(w.strokes.len(), 1, "one stroke");
                w.strokes[0].points.clone()
            }
            _ => {
                assert_eq!(w.fill_edits.len(), 1, "one lasso edit");
                serde_json::from_value(w.fill_edits[0]["points"].clone()).unwrap()
            }
        };
        let centre = [f64::from(w.canvas.x0 + w.canvas.x1) / 2.0, f64::from(w.canvas.y0 + w.canvas.y1) / 2.0];
        assert_eq!(points.len(), 6, "{tool}: the press and every move are points: {points:?}");
        let d = ((points[0][0] - centre[0]).powi(2) + (points[0][1] - centre[1]).powi(2)).sqrt();
        assert!(d < 1.0, "{tool}: the first point is the press ({:?} vs {centre:?})", points[0]);
        assert!(w.cur.is_none() && w.lasso.is_none() && !w.pressed, "{tool}: the release ended it");
    }
}

/// The preview texture fits the device: a plan rectangle wider than the texture limit is
/// box-averaged to fit, and is still drawn over the plan's rectangle.
#[test]
fn the_preview_texture_respects_the_device_limit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 2200, "height": 40, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            s.fill_rect(Rect::new(0, 0, 2200, 40), &[0.2, 0.6, 0.4, 1.0]);
            s.fill_rect(Rect::new(1090, 10, 1110, 30), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 1090, "y": 10, "width": 20, "height": 20})).unwrap();
    app.sync_views();
    open(&mut app).unwrap();
    // Custom sampling with a stroke across the whole width: the plan's rectangle is 2200 px wide.
    control(&mut app, &json!({"sampling": "custom", "stroke": {"points": [[0.0, 5.0], [2200.0, 5.0]], "size": 8}})).unwrap();
    for (limit, want) in [(2048usize, 1100usize), (4096, 2200)] {
        let w = app.distort.caf.as_mut().unwrap();
        w.texture_limit = limit;
        w.stale();
        control(&mut app, &json!({"waitPreview": true})).unwrap();
        let (rect, img) = ws(&app).preview.pending_image.clone().unwrap();
        assert_eq!((rect.width(), img.size[0]), (2200, want), "limit {limit}");
        assert!(img.size.iter().all(|s| *s <= limit), "limit {limit}: {:?}", img.size);
        let (sum, n) = img.pixels.iter().fold((0u32, 0u32), |(s, n), p| (s + u32::from(p.g()), n + 1));
        assert!(sum / n > 100, "limit {limit}: the reduced preview keeps the image ({})", sum / n);
    }
}

/// A preview that fails stands for that state: nothing is shown, the error is reported, and
/// the same preview is not started again on every frame.
#[test]
fn a_failing_preview_is_reported_once_not_restarted() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    control(&mut app, &json!({"waitPreview": true})).unwrap();
    assert!(ws(&app).preview.pending_image.is_some());
    // The selection goes away under the workspace: there is nothing to fill.
    app.session.execute("select.deselect", json!({})).unwrap();
    app.distort.caf.as_mut().unwrap().stale();
    assert!(control(&mut app, &json!({"waitPreview": true})).is_err());
    let pv = &ws(&app).preview;
    assert_eq!(pv.shown, pv.want, "the failed state counts as shown: no retry");
    assert!(pv.pending_image.is_none() && pv.tex.is_none());
    let d = ws(&app).describe();
    assert_eq!(d["previewReady"], json!(false));
    assert!(d["previewError"].as_str().is_some_and(|e| e.contains("selection")), "{d}");
}

/// The view shows the document, not the layer alone: after Apply to a new layer the fill
/// (on that layer) covers the blemish.
#[test]
fn the_view_shows_the_document_after_an_apply_to_a_new_layer() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let red = |img: &egui::ColorImage| {
        let p = img.pixels[80 * img.size[0] + 100];
        p.r() > 200 && p.g() < 60
    };
    let before = view_image(&app).unwrap();
    assert_eq!(before.size, [200, 160]);
    assert!(red(&before), "the blemish shows before the fill");
    control(&mut app, &json!({"output": "new", "colorAdaptation": "none", "apply": true})).unwrap();
    assert!(!red(&view_image(&app).unwrap()), "the fill on the new layer shows");
}

/// The engine's limits (MAX_CAF_EDITS strokes or lasso edits, MAX_CAF_POINTS points in all)
/// hold at the brush: a stroke past them is refused with a message, so Apply and the
/// preview never fail on what was painted.
#[test]
fn strokes_past_the_engines_limits_are_refused_at_the_brush() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    control(&mut app, &json!({"sampling": "rectangular"})).unwrap();
    for i in 0..MAX_CAF_EDITS {
        let x = (i % 190) as f64;
        control(&mut app, &json!({"stroke": {"points": [[x, 5.0], [x + 3.0, 5.0]], "size": 2}})).unwrap();
    }
    let d = control(&mut app, &json!({"stroke": {"points": [[10.0, 50.0], [20.0, 50.0]], "size": 2}})).unwrap();
    assert_eq!(d["strokes"], json!(MAX_CAF_EDITS));
    assert!(d["error"].as_str().is_some_and(|e| e.contains("at most")), "{d}");
    control(&mut app, &json!({"waitPreview": true})).unwrap();
    // The points budget: a long stroke stops at it.
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let pts: Vec<[f64; 2]> = (0..MAX_CAF_POINTS + 100).map(|i| [10.0 + i as f64 * 0.0005, 5.0]).collect();
    let d = control(&mut app, &json!({"stroke": {"points": pts, "size": 2}})).unwrap();
    assert_eq!(ws(&app).points_used(), MAX_CAF_POINTS);
    assert!(d["error"].as_str().is_some_and(|e| e.contains("at most")), "{d}");
    control(&mut app, &json!({"waitPreview": true})).unwrap();
    // A lasso edit past the points budget is refused too.
    let d = control(&mut app, &json!({"lasso": {"op": "add", "points": [[130.0, 70.0], [150.0, 70.0], [150.0, 90.0]]}})).unwrap();
    assert_eq!(d["fillEdits"], json!(0));
    assert!(d["error"].as_str().is_some_and(|e| e.contains("at most")), "{d}");
}

fn press(app: &mut PhotocraftApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
    let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let input = egui::RawInput { events: vec![ev], ..Default::default() };
    ctx.run_ui(input, |ui| keys(app, ui.ctx())).textures_delta.clear();
}

/// With the overlay colour's Color Picker open over the workspace, its keys (brush size, undo)
/// belong to the picker, not to the workspace under it.
#[test]
fn keys_wait_while_the_overlay_colour_picker_is_open() {
    use egui::{Key, Modifiers};
    let mut app = app_with_selection();
    let ctx = egui::Context::default();
    open(&mut app).unwrap();
    control(&mut app, &json!({"sampling": "rectangular"})).unwrap();
    let all = ws(&app).describe()["sampledProxyPixels"].as_u64().unwrap();
    let less = control(&mut app, &json!({"stroke": {"points": [[20.0, 20.0], [80.0, 20.0]], "size": 10}})).unwrap()["sampledProxyPixels"].as_u64().unwrap();
    assert_ne!(less, all);
    let size = ws(&app).brush_size;
    crate::color_picker_ui::request_for_widget(&ctx, egui::Id::unique("caf-overlay-swatch"), "Color Picker (Overlay Color)", [0.0, 1.0, 0.0]);
    crate::color_picker_ui::widget_tick(&mut app, &ctx);
    assert!(crate::color_picker_ui::any_open(&app));
    press(&mut app, &ctx, Key::OpenBracket, Modifiers::NONE);
    assert_eq!(ws(&app).brush_size, size, "[ went to the picker");
    press(&mut app, &ctx, Key::Z, Modifiers::COMMAND);
    assert_eq!(ws(&app).describe()["sampledProxyPixels"].as_u64(), Some(less), "Ctrl+Z went to the picker");
    app.ui.dialogs.clear();
    press(&mut app, &ctx, Key::OpenBracket, Modifiers::NONE);
    assert!(ws(&app).brush_size < size);
    press(&mut app, &ctx, Key::Z, Modifiers::COMMAND);
    assert_eq!(ws(&app).describe()["sampledProxyPixels"].as_u64(), Some(all));
}

/// Picking Custom before anything is painted is how Photoshop starts it: the preview shows the
/// image unfilled and the panel a hint to paint with the Sampling Brush, not the fill's
/// "custom sampling needs `area`…" refusal (#10621). The first added stroke previews the fill.
#[test]
fn custom_sampling_with_nothing_painted_hints_instead_of_failing() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let d = control(&mut app, &json!({"sampling": "custom", "waitPreview": true})).unwrap();
    assert!(d["previewError"].is_null(), "{d}");
    assert!(d["error"].is_null(), "{d}");
    assert_eq!(d["previewHint"], json!(CUSTOM_HINT), "{d}");
    assert!(ws(&app).preview.pending_image.is_none() && ws(&app).preview.tex.is_none(), "the image is shown unfilled");
    // A subtracting stroke paints nothing in an empty area: still the hint.
    let d = control(&mut app, &json!({"stroke": {"points": [[20.0, 120.0]], "size": 8, "alt": true}, "waitPreview": true})).unwrap();
    assert!(d["previewError"].is_null() && d["previewHint"] == json!(CUSTOM_HINT), "{d}");
    let d = control(&mut app, &json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8}, "waitPreview": true})).unwrap();
    assert!(d["previewHint"].is_null(), "{d}");
    assert!(d["previewError"].is_null(), "{d}");
    assert!(ws(&app).preview.pending_image.is_some(), "the fill previews");
}

/// Erasing everything the Sampling Brush added in Custom leaves the area as empty as before
/// anything was painted: the same hint, the image unfilled, and Apply and OK say so too instead of
/// running the fill into "nothing to sample from" (or, with nothing painted at all, the fill's
/// "custom sampling needs `area`…" refusal).
#[test]
fn custom_sampling_erased_again_hints_and_apply_and_ok_do_not_run_the_fill() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    control(&mut app, &json!({"sampling": "custom"})).unwrap();
    assert_eq!(control(&mut app, &json!({"apply": true})).unwrap_err(), CUSTOM_HINT, "Apply with nothing painted");
    control(&mut app, &json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8}})).unwrap();
    let d = control(&mut app, &json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8, "alt": true}, "waitPreview": true})).unwrap();
    assert_eq!(d["sampledProxyPixels"].as_u64(), Some(0), "{d}");
    assert!(d["previewError"].is_null(), "{d}");
    assert_eq!(d["previewHint"], json!(CUSTOM_HINT), "{d}");
    assert!(ws(&app).preview.pending_image.is_none() && ws(&app).preview.tex.is_none(), "the image is shown unfilled");
    assert_eq!(control(&mut app, &json!({"apply": true})).unwrap_err(), CUSTOM_HINT, "Apply");
    assert_eq!(control(&mut app, &json!({"commit": true})).unwrap_err(), CUSTOM_HINT, "OK");
    commit(&mut app);
    assert!(app.distort.caf.is_some(), "OK keeps the workspace open");
    assert!(ws(&app).error.is_none() && ws(&app).awaiting_custom_area(), "the panel keeps the hint");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "nothing was filled");
    // Painting again previews the fill.
    let d = control(&mut app, &json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8}, "waitPreview": true})).unwrap();
    assert!(d["previewHint"].is_null() && d["previewError"].is_null() && d["error"].is_null(), "{d}");
    assert!(ws(&app).preview.pending_image.is_some(), "the fill previews");
}

/// Apply and OK with nothing to sample in Custom leave the panel's hint as it is, in the hint's
/// style, and the status bar says the same in the UI's language: not the English hint as an
/// error, in the error colour, that replaces it in the panel (#10621). An earlier Apply's error
/// gives way to the hint too.
#[test]
fn custom_apply_and_ok_with_nothing_painted_show_the_translated_hint_not_an_error() {
    let ja = crate::i18n::Lang::from_code("ja").unwrap();
    let hint = crate::i18n::tr(ja, CUSTOM_HINT);
    assert_ne!(hint, CUSTOM_HINT, "the catalog translates the hint");
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    control(&mut app, &json!({"sampling": "custom"})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    for ok in [false, true] {
        app.distort.caf.as_mut().unwrap().error = Some("an earlier Apply's error".into());
        crate::i18n::with_language(ja, || {
            if ok {
                commit(&mut app);
            } else if let Err(e) = apply(&mut app) {
                lifecycle::report(&mut app, e);
            }
        });
        let what = if ok { "OK" } else { "Apply" };
        assert!(app.distort.caf.is_some(), "{what} keeps the workspace open");
        assert!(ws(&app).error.is_none() && ws(&app).awaiting_custom_area(), "{what}: the panel shows the hint");
        assert_eq!(app.ui.status, hint, "{what}");
        assert!(!app.ui.status_error, "{what}: not an error");
    }
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "nothing was filled");
}

/// Custom's hint follows the fill area as well as the strokes: a lasso or Expand that covers
/// everything painted brings it back, and one that uncovers a stroke (Contract, or undoing the
/// lasso) takes it away, without anything being painted again.
#[test]
fn custom_hint_follows_fill_area_changes_under_the_strokes() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    control(&mut app, &json!({"sampling": "custom"})).unwrap();
    let hint = |d: &Value| d["previewHint"] == json!(CUSTOM_HINT);
    // A stroke inside the fill area (90..110) samples nothing; Contract uncovers it.
    let d = control(&mut app, &json!({"stroke": {"points": [[92.0, 76.0], [92.0, 84.0]], "size": 2}})).unwrap();
    assert!(hint(&d), "{d}");
    let d = control(&mut app, &json!({"contract": 5})).unwrap();
    assert!(!hint(&d) && d["error"].is_null(), "contracted off the stroke: {d}");
    let d = control(&mut app, &json!({"undo": true})).unwrap();
    assert!(hint(&d), "the fill area covers the stroke again: {d}");
    // A stroke just right of it samples; a lasso over it covers it, Expand too.
    let d = control(&mut app, &json!({"undo": true})).unwrap();
    assert_eq!(d["strokes"].as_u64(), Some(0), "{d}");
    let d = control(&mut app, &json!({"stroke": {"points": [[112.0, 76.0], [112.0, 84.0]], "size": 2}})).unwrap();
    assert!(!hint(&d), "{d}");
    let d = control(&mut app, &json!({"lasso": {"op": "add", "points": [[105.0, 70.0], [125.0, 70.0], [125.0, 90.0], [105.0, 90.0]]}})).unwrap();
    assert!(hint(&d) && d["error"].is_null(), "the lasso covers the stroke: {d}");
    assert_eq!(d["strokes"].as_u64(), Some(1), "the stroke is kept: {d}");
    let d = control(&mut app, &json!({"undo": true})).unwrap();
    assert!(!hint(&d), "undoing the lasso uncovers it: {d}");
    let d = control(&mut app, &json!({"expand": 4})).unwrap();
    assert!(hint(&d) && d["error"].is_null(), "Expand covers the stroke: {d}");
    assert_eq!(control(&mut app, &json!({"apply": true})).unwrap_err(), CUSTOM_HINT, "Apply");
}

/// Picking Custom shows the image unfilled straight away, even while the preview of the
/// previous state is still winding down: its fill (or error) isn't left on view until the
/// superseded worker exits.
#[test]
fn picking_custom_clears_the_preview_while_a_superseded_one_still_runs() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    control(&mut app, &json!({"waitPreview": true})).unwrap();
    assert!(ws(&app).preview.pending_image.is_some());
    // A worker for that state that hasn't answered yet.
    let (_tx, rx) = std::sync::mpsc::sync_channel(1);
    let w = app.distort.caf.as_mut().unwrap();
    w.preview.running = Some((w.preview.want, rx, photocraft_engine::jobs::JobCtx::new()));
    control(&mut app, &json!({"sampling": "custom"})).unwrap();
    preview::drive_preview(&mut app, &egui::Context::default(), false);
    let pv = &ws(&app).preview;
    assert!(pv.pending_image.is_none() && pv.tex.is_none() && pv.error.is_none(), "the image is shown unfilled");
    assert_eq!(pv.shown, pv.want);
    assert!(pv.running.is_some(), "the superseded worker is still winding down");
}

/// Custom's "nothing painted yet" follows what the fill samples, the full-resolution area,
/// not the proxy (2 document px per proxy px here): a 1 px stroke on the fill area's left edge
/// keeps a column outside it that the proxy, sampling every other column, doesn't see, and one
/// just past the canvas's right edge marks the proxy's last column (half of it off the canvas)
/// while the fill has nothing there.
#[test]
fn custom_hint_follows_the_full_resolution_area_not_the_proxy() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 2201, "height": 40, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            s.fill_rect(Rect::new(0, 0, 2201, 40), &[0.2, 0.6, 0.4, 1.0]);
            s.fill_rect(Rect::new(1091, 10, 1111, 30), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 1091, "y": 10, "width": 20, "height": 20})).unwrap();
    app.sync_views();
    open(&mut app).unwrap();
    assert_eq!(ws(&app).k, 2);
    control(&mut app, &json!({"sampling": "custom"})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    // Column 1090 is sampled; the proxy's column 545 (document column 1091) is in the fill area.
    let d = control(&mut app, &json!({"stroke": {"points": [[1091.0, 15.0], [1091.0, 25.0]], "size": 1}})).unwrap();
    assert_eq!(d["sampledProxyPixels"].as_u64(), Some(0), "{d}");
    assert!(d["previewHint"].is_null(), "{d}");
    let (_, source, hole) = engine_source(&app);
    assert!(source.iter().zip(&hole).any(|(s, h)| *s && !*h), "the fill samples column 1090");
    control(&mut app, &json!({"apply": true})).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "Apply fills");
    // Past the canvas: the proxy's column 1100 (document column 2201) is marked, the fill
    // samples nothing.
    control(&mut app, &json!({"undo": true})).unwrap();
    let d = control(&mut app, &json!({"stroke": {"points": [[2202.0, 5.0], [2202.0, 35.0]], "size": 1}})).unwrap();
    assert!(d["sampledProxyPixels"].as_u64().unwrap() > 0, "{d}");
    assert_eq!(d["previewHint"], json!(CUSTOM_HINT), "{d}");
    assert_eq!(control(&mut app, &json!({"apply": true})).unwrap_err(), CUSTOM_HINT, "Apply");
}

/// A big Sampling Brush stroke kept inside the fill area can't sample anything, so working that
/// out paints nothing: each segment of it would otherwise paint its whole brush (here 100 px
/// across, up to 25 million px at the largest size) at full resolution, again and again while the
/// area stays empty. One that reaches past the fill area paints only the pixels outside it.
#[test]
fn custom_hint_paints_nothing_for_a_stroke_inside_the_fill_area() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 300, "height": 200, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 300, 200), &[0.2, 0.6, 0.4, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 50, "y": 40, "width": 200, "height": 120})).unwrap();
    app.sync_views();
    open(&mut app).unwrap();
    control(&mut app, &json!({"sampling": "custom"})).unwrap();
    let inside = SamplingStroke { add: true, size: 100.0, points: vec![[120.0, 100.0], [180.0, 100.0]] };
    assert!(!inside.bounds().is_empty() && Rect::new(50, 40, 250, 160).contains_rect(&inside.bounds()));
    assert_eq!(ws(&app).sampling_reach(std::slice::from_ref(&inside)), Rect::EMPTY);
    let d = control(&mut app, &json!({"stroke": {"points": [[120.0, 100.0], [180.0, 100.0]], "size": 100}})).unwrap();
    assert_eq!(d["previewHint"], json!(CUSTOM_HINT), "{d}");
    // 10 px past the fill area's right edge: only those columns.
    let past = SamplingStroke { add: true, size: 100.0, points: vec![[160.0, 100.0], [210.0, 100.0]] };
    assert_eq!(ws(&app).sampling_reach(std::slice::from_ref(&past)), Rect::new(250, 50, 261, 151));
    let d = control(&mut app, &json!({"stroke": {"points": [[160.0, 100.0], [210.0, 100.0]], "size": 100}})).unwrap();
    assert!(d["previewHint"].is_null(), "{d}");
}

/// The pixels outside the fill area are found pixel by pixel, not from its bounds: a gap in it
/// is found, and anything past the fill area's rectangle is outside it.
#[test]
fn outside_fill_bounds_follow_the_fill_area_pixel_by_pixel() {
    let rect = Rect::new(10, 10, 20, 20);
    let mut hole = vec![true; 100];
    let b = |hole: &Vec<bool>| preview::Base::new(rect, rect, hole.clone(), vec![false; 100]);
    assert_eq!(b(&hole).outside_fill_bounds(Rect::new(12, 12, 18, 18)), Rect::EMPTY);
    assert_eq!(b(&hole).outside_fill_bounds(rect), Rect::EMPTY);
    hole[3 * 10 + 6] = false; // (16, 13)
    hole[7 * 10 + 4] = false; // (14, 17)
    assert_eq!(b(&hole).outside_fill_bounds(Rect::new(12, 12, 18, 18)), Rect::new(14, 13, 17, 18));
    assert_eq!(b(&hole).outside_fill_bounds(Rect::new(10, 10, 16, 16)), Rect::EMPTY);
    assert_eq!(b(&hole).outside_fill_bounds(Rect::new(15, 5, 18, 12)), Rect::new(15, 5, 18, 10));
    assert_eq!(b(&hole).outside_fill_bounds(Rect::new(0, 0, 5, 5)), Rect::new(0, 0, 5, 5));
    assert_eq!(b(&hole).outside_fill_bounds(Rect::new(18, 16, 25, 18)), Rect::new(20, 16, 25, 18));
}

/// The pixels outside the fill area are looked up row by row in an index built with the base,
/// not scanned under each Sampling Brush segment: a big brush over a fill area in the middle of
/// the window (rows with pixels outside it on both sides) would otherwise check every pixel it
/// covers, again and again while Custom's area stays empty. With the pixels dropped once the
/// index is built (a missing pixel counts as outside), the answers stay the same.
#[test]
fn outside_fill_bounds_look_rows_up_instead_of_scanning_pixels() {
    let rect = Rect::new(0, 0, 1000, 1000);
    let mut hole = vec![false; 1000 * 1000];
    for y in 100..900 {
        hole[y * 1000 + 100..y * 1000 + 900].fill(true);
    }
    hole[300 * 1000 + 500] = false; // (500, 300)
    let mut b = preview::Base::new(rect, rect, hole, vec![false; 1000 * 1000]);
    b.hole = Vec::new();
    assert_eq!(b.outside_fill_bounds(Rect::new(200, 400, 800, 800)), Rect::EMPTY, "all inside");
    assert_eq!(b.outside_fill_bounds(Rect::new(200, 200, 800, 800)), Rect::new(500, 300, 501, 301), "the gap");
    assert_eq!(b.outside_fill_bounds(Rect::new(50, 400, 150, 450)), Rect::new(50, 400, 100, 450), "left of it");
    assert_eq!(b.outside_fill_bounds(Rect::new(850, 880, 1100, 900)), Rect::new(900, 880, 1100, 900), "right of it and past the window");
}
