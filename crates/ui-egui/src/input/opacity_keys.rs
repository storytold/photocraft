//! Number keys set opacity (#352): 1 = 10% … 9 = 90%, 0 = 100%; two digits
//! typed quickly set an exact value (4 then 5 = 45%; 0 then 0 = 100% for the Brush and the Pencil, #11812). With the Brush, Pencil or
//! Eraser they set the brush's Opacity (⇧: Flow, swapped while airbrush build-up is on; the Pencil has no Flow, so its ⇧
//! digits set Opacity too, #11967); with the Gradient or Paint Bucket their
//! Opacity; with tools that have no opacity of their own (Move, the selection tools, Type…) the
//! selected layers' Opacity (⇧: Fill); with the Background Eraser its Tolerance (#11907); with Color
//! Replacement its Tolerance (#11927); with Magic Eraser its Opacity (#11999). Tools whose number
//! is a strength or exposure are left alone.
//! The `tools.smoothing.N` commands (⌥+digit) set the Smoothing of the tools that show it (Brush, Pencil, Eraser; #11892).
//!
//! The keys arrive as fired commands: the framework's `OpacityDigit` set (pair timing in `DigitKeys`) and the app
//! catalog's `tools.smoothing.N`. Each fired command is applied once by the pass that received it, so a second pass in
//! the same frame applies only the commands it received itself.

use craft::behavior::DigitKeys;
use craft::ui::canvas::OpacityDigit;
use craft::ui::runtime::commands::{CommandRef, FirePhase, Fired};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Prefix of the catalog commands `tools.smoothing.1` … `tools.smoothing.0` (⌥+digit).
const SMOOTHING_PREFIX: &str = "tools.smoothing.";

/// What a tool's number keys change.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    /// The shared brush settings' opacity (⇧: flow, when the tool has one).
    Brush {
        flow: bool,
    },
    /// The Gradient / Paint Bucket opacity option.
    ToolOpacity,
    /// The Background Eraser's Tolerance option.
    BgTolerance,
    /// Color Replacement's Tolerance option.
    CrTolerance,
    /// The Magic Eraser's Opacity option.
    MagicEraserOpacity,
    /// The selected layers' opacity (⇧: fill).
    Layer,
    None,
}

fn target(tool: Tool) -> Target {
    match tool {
        Tool::Brush | Tool::Eraser | Tool::CloneStamp | Tool::HistoryBrush => Target::Brush { flow: true },
        Tool::Pencil => Target::Brush { flow: false },
        Tool::Gradient | Tool::PaintBucket => Target::ToolOpacity,
        Tool::BackgroundEraser => Target::BgTolerance,
        Tool::ColorReplacement => Target::CrTolerance,
        Tool::MagicEraser => Target::MagicEraserOpacity,
        // Their options-bar number is a strength, exposure or tolerance, not this opacity.
        t if t.is_brushlike() => Target::None,
        _ => Target::Layer,
    }
}

/// The digit of a `tools.smoothing.N` command.
fn smoothing_digit(command: &CommandRef) -> Option<u8> {
    let digit = command.id.strip_prefix(SMOOTHING_PREFIX)?;
    let mut chars = digit.chars();
    let d = chars.next()?.to_digit(10)?;
    if chars.next().is_some() { None } else { u8::try_from(d).ok() }
}

/// The tools whose options bar has a Smoothing field, which `tools.smoothing.N` sets (#11892).
fn has_smoothing(tool: Tool) -> bool {
    matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser)
}

/// Applies one fired command if it is a number key or a Smoothing key the current tool takes; two digits pair by the
/// keys' own times (`Fired::time`). Returns whether the command was taken.
pub(crate) fn fired(app: &mut PhotocraftApp, f: &Fired) -> bool {
    if let Some(d) = smoothing_digit(&f.command) {
        if f.phase != FirePhase::Press || !has_smoothing(app.ui.tool) {
            return false;
        }
        // A Smoothing key ends a two-digit Opacity value: 4, ⌥3, 5 is 50%, not 45%.
        app.opacity_keys = DigitKeys::default();
        // One tens value each (1 = 10% … 0 = 100%), never two digits.
        let amount = if d == 0 { 1.0 } else { f64::from(d) / 10.0 };
        let _ = app.run("tools.setBrush", json!({ "brush": { "smoothing": { "enabled": true, "amount": amount } } }));
        return true;
    }
    if target(app.ui.tool) == Target::None {
        return false;
    }
    let Some(entry) = OpacityDigit::entry(&mut app.opacity_keys, f) else { return false };
    apply(app, entry.percent as f32, entry.second);
    true
}

fn apply(app: &mut PhotocraftApp, percent: f32, shift: bool) {
    let v = percent / 100.0;
    // With airbrush build-up on, the keys swap roles, as in Photoshop: plain digits set Flow, ⇧ digits Opacity.
    // A Pencil- or Block-mode Eraser has no build-up (its options bar disables Flow and the airbrush).
    let tool = app.ui.tool;
    let build_up = app.session.tools.brush.build_up && !crate::eraser_ui::pencil_mode(app, tool) && !crate::eraser_ui::block_mode(app, tool);
    let shift = shift ^ (build_up && matches!(target(tool), Target::Brush { flow: true }));
    match (target(app.ui.tool), shift) {
        // The Pencil has no Flow: its ⇧ digits set Opacity like the plain ones (#11967).
        (Target::Brush { flow: false }, _) | (Target::Brush { flow: true }, false) => {
            // The painting tools' Opacity fields (and `tools.setBrush`) start at 1%, so `00` sets the
            // 1% they actually paint with rather than a 0% they can't do. The Brush's and the
            // Pencil's `00` is 100%, as in Photoshop (#11812); only `00` gives 0 here.
            let v = match tool {
                Tool::Brush | Tool::Pencil if v <= 0.0 => 1.0,
                _ => v.max(0.01),
            };
            let _ = app.run("tools.setBrush", json!({ "brush": { "opacity": v } }));
        }
        // The options bar's Flow field starts at 1%.
        (Target::Brush { flow: true }, true) => {
            let _ = app.run("tools.setBrush", json!({ "brush": { "flow": v.max(0.01) } }));
        }
        (Target::ToolOpacity, false) => app.ui.tool_options.fill_opacity = percent,
        // The Tolerance field starts at 1%; `00` (0%) keeps today's value rather than guessing.
        (Target::BgTolerance, false) if percent >= 1.0 => app.ui.tool_options.bg_tolerance = percent,
        // The field starts at 1%; an unmeasured 00 keeps its existing value.
        (Target::CrTolerance, false) if percent >= 1.0 => app.ui.tool_options.cr_tolerance = percent,
        (Target::MagicEraserOpacity, _) if percent >= 1.0 => app.ui.tool_options.magic_eraser_opacity = percent,
        (Target::Layer, _) => {
            let Some(st) = app.session.active() else { return };
            // The locked Background has no opacity or fill. Batch the other layers into one undo step.
            let ids: Vec<u64> = st.selected_layers().iter().copied().filter(|id| !st.doc.is_background(*id)).map(|id| id.0).collect();
            let key = if shift { "fill" } else { "opacity" };
            let params = match ids.as_slice() {
                [] => return,
                // The recorder can make the single-layer target portable.
                [id] => json!({ "layer": id, key: v }),
                _ => json!({ "layers": ids, key: v }),
            };
            let _ = app.run("layer.setProps", params);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend_keys::test_fired;
    use OpacityDigit as D;
    use egui::{Event, Key};

    fn app(tool: Tool) -> Box<PhotocraftApp> {
        prepared(PhotocraftApp::new(photocraft_engine::Session::new(), Default::default()), tool)
    }

    /// The app on `ctx`, a harness's context that has the framework installed.
    fn app_on(ctx: egui::Context, tool: Tool) -> Box<PhotocraftApp> {
        prepared(PhotocraftApp::with_context(ctx, photocraft_engine::Session::new(), Default::default()), tool)
    }

    fn prepared(mut app: Box<PhotocraftApp>, tool: Tool) -> Box<PhotocraftApp> {
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = tool;
        app
    }

    /// Fires `command` at `now` seconds.
    fn fire(app: &mut PhotocraftApp, command: impl Into<CommandRef>, now: f64) -> bool {
        let mut f = test_fired(command.into());
        f.time = now;
        fired(app, &f)
    }

    /// Everything a number key can change: brush opacity, brush flow, layer opacity, layer fill, the Gradient and Paint
    /// Bucket opacity and the Background Eraser tolerance, each as a fraction.
    fn snap(app: &PhotocraftApp) -> [f32; 6] {
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let b = &app.session.tools.brush;
        [b.opacity, b.flow, l.opacity, l.fill_opacity, app.ui.tool_options.fill_opacity / 100.0, app.ui.tool_options.bg_tolerance / 100.0]
    }

    fn assert_snap(got: [f32; 6], base: [f32; 6], changes: &[(usize, f32)], what: &str) {
        let mut want = base;
        for (at, v) in changes {
            want[*at] = *v;
        }
        for i in 0..6 {
            assert!((got[i] - want[i]).abs() < 1e-6, "{what}: field {i} is {} not {}", got[i], want[i]);
        }
    }

    const OPACITY: usize = 0;
    const FLOW: usize = 1;
    const LAYER: usize = 2;
    const FILL: usize = 3;
    const TOOL_OPACITY: usize = 4;
    const TOLERANCE: usize = 5;

    #[test]
    fn digit_keys_set_each_tools_value() {
        // (name, tool, airbrush build-up, command, what changes)
        type Row<'a> = (&'a str, Tool, bool, D, &'a [(usize, f32)]);
        let rows: &[Row<'_>] = &[
            ("brush plain sets opacity", Tool::Brush, false, D::OpacityFive, &[(OPACITY, 0.5)]),
            ("brush shift sets flow", Tool::Brush, false, D::SecondTwo, &[(FLOW, 0.2)]),
            ("build-up swaps plain to flow", Tool::Brush, true, D::OpacityThree, &[(FLOW, 0.3)]),
            ("build-up swaps shift to opacity", Tool::Brush, true, D::SecondSix, &[(OPACITY, 0.6)]),
            ("pencil shift sets opacity", Tool::Pencil, false, D::SecondSeven, &[(OPACITY, 0.7)]),
            ("pencil has no build-up swap", Tool::Pencil, true, D::OpacityFour, &[(OPACITY, 0.4)]),
            ("history brush plain", Tool::HistoryBrush, false, D::OpacitySeven, &[(OPACITY, 0.7)]),
            ("history brush shift", Tool::HistoryBrush, false, D::SecondFour, &[(FLOW, 0.4)]),
            ("clone stamp plain", Tool::CloneStamp, false, D::OpacitySeven, &[(OPACITY, 0.7)]),
            ("clone stamp shift", Tool::CloneStamp, false, D::SecondFour, &[(FLOW, 0.4)]),
            ("paint bucket sets the tool opacity", Tool::PaintBucket, false, D::OpacityTwo, &[(TOOL_OPACITY, 0.2)]),
            ("gradient sets the tool opacity", Tool::Gradient, false, D::OpacityEight, &[(TOOL_OPACITY, 0.8)]),
            ("move plain sets the layer opacity", Tool::Move, false, D::OpacitySix, &[(LAYER, 0.6)]),
            ("move shift sets the layer fill", Tool::Move, false, D::SecondThree, &[(FILL, 0.3)]),
            ("background eraser sets the tolerance", Tool::BackgroundEraser, false, D::OpacityFive, &[(TOLERANCE, 0.5)]),
            ("background eraser shift changes nothing", Tool::BackgroundEraser, false, D::SecondFive, &[]),
            ("dodge's number is its exposure", Tool::Dodge, false, D::OpacityNine, &[]),
        ];
        for (name, tool, build_up, command, changes) in rows {
            let mut a = app(*tool);
            a.session.tools.brush.build_up = *build_up;
            let base = snap(&a);
            let taken = fire(&mut a, *command, 0.0);
            assert_eq!(taken, !changes.is_empty() || *tool == Tool::BackgroundEraser, "{name}: taken");
            assert_snap(snap(&a), base, changes, name);
        }
        // The Eraser swaps too, except in Pencil mode, which has no build-up.
        let mut e = app(Tool::Eraser);
        e.session.tools.brush.build_up = true;
        let base = snap(&e);
        fire(&mut e, D::OpacityTwo, 0.0);
        assert_snap(snap(&e), base, &[(FLOW, 0.2)], "eraser with build-up");
        e.ui.tool_options.eraser_mode = "pencil".into();
        fire(&mut e, D::OpacitySeven, 100.0);
        assert_snap(snap(&e), base, &[(FLOW, 0.2), (OPACITY, 0.7)], "pencil-mode eraser");
        // The locked Background keeps its opacity.
        let mut m = app(Tool::Move);
        let bg = m.session.active().unwrap().doc.layers[0].id;
        m.run("layer.select", json!({"layer": bg.0})).unwrap();
        fire(&mut m, D::OpacityFive, 0.0);
        assert_eq!(m.session.active().unwrap().doc.layers[0].opacity, 1.0);
        // A layer change is an edit.
        let mut l = app(Tool::Move);
        let rev = l.session.active().unwrap().revision;
        fire(&mut l, D::OpacitySix, 0.0);
        assert!(l.session.active().unwrap().revision > rev);
        // No document: nothing to set, nothing panics.
        let mut none = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        none.ui.tool = Tool::Move;
        fire(&mut none, D::OpacityFive, 0.0);
    }

    #[test]
    fn double_zero_gives_each_tool_its_value() {
        // (name, tool, the key typed twice, field, value after 00)
        let rows: &[(&str, Tool, D, usize, f32)] = &[
            ("brush 00 is 100%", Tool::Brush, D::OpacityZero, OPACITY, 1.0),
            ("pencil 00 is 100%", Tool::Pencil, D::OpacityZero, OPACITY, 1.0),
            ("brush shift 00 keeps flow's 1%", Tool::Brush, D::SecondZero, FLOW, 0.01),
            ("history brush 00 floors at 1%", Tool::HistoryBrush, D::OpacityZero, OPACITY, 0.01),
            ("clone stamp 00 floors at 1%", Tool::CloneStamp, D::OpacityZero, OPACITY, 0.01),
            // The first 0 sets 100%; the pair's 0% keeps it.
            ("background eraser 00 keeps the tolerance", Tool::BackgroundEraser, D::OpacityZero, TOLERANCE, 1.0),
        ];
        for (name, tool, command, field, want) in rows {
            let mut a = app(*tool);
            // A start away from every result, so a missed update fails.
            a.run("tools.setBrush", json!({ "brush": { "opacity": 0.5, "flow": 0.5 } })).unwrap();
            fire(&mut a, *command, 0.0);
            fire(&mut a, *command, 0.1);
            let got = snap(&a)[*field];
            assert!((got - want).abs() < 1e-6, "{name}: {got} not {want}");
        }
    }

    #[test]
    fn two_digits_pair_only_within_the_window() {
        let mut a = app(Tool::Brush);
        fire(&mut a, D::OpacityFour, 0.0);
        fire(&mut a, D::OpacityFive, 0.1);
        assert!((snap(&a)[OPACITY] - 0.45).abs() < 1e-6);
        // A second digit after the window starts over.
        fire(&mut a, D::OpacityFour, 10.0);
        fire(&mut a, D::OpacityFive, 12.0);
        assert!((snap(&a)[OPACITY] - 0.5).abs() < 1e-6);
        // A release types no digit.
        let mut release = test_fired(D::OpacityThree.into());
        release.phase = FirePhase::Release;
        release.time = 20.0;
        assert!(!fired(&mut a, &release));
        assert!((snap(&a)[OPACITY] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn smoothing_commands_set_the_smoothing_of_the_tools_that_show_it() {
        for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser] {
            for (d, amount) in [(3u8, 0.3f32), (0, 1.0)] {
                let mut a = app(tool);
                a.run("tools.setBrush", json!({ "brush": { "smoothing": { "enabled": false, "amount": 0.5, "pulledString": true } } })).unwrap();
                // A first digit pending, then a Smoothing key, then a digit: not a pair.
                fire(&mut a, D::OpacityFour, 0.0);
                assert!(fire(&mut a, CommandRef::new(format!("tools.smoothing.{d}")), 0.1), "{tool:?}");
                let s = &a.session.tools.brush.smoothing;
                assert!(s.enabled && (s.amount - amount).abs() < 1e-6, "{tool:?} {d}: {s:?}");
                assert!(s.pulled_string, "the other smoothing options stay");
                fire(&mut a, D::OpacityFive, 0.2);
                assert!((snap(&a)[OPACITY] - 0.5).abs() < 1e-6, "{tool:?}: 4, smoothing, 5 is 50%, not 45%");
            }
        }
        // A tool without a Smoothing field leaves the key alone.
        let mut m = app(Tool::Move);
        let before = m.session.tools.brush.smoothing.clone();
        assert!(!fire(&mut m, CommandRef::new("tools.smoothing.5"), 0.0));
        assert_eq!(m.session.tools.brush.smoothing.amount, before.amount);
        assert_eq!(m.session.tools.brush.smoothing.enabled, before.enabled);
    }

    #[test]
    fn a_single_layer_opacity_key_records_a_portable_action_step() {
        let mut a = app(Tool::Move);
        a.run("actions.record", json!({ "name": "Set layer opacity" })).unwrap();
        assert!(fire(&mut a, D::OpacityFive, 0.0));
        a.run("actions.stop", json!({})).unwrap();

        let action = a.run("actions.get", json!({ "action": "Set layer opacity" })).unwrap();
        assert_eq!(action["steps"], json!([["layer.setProps", { "opacity": 0.5 }]]));
    }

    #[test]
    fn layer_digit_keys_set_all_selected_groups_in_one_step() {
        use crate::shortcut_dispatch::tests::tap;
        use egui::Modifiers;
        use egui_kittest::Harness;
        use photocraft_doc::LayerId;

        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app_on(cc.egui_ctx.clone(), Tool::Move)
        });
        let a = h.state_mut();
        let st = a.session.active().unwrap();
        let anchor = st.active_layer.unwrap();
        let background = st.doc.layers[0].id;
        let groups = [("First", 70), ("Second", 80), ("Unselected", 90)].map(|(name, opacity)| {
            // Add each group beside the same raster layer, so the groups are siblings.
            a.run("layer.select", json!({ "layer": anchor.0 })).unwrap();
            LayerId(a.run("layer.new.group", json!({ "name": name, "opacity": opacity })).unwrap()["layer"].as_u64().unwrap())
        });
        let [first, second, unselected] = groups;
        a.run("layer.select", json!({ "layer": first.0 })).unwrap();
        for id in [background, second] {
            a.run("layer.select", json!({ "layer": id.0, "mode": "add" })).unwrap();
        }
        h.run_steps(4);

        let values = |h: &Harness<'_, Box<PhotocraftApp>>| {
            let st = h.state().session.active().unwrap();
            assert!(st.doc.is_background(background), "the selected Background stays locked");
            [first, second, unselected, background].map(|id| {
                let layer = st.doc.layer(id).unwrap();
                (layer.opacity, layer.fill_opacity)
            })
        };
        let history = |h: &Harness<'_, Box<PhotocraftApp>>| h.state().session.active().unwrap().history.past_len();
        let original = [(0.7, 1.0), (0.8, 1.0), (0.9, 1.0), (1.0, 1.0)];
        assert_eq!(values(&h), original);
        let before = history(&h);

        tap(&mut h, Key::Num4, Modifiers::NONE);
        assert_eq!(values(&h), [(0.4, 1.0), (0.4, 1.0), (0.9, 1.0), (1.0, 1.0)], "both selected groups change opacity");
        assert_eq!(history(&h), before + 1);
        tap(&mut h, Key::Z, Modifiers::COMMAND);
        assert_eq!(values(&h), original, "one Undo restores both opacities");

        tap(&mut h, Key::Num6, Modifiers::SHIFT);
        assert_eq!(values(&h), [(0.7, 0.6), (0.8, 0.6), (0.9, 1.0), (1.0, 1.0)], "Shift changes Fill for both selected groups");
        assert_eq!(history(&h), before + 1);
        tap(&mut h, Key::Z, Modifiers::COMMAND);
        assert_eq!(values(&h), original, "one Undo restores both Fill values");
    }

    #[test]
    fn layer_digit_keys_apply_to_more_than_ten_thousand_selected_layers() {
        use photocraft_doc::{ColorMode, Document, Layer, SampleType, Size};

        let mut a = app(Tool::Move);
        let mut doc = Document::new("Large selection", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        doc.layers.extend((0..10_002).map(|_| Layer::group("Group", Vec::new())));
        let selected: Vec<_> = doc.layers.iter().take(10_001).map(|layer| layer.id).collect();
        a.session.add_document(doc, None);
        let st = a.session.active_mut().unwrap();
        st.set_active_layer(selected.last().copied());
        *st.selection_mut() = selected;
        let before = st.history.past_len();

        assert!(fire(&mut a, D::OpacityFive, 0.0));
        let st = a.session.active().unwrap();
        assert!(st.doc.layers.iter().take(10_001).all(|layer| layer.opacity == 0.5));
        assert_eq!(st.doc.layers.last().unwrap().opacity, 1.0);
        assert_eq!(st.history.past_len(), before + 1);

        assert!(a.session.undo());
        assert!(a.session.active().unwrap().doc.layers.iter().all(|layer| layer.opacity == 1.0));
    }

    /// Through the whole app: number keys and ⇧[ ⇧] from the canvas (no widget focused).
    #[test]
    fn keys_work_in_the_app() {
        use egui_kittest::Harness;
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut a = app_on(cc.egui_ctx.clone(), Tool::Brush);
            a.run("tools.setBrush", json!({"brush": {"hardness": 0.5, "opacity": 1.0, "flow": 1.0}})).unwrap();
            a
        });
        h.run_steps(4);
        let key = |h: &mut Harness<'_, Box<PhotocraftApp>>, k: Key, m: egui::Modifiers| {
            h.event(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
            h.event(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
            h.run_steps(2);
        };
        key(&mut h, Key::Num5, egui::Modifiers::NONE);
        key(&mut h, Key::CloseBracket, egui::Modifiers::SHIFT);
        let b = &h.state().session.tools.brush;
        assert_eq!((b.opacity, b.hardness), (0.5, 0.75));
        key(&mut h, Key::OpenBracket, egui::Modifiers::SHIFT);
        key(&mut h, Key::OpenBracket, egui::Modifiers::SHIFT);
        key(&mut h, Key::OpenBracket, egui::Modifiers::SHIFT);
        assert_eq!(h.state().session.tools.brush.hardness, 0.0, "stops at 0%");
        let size = h.state().session.tools.brush.size;
        key(&mut h, Key::CloseBracket, egui::Modifiers::NONE);
        assert!(h.state().session.tools.brush.size > size, "] still resizes");
    }
}
