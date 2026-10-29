//! Photoshop shows the active function in the pointer: held ⌘ (Ctrl off the Mac) is the move
//! cursor and ⌥ added to it the white-and-black duplicate pair (`canvas::move_pointer`), the
//! Move tool's own drag the same, and the lasso family, the Magic Wand and the Object Selection
//! tool carry their tool glyph as Photoshop's standard cursors (Other Cursors: Precise stays the
//! bare crosshair, like the pens and the bucket).

use egui::{CursorIcon, Event, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{Drag, ViewXform, move_pointer, move_pointer_copy};
use crate::state::Tool;

fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn doc_screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> egui::Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    let xf = ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    };
    xf.to_screen(x, y)
}

/// The pointer over the document point with `m` held: the OS icon and whether a bitmap cursor
/// (the brush tip, the crosshair) replaced it instead of an Area-drawn tool icon.
fn pointer_over(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) -> (CursorIcon, Option<[u16; 2]>) {
    h.event(Event::ModifiersChanged(m));
    let p = doc_screen(h, x, y);
    h.event(Event::PointerMoved(p));
    h.run_steps(2);
    let out = h.output();
    (out.platform_output.cursor_icon, out.platform_output.cursor_image.as_ref().map(|i| i.size))
}

/// ⌘ held with the Brush hands the pointer to the move icon: the brush tip's bitmap is gone and
/// the move glyph is drawn at the point; releasing ⌘ brings the tip back.
#[test]
fn cmd_held_replaces_the_brush_tip_with_the_move_pointer() {
    let mut h = harness(Tool::Brush);
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 80}})).unwrap();
    h.run_steps(1);
    let (_, tip) = pointer_over(&mut h, 150.0, 100.0, Modifiers::NONE);
    let side = tip.expect("the brush tip is a bitmap cursor")[0];
    assert!(side > 60, "an 80 px brush tip: {side}");
    let tip_shapes = h.output().shapes.len();
    let (icon, img) = pointer_over(&mut h, 150.0, 100.0, Modifiers::COMMAND);
    assert_eq!((icon, img), (CursorIcon::None, None), "⌘: the move icon is drawn at the point itself");
    assert!(h.output().shapes.len() > tip_shapes, "the move glyph is on screen");
    let (_, back) = pointer_over(&mut h, 150.0, 100.0, Modifiers::NONE);
    assert_eq!(back.map(|s| s[0]), Some(side), "the Brush is back when ⌘ comes up");
}

/// Caps Lock swaps the move-function pointer to the precise crosshair like the other drawn
/// glyphs (#4639's rule): held ⌘ shows the move glyph under Standard and the crosshair once
/// Caps is on. The Move tool's own resting pointer is left alone (#5646, pinned by
/// caps_lock_tests::caps_lock_leaves_non_painting_tools_cursors_alone) — the swap only
/// reaches the pointer where this PR introduced a function glyph.
#[test]
fn caps_lock_swaps_the_move_function_to_the_precise_crosshair() {
    let mut h = harness(Tool::Brush);
    let (icon, _) = pointer_over(&mut h, 150.0, 100.0, Modifiers::COMMAND);
    assert_eq!(icon, CursorIcon::None, "standard: held ⌘ shows the move glyph at the point");
    h.state_mut().caps_lock = true;
    h.run_steps(2);
    let (icon, _) = pointer_over(&mut h, 150.0, 100.0, Modifiers::COMMAND);
    assert_eq!(icon, CursorIcon::Crosshair, "Caps Lock: the swap takes the function to the precise crosshair");
    h.state_mut().caps_lock = false;
    h.run_steps(2);
    let (icon, _) = pointer_over(&mut h, 150.0, 100.0, Modifiers::COMMAND);
    assert_eq!(icon, CursorIcon::None, "and back to the glyph when Caps comes off");
}

/// #6102: hovering with the Move tool and ⌥ held shows the white-and-black duplicate pair at
/// the point instead of the standard arrow; releasing ⌥ gives the arrow back (#5646).
#[test]
fn alt_hover_with_the_move_tool_shows_the_duplicate_pair() {
    let mut h = harness(Tool::Move);
    let (icon, _) = pointer_over(&mut h, 150.0, 100.0, Modifiers::NONE);
    assert_eq!(icon, CursorIcon::Default, "at rest the Move tool keeps the arrow (#5646)");
    let plain_shapes = h.output().shapes.len();
    let (icon, _) = pointer_over(&mut h, 150.0, 100.0, Modifiers { alt: true, ..Default::default() });
    assert_eq!(icon, CursorIcon::None, "⌥ hover hands the pointer to the drawn pair");
    assert!(h.output().shapes.len() > plain_shapes, "the duplicate pair is on screen");
    let (icon, _) = pointer_over(&mut h, 150.0, 100.0, Modifiers::NONE);
    assert_eq!(icon, CursorIcon::Default, "the arrow returns when ⌥ comes up");
}

/// The lasso family and Object Selection keep the crosshair and add their glyph beside it; the
/// Magic Wand is its own drawn pointer. Other Cursors: Precise takes the glyph away.
#[test]
fn selection_tools_show_their_glyph_and_precise_takes_it_away() {
    for tool in [Tool::Lasso, Tool::PolygonLasso, Tool::ObjectSelection, Tool::MagicWand] {
        let mut h = harness(tool);
        let (_, _) = pointer_over(&mut h, 150.0, 100.0, Modifiers::NONE);
        let standard = h.output().shapes.len();
        h.state_mut().session.prefs.edit(|p| p.cursors.other = photocraft_engine::prefs::OtherCursor::Precise);
        h.run_steps(4);
        let (icon, _) = pointer_over(&mut h, 150.0, 100.0, Modifiers::NONE);
        assert_eq!(icon, CursorIcon::Crosshair, "{tool:?}: Precise is the crosshair");
        assert!(standard > h.output().shapes.len(), "{tool:?}: the standard cursor draws the tool glyph too");
    }
}

/// The pure decision: the temporary Move, a Move-tool drag, or the Move tool with ⌥ held
/// (#6102) shows the move pointer, and the duplicate pair follows ⌥ — from the drag's press
/// modifiers once a drag runs, else from the held keys.
#[test]
fn the_move_pointer_and_its_duplicate_are_decided_from_the_modifiers() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let cmd = Modifiers::COMMAND;
    let alt = Modifiers { alt: true, ..Default::default() };
    assert!(!move_pointer(&app, None, Modifiers::NONE), "no ⌘, no drag: the tool's own pointer");
    assert!(move_pointer(&app, Some(crate::hold_keys::Temporary::Move), cmd), "held ⌘ is the move pointer");
    assert!(!move_pointer_copy(&app, cmd), "⌘ alone moves");
    assert!(move_pointer_copy(&app, Modifiers { alt: true, command: true, ..Default::default() }), "⌘⌥ duplicates");

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.ui.tool = Tool::Move;
    assert!(!move_pointer(&app, None, Modifiers::NONE), "the Move tool at rest keeps the arrow (#5646)");
    assert!(move_pointer(&app, None, alt), "#6102: ⌥ hover with the Move tool is the duplicate pointer");
    assert!(move_pointer_copy(&app, alt), "and it is the pair, not the plain move glyph");
    let plain = Drag::new(Tool::Move, [10.0, 10.0], vec![[10.0, 10.0, 1.0]], Modifiers::NONE, false);
    let with_alt = Drag::new(Tool::Move, [10.0, 10.0], vec![[10.0, 10.0, 1.0]], Modifiers { alt: true, ..Default::default() }, false);
    app.drag = Some(plain);
    assert!(move_pointer(&app, None, alt), "a Move-tool drag shows the move pointer");
    assert!(!move_pointer_copy(&app, alt), "a drag without ⌥ on the press moves, never copies");
    app.drag = Some(with_alt);
    assert!(move_pointer_copy(&app, Modifiers::NONE), "⌥ on the press keeps the duplicate until release");
}

/// Canvas-level version of the press-time rule (#6102 and the PR review's ask): a real Move
/// drag driven through pointer events — not a direct call of the pure decision — so the
/// RENDERED pointer is asserted end-to-end. What the press committed holds until release:
/// ⌥ on the press draws the duplicate pair (eighteen arrowhead images over the baseline) and
/// lifting ⌥ mid-drag keeps the pair; a plain press draws the single move glyph and ⌥ going
/// down mid-drag does not upgrade it. The shape counts below were measured in these exact
/// states, so the assertions compare drawn frames, not internals.
#[test]
fn a_move_drag_keeps_the_pointer_of_its_press_across_modifier_changes() {
    let mut h = harness(Tool::Move);
    h.state_mut().run("layer.new.layer", json!({"name": "Box"})).unwrap();
    h.state_mut()
        .session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(150, 100, 250, 200), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    h.run_steps(2);
    let c = doc_screen(&h, 200.0, 150.0);
    let alt = Modifiers { alt: true, ..Default::default() };
    let press = |h: &mut Harness<'static, PhotocraftApp>, pos: egui::Pos2, pressed: bool, m: Modifiers| {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: m });
        h.run_steps(2);
    };
    let move_to = |h: &mut Harness<'static, PhotocraftApp>, pos: egui::Pos2| {
        h.event(Event::PointerMoved(pos));
        h.run_steps(2);
    };
    let drawn = |h: &Harness<'static, PhotocraftApp>| h.output().shapes.len();
    let icon = |h: &Harness<'static, PhotocraftApp>| h.output().platform_output.cursor_icon;

    // ⌥ on the press, then a real drag (egui starts it only once the pointer moves): the
    // drawn pair, and lifting ⌥ mid-drag changes the drawn frame not at all until release.
    h.event(Event::ModifiersChanged(alt));
    move_to(&mut h, c);
    press(&mut h, c, true, alt);
    move_to(&mut h, c + vec2(16.0, 12.0));
    assert!(h.state().drag.is_some(), "the ⌥ press and move started a drag");
    assert_eq!(icon(&h), CursorIcon::None, "the ⌥ drag runs on the drawn pair");
    let copy_drag = drawn(&h);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    move_to(&mut h, c + vec2(30.0, 22.0));
    assert_eq!(icon(&h), CursorIcon::None, "⌥ up mid-drag: still a drawn pointer, not the arrow");
    assert!((drawn(&h) as i32 - copy_drag as i32).abs() <= 4, "the pair of the press holds until release");
    press(&mut h, c + vec2(30.0, 22.0), false, Modifiers::NONE);

    // No ⌥ on the press: the single move glyph; ⌥ going down mid-drag does not add the
    // pair's images — the press decided the function, as the pointer then shows until release.
    move_to(&mut h, c);
    press(&mut h, c, true, Modifiers::NONE);
    move_to(&mut h, c + vec2(18.0, 6.0));
    assert!(h.state().drag.is_some(), "the plain press and move started a drag");
    assert_eq!(icon(&h), CursorIcon::None, "the plain drag runs on the drawn glyph");
    let move_drag = drawn(&h);
    h.event(Event::ModifiersChanged(alt));
    move_to(&mut h, c + vec2(34.0, 16.0));
    assert_eq!(icon(&h), CursorIcon::None, "⌥ down mid-drag: still the glyph's own drawn pointer");
    assert!((drawn(&h) as i32 - move_drag as i32).abs() <= 4, "⌥ down mid-drag does not upgrade the drag's pointer");
    assert!(copy_drag as i32 - drawn(&h) as i32 >= 16, "the pair drag draws many more shapes than the glyph drag: {copy_drag} vs {}", drawn(&h));
    press(&mut h, c + vec2(34.0, 16.0), false, alt);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let (rest, _) = pointer_over(&mut h, 200.0, 150.0, Modifiers::NONE);
    assert_eq!(rest, CursorIcon::Default, "released and idle: the Move tool's resting arrow again (#5646)");
}
