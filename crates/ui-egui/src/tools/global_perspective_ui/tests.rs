//! The Global Perspective Warp guide editor: the session's own rules (guides, axes, diagnostics, commit), then the real
//! canvas pass (pointer capture, stale and foreign sessions, keys, several windows).

use egui::Modifiers;

use super::*;
use crate::canvas::testing::{self, ToolEvent};
use crate::state::Tool;

/// Document points per hit radius used by the session tests that call the pointer functions directly (zoom 1).
const TOL: f64 = HIT_POINTS;

fn fixture(a: &mut PhotocraftApp) {
    a.session.execute("layer.new.layer", json!({})).unwrap();
    a.session
        .edit("fixture", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 160, 140), &[0.2, 0.5, 0.8, 1.0]);
            Ok(())
        })
        .unwrap();
}

fn closed() -> Box<PhotocraftApp> {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    a.session.execute("file.new", json!({"width":160,"height":140,"depth":8})).unwrap();
    fixture(&mut a);
    a.sync_views();
    a
}

fn app() -> Box<PhotocraftApp> {
    let mut a = closed();
    let ctx = a.ctx().clone();
    begin(&mut a, &ctx).unwrap();
    a
}

fn guides() -> Value {
    json!([{"id":1,"corners":[[25,20],[100,16],[109,105],[21,110]]}])
}

fn session(a: &PhotocraftApp) -> &GlobalSession {
    a.distort.global_perspective.as_ref().unwrap()
}

fn corners(a: &PhotocraftApp) -> [[f64; 2]; 4] {
    session(a).guides[0].corners
}

fn surface_of(a: &PhotocraftApp) -> photocraft_raster::Surface {
    let st = a.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone()
}

fn past(a: &PhotocraftApp) -> usize {
    a.session.active().unwrap().history.past_len()
}

fn none() -> Keys {
    Keys::default()
}
fn ctrl() -> Keys {
    Keys { ctrl: true, shift: false }
}
fn shift() -> Keys {
    Keys { ctrl: false, shift: true }
}
fn both() -> Keys {
    Keys { ctrl: true, shift: true }
}

fn down(a: &mut PhotocraftApp, p: [f64; 2], keys: Keys) -> bool {
    pointer_down(a, p, TOL, keys)
}

fn view() -> CanvasView {
    CanvasView::new(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0)), Point::new(80.0, 70.0), 1.0)
}

// ------------------------------------------------------------------------------------------ control and commit

#[test]
fn ai_solve_preview_commit_share_the_exact_matrix() {
    let mut a = app();
    let n = past(&a);
    let r = control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    assert_eq!(r["solved"], true);
    let out = commit(&mut a).unwrap();
    assert_eq!(out["committed"], true);
    assert_eq!(r["solution"]["matrix"], out["result"]["solution"]["matrix"]);
    assert_eq!(past(&a), n + 1, "one engine command, one history step");
    assert!(a.distort.global_perspective.is_none());
}

#[test]
fn commit_is_one_undo_step_that_redo_restores() {
    let mut a = app();
    let before = surface_of(&a);
    control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    commit(&mut a).unwrap();
    let after = surface_of(&a);
    assert_ne!(before, after);
    a.session.undo();
    assert_eq!(surface_of(&a), before);
    a.session.redo();
    assert_eq!(surface_of(&a), after);
}

#[test]
fn malformed_controls_leave_preview_unchanged() {
    let mut a = app();
    let before = control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    for p in [
        json!({"typo":true}),
        json!({"solve":"yes"}),
        json!({"mode":"warp"}),
        json!({"selectedGuide":999}),
        json!({"options":{"aspect":0}}),
        json!({"commit":true,"guides":[]}),
    ] {
        assert!(control(&mut a, &p).is_err(), "{p}");
        assert_eq!(session(&a).describe(), before, "{p}");
    }
}

#[test]
fn changed_guides_invalidate_solution_and_forbid_stale_commit() {
    let mut a = app();
    control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    control(&mut a, &json!({"guides":guides()})).unwrap();
    let n = past(&a);
    assert!(commit(&mut a).is_err());
    assert!(a.distort.global_perspective.is_some(), "a refused commit keeps the session");
    assert_eq!(past(&a), n);
}

#[test]
fn a_commit_after_the_source_changed_runs_no_command() {
    let mut a = app();
    control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    a.session
        .edit("lock", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().locks.pixels = true;
            Ok(())
        })
        .unwrap();
    // The lock replaced the snapshot: the session is stale, and a commit must not run the command at all.
    let n = past(&a);
    assert!(commit(&mut a).is_err());
    assert!(a.distort.global_perspective.is_some());
    assert_eq!(past(&a), n);
}

#[test]
fn cross_document_and_source_edits_are_rejected() {
    let mut a = app();
    control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    a.session.execute("file.new", json!({"width":20,"height":20})).unwrap();
    assert!(commit(&mut a).is_err());
    assert!(a.distort.global_perspective.is_some());
    let mut a = app();
    control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    a.session.execute("layer.new.layer", json!({})).unwrap();
    assert!(commit(&mut a).is_err());
}

#[test]
fn cancellation_preserves_pixels_and_history() {
    let mut a = app();
    let before = a.session.active().unwrap().doc.clone();
    let n = past(&a);
    control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    control(&mut a, &json!({"cancel":true})).unwrap();
    assert!(a.distort.global_perspective.is_none());
    assert!(Arc::ptr_eq(&before, &a.session.active().unwrap().doc));
    assert_eq!(past(&a), n);
}

#[test]
fn selecting_a_guide_preserves_solved_geometry() {
    let mut a = app();
    let before = control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    let after = control(&mut a, &json!({"selectedGuide":1})).unwrap();
    assert_eq!(before["solution"], after["solution"]);
    assert_eq!(after["dirty"], false);
}

#[test]
fn diagnostics_report_the_solver_without_touching_the_document() {
    let mut a = app();
    let doc = a.session.active().unwrap().doc.clone();
    let n = past(&a);
    let r = control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    assert_eq!(r["mode"], "result");
    assert_eq!(r["scope"], "wholeRasterLayer");
    assert!(r["solution"]["matrix"].as_array().is_some_and(|m| m.len() == 9));
    assert!(session(&a).solution.as_ref().unwrap().used_guide_ids.contains(&1));
    assert!(a.ui.status.contains("deg"), "{}", a.ui.status);
    // Back to the guides: the solution stays, nothing was written.
    let back = control(&mut a, &json!({"mode":"layout"})).unwrap();
    assert_eq!(back["mode"], "layout");
    assert_eq!(back["solved"], true);
    assert!(Arc::ptr_eq(&doc, &a.session.active().unwrap().doc));
    assert_eq!(past(&a), n);
}

#[test]
fn a_disabled_guide_is_not_used_and_stays_in_source_space() {
    let mut a = app();
    let mut list = guides().as_array().unwrap().clone();
    list.push(json!({"id":99,"enabled":false,"corners":[[-60,10],[-45,10],[-45,30],[-60,30]]}));
    control(&mut a, &json!({"guides":list,"solve":true})).unwrap();
    let s = a.distort.global_perspective.as_mut().unwrap();
    let report = s.solution.as_ref().unwrap().guides.iter().find(|r| r.id == 99).unwrap();
    assert!(!report.used && report.transformed_corners.is_none());
    // The image-domain homography may have an off-image pole at x=-60: an unused guide never evaluates it.
    s.solution.as_mut().unwrap().matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0 / 60.0, 0.0, 1.0];
    let unused = s.guides.iter().find(|g| g.id == 99).unwrap();
    assert_eq!(s.display_corners(unused), Some(unused.corners));
    assert!(matches!(hit_handle(s, [-60.0, 10.0], 1.0), Some(Handle::Corner { id: 99, .. })));
    let used = s.guides.iter().find(|g| g.id == 1).unwrap();
    let validated = s.solution.as_ref().unwrap().guides.iter().find(|g| g.id == 1).unwrap().transformed_corners;
    assert_eq!(s.display_corners(used), validated);
    s.show_result = false;
    assert_eq!(s.display_corners(used), Some(used.corners));
}

#[test]
fn menu_route_opens_and_controls_the_tool() {
    let mut a = app();
    cancel(&mut a);
    let ctx = a.ctx().clone();
    crate::distort_ui::menu(&mut a, &ctx, COMMAND, &json!({})).unwrap().unwrap();
    assert!(a.distort.global_perspective.is_some());
    let r = crate::distort_ui::menu(&mut a, &ctx, COMMAND, &json!({"ui":{"guides":guides(),"solve":true}})).unwrap().unwrap();
    assert_eq!(r["solved"], true);
    // Other top-level parameters are the headless command's, not the editor's.
    let error = crate::distort_ui::menu(&mut a, &ctx, COMMAND, &json!({"guides":guides()})).unwrap().unwrap_err();
    assert!(error.contains("engine.execute"), "{error}");
}

#[test]
fn the_edit_menu_lists_the_editor_without_joining_the_photoshop_catalogue() {
    let a = closed();
    let rows = crate::menus::base_rows(&a);
    let row = crate::menus::row(&rows, COMMAND).expect("Edit lists Global Perspective Warp");
    assert_eq!(row.path, ["Edit"]);
    assert_eq!(row.label, MENU_LABEL);
    assert!(crate::menus::is_live(COMMAND));
    assert!(!crate::menu_catalog::CATALOG.iter().any(|entry| entry.3 == COMMAND));
    let at = |id: &str| rows.iter().position(|r| r.command.id == id);
    assert_eq!(at(COMMAND), at("edit.perspectiveWarp").map(|i| i + 1), "next to the local Perspective Warp");
}

#[test]
fn repeated_begin_does_not_replace_an_editable_session() {
    let mut a = app();
    let before = control(&mut a, &json!({"guides":guides()})).unwrap();
    let ctx = a.ctx().clone();
    assert!(begin(&mut a, &ctx).is_err());
    assert_eq!(session(&a).describe(), before);
}

// ------------------------------------------------------------------------------------------ opening guards

#[test]
fn unsupported_layer_compositing_is_rejected_without_mutating_pixels_or_history() {
    for case in 0..6 {
        let mut a = app();
        cancel(&mut a);
        a.session
            .edit("unsupported preview metadata", |doc, active| {
                let layer = doc.layer_mut(active.unwrap()).unwrap();
                match case {
                    0 => layer.blend = photocraft_doc::BlendMode::Multiply,
                    1 => layer.clipped = true,
                    2 => layer.blend_if.set(0, [photocraft_doc::BlendRange { black: [40, 40], white: [255, 255] }, photocraft_doc::BlendRange::FULL]),
                    3 => layer.advanced.knockout = photocraft_doc::Knockout::Deep,
                    4 => layer.excluded_channels = 1,
                    _ => layer.visible = false,
                }
                Ok(())
            })
            .unwrap();
        let doc = a.session.active().unwrap().doc.clone();
        let n = past(&a);
        let ctx = a.ctx().clone();
        let error = begin(&mut a, &ctx).unwrap_err();
        assert!(error.contains("preview"), "case {case}: {error}");
        assert!(a.distort.global_perspective.is_none());
        assert!(Arc::ptr_eq(&doc, &a.session.active().unwrap().doc));
        assert_eq!(past(&a), n);
    }
}

#[test]
fn foreground_and_nested_layers_cannot_be_painted_over_by_a_raw_preview_mesh() {
    for nested in [false, true] {
        let mut a = app();
        cancel(&mut a);
        a.session
            .edit("foreground or nested fixture", |doc, active| {
                if nested {
                    let index = doc.layers.iter().position(|l| l.id == active.unwrap()).unwrap();
                    let target = doc.layers.remove(index);
                    doc.layers.push(photocraft_doc::Layer::group("container", vec![target]));
                } else {
                    let format = doc.pixel_format();
                    doc.layers.push(photocraft_doc::Layer::raster("foreground", format));
                }
                Ok(())
            })
            .unwrap();
        let ctx = a.ctx().clone();
        let error = begin(&mut a, &ctx).unwrap_err();
        assert!(error.contains("topmost"), "{error}");
        assert!(a.distort.global_perspective.is_none());
    }
}

#[test]
fn selection_locks_and_empty_layers_refuse_before_a_session_opens() {
    let mut a = closed();
    a.session.execute("select.all", json!({})).unwrap();
    let ctx = a.ctx().clone();
    assert!(begin(&mut a, &ctx).unwrap_err().contains("deselect"));
    a.session.execute("select.deselect", json!({})).unwrap();
    a.session
        .edit("lock", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().locks.pixels = true;
            Ok(())
        })
        .unwrap();
    assert!(begin(&mut a, &ctx).unwrap_err().contains("locked"));
    assert!(a.distort.global_perspective.is_none());
}

#[test]
fn active_effects_cannot_open_a_misleading_raster_preview() {
    let mut a = app();
    cancel(&mut a);
    a.session
        .edit("styled fixture", |doc, active| {
            let layer = doc.layer_mut(active.unwrap()).unwrap();
            layer.fill_opacity = 0.0;
            layer.effects.items.push(photocraft_doc::Effect::default_drop_shadow());
            Ok(())
        })
        .unwrap();
    let old = a.session.active().unwrap().doc.clone();
    let n = past(&a);
    let ctx = a.ctx().clone();
    let error = begin(&mut a, &ctx).unwrap_err();
    assert!(error.contains("layer effects"), "{error}");
    assert!(a.distort.global_perspective.is_none());
    assert!(Arc::ptr_eq(&old, &a.session.active().unwrap().doc));
    assert_eq!(past(&a), n);
    a.session
        .edit("disable style", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().effects.enabled = false;
            Ok(())
        })
        .unwrap();
    begin(&mut a, &ctx).unwrap();
    assert!(a.distort.global_perspective.is_some());
}

#[test]
fn raw_warp_preview_rejects_hdr_and_color_transformed_documents() {
    let mut hdr = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    hdr.session.execute("file.new", json!({"width":32,"height":24,"depth":32})).unwrap();
    hdr.session.execute("layer.new.layer", json!({})).unwrap();
    hdr.session
        .edit("HDR fixture", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 24), &[1.5, 0.5, 0.2, 1.0]);
            Ok(())
        })
        .unwrap();
    hdr.sync_views();
    let ctx = hdr.ctx().clone();
    assert!(begin(&mut hdr, &ctx).unwrap_err().contains("8-bit RGB"));
    assert!(hdr.distort.global_perspective.is_none());

    // Proof Colors put a transform between the document and the screen that a raw mesh cannot reproduce.
    let mut a = closed();
    a.run("view.proofColors", json!({"on": true})).unwrap();
    let ctx = a.ctx().clone();
    assert!(begin(&mut a, &ctx).unwrap_err().contains("color transform"));
}

#[test]
fn changing_the_color_display_while_editing_invalidates_the_preview_but_keeps_cancel() {
    let mut a = app();
    let idx = a.session.active_index().unwrap();
    assert!(bound_to_active_document(&a, session(&a)));
    assert!(owns_pointer(&a, idx));
    a.run("view.proofColors", json!({"on": true})).unwrap();
    assert!(!bound_to_active_document(&a, session(&a)));
    assert!(!owns_pointer(&a, idx), "a stale session must not take the pointer");
    assert!(crate::distort_ui::display_doc(&a, idx).is_none());
    assert!(commit(&mut a).is_err());
    assert_eq!(control(&mut a, &json!({"cancel":true})).unwrap()["cancelled"], true);
}

#[test]
fn competing_transformations_cannot_replace_the_editor() {
    let mut a = app();
    let original = a.session.active().unwrap().doc.clone();
    let n = past(&a);
    let key = session(&a).key;
    let ctx = a.ctx().clone();
    for id in ["edit.puppetWarp", "layer.smartObjects.puppetWarp", "edit.perspectiveWarp", "layer.smartObjects.perspectiveWarp"] {
        let error = crate::distort_ui::menu(&mut a, &ctx, id, &json!({})).unwrap().unwrap_err();
        assert!(error.contains("Global Perspective Warp"), "{id}: {error}");
    }
    for id in [
        "edit.freeTransform",
        "edit.freeTransformCopy",
        "layer.smartObjects.warp",
        "edit.puppetWarp",
        "filter.liquify",
        "filter.filterGallery",
        "filter.vanishingPoint",
        "file.placeEmbedded",
        "edit.transform.rotate",
    ] {
        let error = crate::menus::invoke(&mut a, &ctx, id, json!({})).unwrap_err();
        assert!(error.contains("Global Perspective Warp"), "{id}: {error}");
    }
    for start in [crate::transform_tool::begin, crate::transform_tool::begin_copy, crate::transform_tool::begin_warp] {
        assert!(start(&mut a, &ctx).unwrap_err().contains("Global Perspective Warp"));
    }
    assert!(a.ui.transform.is_none() && a.distort.puppet.is_none() && a.distort.perspective.is_none());
    assert_eq!(session(&a).key, key);
    assert!(Arc::ptr_eq(&original, &a.session.active().unwrap().doc));
    assert_eq!(past(&a), n);
    // The session itself, once closed, no longer blocks them.
    cancel(&mut a);
    assert!(refuses(&a, "edit.freeTransform").is_none());
}

#[test]
fn another_session_blocks_opening_the_editor() {
    let mut a = closed();
    let ctx = a.ctx().clone();
    crate::distort_ui::menu(&mut a, &ctx, "edit.perspectiveWarp", &json!({})).unwrap().unwrap();
    assert!(begin(&mut a, &ctx).unwrap_err().contains("finish or cancel"));
    assert!(a.distort.global_perspective.is_none());
}

#[test]
fn pattern_preview_is_refused_without_resetting_either_mode() {
    let mut a = app();
    let ctx = a.ctx().clone();
    let idx = a.session.active_index().unwrap();
    let before = control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    let n = past(&a);
    assert!(!crate::menus::is_enabled(&a, "view.patternPreview"));
    assert!(crate::menus::invoke(&mut a, &ctx, "view.patternPreview", json!({"on":true})).is_err());
    assert!(crate::view_cmds::invoke(&mut a, &ctx, "view.patternPreview", &json!({"on":true})).unwrap().is_err(), "direct shell dispatch also refuses");
    assert!(!a.ui.view.pattern_preview);
    assert_eq!(session(&a).describe(), before);
    // Even a direct state write must not tile a composite with the source layer missing.
    a.ui.view.pattern_preview = true;
    assert!(canvas_shapes(&mut a, &ctx, &view(), idx, None).is_empty());
    assert!(crate::distort_ui::display_doc(&a, idx).is_none());
    assert_eq!(session(&a).describe(), before);
    assert_eq!(past(&a), n);
    cancel(&mut a);
    assert!(a.ui.view.pattern_preview, "cancel does not reset the user's view preference");
    assert!(begin(&mut a, &ctx).is_err(), "an already repeating view cannot open the editor");
    assert!(crate::menus::is_enabled(&a, "view.patternPreview"));
    crate::view_cmds::invoke(&mut a, &ctx, "view.patternPreview", &json!({"on":false})).unwrap().unwrap();
    assert!(!a.ui.view.pattern_preview);
    crate::view_cmds::invoke(&mut a, &ctx, "view.patternPreview", &json!({"on":true})).unwrap().unwrap();
    assert!(a.ui.view.pattern_preview);
}

#[test]
fn nearest_corner_wins_over_overlapping_hit_circles_and_edges() {
    let mut a = app();
    control(
        &mut a,
        &json!({"guides":[
        {"id":1,"corners":[[20,20],[90,20],[90,90],[20,90]]},
        {"id":2,"corners":[[24,20],[94,20],[94,90],[24,90]]}
    ],"selectedGuide":1}),
    )
    .unwrap();
    assert_eq!(hit_handle(session(&a), [20.0, 20.0], TOL), Some(Handle::Corner { id: 1, corner: 0 }));
    assert_eq!(hit_handle(session(&a), [24.0, 20.0], TOL), Some(Handle::Corner { id: 2, corner: 0 }));
    assert_eq!(hit_handle(session(&a), [22.0, 20.0], TOL), Some(Handle::Corner { id: 1, corner: 0 }));
    a.distort.global_perspective.as_mut().unwrap().selected = None;
    assert_eq!(hit_handle(session(&a), [22.0, 20.0], TOL), Some(Handle::Corner { id: 2, corner: 0 }));
    assert!(down(&mut a, [20.0, 20.0], none()));
    pointer_up(&mut a, [17.0, 17.0], none());
    assert_eq!(corners(&a)[0], [17.0, 17.0]);
    assert_eq!(session(&a).guides[1].corners[0], [24.0, 20.0]);
}

#[test]
fn no_movement_corner_and_edge_clicks_keep_the_solution_committable() {
    for keys in [none(), ctrl(), shift(), both()] {
        for edge in [false, true] {
            let mut a = app();
            control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
            let before = control(&mut a, &json!({"mode":"layout"})).unwrap();
            let q = corners(&a);
            let p = if edge { [(q[0][0] + q[1][0]) * 0.5, (q[0][1] + q[1][1]) * 0.5] } else { q[0] };
            let ctx = a.ctx().clone();
            let idx = a.session.active_index().unwrap();
            let mesh = mesh_of(&canvas_shapes(&mut a, &ctx, &view(), idx, None));
            assert!(down(&mut a, p, keys));
            pointer_move(&mut a, p, keys);
            pointer_up(&mut a, p, keys);
            assert_eq!(session(&a).describe(), before, "edge={edge}, keys={keys:?}");
            assert!(Arc::ptr_eq(&mesh, &mesh_of(&canvas_shapes(&mut a, &ctx, &view(), idx, None))));
            assert_eq!(commit(&mut a).unwrap()["committed"], true);
        }
    }
}

#[test]
fn hover_only_offers_drawing_where_a_new_guide_can_start() {
    let mut a = app();
    assert_eq!(hover(&a, [150.0, 130.0], TOL), Hover::Draw);
    for p in [[-20.0, 70.0], [180.0, 70.0], [80.0, -20.0], [80.0, 160.0]] {
        assert_eq!(hover(&a, p, TOL), Hover::Idle);
        assert!(!down(&mut a, p, none()));
    }
    let guides: Vec<Guide> = (1..=MAX_GUIDES)
        .map(|id| Guide { id: id as u64, corners: [[-20.0, 20.0], [60.0, 20.0], [60.0, 80.0], [-20.0, 80.0]], enabled: true, weight: 1.0 })
        .collect();
    control(&mut a, &json!({"guides":guides})).unwrap();
    assert_eq!(hover(&a, [150.0, 130.0], TOL), Hover::Idle);
    assert!(!down(&mut a, [150.0, 130.0], none()));
    assert_eq!(hover(&a, [-20.0, 20.0], TOL), Hover::Handle, "off-image handles remain editable at the guide limit");
    assert!(down(&mut a, [-20.0, 20.0], none()));
    pointer_up(&mut a, [-25.0, 15.0], none());
    assert_eq!(session(&a).guides.last().unwrap().corners[0], [-25.0, 15.0]);
}

#[test]
fn each_canvas_gates_the_raw_preview_on_its_actual_monitor() {
    let mut a = closed();
    let displays = [craft::cms::Builtin::Srgb, craft::cms::Builtin::DisplayP3, craft::cms::Builtin::Srgb]
        .into_iter()
        .enumerate()
        .map(|(index, profile)| photocraft_engine::display_color::Display {
            id: index as u32 + 1,
            name: format!("Test display {index}"),
            frame: [index as f64 * 400.0, 0.0, 400.0, 300.0],
            profile_name: None,
            icc: Some(Arc::new(profile.profile().to_bytes().to_vec())),
        })
        .collect();
    a.session.color.set_displays(Ok(displays));
    a.session.color.main_display = Some(1);
    let ctx = a.ctx().clone();
    begin(&mut a, &ctx).unwrap();
    let idx = a.session.active_index().unwrap();
    let before = session(&a).describe();
    assert!(Screen::of(&a, &session(&a).source_doc, Some(2)).transform.is_some());
    assert!(canvas_shapes(&mut a, &ctx, &view(), idx, Some(2)).is_empty());
    assert!(crate::distort_ui::display_doc_for_output(&a, idx, Some(2)).is_none(), "the incompatible canvas must retain the source layer");
    for output in [None, Some(1), Some(3)] {
        assert!(!canvas_shapes(&mut a, &ctx, &view(), idx, output).is_empty());
        assert!(crate::distort_ui::display_doc_for_output(&a, idx, output).is_some());
    }
    assert_eq!(session(&a).describe(), before, "a secondary monitor never cancels the primary session");
}

// ------------------------------------------------------------------------------------------ guides and axes

#[test]
fn mouse_guides_are_independent_and_survive_list_replacement_during_drag() {
    let mut a = app();
    assert!(down(&mut a, [10.0, 10.0], none()));
    pointer_up(&mut a, [50.0, 60.0], none());
    assert!(down(&mut a, [90.0, 80.0], none()));
    pointer_up(&mut a, [140.0, 130.0], none());
    assert_eq!(session(&a).guides.len(), 2);
    assert_ne!(session(&a).guides[0].id, session(&a).guides[1].id);
    down(&mut a, [10.0, 10.0], none());
    control(&mut a, &json!({"guides":[]})).unwrap();
    pointer_up(&mut a, [30.0, 30.0], none());
    assert!(session(&a).guides.is_empty(), "the grab ended with the list it was made on");
}

#[test]
fn a_tiny_drag_makes_no_guide_and_the_press_stays_the_sessions() {
    let mut a = app();
    assert!(down(&mut a, [10.0, 10.0], none()));
    pointer_up(&mut a, [12.0, 12.0], none());
    assert!(session(&a).guides.is_empty());
    // A press past the photo starts nothing, and is still consumed by the tool (it returns false: no drag).
    assert!(!down(&mut a, [500.0, 500.0], none()));
    assert!(session(&a).drag.is_none());
}

#[test]
fn max_integer_guide_id_does_not_break_new_mouse_guides() {
    let mut a = app();
    let mut g = guides();
    g[0]["id"] = json!(u64::MAX);
    control(&mut a, &json!({"guides":g})).unwrap();
    down(&mut a, [120.0, 10.0], none());
    pointer_up(&mut a, [150.0, 70.0], none());
    assert_eq!(session(&a).guides.len(), 2);
}

#[test]
fn ctrl_corner_drag_chooses_and_keeps_the_dominant_horizontal_axis() {
    let mut a = app();
    control(&mut a, &json!({"guides":guides()})).unwrap();
    down(&mut a, [25.0, 20.0], none());
    pointer_move(&mut a, [42.0, 24.0], ctrl());
    pointer_move(&mut a, [52.0, 55.0], ctrl());
    pointer_up(&mut a, [52.0, 55.0], ctrl());
    let c = corners(&a);
    assert_eq!(c[0], [52.0, 20.0]);
    assert_eq!(c[1], [100.0, 16.0]);
}

#[test]
fn ctrl_corner_vertical_drag_and_release_ctrl_restores_free_movement() {
    let mut a = app();
    control(&mut a, &json!({"guides":guides()})).unwrap();
    down(&mut a, [25.0, 20.0], none());
    pointer_move(&mut a, [27.0, 40.0], ctrl());
    assert_eq!(corners(&a)[0], [25.0, 40.0]);
    pointer_move(&mut a, [30.0, 43.0], none());
    pointer_up(&mut a, [30.0, 43.0], none());
    assert_eq!(corners(&a)[0], [30.0, 43.0]);
}

#[test]
fn hover_edge_moves_both_endpoints_while_corner_wins_near_a_vertex() {
    let mut a = app();
    control(&mut a, &json!({"guides":guides()})).unwrap();
    let s = session(&a);
    assert!(matches!(hit_handle(s, [25.0, 20.0], TOL), Some(Handle::Corner { corner: 0, .. })));
    assert!(matches!(hit_handle(s, [60.0, 18.0], TOL), Some(Handle::Edge { edge: 0, .. })));
    assert_eq!(hover(&a, [60.0, 18.0], TOL), Hover::Handle);
    assert_eq!(hover(&a, [80.0, 60.0], TOL), Hover::Draw);
    down(&mut a, [60.0, 18.0], none());
    assert_eq!(hover(&a, [0.0, 0.0], TOL), Hover::Grabbing);
    pointer_move(&mut a, [70.0, -15.0], none());
    pointer_up(&mut a, [70.0, -15.0], none());
    let c = corners(&a);
    assert_eq!(c[0], [35.0, -13.0]);
    assert_eq!(c[1], [110.0, -17.0]);
    assert_eq!(c[2], [109.0, 105.0]);
    assert_eq!(c[3], [21.0, 110.0]);
    assert!(control(&mut a, &json!({"solve":true})).is_ok());
}

#[test]
fn ctrl_edge_drag_preserves_both_other_endpoints_and_locks_axis() {
    let mut a = app();
    control(&mut a, &json!({"guides":guides()})).unwrap();
    down(&mut a, [23.0, 65.0], none());
    pointer_move(&mut a, [-20.0, 67.0], ctrl());
    pointer_up(&mut a, [-20.0, 100.0], ctrl());
    let c = corners(&a);
    assert_eq!(c[0], [-18.0, 20.0]);
    assert_eq!(c[3], [-22.0, 110.0]);
    assert_eq!(c[1], [100.0, 16.0]);
    assert_eq!(c[2], [109.0, 105.0]);
}

#[test]
fn corner_off_image_allowed_and_bad_quad_still_rejected() {
    let mut a = app();
    control(&mut a, &json!({"guides":guides()})).unwrap();
    down(&mut a, [25.0, 20.0], none());
    pointer_move(&mut a, [-20.0, -12.0], none());
    pointer_up(&mut a, [-20.0, -12.0], none());
    assert_eq!(corners(&a)[0], [-20.0, -12.0]);
    assert!(control(&mut a, &json!({"solve":true})).is_ok());
    control(&mut a, &json!({"mode":"layout"})).unwrap();
    let s = a.distort.global_perspective.as_mut().unwrap();
    let saved = s.guides[0].corners;
    s.move_corner(1, 0, [110.0, 120.0]);
    assert_eq!(s.guides[0].corners, saved);
}

#[test]
fn result_handles_cannot_create_local_warps() {
    let mut a = app();
    let before = control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    let point = before["solution"]["guides"][0]["transformedCorners"][0].as_array().unwrap();
    let (x, y) = (point[0].as_f64().unwrap(), point[1].as_f64().unwrap());
    assert!(!down(&mut a, [x, y], shift()), "the result is inspected, not dragged");
    pointer_move(&mut a, [x + 10.0, y + 10.0], none());
    pointer_up(&mut a, [x + 10.0, y + 10.0], none());
    let after = session(&a).describe();
    assert_eq!(after["guides"], before["guides"]);
    assert_eq!(after["solution"]["matrix"], before["solution"]["matrix"]);
    assert_eq!(hover(&a, [x, y], TOL), Hover::Idle);
}

#[test]
fn ctrl_dead_zone_is_two_screen_points_at_multiple_zooms_for_corners_and_edges() {
    for zoom in [0.1_f64, 1.0, 16.0] {
        let mut a = app();
        control(&mut a, &json!({"guides":guides()})).unwrap();
        let tol = HIT_POINTS / zoom;
        let threshold = tol * (AXIS_ACTIVATION_POINTS / HIT_POINTS);
        assert!((threshold * zoom - AXIS_ACTIVATION_POINTS).abs() < 1e-9);

        assert!(pointer_down(&mut a, [25.0, 20.0], tol, none()));
        assert!(matches!(&session(&a).drag, Some(Drag::Corner { activation_distance, .. }) if (*activation_distance - threshold).abs() < 1e-9));
        pointer_move(&mut a, [25.0 + 1.0 / zoom, 20.0 + 0.1 / zoom], both());
        assert_eq!(corners(&a)[0], [25.0, 20.0]);
        pointer_move(&mut a, [25.0 + 3.0 / zoom, 20.0 + 0.2 / zoom], both());
        let c = corners(&a)[0];
        assert!(c[0] > 25.0 && c[1] == 20.0, "Ctrl must win over Shift at zoom {zoom}");

        let s = a.distort.global_perspective.as_mut().unwrap();
        s.drag = Some(Drag::Edge { id: 1, edge: 0, mouse_start: [60.0, 18.0], original: s.guides[0].corners, axis: None, activation_distance: threshold });
        let old = s.guides[0].corners;
        s.drag_to([60.0 + 0.2 / zoom, 18.0 + 1.0 / zoom], ctrl());
        assert_eq!(s.guides[0].corners, old);
        s.drag_to([60.0 + 0.2 / zoom, 18.0 + 3.0 / zoom], ctrl());
        assert_eq!(s.guides[0].corners[0][0], old[0][0]);
        assert!(s.guides[0].corners[0][1] > old[0][1]);
    }
}

// ------------------------------------------------------------------------------------------ Shift: guide axes

fn points() -> [[f64; 2]; 4] {
    [[60.0, 45.0], [230.0, 25.0], [250.0, 195.0], [40.0, 220.0]]
}

/// A 320 × 240 document with the guide `points()`.
fn wide() -> Box<PhotocraftApp> {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    a.session.execute("file.new", json!({"width":320,"height":240,"depth":8})).unwrap();
    a.session.execute("layer.new.layer", json!({})).unwrap();
    a.session
        .edit("fixture", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 320, 240), &[0.2, 0.5, 0.8, 1.0]);
            Ok(())
        })
        .unwrap();
    a.sync_views();
    let ctx = a.ctx().clone();
    begin(&mut a, &ctx).unwrap();
    control(&mut a, &json!({"guides":[{"id":1,"corners":points()}]})).unwrap();
    a
}

fn on_axis(p: [f64; 2], anchor: [f64; 2], vector: [f64; 2]) {
    let d = [p[0] - anchor[0], p[1] - anchor[1]];
    assert!((d[0] * vector[1] - d[1] * vector[0]).abs() < 1e-8, "{p:?} is not on {anchor:?} + t {vector:?}");
}

fn close(p: [f64; 2], q: [f64; 2]) {
    assert!((p[0] - q[0]).abs() + (p[1] - q[1]).abs() < 1e-8, "{p:?} != {q:?}");
}

#[test]
fn shift_projects_corner_along_the_guides_own_top_edge() {
    let mut a = wide();
    down(&mut a, points()[0], shift());
    pointer_move(&mut a, [100.0, 47.0], shift());
    let q = corners(&a);
    on_axis(q[0], points()[0], [170.0, -20.0]);
    assert!(q[0][1] < 45.0 && q[0][0] > 90.0);
    assert_eq!(&q[1..], &points()[1..]);
    let d = session(&a).describe();
    assert_eq!(d["localAxisConstraint"]["space"], "guide");
    assert_eq!(d["localAxisConstraint"]["frozen"], true);
    pointer_up(&mut a, [100.0, 47.0], shift());
    close(corners(&a)[0], q[0]);
    assert!(session(&a).active_local_axis().is_none());
}

#[test]
fn shift_choice_does_not_follow_diagonal_mouse_noise() {
    let mut a = wide();
    down(&mut a, points()[0], shift());
    pointer_move(&mut a, [100.0, 47.0], shift());
    let axis = session(&a).active_local_axis();
    pointer_move(&mut a, [100.0, 110.0], shift());
    assert_eq!(session(&a).active_local_axis(), axis);
    on_axis(corners(&a)[0], points()[0], [170.0, -20.0]);
}

#[test]
fn shift_released_returns_free_and_repress_selects_a_new_axis() {
    let mut a = wide();
    down(&mut a, points()[0], shift());
    pointer_move(&mut a, [100.0, 47.0], shift());
    pointer_move(&mut a, [103.0, 85.0], none());
    assert_eq!(corners(&a)[0], [103.0, 85.0]);
    assert!(session(&a).active_local_axis().is_none());
    pointer_move(&mut a, [60.0, 95.0], shift());
    on_axis(corners(&a)[0], points()[0], [-20.0, 175.0]);
}

#[test]
fn ctrl_has_priority_over_shift_and_can_switch_back() {
    let mut a = wide();
    down(&mut a, points()[0], both());
    pointer_move(&mut a, [110.0, 53.0], both());
    assert_eq!(corners(&a)[0], [110.0, 45.0]);
    assert!(session(&a).active_local_axis().is_none());
    pointer_move(&mut a, [60.0, 95.0], shift());
    on_axis(corners(&a)[0], points()[0], [-20.0, 175.0]);
    pointer_move(&mut a, [110.0, 53.0], both());
    assert_eq!(corners(&a)[0], [110.0, 45.0]);
}

#[test]
fn late_shift_uses_the_original_guide_not_modified_edges() {
    let mut a = wide();
    down(&mut a, points()[0], none());
    pointer_move(&mut a, [95.0, 100.0], none());
    assert_eq!(corners(&a)[0], [95.0, 100.0]);
    pointer_move(&mut a, [120.0, 55.0], shift());
    on_axis(corners(&a)[0], points()[0], [170.0, -20.0]);
}

#[test]
fn shift_corner_moves_outside_the_photo_but_cannot_fold_the_quad() {
    let mut a = wide();
    down(&mut a, points()[0], shift());
    pointer_move(&mut a, [-40.0, 59.0], shift());
    pointer_up(&mut a, [-40.0, 59.0], shift());
    assert!(corners(&a)[0][0] < 0.0);
    on_axis(corners(&a)[0], points()[0], [170.0, -20.0]);
    assert!(validate_guides(&session(&a).guides).is_ok());

    let mut a = wide();
    let old = corners(&a);
    down(&mut a, old[0], shift());
    pointer_move(&mut a, [400.0, 5.0], shift());
    assert_eq!(corners(&a), old);
    pointer_up(&mut a, [400.0, 5.0], shift());
    assert_eq!(corners(&a), old);
}

#[test]
fn shift_final_release_sample_is_constrained_even_without_a_move() {
    let mut a = wide();
    down(&mut a, points()[0], shift());
    pointer_up(&mut a, [105.0, 50.0], shift());
    on_axis(corners(&a)[0], points()[0], [170.0, -20.0]);
    assert_ne!(corners(&a)[0], points()[0]);
}

#[test]
fn shift_keeps_the_corner_grab_offset() {
    let mut a = wide();
    down(&mut a, [62.0, 46.0], shift());
    pointer_move(&mut a, [102.0, 42.0], shift());
    let raw = [40.0, -4.0];
    let dot = (raw[0] * 170.0 + raw[1] * -20.0) / (170.0 * 170.0 + 20.0 * 20.0);
    close(corners(&a)[0], [60.0 + dot * 170.0, 45.0 - dot * 20.0]);
}

#[test]
fn replacing_guides_cancels_the_frozen_shift_drag() {
    let mut a = wide();
    down(&mut a, points()[0], shift());
    pointer_move(&mut a, [100.0, 47.0], shift());
    control(&mut a, &json!({"guides":[{"id":1,"corners":points()}]})).unwrap();
    pointer_up(&mut a, [130.0, 50.0], shift());
    assert_eq!(corners(&a), points());
    assert!(session(&a).active_local_axis().is_none());
}

#[test]
fn shift_inside_a_guide_still_creates_a_new_guide() {
    let mut a = wide();
    down(&mut a, [100.0, 90.0], shift());
    pointer_move(&mut a, [140.0, 140.0], shift());
    pointer_up(&mut a, [140.0, 140.0], shift());
    assert_eq!(session(&a).guides.len(), 2);
    assert_eq!(corners(&a), points());
}

#[test]
fn a_click_inside_a_guide_selects_it_without_moving_it() {
    let mut a = wide();
    control(
        &mut a,
        &json!({"guides":[
        {"id":1,"corners":points()},
        {"id":2,"corners":[[270.0,10.0],[310.0,10.0],[310.0,60.0],[270.0,60.0]]}
    ],"selectedGuide":2}),
    )
    .unwrap();
    assert!(!down(&mut a, [150.0, 120.0], none()));
    assert_eq!(session(&a).selected, Some(1));
    assert_eq!(corners(&a), points());
}

// ------------------------------------------------------------------------------------------ preview mesh

#[test]
fn preview_mesh_adapts_to_projective_error_or_refuses_a_misleading_result() {
    let bounds = Rect::new(0, 0, 4096, 64);
    let affine = photocraft_algo::transform::Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    assert_eq!(preview_grid_size(bounds, &affine).unwrap(), GRID);
    let perspective = photocraft_algo::transform::Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.0, 1.0]);
    let refined = preview_grid_size(bounds, &perspective).unwrap();
    assert!(refined > GRID, "stronger perspective should refine beyond the default {GRID}");
    assert!(preview_mesh_exceeds_error(bounds, &perspective, GRID));
    assert!(!preview_mesh_exceeds_error(bounds, &perspective, refined));
    let too_steep = photocraft_algo::transform::Homography([1000.0, 0.0, 0.0, 0.0, 1000.0, 0.0, 0.01, 0.0, 1.0]);
    assert!(preview_grid_size(bounds, &too_steep).is_err(), "an unrepresentable mesh cannot be displayed as accurate");
}

fn mesh_of(shapes: &[Shape]) -> Arc<egui::Mesh> {
    match shapes.first() {
        Some(Shape::Mesh(mesh)) => Arc::clone(mesh),
        other => panic!("the preview mesh comes first, got {other:?}"),
    }
}

#[test]
fn every_canvas_gets_its_own_mesh_and_a_geometry_change_rebuilds_them() {
    let mut a = app();
    control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    let ctx = a.ctx().clone();
    let idx = a.session.active_index().unwrap();
    let main = view();
    let window = CanvasView::new(egui::Rect::from_min_size(Pos2::new(10.0, 10.0), egui::vec2(200.0, 150.0)), Point::new(80.0, 70.0), 2.0);
    let first = mesh_of(&canvas_shapes(&mut a, &ctx, &main, idx, None));
    let second = mesh_of(&canvas_shapes(&mut a, &ctx, &window, idx, None));
    assert!(!Arc::ptr_eq(&first, &second), "a window with another view needs its own mesh");
    // Drawing the main canvas again does not rebuild what the window's pass left beside it.
    assert!(Arc::ptr_eq(&first, &mesh_of(&canvas_shapes(&mut a, &ctx, &main, idx, None))));
    assert_eq!(session(&a).meshes.len(), 2);
    let n = (GRID + 1) * (GRID + 1);
    assert!(first.vertices.len() >= n, "the grid was refined, never coarsened");
    // Selecting keeps the cache; changing the guides or the mode drops it.
    control(&mut a, &json!({"selectedGuide":1})).unwrap();
    assert_eq!(session(&a).meshes.len(), 2);
    control(&mut a, &json!({"guides":guides()})).unwrap();
    assert!(session(&a).meshes.is_empty());
    let rebuilt = mesh_of(&canvas_shapes(&mut a, &ctx, &main, idx, None));
    assert!(!Arc::ptr_eq(&first, &rebuilt));
}

#[test]
fn a_stale_or_foreign_canvas_shows_no_preview() {
    let mut a = app();
    let ctx = a.ctx().clone();
    let idx = a.session.active_index().unwrap();
    assert!(!canvas_shapes(&mut a, &ctx, &view(), idx, None).is_empty());
    assert!(canvas_shapes(&mut a, &ctx, &view(), idx + 1, None).is_empty(), "no canvas of another document shows it");
    let source = a.session.active().unwrap().doc.id;
    a.session.execute("layer.new.layer", json!({})).unwrap();
    assert_eq!(a.session.active().unwrap().doc.id, source);
    assert!(canvas_shapes(&mut a, &ctx, &view(), idx, None).is_empty(), "the source changed: the old snapshot must not paint over it");
    assert!(crate::distort_ui::display_doc(&a, idx).is_none());
    assert!(!owns_pointer(&a, idx));
    assert!(commit(&mut a).is_err());
    assert!(a.distort.global_perspective.is_some(), "a stale session can still be cancelled");
}

#[test]
fn switching_documents_drops_the_session_and_its_preview() {
    let mut a = app();
    let first = a.session.active().unwrap().doc.id;
    a.session.execute("file.new", json!({"width":60,"height":40,"depth":8})).unwrap();
    a.sync_views();
    assert!(a.distort.global_perspective.is_none());
    assert!(crate::distort_ui::display_doc(&a, a.session.active_index().unwrap()).is_none());
    assert!(a.session.set_active(0));
    a.sync_views();
    assert_eq!(a.session.active().unwrap().doc.id, first);
    assert!(a.distort.global_perspective.is_none(), "it does not come back with the tab");
}

#[test]
fn the_editor_is_part_of_ui_inspect_state() {
    let mut a = app();
    assert!(a.distort.active());
    assert_eq!(a.distort.describe()["globalPerspective"]["version"], "V3");
    cancel(&mut a);
    assert!(!a.distort.active());
    assert!(a.distort.describe()["globalPerspective"].is_null());
}

// ------------------------------------------------------------------------------------------ the canvas pass

fn canvas_app() -> Box<PhotocraftApp> {
    let mut a = app();
    a.ui.tool = Tool::Brush;
    a
}

fn drag(a: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    testing::tool_event(a, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    testing::tool_event(a, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    testing::tool_event(a, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    testing::tool_event(a, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn near(p: [f64; 2], q: [f64; 2]) {
    assert!((p[0] - q[0]).abs() < 0.1 && (p[1] - q[1]).abs() < 0.1, "{p:?} is not {q:?}");
}

#[test]
fn a_drag_on_the_canvas_draws_a_guide_whichever_tool_is_selected_and_paints_nothing() {
    for tool in [Tool::Brush, Tool::Hand, Tool::Zoom, Tool::RotateView, Tool::RectMarquee, Tool::Move] {
        let mut a = app();
        a.ui.tool = tool;
        let doc = a.session.active().unwrap().doc.clone();
        let n = past(&a);
        drag(&mut a, [10.0, 10.0], [65.0, 75.0], Modifiers::NONE);
        assert_eq!(session(&a).guides.len(), 1, "{tool:?}");
        assert!(Arc::ptr_eq(&doc, &a.session.active().unwrap().doc), "{tool:?} must not edit the document");
        assert_eq!(past(&a), n, "{tool:?}");
        assert!(a.session.active().unwrap().doc.selection.is_none(), "{tool:?}");
    }
}

#[test]
fn the_corner_grab_follows_the_pointer_and_is_released_with_the_button() {
    let mut a = canvas_app();
    control(&mut a, &json!({"guides":guides()})).unwrap();
    testing::tool_event(&mut a, ToolEvent::Down { x: 25.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(session(&a).drag.is_some(), "the press took the grab");
    testing::tool_event(&mut a, ToolEvent::Move { x: 33.0, y: 28.0, pressure: 1.0 }, Modifiers::NONE);
    testing::tool_event(&mut a, ToolEvent::Move { x: 40.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    near(corners(&a)[0], [40.0, 30.0]);
    testing::tool_event(&mut a, ToolEvent::Up { x: 40.0, y: 30.0 }, Modifiers::NONE);
    assert!(session(&a).drag.is_none(), "the release ended the grab");
    let kept = corners(&a);
    testing::tool_event(&mut a, ToolEvent::Move { x: 90.0, y: 90.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(corners(&a), kept, "a move after the release drags nothing");
}

#[test]
fn ctrl_held_during_a_canvas_drag_locks_the_image_axis() {
    let mut a = canvas_app();
    control(&mut a, &json!({"guides":guides()})).unwrap();
    let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
    testing::tool_event(&mut a, ToolEvent::Down { x: 25.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    testing::tool_event(&mut a, ToolEvent::Move { x: 45.0, y: 24.0, pressure: 1.0 }, ctrl);
    testing::tool_event(&mut a, ToolEvent::Move { x: 55.0, y: 60.0, pressure: 1.0 }, ctrl);
    testing::tool_event(&mut a, ToolEvent::Up { x: 55.0, y: 60.0 }, ctrl);
    let c = corners(&a)[0];
    assert!(c[0] > 30.0 && (c[1] - 20.0).abs() < 0.1, "{c:?}");
}

#[test]
fn a_stale_session_does_not_swallow_clicks() {
    let mut a = app();
    a.ui.tool = Tool::RectMarquee;
    a.session.execute("layer.new.layer", json!({})).unwrap();
    let idx = a.session.active_index().unwrap();
    assert!(!owns_pointer(&a, idx));
    drag(&mut a, [10.0, 10.0], [60.0, 60.0], Modifiers::NONE);
    assert!(session(&a).guides.is_empty(), "the stale editor took no press");
    assert!(a.session.active().unwrap().doc.selection.is_some(), "the selected tool got the drag");
}

#[test]
fn a_secondary_press_is_not_the_editors() {
    let mut a = canvas_app();
    control(&mut a, &json!({"guides":guides()})).unwrap();
    let before = session(&a).describe();
    testing::secondary_click(&mut a, [25.0, 20.0], Modifiers::NONE);
    assert_eq!(session(&a).describe()["guides"], before["guides"]);
    assert!(session(&a).drag.is_none());
}

#[test]
fn escape_cancels_and_enter_commits_through_the_tool_host() {
    let mut a = canvas_app();
    control(&mut a, &json!({"guides":guides()})).unwrap();
    testing::frame(&mut a);
    let doc = a.session.active().unwrap().doc.clone();
    testing::key(&mut a, egui::Key::Escape, Modifiers::NONE);
    assert!(a.distort.global_perspective.is_none());
    assert!(Arc::ptr_eq(&doc, &a.session.active().unwrap().doc));

    let mut a = canvas_app();
    control(&mut a, &json!({"guides":guides(),"solve":true})).unwrap();
    testing::frame(&mut a);
    let n = past(&a);
    testing::key(&mut a, egui::Key::Enter, Modifiers::NONE);
    assert!(a.distort.global_perspective.is_none());
    assert_eq!(past(&a), n + 1);
}

#[test]
fn delete_removes_the_selected_guide() {
    let mut a = app();
    control(
        &mut a,
        &json!({"guides":[
        {"id":1,"corners":[[25,20],[100,16],[109,105],[21,110]]},
        {"id":2,"corners":[[110,10],[150,10],[150,60],[110,60]]}
    ],"selectedGuide":2}),
    )
    .unwrap();
    assert!(delete_selected(&mut a));
    assert_eq!(session(&a).guides.len(), 1);
    assert_eq!(session(&a).selected, Some(1));
    assert!(delete_selected(&mut a));
    assert!(session(&a).guides.is_empty());
    assert!(!delete_selected(&mut a), "nothing is left to delete");
}
