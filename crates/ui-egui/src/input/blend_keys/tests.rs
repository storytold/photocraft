use craft::ui::runtime::commands::CommandRef;

use super::*;

fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = tool;
    app
}

fn layer_blend(app: &PhotocraftApp) -> BlendMode {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().blend
}

fn blend_of(app: &PhotocraftApp, i: usize) -> BlendMode {
    app.session.active().unwrap().doc.layers[i].blend
}

/// Fires one blend key command through the module's entry point; returns whether it was taken.
fn key(app: &mut PhotocraftApp, command: impl Into<CommandRef>) -> bool {
    fired(app, &test_fired(command.into()))
}

fn undo_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// Every blend key is taken by the module, whatever the target does with it, so the letter never reaches the tool keys.
#[test]
fn blend_keys_reach_each_tools_target() {
    use BlendShortcut as S;
    // Tools with a Mode of their own: (tool, key, mode after). The layer stays Normal.
    let own: &[(Tool, BlendShortcut, BlendMode)] = &[
        (Tool::Brush, S::Multiply, BlendMode::Multiply),
        (Tool::Pencil, S::Screen, BlendMode::Screen),
        (Tool::CloneStamp, S::Screen, BlendMode::Screen),
        (Tool::PatternStamp, S::Overlay, BlendMode::Overlay),
        (Tool::HistoryBrush, S::Darken, BlendMode::Darken),
        (Tool::Healing, S::Luminosity, BlendMode::Luminosity),
        // Overlay isn't in the Healing menu: nothing is picked.
        (Tool::Healing, S::Overlay, BlendMode::Normal),
    ];
    for (tool, letter, mode) in own {
        let mut a = app(*tool);
        assert!(key(&mut a, *letter), "{tool:?} {letter:?}: taken");
        assert_eq!(tool_mode(&mut a), Some(*mode), "{tool:?} {letter:?}");
        assert_eq!(layer_blend(&a), BlendMode::Normal, "{tool:?} leaves the layer alone");
        assert_eq!(a.ui.tool, *tool);
    }
    // The stamps and the History Brush have no Clear; the Gradient has neither Clear nor Behind.
    for tool in [Tool::CloneStamp, Tool::PatternStamp, Tool::HistoryBrush, Tool::Gradient] {
        let mut a = app(tool);
        let before = tool_mode(&mut a);
        assert!(key(&mut a, S::Clear));
        assert_eq!(tool_mode(&mut a), before, "{tool:?} has no Clear");
    }
    // Each stamp's mode is its own brush's: stepping moves it, and another tool's brush keeps its own.
    let mut a = app(Tool::Brush);
    key(&mut a, S::Multiply);
    a.ui.tool = Tool::CloneStamp;
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Normal), "starts in Normal");
    key(&mut a, BlendStep::Next);
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Dissolve));
    a.ui.tool = Tool::Brush;
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Multiply), "the Brush keeps its own mode");
    // The Gradient takes the letters into its option.
    let mut a = app(Tool::Gradient);
    key(&mut a, S::Overlay);
    assert_eq!(a.ui.tool_options.gradient_blend_mode, BlendMode::Overlay);
    // The Paint Bucket has its own Mode, Clear included; Clear is followed by Darken.
    let mut a = app(Tool::PaintBucket);
    key(&mut a, S::Multiply);
    assert_eq!(a.ui.tool_options.bucket_blend_mode, BlendMode::Multiply);
    key(&mut a, S::Clear);
    assert!(a.ui.tool_options.bucket_clear);
    key(&mut a, BlendStep::Next);
    assert!(!a.ui.tool_options.bucket_clear);
    assert_eq!(a.ui.tool_options.bucket_blend_mode, BlendMode::Darken);
    assert_eq!(layer_blend(&a), BlendMode::Normal);
    // The Healing Brush steps within its short list and wraps; a mode outside it counts as Normal.
    let mut a = app(Tool::Healing);
    key(&mut a, S::Luminosity);
    key(&mut a, BlendStep::Next);
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Normal), "wraps within the Healing list");
    key(&mut a, BlendStep::Next);
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Multiply));
    let mut a = app(Tool::Healing);
    crate::paint_mouse::sync_tool_brush(&mut a);
    a.session.tools.brush.mode = BlendMode::Overlay;
    key(&mut a, BlendStep::Next);
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Multiply), "steps from Normal");
}

/// The Brush's Clear and Behind rows and the steps through the menu's rows, and the Pencil's own Clear.
#[test]
fn brush_blend_keys_walk_the_menus_rows() {
    use BlendShortcut as S;
    // The Brush: Clear is a menu row after Dissolve and Behind, and the steps walk the menu's rows.
    let mut a = app(Tool::Brush);
    key(&mut a, S::Clear);
    assert!(a.ui.tool_options.brush_clear);
    key(&mut a, S::PassThrough);
    assert!(a.ui.tool_options.brush_clear, "no Pass Through for a brush");
    key(&mut a, S::Dissolve);
    assert!(!a.ui.tool_options.brush_clear);
    assert_eq!(a.session.tools.brush.mode, BlendMode::Dissolve);
    key(&mut a, BlendStep::Next);
    assert!(a.session.tools.brush.behind, "Behind follows Dissolve");
    key(&mut a, BlendStep::Next);
    assert!(a.ui.tool_options.brush_clear && !a.session.tools.brush.behind, "Clear follows Behind");
    key(&mut a, BlendStep::Next);
    assert!(!a.ui.tool_options.brush_clear);
    assert_eq!(a.session.tools.brush.mode, BlendMode::Darken);
    assert_eq!(layer_blend(&a), BlendMode::Normal, "the layer is untouched");
    // The Pencil keeps its own Clear.
    a.ui.tool = Tool::Pencil;
    assert!(!a.ui.tool_options.pencil_clear);
    key(&mut a, S::Clear);
    assert!(a.ui.tool_options.pencil_clear);
    // Shift- from Normal wraps to the menu's last row.
    let mut a = app(Tool::Brush);
    key(&mut a, BlendStep::Previous);
    assert_eq!(a.session.tools.brush.mode, BlendMode::LAYER_MODES[BlendMode::LAYER_MODES.len() - 1]);
    assert_eq!(layer_blend(&a), BlendMode::Normal);
    // Other commands are not blend keys.
    assert!(!key(&mut a, CommandRef::new("tools.smoothing.3")));
}

/// Tools without a Mode control take the key and leave the layer alone; tools without a mode of their own set the
/// layer; an app without a document takes the key and does nothing.
#[test]
fn blend_keys_are_taken_by_tools_without_a_mode() {
    use BlendShortcut as S;
    // Tools whose Mode has no control here, and Dodge, Burn and Sponge (whose ⇧⌥ letters are their own Range and
    // mode in Photoshop): the command is taken, nothing changes, the layer is never reached.
    for tool in [Tool::SpotHealing, Tool::Blur, Tool::Sharpen, Tool::Smudge, Tool::ArtHistoryBrush, Tool::Dodge, Tool::Burn, Tool::Sponge] {
        let mut a = app(tool);
        let undo = undo_len(&a);
        for letter in [S::Multiply, S::Screen, S::ColorDodge, S::Overlay] {
            assert!(key(&mut a, letter), "{tool:?}");
        }
        assert!(key(&mut a, BlendStep::Next), "{tool:?}");
        assert_eq!(layer_blend(&a), BlendMode::Normal, "{tool:?}");
        assert_eq!(undo_len(&a), undo, "{tool:?}");
        assert_eq!(a.ui.tool, tool);
    }
    // Tools without a blend mode of their own set the layer.
    for (tool, letter, mode) in [
        (Tool::Eraser, S::Multiply, BlendMode::Multiply),
        (Tool::MagicEraser, S::Screen, BlendMode::Screen),
        (Tool::MixerBrush, S::Difference, BlendMode::Difference),
    ] {
        let mut a = app(tool);
        key(&mut a, letter);
        assert_eq!(layer_blend(&a), mode, "{tool:?}");
        assert_eq!(a.ui.tool, tool);
    }
    // A document-less app takes the key and does nothing.
    let mut none = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    none.ui.tool = Tool::Move;
    assert!(key(&mut none, BlendStep::Next));
}

/// Color Replacement's letters set the active layer and leave the tool's Mode.
#[test]
fn color_replacement_blend_keys_set_the_layer() {
    use BlendShortcut as S;
    // Color Replacement: four letters set the active layer and the tool's Mode stays (#11928).
    let mut a = app(Tool::ColorReplacement);
    let tool_mode_before = tool_mode(&mut a);
    for (letter, mode) in [(S::Multiply, BlendMode::Multiply), (S::Hue, BlendMode::Hue), (S::Luminosity, BlendMode::Luminosity), (S::Normal, BlendMode::Normal)]
    {
        key(&mut a, letter);
        assert_eq!(layer_blend(&a), mode, "{letter:?}");
    }
    assert_eq!(tool_mode(&mut a), tool_mode_before);
    // Another letter, a step, Clear and Behind do nothing.
    key(&mut a, S::Multiply);
    for command in [CommandRef::from(S::Screen), S::Clear.into(), S::Behind.into(), BlendStep::Next.into(), BlendStep::Previous.into()] {
        assert!(key(&mut a, command.clone()), "{command}: taken");
        assert_eq!(layer_blend(&a), BlendMode::Multiply, "{command}");
    }
    // Several selected layers: nothing.
    let ids: Vec<u64> = a.session.active().unwrap().doc.layers.iter().map(|l| l.id.0).collect();
    for id in &ids {
        a.run("layer.select", json!({"layer": id, "mode": "add"})).unwrap();
    }
    assert!(a.session.active().unwrap().selected_layers().len() > 1);
    let undo = undo_len(&a);
    key(&mut a, S::Hue);
    assert_eq!(undo_len(&a), undo);
    assert_eq!(layer_blend(&a), BlendMode::Multiply);
}

/// ⇧+ and ⇧− step the active layer's mode with a layer-target tool, one undo step each, wrapping at the Background's
/// Normal; a letter sets the layer and keeps the tool.
#[test]
fn layer_mode_keys_set_the_selected_layers_in_one_step() {
    use BlendShortcut as S;
    let mut a = app(Tool::Move);
    let undo = undo_len(&a);
    key(&mut a, BlendStep::Next);
    assert_eq!(layer_blend(&a), BlendMode::Dissolve);
    key(&mut a, BlendStep::Next);
    assert_eq!(layer_blend(&a), BlendMode::Darken);
    key(&mut a, BlendStep::Previous);
    assert_eq!(layer_blend(&a), BlendMode::Dissolve);
    assert_eq!(undo_len(&a), undo + 3, "one undo step per press");
    key(&mut a, S::Multiply);
    assert_eq!(layer_blend(&a), BlendMode::Multiply);
    assert_eq!(a.ui.tool, Tool::Move, "not the Marquee tool");
    assert_eq!(undo_len(&a), undo + 4);
    // Clear, Behind and Pass Through are not layer modes here.
    for letter in [S::Clear, S::Behind, S::PassThrough] {
        assert!(key(&mut a, letter));
    }
    assert_eq!(layer_blend(&a), BlendMode::Multiply);
    // The Background keeps Normal when stepped.
    let bg = a.session.active().unwrap().doc.layers[0].id;
    a.run("layer.select", json!({"layer": bg.0})).unwrap();
    key(&mut a, BlendStep::Next);
    assert_eq!(blend_of(&a, 0), BlendMode::Normal);

    // Every selected layer takes the mode in one undo step; the Background keeps Normal.
    let mut a = app(Tool::Move);
    let ids: Vec<u64> = a.session.active().unwrap().doc.layers.iter().map(|l| l.id.0).collect();
    a.run("layer.new.layer", json!({})).unwrap();
    for id in &ids {
        a.run("layer.select", json!({"layer": id, "mode": "add"})).unwrap();
    }
    assert_eq!(a.session.active().unwrap().selected_layers().len(), 3);
    let undo = undo_len(&a);
    key(&mut a, S::Screen);
    assert_eq!([blend_of(&a, 0), blend_of(&a, 1), blend_of(&a, 2)], [BlendMode::Normal, BlendMode::Screen, BlendMode::Screen]);
    assert_eq!(undo_len(&a), undo + 1, "one undo step for both layers");
    key(&mut a, BlendStep::Next);
    assert_eq!([blend_of(&a, 1), blend_of(&a, 2)], [BlendMode::ColorDodge, BlendMode::ColorDodge]);
    assert_eq!(undo_len(&a), undo + 2);
    a.session.undo();
    assert_eq!([blend_of(&a, 1), blend_of(&a, 2)], [BlendMode::Screen, BlendMode::Screen]);

    // Pass Through is for groups only.
    let mut a = app(Tool::Move);
    key(&mut a, S::PassThrough);
    assert_eq!(layer_blend(&a), BlendMode::Normal, "a plain layer has no Pass Through");
    a.run("layer.new.group", json!({})).unwrap();
    key(&mut a, S::Normal);
    assert_eq!(layer_blend(&a), BlendMode::Normal);
    key(&mut a, S::PassThrough);
    assert_eq!(layer_blend(&a), BlendMode::PassThrough);
}

/// With a Free Transform box open the keys still go where the tool sends them and never close the box.
#[test]
fn keys_follow_the_tool_while_a_transform_is_open() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Brush);
    a.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    a.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    a.run("select.deselect", json!({})).unwrap();
    a.sync_views();
    crate::transform_tool::begin(&mut a, &ctx).unwrap();
    let moved = {
        use craft::behavior::kurbo::Vec2;
        use craft::behavior::{EditEvent, Mixed};
        use craft::draw::ui::transform::TransformEdit;
        let session = &mut a.ui.transform.as_mut().unwrap().session;
        let before = *session.state();
        let after = before.moved(Vec2::new(3.0, 2.0));
        session.apply(TransformEdit::Box(EditEvent::set(Mixed::Same(before), after, "Move".into(), None)), 0.0);
        after
    };
    key(&mut a, BlendShortcut::Multiply);
    assert_eq!(a.session.tools.brush.mode, BlendMode::Multiply, "the Brush's own mode");
    assert_eq!(layer_blend(&a), BlendMode::Normal, "the layer is left alone");
    assert!(a.ui.transform.is_some(), "the box stays open");
    assert_eq!(*a.ui.transform.as_ref().unwrap().session.state(), moved, "the box keeps its place");
    assert_eq!(a.ui.tool, Tool::Brush, "no tool letter");
    a.ui.tool = Tool::Move;
    key(&mut a, BlendShortcut::Difference);
    assert_eq!(layer_blend(&a), BlendMode::Difference);
    assert!(a.ui.transform.is_some());
    // PLAN: a Puppet or Perspective Warp keeps these keys to itself; that rule is `run_fired`'s (top-w29), which
    // decides before calling `fired`, so no trial here can reach it.
}
