use super::*;
use crate::canvas::{ToolEvent, tool_event};
use craft::behavior::CursorBadge;
use craft::keymap::Mods;

pub(super) fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 16, "hardness": 1.0}})).unwrap();
    app
}

fn drag(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    let m = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: 50.0, y: 30.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: 50.0, y: 30.0 }, m);
    assert!(!app.ui.status_error, "{tool:?}: {}", app.ui.status);
}

/// Down at the first point, a move through each next one, up at the last.
fn gesture(app: &mut PhotocraftApp, points: &[[f64; 2]]) {
    let m = egui::Modifiers::NONE;
    let [x, y] = points[0];
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    for &[x, y] in &points[1..] {
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
    }
    let [x, y] = points[points.len() - 1];
    tool_event(app, ToolEvent::Up { x, y }, m);
}

fn active(app: &PhotocraftApp) -> &photocraft_doc::Layer {
    let st = app.session.active().unwrap();
    st.active_layer.and_then(|id| st.doc.layer(id)).unwrap()
}

pub(super) fn stripes(app: &mut PhotocraftApp, step: usize, target: &str) {
    for x in (0..100).step_by(step) {
        app.run("paint.pencil", json!({"points": [[x, 0], [x, 60]], "size": 2, "color": "#606060", "target": target})).unwrap();
    }
}

/// Numbers compare within rounding of the 32-bit options; everything else exactly.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() < 1e-3,
        _ => a == b,
    }
}

fn shift() -> Mods {
    Mods { shift: true, ..Mods::default() }
}

#[test]
fn clone_stamp_in_pattern_preview_wraps_a_dab_across_the_right_edge() {
    for pattern_preview in [false, true] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[10, 10]], "size": 24, "color": "#ff0000"})).unwrap();
        app.ui.tool = Tool::CloneStamp;
        crate::paint_mouse::sync_tool_brush(&mut app);
        // A hard tip, so the wrapped part of the dab is fully painted.
        app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 1}})).unwrap();
        set_source(&mut app, 10.0, 10.0);
        app.ui.view.pattern_preview = pattern_preview;
        let modifiers = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 98.0, y: 30.0, pressure: 1.0 }, modifiers);
        tool_event(&mut app, ToolEvent::Up { x: 98.0, y: 30.0 }, modifiers);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        let surface = active(&app).surface().unwrap();
        let center = surface.rgba(98, 30);
        assert!(center[0] > 0.9 && center[1] < 0.1, "clone center: {center:?}");
        let opposite = surface.rgba(1, 30);
        if pattern_preview {
            assert!(opposite[0] > 0.9 && opposite[1] < 0.1, "Pattern Preview must wrap the clone footprint: {opposite:?}");
        } else {
            assert!(opposite[0] > 0.9 && opposite[1] > 0.9, "ordinary cloning must not wrap: {opposite:?}");
        }
    }
}

/// A Clone Stamp click paints with the brush's hardness (the Brush popup's), not a hard tip:
/// near the rim a soft brush paints only part of the source, a hard one all of it.
#[test]
fn clone_stamp_paints_with_the_brush_hardness() {
    let rim = |hardness: f32| {
        let mut app = app();
        app.ui.tool = Tool::CloneStamp;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 24, "hardness": hardness}})).unwrap();
        app.run("paint.pencil", json!({"points": [[15, 30]], "size": 40, "color": "#ff0000"})).unwrap();
        app.ui.tool = Tool::CloneStamp;
        set_source(&mut app, 15.0, 30.0);
        let m = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 70.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 30.0 }, m);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        // 9 px from the centre of a 12 px radius: inside a hard tip, on a soft tip's falloff.
        1.0 - active(&app).surface().unwrap().rgba(79, 30)[1]
    };
    let (hard, soft) = (rim(1.0), rim(0.0));
    assert!(hard > 0.99, "a hard tip clones fully near its rim: {hard}");
    assert!(soft < 0.8, "a soft tip clones only part of the source there: {soft}");
}

#[test]
fn alt_stroke_runs_the_opposite_tool() {
    // ⌥ held as the stroke starts: Dodge burns, Burn dodges, Blur sharpens, Sharpen blurs.
    for (tool, cmd) in [(Tool::Dodge, "paint.burn"), (Tool::Burn, "paint.dodge"), (Tool::Blur, "paint.sharpen"), (Tool::Sharpen, "paint.blur")] {
        let mut app = app();
        app.ui.tool = tool;
        let m = egui::Modifiers::ALT;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 30.0 }, m);
        assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some(cmd), "{tool:?} with ⌥");
        assert_eq!(app.ui.tool, tool, "the selected tool stays {tool:?}");
    }
}

type Setup = fn(&mut PhotocraftApp);

/// One row of the stamp table: the tool's brush edit, an extra setup, the command and the params it sends.
struct StampRow {
    name: &'static str,
    tool: Tool,
    cmd: &'static str,
    brush: Value,
    setup: Setup,
    want: Value,
}

fn stamp_row(name: &'static str, tool: Tool, cmd: &'static str, brush: Value, setup: Setup, want: Value) -> StampRow {
    StampRow { name, tool, cmd, brush, setup, want }
}

/// What each stamp tool's bar values become in its command (the JSON names are the engine's protocol).
#[test]
fn stamp_tools_send_the_options_they_paint_with() {
    use photocraft_color::BlendMode;
    let rows = [
        stamp_row(
            "Clone opacity, flow and Multiply",
            Tool::CloneStamp,
            "paint.cloneStamp",
            json!({"opacity": 0.5, "flow": 0.5, "mode": "Multiply"}),
            |_| {},
            json!({"opacity": 50.0, "flow": 50.0, "mode": "Multiply"}),
        ),
        stamp_row("Healing Luminosity", Tool::Healing, "paint.healingBrush", json!({"mode": "Luminosity"}), |_| {}, json!({"mode": "Luminosity"})),
        // A mode outside the Healing Brush's menu (restored with the tool active) heals in Normal.
        stamp_row(
            "Healing holding Overlay",
            Tool::Healing,
            "paint.healingBrush",
            json!({}),
            |app| app.session.tools.brush.mode = BlendMode::Overlay,
            json!({"mode": "Normal"}),
        ),
        stamp_row(
            "History Brush opacity, flow and Screen",
            Tool::HistoryBrush,
            "paint.historyBrush",
            json!({"opacity": 0.5, "flow": 0.5, "mode": "Screen"}),
            |_| {},
            json!({"opacity": 50.0, "flow": 50.0, "mode": "Screen"}),
        ),
        stamp_row(
            "Pattern Stamp Overlay, unaligned, scale and angle",
            Tool::PatternStamp,
            "paint.patternStamp",
            json!({"mode": "Overlay"}),
            |app| {
                let o = &mut app.ui.tool_options;
                (o.pattern_stamp_aligned, o.pattern_stamp_scale, o.pattern_stamp_angle) = (false, 150.0, 30.0);
            },
            json!({"mode": "Overlay", "aligned": false, "scale": 150.0, "angle": 30.0}),
        ),
        stamp_row(
            "Art History opacity, style, area and tolerance",
            Tool::ArtHistoryBrush,
            "paint.artHistoryBrush",
            json!({"opacity": 0.4}),
            |app| {
                let o = &mut app.ui.tool_options;
                (o.art_history_style, o.art_history_area, o.art_history_tolerance) = ("tightMedium".into(), 30.0, 12.0);
            },
            json!({"opacity": 40.0, "style": "tightMedium", "area": 30.0, "tolerance": 12.0}),
        ),
        // PLAN: the Red Eye params `pupilSize` and `darken` come from engine redeye_cmds.rs:289; the caller is outside this set's files.
        stamp_row(
            "Red Eye pupil size and darken",
            Tool::RedEye,
            "paint.redEye",
            json!({}),
            |app| {
                let o = &mut app.ui.tool_options;
                (o.red_eye_pupil_size, o.red_eye_darken) = (20.0, 70.0);
            },
            json!({"pupilSize": 20.0, "darken": 70.0}),
        ),
    ];
    for row in rows {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[0, 30], [100, 30]], "size": 20, "color": "#3060c0"})).unwrap();
        app.ui.tool = row.tool;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": row.brush})).unwrap();
        set_source(&mut app, 10.0, 10.0);
        (row.setup)(&mut app);
        if row.tool == Tool::RedEye {
            gesture(&mut app, &[[50.0, 30.0]]);
        } else {
            drag(&mut app, row.tool);
        }
        assert!(!app.ui.status_error, "{}: {}", row.name, app.ui.status);
        let (id, p) = app.session.journal.last().cloned().unwrap();
        assert_eq!(id, row.cmd, "{}", row.name);
        for (key, want) in row.want.as_object().unwrap() {
            assert!(same(&p[key.as_str()], want), "{}: {key}: {} vs {want}", row.name, p[key.as_str()]);
        }
    }
}

type Edit = fn(&mut crate::state::ToolOptions);

/// One row of the dab table: an edit of the bar's values and the param the tool's command then carries
/// (`None`: the param is absent).
struct DabRow {
    name: &'static str,
    tool: Tool,
    edit: Edit,
    key: &'static str,
    want: Option<Value>,
}

fn dab_row(name: &'static str, tool: Tool, edit: Edit, key: &'static str, want: Option<Value>) -> DabRow {
    DabRow { name, tool, edit, key, want }
}

fn tool_params(app: &PhotocraftApp, tool: Tool) -> Value {
    if tool == Tool::ColorReplacement { color_replacement_params(app) } else { dab_params(app, tool).expect("a dab tool").1 }
}

/// What each dab tool's bar values become in its params, and which layers a stroke samples.
#[test]
fn dab_tools_send_their_bar_values() {
    use photocraft_color::BlendMode;
    let rows = [
        dab_row("Blur own Sample All", Tool::Blur, |o| o.sample_all.blur = true, "sampleAllLayers", Some(json!(true))),
        dab_row("Blur, another tool's Sample All", Tool::Blur, |o| o.sample_all.magic_wand = true, "sampleAllLayers", Some(json!(false))),
        dab_row("Sharpen own Sample All", Tool::Sharpen, |o| o.sample_all.sharpen = true, "sampleAllLayers", Some(json!(true))),
        dab_row("Smudge own Sample All", Tool::Smudge, |o| o.sample_all.smudge = true, "sampleAllLayers", Some(json!(true))),
        dab_row("Smudge, Blur's Sample All", Tool::Smudge, |o| o.sample_all.blur = true, "sampleAllLayers", Some(json!(false))),
        dab_row("Sponge flow 10", Tool::Sponge, |o| o.sponge_flow = 10.0, "flow", Some(json!(10.0))),
        dab_row("Sponge flow 50", Tool::Sponge, |o| o.sponge_flow = 50.0, "flow", Some(json!(50.0))),
        dab_row("Sponge flow 100", Tool::Sponge, |o| o.sponge_flow = 100.0, "flow", Some(json!(100.0))),
        dab_row("Sponge stored 33.7", Tool::Sponge, |o| o.sponge_flow = 33.7, "flow", Some(json!(34.0))),
        dab_row("Smudge mode", Tool::Smudge, |o| o.smudge_mode = BlendMode::Darken, "mode", Some(json!(BlendMode::Darken.label()))),
        dab_row("Blur has no mode", Tool::Blur, |_| {}, "mode", None),
        dab_row("Sharpen has no mode", Tool::Sharpen, |_| {}, "mode", None),
        dab_row("Color Replacement mode", Tool::ColorReplacement, |o| o.cr_mode = "luminosity".into(), "mode", Some(json!("luminosity"))),
        dab_row("Color Replacement limits", Tool::ColorReplacement, |o| o.cr_limits = "findEdges".into(), "limits", Some(json!("findEdges"))),
        dab_row("Color Replacement sampling", Tool::ColorReplacement, |o| o.cr_sampling = "once".into(), "sampling", Some(json!("once"))),
        dab_row("Color Replacement anti-alias", Tool::ColorReplacement, |o| o.cr_anti_alias = false, "antiAlias", Some(json!(false))),
        dab_row("Color Replacement tolerance 55", Tool::ColorReplacement, |o| o.cr_tolerance = 55.0, "tolerance", Some(json!(55.0))),
        dab_row("Color Replacement stored 12.5", Tool::ColorReplacement, |o| o.cr_tolerance = 12.5, "tolerance", Some(json!(13.0))),
    ];
    for row in rows {
        let mut app = app();
        (row.edit)(&mut app.ui.tool_options);
        let p = tool_params(&app, row.tool);
        match row.want {
            Some(want) => assert!(same(&p[row.key], &want), "{}: {p}", row.name),
            None => assert!(p.get(row.key).is_none(), "{}: {p}", row.name),
        }
    }
    // The foreground is the replacement colour.
    let fresh = app();
    assert_eq!(color_replacement_params(&fresh)["color"], json!(fresh.session.tools.foreground));

    // #207, #731: each tool's own Sample All Layers reaches the stroke, another tool's does not (#5524).
    for tool in [Tool::SpotHealing, Tool::Remove, Tool::Blur, Tool::Sharpen, Tool::Smudge] {
        for all in [false, true] {
            let mut app = app();
            stripes(&mut app, 4, "pixels");
            app.run("layer.new.layer", json!({})).unwrap();
            *app.ui.tool_options.sample_all.slot(tool).unwrap() = all;
            drag(&mut app, tool);
            let a = active(&app).surface().unwrap().rgba(30, 30)[3];
            assert_eq!(a > 0.0, all, "{tool:?} sampleAllLayers={all}: alpha {a}");
        }
    }
    for tool in [Tool::Blur, Tool::Sharpen, Tool::Smudge] {
        let mut app = app();
        stripes(&mut app, 4, "pixels");
        app.run("layer.new.layer", json!({})).unwrap();
        let mut others = crate::state::SampleAllLayers::default();
        for t in
            [Tool::MagicWand, Tool::QuickSelection, Tool::ObjectSelection, Tool::PaintBucket, Tool::MagicEraser, Tool::SpotHealing, Tool::Patch, Tool::Remove]
        {
            *others.slot(t).unwrap() = true;
        }
        app.ui.tool_options.sample_all = others;
        assert!(!app.ui.tool_options.sample_all.get(tool));
        drag(&mut app, tool);
        let a = active(&app).surface().unwrap().rgba(30, 30)[3];
        assert_eq!(a, 0.0, "{tool:?} sampled all layers through another tool's option");
    }
}

#[test]
fn retouch_strokes_paint_the_targeted_mask() {
    // #207: with the mask targeted, retouching changes the mask, never the pixels.
    for tool in [Tool::Blur, Tool::Sharpen, Tool::Smudge, Tool::Dodge, Tool::Burn] {
        let mut app = app();
        app.run("layer.new.layer", json!({})).unwrap();
        stripes(&mut app, 4, "pixels");
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        stripes(&mut app, 6, "mask");
        app.ui.mask_target = true;
        app.ui.tool_options.protect_detail = false;
        let b = app.session.active().unwrap().doc.bounds();
        let px0 = active(&app).surface().unwrap().read_region(b);
        let mask0 = active(&app).mask.as_ref().unwrap().surface.read_region(b);
        drag(&mut app, tool);
        assert_eq!(active(&app).surface().unwrap().read_region(b), px0, "{tool:?}: pixels untouched");
        assert_ne!(active(&app).mask.as_ref().unwrap().surface.read_region(b), mask0, "{tool:?}: mask changed");
    }
}

/// One row of the selection table: setup, the gesture and the command with the params it sends.
struct SelectionRow {
    name: &'static str,
    tool: Tool,
    setup: Setup,
    points: &'static [[f64; 2]],
    cmd: &'static str,
    want: Value,
}

#[test]
fn selection_tools_send_their_bar_values() {
    let rows = [
        // A click cannot be a rectangle: it selects at the cursor instead of looking dead.
        SelectionRow {
            name: "Object Selection click without Object Finder",
            tool: Tool::ObjectSelection,
            setup: |app| app.ui.tool_options.object_finder = false,
            points: &[[50.0, 30.0]],
            cmd: "select.quick",
            want: json!({}),
        },
        SelectionRow {
            name: "Object Selection rectangle",
            tool: Tool::ObjectSelection,
            setup: |app| {
                app.run("paint.pencil", json!({"points": [[20, 10], [20, 50]], "size": 8, "color": "#ff0000"})).unwrap();
            },
            points: &[[5.0, 5.0], [60.0, 55.0]],
            cmd: "select.object",
            want: json!({}),
        },
        SelectionRow {
            name: "Patch Content-Aware structure and colour",
            tool: Tool::Patch,
            setup: |app| {
                app.run("paint.pencil", json!({"points": [[68, 30], [74, 30]], "size": 6, "color": "#ff0000"})).unwrap();
                app.run("select.rect", json!({"x": 60, "y": 20, "width": 24, "height": 20})).unwrap();
                let o = &mut app.ui.tool_options;
                (o.patch_content_aware, o.patch_structure, o.patch_color) = (true, 5.0, 3.0);
            },
            points: &[[70.0, 30.0], [50.0, 31.0], [30.0, 30.0]],
            cmd: "paint.patch",
            want: json!({"contentAware": true, "structure": 5.0, "color": 3.0}),
        },
    ];
    for row in rows {
        let mut app = app();
        (row.setup)(&mut app);
        app.ui.tool = row.tool;
        gesture(&mut app, row.points);
        assert!(!app.ui.status_error, "{}: {}", row.name, app.ui.status);
        let (id, p) = app.session.journal.last().cloned().unwrap();
        assert_eq!(id, row.cmd, "{}", row.name);
        for (key, want) in row.want.as_object().unwrap() {
            assert!(same(&p[key.as_str()], want), "{}: {key}: {p}", row.name);
        }
    }
}

/// Bug #668: setting the source (⌥-click) leaves nothing on the canvas; the marker shows while a stroke is painted.
#[test]
fn clone_marker_shows_only_while_painting() {
    let mut app = app();
    app.ui.tool = Tool::CloneStamp;
    set_source(&mut app, 30.0, 30.0);

    assert_eq!(source_marker_point(&app, None), None, "no marker after setting the source");
    assert!(source_marker_point(&app, Some([50.0, 30.0])).is_some(), "the marker shows while a stroke is painted");
    app.ui.tool = Tool::Brush;
    assert_eq!(source_marker_point(&app, Some([50.0, 30.0])), None, "other tools show none");
}

#[test]
fn patch_tool_lassoes_then_drags_the_patch() {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[68, 30], [74, 30]], "size": 6, "color": "#ff0000"})).unwrap();
    let red = |app: &PhotocraftApp| active(app).surface().unwrap().rgba(71, 30)[1] < 0.5;
    assert!(red(&app));
    app.ui.tool = Tool::Patch;
    let m = egui::Modifiers::NONE;
    // Outside any selection the drag is a lasso.
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 20.0, pressure: 1.0 }, m);
    for [x, y] in [[84.0, 20.0], [84.0, 40.0], [60.0, 40.0]] {
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
    }
    tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 40.0 }, m);
    assert!(app.session.active().unwrap().doc.selection.is_some(), "lasso made a selection");
    assert!(red(&app), "the lasso alone changes no pixels");
    // ⇧-drag inside the selection adds to it rather than patching.
    assert!(!patch_drags_selection(&app, [70.0, 30.0], shift()));
    // Dragging inside it patches from where it is dropped; the offset is limited to the canvas.
    assert!(patch_drags_selection(&app, [70.0, 30.0], Mods::default()));
    assert_eq!(patch_offset(&mut app, [70.0, 30.0], [-200.0, 30.0]), [-60, 0]);
    tool_event(&mut app, ToolEvent::Down { x: 70.0, y: 30.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 31.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 30.0 }, m);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let px = active(&app).surface().unwrap().rgba(71, 30);
    assert!(px[0] > 0.95 && px[1] > 0.95 && px[2] > 0.95, "blemish patched with the white background: {px:?}");
}

/// #11024: the Patch and Content-Aware Move cursors show + with ⇧ and − with ⌥, like the Lasso's,
/// but nothing over the selection without them, where a press drags the patch.
#[test]
fn patch_cursor_shows_the_selection_mode_badge() {
    use crate::tool_feedback::patch_badge;
    let none = Mods::default();
    let alt = Mods { alt: true, ..none };
    let both = Mods { alt: true, shift: true, ..none };
    for tool in [Tool::Patch, Tool::ContentAwareMove] {
        let mut app = app();
        app.ui.tool = tool;

        let badge = |app: &PhotocraftApp, at: [f64; 2], mods: Mods| patch_badge(app, at, None, mods);
        let outside = [80.0, 30.0];
        assert_eq!(badge(&app, outside, shift()), Some(CursorBadge::Add), "{tool:?}");
        assert_eq!(badge(&app, outside, alt), None, "{tool:?}: nothing selected, so ⌥ has nothing to subtract");
        assert_eq!(badge(&app, outside, none), None, "{tool:?}: a new selection has no badge");

        app.run("select.rect", json!({"x": 10, "y": 20, "width": 20, "height": 20})).unwrap();
        assert_eq!(badge(&app, outside, shift()), Some(CursorBadge::Add), "{tool:?}");
        assert_eq!(badge(&app, outside, alt), Some(CursorBadge::Subtract), "{tool:?}");
        assert_eq!(badge(&app, outside, both), Some(CursorBadge::Intersect), "{tool:?}");
        app.ui.selection_mode = 1;
        assert_eq!(badge(&app, outside, none), Some(CursorBadge::Add), "{tool:?}: the options-bar mode outside the selection");
        let over = (tool == Tool::Patch).then_some(CursorBadge::Arrow);
        assert_eq!(badge(&app, [20.0, 30.0], none), over, "{tool:?}: a press over the selection drags the patch (Patch shows an arrow, #12130)");
        assert_eq!(badge(&app, [20.0, 30.0], shift()), Some(CursorBadge::Add), "{tool:?}: ⇧ over the selection adds");
    }
}

#[test]
fn content_aware_move_tool_lassoes_then_moves_or_extends() {
    for (mode, keeps) in [("move", false), ("extend", true)] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[20, 30], [24, 30]], "size": 6, "color": "#0000ff"})).unwrap();
        let blue = |app: &PhotocraftApp, x: i32| active(app).surface().unwrap().rgba(x, 30)[0] < 0.5;
        app.ui.tool = Tool::ContentAwareMove;
        app.ui.tool_options.cam_transform_on_drop = false;
        app.ui.tool_options.cam_mode = mode.into();
        app.ui.tool_options.cam_structure = 7.0;
        let m = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 20.0, pressure: 1.0 }, m);
        for [x, y] in [[32.0, 20.0], [32.0, 40.0], [12.0, 40.0]] {
            tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
        }
        tool_event(&mut app, ToolEvent::Up { x: 12.0, y: 40.0 }, m);
        assert!(app.session.active().unwrap().doc.selection.is_some(), "lasso made a selection");
        assert!(blue(&app, 22), "the lasso alone changes no pixels");
        tool_event(&mut app, ToolEvent::Down { x: 22.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 31.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 72.0, y: 30.0 }, m);
        assert!(!app.ui.status_error, "{mode}: {}", app.ui.status);
        assert!(blue(&app, 72), "{mode}: the content lands where it was dropped");
        assert_eq!(blue(&app, 22), keeps, "{mode}: the original place");
        assert_eq!(app.session.journal.last().map(|(id, p)| (id.as_str(), p["mode"].as_str())), Some(("paint.contentAwareMove", Some(mode))));
        let sel = app.session.active().unwrap().doc.selection.as_ref().unwrap();
        assert!(sel.sample_channel(72, 30, 0) > 0.0 && sel.sample_channel(22, 30, 0) == 0.0, "{mode}: the selection follows");
    }
}

#[test]
fn content_aware_move_transform_on_drop_turns_the_patch_on_enter() {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[16, 30], [28, 30]], "size": 4, "color": "#0000ff"})).unwrap();
    let blue = |app: &PhotocraftApp, x: i32, y: i32| active(app).surface().unwrap().rgba(x, y)[0] < 0.5;
    app.ui.tool = Tool::ContentAwareMove;
    app.ui.tool_options.cam_transform_on_drop = true;
    app.ui.tool_options.cam_structure = 7.0;
    let m = egui::Modifiers::NONE;
    let drag = |app: &mut PhotocraftApp, path: &[[f64; 2]]| {
        let ([x, y], rest) = (path[0], &path[1..]);
        tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
        for &[x, y] in rest {
            tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
        }
        let [x, y] = path[path.len() - 1];
        tool_event(app, ToolEvent::Up { x, y }, m);
    };
    drag(&mut app, &[[6.0, 14.0], [38.0, 14.0], [38.0, 46.0], [6.0, 46.0]]);
    let steps = app.session.active().unwrap().history.past_len();
    // The drop puts a box on the patch; nothing is computed yet.
    drag(&mut app, &[[22.0, 30.0], [50.0, 30.0], [72.0, 30.0]]);
    assert!(crate::cam_transform::active(&app), "{}", app.ui.status);
    assert!(blue(&app, 22, 30) && !blue(&app, 72, 30), "the document waits for the commit");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    // Dragging outside the box turns it 90° about its centre (72, 30).
    drag(&mut app, &[[98.0, 55.0], [60.0, 58.0], [47.0, 56.0]]);
    let place = crate::cam_transform::placement(app.ui.cam_transform.as_ref().unwrap()).unwrap();
    assert_eq!((place.offset, place.scale), ([50, 0], [100.0, 100.0]));
    assert!((place.rotate - 90.0).abs() < 1e-3, "{place:?}");
    crate::cam_transform::commit(&mut app);
    assert!(!crate::cam_transform::active(&app) && !app.ui.status_error, "{}", app.ui.status);
    let (id, p) = app.session.journal.last().unwrap();
    assert_eq!((id.as_str(), &p["offset"]), ("paint.contentAwareMove", &json!([50, 0])));
    assert!((p["rotate"].as_f64().unwrap() - 90.0).abs() < 1e-3, "{p}");
    // The horizontal line lands upright, in one History step.
    assert!(blue(&app, 72, 26) && blue(&app, 72, 34), "the turned line");
    assert!(!blue(&app, 66, 30) && !blue(&app, 78, 30), "no unturned line");
    assert!(!blue(&app, 22, 30), "the old place is filled");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}

#[test]
fn red_eye_click_corrects_a_pupil_and_a_miss_adds_an_unchanged_state() {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[50, 30]], "size": 18, "hardness": 100, "color": "#f21818"})).unwrap();
    let before = active(&app).surface().unwrap().rgba(50, 30);
    assert!(before[0] > before[1] + 0.4, "{before:?}");
    app.ui.tool = Tool::RedEye;
    // Pupil Size 15 → search radius 22 px, so a click at (8,8) cannot reach the blob at (50,30).
    app.ui.tool_options.red_eye_pupil_size = 15.0;
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 30.0 }, egui::Modifiers::NONE);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("paint.redEye"));
    let after = active(&app).surface().unwrap().rgba(50, 30);
    assert!(after[0] < before[0] - 0.15, "before {before:?} after {after:?}");
    // A miss changes nothing but still adds a state, as in Photoshop (#12573).
    let steps = app.session.active().unwrap().history.past_len();
    tool_event(&mut app, ToolEvent::Down { x: 8.0, y: 8.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 8.0, y: 8.0 }, egui::Modifiers::NONE);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let history = &app.session.active().unwrap().history;
    assert_eq!(history.past_len(), steps + 1);
    assert_eq!(history.entries().last().map(|e| e.to_string()).as_deref(), Some("Red Eye Removal Tool"));
}

#[test]
fn remove_tool_stroke_removes_what_it_covers() {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[0, 30], [100, 30]], "size": 200, "color": "#808080"})).unwrap();
    app.run("paint.pencil", json!({"points": [[30, 20], [30, 40]], "size": 4, "color": "#000000"})).unwrap();
    let bg = active(&app).surface().unwrap().rgba(70, 30);
    app.ui.tool = Tool::Remove;
    let m = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 18.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 42.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 42.0 }, m);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("paint.remove"));
    let px = active(&app).surface().unwrap().rgba(30, 30);
    assert!(px.iter().zip(bg).all(|(a, b)| (a - b).abs() < 0.02), "{px:?} vs {bg:?}");
}

/// A waiting Remove stroke keeps the brush it was drawn with (#8923 review): changing the brush
/// before ↩ neither changes its tint nor what its removal takes.
#[test]
fn waiting_remove_strokes_keep_their_own_brush() {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[0, 30], [100, 30]], "size": 200, "color": "#808080"})).unwrap();
    app.ui.tool = Tool::Remove;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.ui.tool_options.remove_after_each_stroke = false;
    let m = egui::Modifiers::NONE;
    let stroke = |app: &mut PhotocraftApp, x: f64| {
        tool_event(app, ToolEvent::Down { x, y: 20.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Move { x, y: 40.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Up { x, y: 40.0 }, m);
    };
    app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 0.0}})).unwrap();
    stroke(&mut app, 25.0);
    let tint = |app: &PhotocraftApp| app.remove_trail.as_ref().map(|t| (0..100).filter(|&x| t.coverage(x, 30) > 0).count());
    let first = tint(&app);
    app.run("tools.setBrush", json!({"brush": {"size": 4, "hardness": 1.0}})).unwrap();
    stroke(&mut app, 75.0);
    assert!(tint(&app).unwrap() > first.unwrap(), "the second stroke adds its tint");
    assert!((0..100).filter(|&x| app.remove_trail.as_ref().unwrap().coverage(x, 30) > 0).any(|x| x < 25 - 10), "the first keeps its soft 20 px reach");
    assert!(remove_pending_commit(&mut app));
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let (_, p) = app.session.journal.last().cloned().expect("a journaled command");
    let sizes: Vec<_> = p["strokes"].as_array().unwrap().iter().map(|e| (e["size"].as_f64(), e["hardness"].as_f64())).collect();
    assert_eq!(sizes, [(Some(20.0), Some(0.0)), (Some(4.0), Some(100.0))], "each stroke is removed with its own brush");
}

/// With "Remove after each stroke" off the strokes wait; Escape drops them, Enter (the key route) and the
/// bar's answer (the bar route) each apply them in one command.
#[test]
fn remove_after_each_stroke_off_collects_strokes_until_enter_or_escape() {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[0, 30], [100, 30]], "size": 200, "color": "#808080"})).unwrap();
    app.run("paint.pencil", json!({"points": [[30, 20], [30, 40]], "size": 4, "color": "#000000"})).unwrap();
    app.ui.tool = Tool::Remove;
    app.ui.tool_options.remove_after_each_stroke = false;
    let m = egui::Modifiers::NONE;
    let collect = |app: &mut PhotocraftApp| {
        for x in [28.0, 32.0] {
            tool_event(app, ToolEvent::Down { x, y: 18.0, pressure: 1.0 }, m);
            tool_event(app, ToolEvent::Move { x, y: 42.0, pressure: 1.0 }, m);
            tool_event(app, ToolEvent::Up { x, y: 42.0 }, m);
        }
    };
    let journal = app.session.journal.len();
    collect(&mut app);

    assert!(crate::retouch_ui::bar::pending(&app, Tool::Remove).is_some(), "the strokes wait");
    assert_eq!(app.session.journal.len(), journal, "no command before Enter");
    assert!(app.remove_trail.is_some());
    // Escape drops them.
    crate::canvas::testing::key(&mut app, egui::Key::Escape, m);
    assert!(crate::retouch_ui::bar::pending(&app, Tool::Remove).is_none() && app.remove_trail.is_none(), "Escape drops the strokes");
    assert_eq!(app.session.journal.len(), journal);

    for by_key in [true, false] {
        let bg = active(&app).surface().unwrap().rgba(70, 30);
        collect(&mut app);
        assert!(crate::retouch_ui::bar::pending(&app, Tool::Remove).is_some(), "by_key {by_key}: collected again");
        if by_key {
            crate::canvas::testing::key(&mut app, egui::Key::Enter, m);
        } else {
            crate::retouch_ui::bar::answer(&mut app, Tool::Remove, true, false);
        }
        assert!(!app.ui.status_error, "by_key {by_key}: {}", app.ui.status);
        let (id, p) = app.session.journal.last().cloned().expect("a journaled command");
        assert_eq!(id, "paint.remove", "by_key {by_key}");
        assert_eq!(p["strokes"].as_array().map(Vec::len), Some(2), "by_key {by_key}");
        assert!(crate::retouch_ui::bar::pending(&app, Tool::Remove).is_none(), "by_key {by_key}: nothing left to apply");
        let px = active(&app).surface().unwrap().rgba(30, 30);
        assert!(px.iter().zip(bg).all(|(a, b)| (a - b).abs() < 0.02), "by_key {by_key}: {px:?} vs {bg:?}");
    }
}

#[test]
fn every_waiting_remove_stroke_is_tinted_in_full() {
    // #6555: each stroke used to be stamped only past the previous stroke's point count.
    let mut app = app();
    app.ui.tool = Tool::Remove;
    app.ui.tool_options.remove_after_each_stroke = false;
    let strokes: [&[[f64; 3]]; 3] = [
        &[[10.0, 10.0, 1.0], [40.0, 10.0, 1.0], [70.0, 10.0, 1.0]],
        &[[10.0, 30.0, 1.0], [40.0, 30.0, 1.0]],
        &[[10.0, 50.0, 1.0], [20.0, 50.0, 1.0], [30.0, 50.0, 1.0], [50.0, 50.0, 1.0], [90.0, 50.0, 1.0]],
    ];
    for s in strokes {
        remove_pending_add(&mut app, s);
    }
    assert_eq!(app.remove_pending.len(), 3);
    let t = app.remove_trail.as_ref().expect("the waiting strokes are tinted");
    let k = f64::from(t.scale());
    for s in strokes {
        for w in s.windows(2) {
            for i in 0..=8 {
                let f = f64::from(i) / 8.0;
                let (x, y) = (w[0][0] + (w[1][0] - w[0][0]) * f, w[0][1] + (w[1][1] - w[0][1]) * f);
                assert_eq!(t.coverage((x / k) as usize, (y / k) as usize), 255, "({x}, {y}) on a waiting stroke is tinted");
            }
        }
    }
    // Enter, Esc, another tool and another document all drop the tint with the strokes.
    assert!(remove_pending_commit(&mut app));
    assert!(app.remove_trail.is_none());
    remove_pending_add(&mut app, strokes[0]);
    assert!(remove_pending_cancel(&mut app) && app.remove_trail.is_none());
    remove_pending_add(&mut app, strokes[1]);
    app.ui.tool = Tool::SpotHealing;
    assert!(!remove_pending_live(&mut app) && app.remove_trail.is_none());
    app.ui.tool = Tool::Remove;
    remove_pending_add(&mut app, strokes[2]);
    app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    assert!(!remove_pending_live(&mut app) && app.remove_trail.is_none());
}

#[test]
fn queued_remove_cannot_commit_after_layer_or_channel_target_changes() {
    for change in ["layer", "color", "mask"] {
        let mut app = app();
        app.run("layer.new.layer", json!({"name": "Original"})).unwrap();
        if change == "color" {
            app.run("channel.target", json!({"channel": "red"})).unwrap();
        }
        app.ui.tool = Tool::Remove;
        app.ui.tool_options.remove_after_each_stroke = false;
        remove_pending_add(&mut app, &[[12.0, 10.0, 1.0], [16.0, 12.0, 1.0]]);
        assert_eq!(app.remove_pending.len(), 1);
        assert!(remove_pending_live(&mut app));

        match change {
            "layer" => {
                app.run("layer.new.layer", json!({"name": "Other"})).unwrap();
            }
            "color" => {
                app.run("channel.target", json!({"channel": "blue"})).unwrap();
            }
            _ => {
                app.run("layer.layerMask.revealAll", json!({})).unwrap();
                app.ui.mask_target = true;
            }
        }

        let journal = app.session.journal.len();
        assert!(!remove_pending_commit(&mut app), "{change}: Enter must not retarget a queued stroke");
        assert_eq!(app.session.journal.len(), journal, "{change}: no Remove command ran");
        assert!(app.remove_pending.is_empty() && app.remove_trail.is_none(), "{change}: queue cleared");
        assert!(app.remove_pending_target.is_none());
    }
}

#[test]
fn queued_remove_starts_a_new_queue_after_target_switch() {
    let mut app = app();
    app.run("layer.new.layer", json!({"name": "Original"})).unwrap();
    app.ui.tool = Tool::Remove;
    app.ui.tool_options.remove_after_each_stroke = false;
    remove_pending_add(&mut app, &[[12.0, 10.0, 1.0], [16.0, 12.0, 1.0]]);
    app.run("layer.new.layer", json!({"name": "Other"})).unwrap();
    remove_pending_add(&mut app, &[[40.0, 10.0, 1.0], [44.0, 12.0, 1.0]]);
    assert_eq!(app.remove_pending.len(), 1, "strokes from different layers must not be combined");
    assert_eq!(app.remove_pending[0]["points"][0][0], 40.0);
    assert!(remove_pending_live(&mut app));
}

#[test]
fn subset_remove_uses_the_existing_layer_without_losing_the_saved_new_layer_option() {
    let mut app = app();
    app.ui.tool = Tool::Remove;
    app.ui.tool_options.remove_new_layer = true;
    app.run("channel.target", json!({"colors":[0,2]})).unwrap();
    let layers = app.session.active().unwrap().doc.walk().len();
    tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 18.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 42.0 }, egui::Modifiers::NONE);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let (id, params) = app.session.journal.last().unwrap();
    assert_eq!(id, "paint.remove");
    assert_eq!(params["newLayer"], json!(false));
    assert_eq!(app.session.active().unwrap().doc.walk().len(), layers);
    assert!(app.ui.tool_options.remove_new_layer);
    app.run("channel.target", json!({"channel":"composite"})).unwrap();
    assert_eq!(remove_params(&app.ui.tool_options, true)["newLayer"], json!(true));
}

#[test]
fn pattern_stamp_stroke_paints_pixels_and_keeps_phase_when_aligned() {
    let mut app = app();
    assert!(finish_stroke(&mut app, Tool::PatternStamp, &[[20.0, 30.0, 1.0], [50.0, 30.0, 1.0]], Mods::default(), false));
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("paint.patternStamp"));
    assert!(app.ui.pattern_stamp_phase.is_some());
    let px = active(&app).surface().unwrap().rgba(30, 30);
    assert!(px[3] > 0.5, "the stamp paints through the brush: {px:?}");
    app.ui.tool_options.pattern_stamp_aligned = false;
    app.ui.pattern_stamp_phase = None;
    assert!(finish_stroke(&mut app, Tool::PatternStamp, &[[70.0, 30.0, 1.0]], Mods::default(), false));
    assert!(app.ui.pattern_stamp_phase.is_none(), "unaligned strokes do not keep phase");
}

#[test]
fn art_history_brush_keeps_its_own_opacity_and_reset_tool_restores_it() {
    let mut app = app();
    let ctx = app.ctx().clone();
    app.ui.tool = Tool::ArtHistoryBrush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"opacity": 0.4}})).unwrap();
    app.ui.tool = Tool::Brush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    assert_eq!(app.session.tools.brush.opacity, 1.0, "the Brush has its own opacity");
    app.ui.tool = Tool::ArtHistoryBrush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    assert_eq!(app.session.tools.brush.opacity, 0.4, "Art History keeps its own");
    crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET, json!({})).unwrap();
    assert_eq!(app.session.tools.brush.opacity, 1.0);
}

#[test]
fn history_source_caption_names_the_source() {
    use crate::retouch_ui::bar::history::{SourceCaption, source_caption};

    let mut app = app();
    assert!(matches!(source_caption(&app), SourceCaption::Opening), "a new document paints from its opening state");

    app.run("history.newSnapshot", json!({"name": "MySnap"})).unwrap();
    app.run("history.setSource", json!({"snapshot": 1})).unwrap();
    assert!(matches!(source_caption(&app), SourceCaption::Named(name) if name == "MySnap"));

    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("history.setSource", json!({"state": 1})).unwrap();
    assert!(matches!(source_caption(&app), SourceCaption::Named(name) if name.contains("Fill")));

    // Deleting the source snapshot leaves no source.
    app.run("history.setSource", json!({"snapshot": 1})).unwrap();
    app.run("history.deleteSnapshot", json!({"snapshot": 1})).unwrap();
    assert!(matches!(source_caption(&app), SourceCaption::Deleted), "deleted snapshot");

    // So does discarding the redo state that was the source.
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.run("history.setSource", json!({"state": 2})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    app.run("paint.pencil", json!({"points": [[10, 10]], "size": 1, "color": "#0000ff"})).unwrap();
    assert!(matches!(source_caption(&app), SourceCaption::Deleted), "discarded redo state");
}

/// An empty Clone Source slot refuses to paint, as with no source at all: the ⌥-click point left
/// in `ui.clone_source` carries no document, so it must not be read from another document.
#[test]
fn an_empty_clone_source_slot_refuses_to_paint() {
    let mut app = app();
    app.ui.tool = Tool::CloneStamp;
    crate::retouch_ui::set_source(&mut app, 10.0, 10.0);
    assert!(crate::retouch_ui::clone_params(&app).is_some());
    app.run("cloneSource.select", json!({"index": 1})).unwrap();
    assert!(app.ui.clone_source.is_some(), "the old ⌥-click point is still remembered");
    assert!(crate::retouch_ui::clone_params(&app).is_none());
}
