//! Levels/Curves must use the same grayscale target in their controls, pickers and commands.
use craft::automation::actions::ActionStep;
use craft::automation::ui::actions::ActionSelection;
use craft::ui::dialog::DialogId;
use craft::ui::runtime::memory::{saved_value, set_saved_value};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, DocId, Document, Layer, LayerContent, LayerMask};
use photocraft_engine::{Session, actions_cmds::Action, channel_cmds::ChannelTarget};
use photocraft_geom::Size;
use photocraft_raster::Surface;
use photocraft_ui_egui::control::{ControlRequest, Reply, handle};
use photocraft_ui_egui::{PhotocraftApp, adjust_dialog, adjust_preview, dialogs, filter_dialog};
use serde_json::{Value, json};

const MODES: [ColorMode; 3] = [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab];
const GRAY_TARGET_MODES: [ColorMode; 4] = [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Grayscale];
const DEPTHS: [SampleType; 3] = [SampleType::U8, SampleType::U16, SampleType::F32];
const TARGETS: [ChannelTarget; 3] = [ChannelTarget::Alpha(0), ChannelTarget::LayerMask, ChannelTarget::Composite];
/// The whole-app picker tests check what the UI adds to the engine's gray picker: the canvas press reaches the
/// armed dialog (`canvas/tools/dialog.rs` -> `canvas_input::pick_tone`), and `adjust_dialog::pick` puts the
/// dialog's target into the parameters and writes the result back into the dialog. That code branches on the
/// dialog kind, the target and on RGB / gray / other documents (`picker.supported`), never on the bit depth, so
/// these tests run every kind and target on [`RGB_8`] and visit the other modes once. Sampling the right plane at
/// every colour mode x native depth is `destructive_eyedropper`'s, tested without the app in the engine's
/// `tone_target_picker.rs` (#15648). The full cross product ran a single test past nextest's 600 s limit on
/// Windows (#15636).
const RGB_8: (ColorMode, SampleType) = (ColorMode::Rgb, SampleType::U8);

/// The keys the hosted adjustment dialog lists beside its engine parameters.
const DIALOG_STATE: [&str; 4] = ["preview", "picker", "channel", "clipping"];

/// Build a native-depth document with distinct colour pixels, a named alpha, a layer mask and optional Quick Mask.
fn fixture(mode: ColorMode, depth: SampleType, target: ChannelTarget) -> Box<PhotocraftApp> {
    app_of(fixture_session(mode, depth, target))
}

/// The app over a fixture's session, its jobs run in place.
fn app_of(session: Session) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(session, Default::default());
    app.background_jobs = false;
    app
}

/// [`fixture`]'s session.
fn fixture_session(mode: ColorMode, depth: SampleType, target: ChannelTarget) -> Session {
    let mut doc = Document::new("Tone targets", Size::new(32, 32), mode, depth);
    let mut pixels = Surface::new(PixelFormat::new(mode, depth, true));
    pixels.fill_rect(
        doc.bounds(),
        &match mode {
            ColorMode::Cmyk => vec![0.1, 0.2, 0.3, 0.4, 1.0],
            ColorMode::Lab => vec![0.5, 0.5, 0.5, 1.0],
            ColorMode::Grayscale => vec![0.25, 1.0],
            _ => vec![0.0, 0.0, 0.0, 1.0],
        },
    );
    let mut gray = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    gray.fill_rect(doc.bounds(), &[0.25]);
    let mut layer = Layer::new("Named layer", LayerContent::Raster(pixels));
    layer.mask = Some(LayerMask { surface: gray.clone(), ..LayerMask::reveal_all() });
    let id = layer.id;
    doc.layers.push(layer);
    doc.channels.push(AlphaChannel::new("Saved sky", gray.clone()));
    if target == ChannelTarget::Composite {
        doc.quick_mask = Some(AlphaChannel::new("Quick Mask", gray));
    }
    let mut session = Session::new();
    session.add_document(doc, None);
    session.select_layer(id).unwrap();
    session.active_mut().unwrap().channel_view.target = target;
    session
}

/// Read a representative sample from the fixture’s selected scalar plane.
fn value(app: &PhotocraftApp, target: ChannelTarget) -> f32 {
    let doc = &app.session.active().unwrap().doc;
    match target {
        ChannelTarget::Alpha(i) => doc.channels[i].surface.sample_channel(8, 8, 0),
        ChannelTarget::LayerMask => doc.layers[0].mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
        ChannelTarget::Composite => doc.quick_mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
        _ => panic!("not a gray target"),
    }
}

/// Render real modal controls with accessibility enabled for input-driven assertions.
/// The app's context is set up already ([`PhotocraftApp::new`]).
fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| dialogs::run(app, ui), app)
}

/// The document canvas and the dialogs over it: a picker's samples are pointer requests the canvas pass takes, as in
/// the app. The rest of the window (panels, menus, bars) plays no part in sampling, so it is left out.
fn canvas_harness(mut app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    // The window's frame keeps a view per document; the canvas alone draws the views it is given.
    app.sync_views();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1200.0, 900.0)).with_max_steps(64).build_ui_state(
        |ui, app| {
            egui::CentralPanel::default().show(ui, |ui| photocraft_ui_egui::canvas::document_area(app, ui));
            dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(3);
    show_left_of_dialog(&mut h);
    h
}

/// The dialog's window is centred and stops a press under it: at 12x with the view centred right of the 32 px image,
/// the image's top-left part lies clear of the dialog, at the canvas's left.
fn show_left_of_dialog(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let reply = control(h, "ui.set", json!({"zoom": 12, "center": [38, 16]}));
    assert_eq!(reply["ok"], true, "the view zooms: {reply}");
}

/// One [`canvas_harness`] for all the cases of a test: [`next_case`] puts each case's document in it.
fn bench() -> Harness<'static, Box<PhotocraftApp>> {
    canvas_harness(fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Composite))
}

/// The next case of a test on the bench `h`: the active document of `from` (a fixture's session) opens as a new
/// document in place of what `h` shows, with its active layer and channel target, and `filter_mask` is the Layers
/// panel's filter mask target. What an earlier case left goes first: the open documents, the actions, the Layers
/// panel's mask targets and the tone dialogs' last-used settings.
fn next_case(h: &mut Harness<'static, Box<PhotocraftApp>>, from: &Session, filter_mask: Option<u64>) {
    for kind in ["levels", "curves"] {
        set_saved_value(&h.ctx, &adjust_dialog::last_key(kind), None::<Value>);
    }
    let app = h.state_mut();
    assert_eq!(app.dialog_count(), 0, "the last case closed its dialog");
    while app.session.close(0).is_some() {}
    let st = from.active().unwrap();
    let mut doc = (*st.doc).clone();
    // A document of its own, as in a case's own app: nothing the app keeps per document carries over.
    doc.id = DocId::fresh();
    app.session.add_document(doc, None);
    app.session.select_layer(st.active_layer.unwrap()).unwrap();
    app.session.active_mut().unwrap().channel_view = st.channel_view.clone();
    app.session.actions = from.actions.clone();
    app.ui.mask_target = false;
    app.ui.vector_mask_target = false;
    app.ui.filter_mask_target = filter_mask;
    app.sync_views();
    h.run_steps(3);
    show_left_of_dialog(h);
}

/// Open a neutral tone dialog on `h` and settle its layout before interacting with controls.
fn open_in(mut h: Harness<'static, Box<PhotocraftApp>>, kind: &str) -> Harness<'static, Box<PhotocraftApp>> {
    open_here(&mut h, kind);
    h
}

/// [`open_in`] on a harness kept for more cases.
fn open_here(h: &mut Harness<'static, Box<PhotocraftApp>>, kind: &str) {
    adjust_dialog::open(h.state_mut(), &format!("image.adjustments.{kind}")).unwrap();
    h.run_steps(4);
}

/// Open a neutral tone dialog and settle its layout before interacting with controls.
fn open(app: Box<PhotocraftApp>, kind: &str) -> Harness<'static, Box<PhotocraftApp>> {
    open_in(harness(app), kind)
}

/// [`open`] on the whole app, for the tests that sample the canvas.
fn open_on_canvas(app: Box<PhotocraftApp>, kind: &str) -> Harness<'static, Box<PhotocraftApp>> {
    open_in(canvas_harness(app), kind)
}

/// The top dialog of the stack.
fn top(h: &Harness<'static, Box<PhotocraftApp>>) -> DialogId {
    h.state().top_dialog().expect("a dialog is open")
}

/// One control request through the public channel, then a few frames; the reply is the request's answer.
fn control(h: &mut Harness<'static, Box<PhotocraftApp>>, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (request, _reply) = ControlRequest::new(method, params);
    let reply = match handle(h.state_mut(), &ctx, &request) {
        Reply::Done(reply) => reply,
        _ => Value::Null,
    };
    h.run_steps(4);
    reply
}

/// The fields of the top dialog as `ui.inspect` lists them: the typed value beside `preview`, `picker`, `channel` and `clipping`.
fn fields(h: &Harness<'static, Box<PhotocraftApp>>) -> Value {
    let inspected = photocraft_ui_egui::control::inspect(h.state(), &h.ctx);
    inspected["dialogs"].as_array().and_then(|dialogs| dialogs.last()).map(|dialog| dialog["fields"].clone()).unwrap_or(Value::Null)
}

/// The engine command parameters of the top modal: its value over the parameters it opened with.
fn params(h: &Harness<'static, Box<PhotocraftApp>>) -> Value {
    adjust_dialog::command_params(h.state(), top(h))
}

/// Sets one field of dialog `id`: the dialog's own state (`preview`, `picker`, `channel`, `clipping`) as `ui.dialog.set` does, and any other
/// key as an engine command parameter laid over the dialog's current parameters.
fn set(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId, field: &str, value: Value) {
    if DIALOG_STATE.contains(&field) {
        let reply = control(h, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}));
        assert_eq!(reply["ok"], true, "{field}: {reply}");
        return;
    }
    let mut shown = adjust_dialog::command_params(h.state(), id);
    shown[field] = value;
    assert!(adjust_dialog::apply_params(h.state_mut(), id, &shown), "{field}: the dialog takes the parameter");
    h.run_steps(4);
}

/// OK on dialog `id` through the host.
fn confirm(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId) -> Result<(), String> {
    let reply = control(h, "ui.dialog.confirm", json!({"dialog": id}));
    if reply["ok"] == true { Ok(()) } else { Err(reply["error"].as_str().unwrap_or("not confirmed").to_string()) }
}

/// Cancel on dialog `id` through the host.
fn cancel(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId) {
    let reply = control(h, "ui.dialog.cancel", json!({"dialog": id}));
    assert_eq!(reply["ok"], true, "the dialog cancels: {reply}");
}

/// The rectangle of the window that draws the dialog's OK button.
fn dialog_rect(h: &Harness<'static, Box<PhotocraftApp>>) -> Option<egui::Rect> {
    let ok = h.query_by_label("OK")?.rect().center();
    let layer = h.ctx.layer_id_at(ok)?;
    h.ctx.memory(|memory| memory.area_rect(layer.id))
}

/// Read the visible selected labels of the dialog’s channel dropdowns.
fn names(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    let Some(window) = dialog_rect(h) else { return Vec::new() };
    h.query_all_by_role(egui::accesskit::Role::ComboBox).filter(|n| window.contains_rect(n.rect())).filter_map(|n| n.value()).collect()
}

/// Type into the actual Levels gamma control and blur it to commit the value.
fn edit_gamma(h: &mut Harness<'static, Box<PhotocraftApp>>, gamma: &str) {
    h.get_by_label("Gamma").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text(gamma.into()));
    h.run_steps(1);
    h.get_by_label("Preview").click();
    h.run_steps(2);
}

/// Alt turns Cancel into Reset: hold Alt and press the Reset button, as a user does.
fn reset(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.run_steps(2);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    h.run_steps(2);
    let at = h.get_by_label("Reset").rect().center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::ALT };
    for event in [egui::Event::PointerMoved(at), button(true), button(false)] {
        h.event(event);
        h.run_steps(1);
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

/// Arms the tone picker `picker` (`black`, `gray` or `white`) on the top dialog, as `ui.dialog.set` does.
fn arm(h: &mut Harness<'static, Box<PhotocraftApp>>, picker: &str) {
    let id = top(h);
    set(h, id, "picker", json!(picker));
}

/// Whether a tone picker is armed on the top dialog.
fn picker_armed(h: &Harness<'static, Box<PhotocraftApp>>) -> bool {
    !fields(h)["picker"].is_null()
}

/// A canvas press at document point `(x, y)`, as `ui.pointer` makes it; the armed picker samples there.
fn sample_at(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f64, y: f64) {
    control(h, "ui.pointer", json!({"events": [{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]}));
}

/// Adds the action `name` with one step to the session's list; `dialog` makes the step show its dialog when played.
fn add_action(app: &mut PhotocraftApp, name: &str, command: impl Into<String>, params: Value, dialog: bool) {
    let mut action = Action::new(name, vec![ActionStep::new(command, params)]);
    if dialog {
        action.dialogs.insert(0);
    }
    app.session.actions.list.push(action).unwrap();
}

/// The parameters the first step of the first action is recorded with.
fn recorded_params(h: &Harness<'static, Box<PhotocraftApp>>) -> Value {
    h.state().session.actions.list.to_file().actions[0].steps[0].params.clone()
}

/// Plays `action` on `h`, which pauses at its dialog.
fn play(h: &mut Harness<'static, Box<PhotocraftApp>>, action: &str) {
    h.state_mut().run("actions.play", json!({"action": action})).unwrap();
    h.run_steps(4);
}

/// Seeds the settings the dialog recalls with ⌥, as an earlier OK leaves them.
/// The last-used settings live in the saved tier under the dialog kind; seeding them directly lets a test store settings an OK would
/// never write (malformed, address-bearing).
fn remember_last(h: &Harness<'static, Box<PhotocraftApp>>, command: &str, last: Value) {
    set_saved_value(&h.ctx, &adjust_dialog::last_key(kind_of(command)), Some(last));
}

/// The dialog kind of an `image.adjustments.<kind>` command.
fn kind_of(command: &str) -> &str {
    command.strip_prefix("image.adjustments.").unwrap_or(command)
}

/// The last-used settings of `command`.
fn remembered(h: &Harness<'static, Box<PhotocraftApp>>, command: &str) -> Value {
    saved_value::<Option<Value>>(&h.ctx, &adjust_dialog::last_key(kind_of(command))).unwrap_or(Value::Null)
}

/// Real Levels gamma edits affect the named scalar target and agree with preview, commit and undo.
#[test]
fn levels_gray_target_controls_change_the_target_in_every_mode_and_depth() {
    for mode in GRAY_TARGET_MODES {
        for depth in DEPTHS {
            for target in TARGETS {
                let mut h = open(fixture(mode, depth, target), "levels");
                let name = match target {
                    ChannelTarget::Alpha(_) => "Saved sky",
                    ChannelTarget::LayerMask => "Named layer Mask",
                    _ => "Quick Mask",
                };
                assert_eq!(names(&h), [name], "{mode:?} {depth:?} {target:?}");
                // Type into the actual visible gamma field and blur it without confirming.
                edit_gamma(&mut h, "2.0");
                assert_eq!(params(&h)["gamma"], 2.0);
                let st = h.state().session.active().unwrap();
                let pixels = st.doc.layers[0].content.clone();
                let history = st.history.past_len();
                let before = value(h.state(), target);
                // The standalone preview helper starts from a composite session. Supply the
                // plane that the live preview request captures from Channels/Quick Mask.
                let mut preview_params = params(&h);
                preview_params["target"] = match target {
                    ChannelTarget::Alpha(i) => json!({"channel":i}),
                    ChannelTarget::LayerMask => json!("mask"),
                    _ => json!("quickMask"),
                };
                let preview = filter_dialog::preview_document(&st.doc, st.active_layer, "image.adjustments.levels", &preview_params, 1).unwrap();
                let preview_value = match target {
                    ChannelTarget::Alpha(i) => preview.channels[i].surface.sample_channel(8, 8, 0),
                    ChannelTarget::LayerMask => preview.layers[0].mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
                    _ => preview.quick_mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
                };
                let id = top(&h);
                confirm(&mut h, id).unwrap();
                let after = value(h.state(), target);
                assert_eq!(preview_value, after, "the gamma preview matches committed native samples");
                assert!(after > before + 0.1, "{mode:?} {depth:?} {target:?}: {before} -> {after}");
                assert_eq!(h.state().session.active().unwrap().doc.layers[0].content, pixels);
                assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
                h.state_mut().run("edit.undo", json!({})).unwrap();
                assert_eq!(value(h.state(), target), before);
                h.state_mut().run("edit.redo", json!({})).unwrap();
                assert_eq!(value(h.state(), target), after);
            }
        }
    }
}

/// Each RGB/CMYK/Lab component exposes only its own Levels record and preserves other components.
#[test]
fn levels_isolates_each_colour_channel_and_edits_its_matching_record() {
    for mode in MODES {
        let labels: &[&str] = match mode {
            ColorMode::Rgb => &["Red", "Green", "Blue"],
            ColorMode::Cmyk => &["Cyan", "Magenta", "Yellow", "Black"],
            ColorMode::Lab => &["Lightness", "a", "b"],
            _ => unreachable!(),
        };
        let keys = photocraft_engine::adjust_params::channel_keys(photocraft_engine::adjust_params::default_space(mode));
        for depth in DEPTHS {
            for (k, label) in labels.iter().enumerate() {
                let mut app = fixture(mode, depth, ChannelTarget::Color(k));
                // Nonzero samples let gamma change each RGB component as well as Lab/CMYK.
                app.session
                    .edit("Colour pattern", |doc, _| {
                        let bounds = doc.bounds();
                        let n = mode.color_channels();
                        let mut values = vec![0.25; n + 1];
                        values[n] = 0.6;
                        doc.layers[0].surface_mut().unwrap().fill_rect(bounds, &values);
                        Ok(())
                    })
                    .unwrap();
                let mut h = open(app, "levels");
                assert_eq!(names(&h), [*label], "{mode:?} {depth:?} {k}");
                let before = params(&h);
                let original = h.state().session.active().unwrap().doc.clone();
                edit_gamma(&mut h, "2.0");
                let after = params(&h);
                assert_eq!(after[keys[k]]["gamma"], 2.0);
                for (i, key) in keys.iter().enumerate().filter(|(i, _)| *i != k) {
                    assert_eq!(after[key], before[key], "untargeted {i}");
                }
                let id = top(&h);
                confirm(&mut h, id).unwrap();
                let doc = &h.state().session.active().unwrap().doc;
                let old = original.layers[0].surface().unwrap().pixel(8, 8);
                let new = doc.layers[0].surface().unwrap().pixel(8, 8);
                assert!((new[k] - old[k]).abs() > 0.01, "{mode:?} {depth:?} {k}: {old:?} -> {new:?}");
                for i in (0..old.len()).filter(|i| *i != k) {
                    assert_eq!(new[i], old[i], "untargeted sample/transparency {i}");
                }
                assert_eq!(doc.channels, original.channels);
                assert_eq!(doc.layers[0].mask, original.layers[0].mask);
            }
        }
    }
}

/// Spot names stay literal and Layers-panel mask targeting exposes the same named gray controls.
#[test]
fn levels_uses_literal_spot_name_and_layers_panel_mask_name() {
    for mode in GRAY_TARGET_MODES {
        let mut app = fixture(mode, SampleType::U8, ChannelTarget::Alpha(0));
        app.session
            .edit("Named spot", |doc, _| {
                doc.channels[0].name = "Sky α — ink 1".into();
                doc.channels[0].spot = Some((photocraft_color::Color::rgb(0.2, 0.4, 0.6), 0.8));
                Ok(())
            })
            .unwrap();
        assert_eq!(names(&open(app, "levels")), ["Sky α — ink 1"]);

        let mut app = fixture(mode, SampleType::U8, ChannelTarget::LayerMask);
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
        app.ui.mask_target = true;
        let mut h = open(app, "levels");
        assert_eq!(names(&h), ["Named layer Mask"]);
        assert_eq!(params(&h)["target"], "mask");
        reset(&mut h);
        assert_eq!(names(&h), ["Named layer Mask"]);
        assert_eq!(params(&h)["target"], "mask");
    }
}

/// Levels Auto derives endpoints from the selected component’s histogram rather than the composite.
#[test]
fn levels_auto_uses_the_selected_colour_channels_histogram() {
    let mut app = fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Color(1));
    app.session
        .edit("Distinct colour components", |doc, _| {
            let bounds = doc.bounds();
            doc.layers[0].mask = None;
            doc.layers[0].surface_mut().unwrap().fill_rect(bounds, &[0.1, 0.4, 0.8, 1.0]);
            Ok(())
        })
        .unwrap();
    let mut h = open(app, "levels");
    assert_eq!(names(&h), ["Green"]);
    let before = params(&h);
    h.get_by_label("Auto").click();
    h.run_steps(2);
    let after = params(&h);
    assert_eq!(after["green"]["inBlack"], 102.0);
    assert_eq!(after["green"]["inWhite"], 104.0);
    for key in ["red", "blue", "inBlack", "inWhite"] {
        assert_eq!(after[key], before[key]);
    }
}

/// Black/white picker controls sample the gray plane and update the visible neutral curve: every dialog and target
/// on an RGB document, then a grayscale, a CMYK and a Lab document once each (see [`RGB_8`]).
#[test]
fn grayscale_picker_controls_sample_the_plane_and_change_the_visible_curve() {
    let mut h = bench();
    let mut cases = Vec::new();
    for kind in ["curves", "levels"] {
        for target in TARGETS {
            cases.push((kind, RGB_8.0, RGB_8.1, target));
        }
    }
    cases.extend([
        ("curves", ColorMode::Grayscale, SampleType::U16, ChannelTarget::LayerMask),
        ("levels", ColorMode::Grayscale, SampleType::F32, ChannelTarget::Composite),
        ("levels", ColorMode::Cmyk, SampleType::U16, ChannelTarget::Alpha(0)),
        ("curves", ColorMode::Lab, SampleType::F32, ChannelTarget::Alpha(0)),
    ]);
    for (kind, mode, depth, target) in cases {
        next_case(&mut h, &fixture_session(mode, depth, target), None);
        open_here(&mut h, kind);
        assert!(h.get_by_label("Set Neutral Gray Point").accesskit_node().is_disabled());
        assert!(!h.get_by_label("Set White Point").accesskit_node().is_disabled());
        h.get_by_label("Set White Point").click();
        h.run_steps(2);
        assert!(picker_armed(&h));
        let before = params(&h);
        let original = h.state().session.active().unwrap().doc.clone();
        sample_at(&mut h, 4.0, 4.0);
        let after = params(&h);
        let key = if kind == "curves" { "points" } else { "inWhite" };
        assert_ne!(before[key], after[key], "{kind} {mode:?} {depth:?} {target:?}");
        let sampled_level =
            if kind == "curves" { after["points"].as_array().unwrap().last().unwrap()[0].as_f64().unwrap() } else { after["inWhite"].as_f64().unwrap() };
        assert!((sampled_level - 63.75).abs() < 0.3, "sample the plane's 25% gray, not the colour layer: {after}");
        for key in ["red", "green", "blue"] {
            assert_eq!(before[key], after[key], "hidden {key} must not change");
        }
        assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        let id = top(&h);
        confirm(&mut h, id).unwrap();
        assert!(value(h.state(), target) > 0.99, "{kind} {mode:?} {depth:?} {target:?}");
        assert_eq!(h.state().session.active().unwrap().doc.layers[0].content, original.layers[0].content);
    }
}

/// Replay a recorded tone step with its dialog enabled while a different channel is selected; `on_canvas` runs the
/// whole app, for the tests that sample the canvas.
fn action_dialog(mode: ColorMode, target: Value, panel: ChannelTarget, kind: &str, on_canvas: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = fixture(mode, SampleType::U8, panel);
    app.run("actions.record", json!({"name": "Tone step"})).unwrap();
    // Imported/serialized steps can carry document-space defaults alongside their explicit
    // grayscale target. Those records must not leak back into the dialog after normalization.
    let mut recorded = photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for(kind, mode).unwrap());
    recorded["target"] = target;
    for key in ["red", "green", "blue"] {
        recorded[key] = if kind == "levels" { json!({"gamma": 4}) } else { json!([[0, 255], [255, 255]]) };
    }
    app.run(&format!("image.adjustments.{kind}"), recorded).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    app.run("actions.dialog", json!({"action": "Tone step", "step": 0, "enabled": true})).unwrap();
    let mut h = if on_canvas { canvas_harness(app) } else { harness(app) };
    let result = h.state_mut().run("actions.play", json!({"action": "Tone step"})).unwrap();
    assert!(result.get("paused").is_some(), "{result:?}");
    h.run_steps(4);
    h
}

/// A recorded alpha target overrides a currently selected colour component in controls and commit.
#[test]
fn action_dialog_uses_recorded_alpha_instead_of_current_red_channel() {
    for mode in MODES {
        for kind in ["curves", "levels"] {
            let mut h = action_dialog(mode, json!({"channel": 0}), ChannelTarget::Color(0), kind, false);
            assert_eq!(names(&h), ["Saved sky"]);
            assert_eq!(params(&h)["target"], json!({"channel": 0}));
            let neutral = photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for(kind, ColorMode::Grayscale).unwrap());
            for key in ["red", "green", "blue"] {
                // A gray plane's command parameters carry no colour records (the engine drops them); one that is carried is neutral.
                assert!(params(&h).get(key).is_none_or(|record| *record == neutral[key]), "hidden {key} starts neutral: {}", params(&h));
            }
            for key in ["cyan", "black", "lightness", "a", "b"] {
                assert!(params(&h).get(key).is_none());
            }
            let id = top(&h);
            let expected = if kind == "curves" {
                set(&mut h, id, "points", json!([[0, 255], [255, 0]]));
                0.75
            } else {
                edit_gamma(&mut h, "2.0");
                0.5
            };
            let before = h.state().session.active().unwrap().doc.layers[0].content.clone();
            confirm(&mut h, id).unwrap();
            assert!((value(h.state(), ChannelTarget::Alpha(0)) - expected).abs() < 0.01);
            assert_eq!(h.state().session.active().unwrap().doc.layers[0].content, before);
        }
    }
}

/// Reset, pencil conversion and sampling retain an Action’s explicit mask target.
#[test]
fn action_mask_target_survives_reset_pencil_and_picker() {
    for kind in ["curves", "levels"] {
        let mut h = action_dialog(ColorMode::Lab, json!("mask"), ChannelTarget::Alpha(0), kind, true);
        assert_eq!(names(&h), ["Named layer Mask"]);
        let id = top(&h);
        reset(&mut h);
        assert_eq!(params(&h)["target"], "mask");
        if kind == "curves" {
            h.get_by_label("Draw to modify the curve").click();
            h.run_steps(2);
            assert_eq!(params(&h)["target"], "mask");
            assert_eq!(params(&h)["pencil"], true);
        }
        h.get_by_label("Set White Point").click();
        h.run_steps(2);
        sample_at(&mut h, 4.0, 4.0);
        assert_eq!(params(&h)["target"], "mask");
        let alpha = h.state().session.active().unwrap().doc.channels[0].clone();
        confirm(&mut h, id).unwrap();
        assert!(value(h.state(), ChannelTarget::LayerMask) > 0.99);
        assert_eq!(h.state().session.active().unwrap().doc.channels[0], alpha);
    }
}

/// Explicit scalar Action parameters use the channel preview path independently of panel state.
#[test]
fn explicit_channel_action_uses_channel_preview_even_with_composite_selected() {
    let mut h = action_dialog(ColorMode::Rgb, json!({"channel":0}), ChannelTarget::Color(0), "curves", false);
    h.state_mut().session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
    assert!(!adjust_preview::on_layer(h.state_mut(), 0));
    assert_eq!(names(&h), ["Saved sky"]);
    let id = top(&h);
    set(&mut h, id, "points", json!([[0, 255], [255, 0]]));
    let st = h.state().session.active().unwrap();
    let preview = filter_dialog::preview_document(&st.doc, st.active_layer, "image.adjustments.curves", &params(&h), 1).unwrap();
    confirm(&mut h, id).unwrap();
    assert_eq!(preview.channels, h.state().session.active().unwrap().doc.channels);
    assert_eq!(preview.layers, h.state().session.active().unwrap().doc.layers);
}

/// An explicit pixels Action keeps colour controls and edits pixels while an alpha is selected.
#[test]
fn explicit_pixels_action_bypasses_current_alpha_target() {
    let mut app = fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Alpha(0));
    let mut raw = json!({"target":"pixels","points":[[0,0],[255,255]]});
    for key in ["red", "green", "blue"] {
        raw[key] = json!([[0, 255], [255, 255]]);
    }
    add_action(&mut app, "Pixel edit", "image.adjustments.curves", raw, true);
    let mut h = canvas_harness(app);
    play(&mut h, "Pixel edit");
    assert_eq!(names(&h), ["RGB"]);
    for key in ["red", "green", "blue"] {
        assert_eq!(params(&h)[key], json!([[0.0, 255.0], [255.0, 255.0]]), "pixel targets retain their recorded {key} curve");
    }
    // Black and white points sampled on the black pixel leave these curves as recorded; the gray point moves them.
    h.get_by_label("Set Neutral Gray Point").click();
    h.run_steps(2);
    let base = params(&h);
    let expected = photocraft_engine::adjust_params::adjustment_eyedropper(
        "curves",
        &base,
        photocraft_engine::adjust_params::CurvesEyedropper::NeutralGray,
        [0.0; 3],
        ColorMode::Rgb,
    )
    .unwrap();
    sample_at(&mut h, 4.0, 4.0);
    let sampled = photocraft_engine::adjust_params::from_params("curves", &params(&h), None, ColorMode::Rgb).unwrap();
    assert_eq!(sampled, expected, "the picker writes the curve for the committed black pixel");
    assert_ne!(params(&h)["red"], base["red"], "the sample cannot silently do nothing");
    assert_eq!(params(&h)["target"], "pixels");
    let before = h.state().session.active().unwrap().doc.clone();
    let id = top(&h);
    // A known non-neutral replay result must actually reach the pixels, while the selected
    // alpha remains untouched. A silently dropped adjustment must fail this assertion.
    set(&mut h, id, "points", json!([[0, 64], [255, 255]]));
    for key in ["red", "green", "blue"] {
        set(&mut h, id, key, json!([[0, 0], [255, 255]]));
    }
    confirm(&mut h, id).unwrap();
    let after = &h.state().session.active().unwrap().doc;
    assert_eq!(after.channels, before.channels);
    let old = before.layers[0].surface().unwrap().pixel(8, 8);
    let new = after.layers[0].surface().unwrap().pixel(8, 8);
    assert!(old[..3].iter().all(|v| *v == 0.0));
    assert!(new[..3].iter().all(|v| (*v - 64.0 / 255.0).abs() < 0.005));
    assert_eq!(new[3], old[3]);
}

/// Rejected samples and Cancel leave all committed document data and history unchanged.
#[test]
fn invalid_grayscale_sample_and_cancel_preserve_the_document() {
    let mut h = open_on_canvas(fixture(ColorMode::Lab, SampleType::F32, ChannelTarget::Alpha(0)), "curves");
    h.get_by_label("Set White Point").click();
    h.run_steps(2);
    let before = params(&h);
    let original = h.state().session.active().unwrap().doc.clone();
    // JSON carries no NaN, so the points are the ones outside the plane.
    for (x, y) in [(-1.0, 8.0), (99.0, 8.0), (8.0, -1.0)] {
        sample_at(&mut h, x, y);
        assert_eq!(params(&h), before, "({x}, {y})");
        assert!(picker_armed(&h), "an invalid sample does not disarm the picker");
    }
    let id = top(&h);
    cancel(&mut h, id);
    assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
}

/// Reset discards the armed picker’s saved curve so the next sample uses neutral settings.
#[test]
fn resetting_an_armed_picker_does_not_restore_its_old_curve() {
    let mut h = open_on_canvas(fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Alpha(0)), "curves");
    let id = top(&h);
    set(&mut h, id, "points", json!([[0, 64], [255, 255]]));
    h.get_by_label("Set White Point").click();
    h.run_steps(2);
    sample_at(&mut h, 4.0, 4.0);
    reset(&mut h);
    assert!(picker_armed(&h), "Reset keeps the picker armed");
    sample_at(&mut h, 4.0, 4.0);
    let points = params(&h)["points"].clone();
    assert_eq!(points[0][1], 0.0, "{points}");
    let white = points.as_array().unwrap().last().unwrap()[0].as_f64().unwrap();
    assert!((white - 63.75).abs() < 0.3, "the sample reached the neutral curve: {points}");
}

/// A deleted explicit channel fails preview and commit instead of falling back to colour pixels.
#[test]
fn missing_explicit_channel_cannot_preview_as_a_colour_layer() {
    let mut h = open(fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Color(0)), "curves");
    h.state_mut().session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
    let id = top(&h);
    set(&mut h, id, "target", json!({"channel":999}));
    assert!(!adjust_preview::on_layer(h.state_mut(), 0));
    let original = h.state().session.active().unwrap().doc.clone();
    assert!(confirm(&mut h, id).is_err());
    assert_eq!(h.state().session.active().unwrap().doc.channels, original.channels);
    assert_eq!(h.state().session.active().unwrap().doc.layers, original.layers);
}

/// A recorded `gray` alias must seed editable composite controls, rather than override them invisibly.
#[test]
fn scalar_action_gray_alias_seeds_the_visible_controls() {
    for kind in ["levels", "curves"] {
        let mut app = fixture(ColorMode::Lab, SampleType::U8, ChannelTarget::Color(0));
        let gray = if kind == "levels" { json!({"gamma":2}) } else { json!([[0, 255], [255, 0]]) };
        add_action(&mut app, "Gray alias", format!("image.adjustments.{kind}"), json!({"target":{"channel":0},"gray":gray}), true);
        let mut h = harness(app);
        play(&mut h, "Gray alias");
        assert_eq!(names(&h), ["Saved sky"]);
        let id = top(&h);
        if kind == "levels" {
            assert_eq!(params(&h)["gamma"].as_f64(), Some(2.0), "the visible gamma agrees with the recorded gray gamma");
            edit_gamma(&mut h, "1.0");
        } else {
            assert_eq!(params(&h)["points"], json!([[0.0, 255.0], [255.0, 0.0]]), "the visible curve is the recorded gray curve");
            set(&mut h, id, "points", json!([[0, 0], [255, 255]]));
        }
        assert!(params(&h).get("gray").is_none(), "a hidden alias must not override subsequent control edits");
        let before = h.state().session.active().unwrap().doc.clone();
        confirm(&mut h, id).unwrap();
        assert!((value(h.state(), ChannelTarget::Alpha(0)) - 0.25).abs() < 0.005, "editing the visible controls back to neutral is effective");
        assert_eq!(h.state().session.active().unwrap().doc.layers[0].content, before.layers[0].content);
    }
}

/// Malformed aliases remain command errors; neutral dialog defaults must not conceal them.
#[test]
fn invalid_scalar_action_gray_alias_cannot_commit() {
    for (kind, gray) in [("levels", json!({"gamma":"invalid"})), ("curves", json!("invalid")), ("curves", json!([[0, 0]]))] {
        let mut app = fixture(ColorMode::Lab, SampleType::U8, ChannelTarget::Color(0));
        let command = format!("image.adjustments.{kind}");
        add_action(&mut app, "Invalid alias", command.clone(), json!({"target":{"channel":0},"gray":gray}), true);
        let mut h = harness(app);
        play(&mut h, "Invalid alias");
        let original = h.state().session.active().unwrap().doc.clone();
        let steps = h.state().session.active().unwrap().history.past_len();
        let layer = h.state().session.active().unwrap().active_layer;
        assert!(filter_dialog::preview_document(&original, layer, &command, &params(&h), 1).is_none());
        let id = top(&h);
        assert!(confirm(&mut h, id).is_err());
        assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps);
    }
}

/// Play an actual recorded modal step while a different scalar channel is selected.
fn colour_set_action(mode: ColorMode, depth: SampleType, kind: &str, recorded: Value) -> Harness<'static, Box<PhotocraftApp>> {
    colour_set_action_with_selection(mode, depth, kind, recorded, false)
}

/// Set up selection before playback: a paused Action owns its captured document/history.
fn colour_set_action_with_selection(mode: ColorMode, depth: SampleType, kind: &str, recorded: Value, feathered: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = fixture(mode, depth, ChannelTarget::Alpha(0));
    app.session
        .edit("Colour pattern", |doc, _| {
            let bounds = doc.bounds();
            let n = mode.color_channels();
            let mut samples = Vec::new();
            for y in 0..32 {
                for x in 0..32 {
                    for k in 0..n {
                        samples.push(((x * 7 + y * 11 + k * 37) % 221 + 15) as f32 / 255.0);
                    }
                    samples.push(0.65);
                }
            }
            doc.layers[0].surface_mut().unwrap().write_region(bounds, &samples);
            if feathered {
                let mut selection = Surface::new(PixelFormat::GRAY8);
                selection.fill_rect(photocraft_geom::Rect::new(4, 4, 24, 24), &[0.5]);
                doc.selection = Some(selection);
            }
            Ok(())
        })
        .unwrap();
    add_action(&mut app, "Colour set alias", format!("image.adjustments.{kind}"), recorded, true);
    let mut h = harness(app);
    play(&mut h, "Colour set alias");
    h
}

/// Editing the visible combined record back to neutral removes the recorded alias's effect.
#[test]
fn colour_set_action_gray_alias_does_not_override_visible_edits() {
    for mode in MODES {
        for (kind, pencil) in [("levels", false), ("curves", false), ("curves", true)] {
            let mut raw = if kind == "levels" { json!({"gray":{"gamma":2}}) } else { json!({"gray":[[0,255],[255,0]],"pencil":pencil}) };
            raw["target"] = json!({"colors":[0,2]});
            let mut h = colour_set_action(mode, SampleType::U16, kind, raw);
            let original = h.state().session.active().unwrap().doc.clone();
            let id = top(&h);
            if kind == "levels" {
                edit_gamma(&mut h, "1.0");
            } else {
                let identity = if pencil { json!((0..256).collect::<Vec<_>>()) } else { json!([[0, 0], [255, 255]]) };
                set(&mut h, id, "points", identity);
            }
            assert!(params(&h).get("gray").is_none());
            confirm(&mut h, id).unwrap();
            assert_eq!(h.state().session.active().unwrap().doc.layers, original.layers, "{mode:?} {kind} pencil={pencil}");
            assert_eq!(h.state().session.active().unwrap().doc.channels, original.channels);
        }
    }
}

/// Invalid aliases cannot be hidden by seeded identity controls or mutate document pixels.
#[test]
fn invalid_colour_set_action_gray_alias_cannot_preview_or_commit() {
    for mode in MODES {
        for (kind, gray) in [("levels", json!({"gamma":"invalid"})), ("curves", json!("invalid")), ("curves", json!([[0, 0]]))] {
            let mut h = colour_set_action(mode, SampleType::F32, kind, json!({"target":{"colors":[0,2]},"gray":gray}));
            let st = h.state().session.active().unwrap();
            let original = st.doc.clone();
            let command = format!("image.adjustments.{kind}");
            assert!(filter_dialog::preview_document(&original, st.active_layer, &command, &params(&h), 1).is_none(), "{mode:?} {kind}");
            let id = top(&h);
            assert!(confirm(&mut h, id).is_err());
            assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        }
    }
}

/// Alt+adjustment recalls settings for the current scalar plane without resurrecting colour records or old addressing.
#[test]
fn last_tone_settings_follow_the_current_scalar_target() {
    for mode in GRAY_TARGET_MODES {
        for target in TARGETS {
            for kind in ["levels", "curves"] {
                let app = fixture(mode, SampleType::U8, target);
                let command = format!("image.adjustments.{kind}");
                let mut last = if kind == "levels" { json!({"gamma":2.0}) } else { json!({"points":[[0,255],[255,0]]}) };
                let colour = if kind == "levels" { json!({"gamma":3.0}) } else { json!([[0, 255], [255, 0]]) };
                for key in ["red", "green", "blue", "cyan", "magenta", "yellow", "black", "lightness", "a", "b"] {
                    last[key] = colour.clone();
                }
                // Settings from an earlier explicit-target dialog must never restore that address.
                last["target"] = json!({"channel":999});
                last["layer"] = json!(999);
                last["document"] = json!(999);
                let mut h = harness(app);
                remember_last(&h, &command, last);
                let id = adjust_dialog::open_last(h.state_mut(), &command).unwrap();
                h.run_steps(4);
                let fields = params(&h);
                for key in ["red", "green", "blue"] {
                    // The gray plane's command parameters carry no colour records; one that is carried is neutral.
                    let neutral = fields
                        .get(key)
                        .is_none_or(|record| if kind == "levels" { record["gamma"] == 1.0 } else { *record == json!([[0.0, 0.0], [255.0, 255.0]]) });
                    assert!(neutral, "{mode:?} {target:?} {kind}: hidden {key} must stay neutral: {fields}");
                }
                assert!(fields.get("layer").is_none() && fields.get("document").is_none());
                if target == ChannelTarget::LayerMask {
                    assert_eq!(fields["target"], "mask", "retain the current mask target");
                } else {
                    assert!(fields.get("target").is_none(), "do not restore the old explicit target");
                }
                let before = h.state().session.active().unwrap().doc.layers[0].content.clone();
                confirm(&mut h, id).unwrap();
                assert!(value(h.state(), target) > 0.4, "recalled settings must affect {mode:?} {target:?} {kind}");
                assert_eq!(h.state().session.active().unwrap().doc.layers[0].content, before);
                let remembered = remembered(&h, &command);
                for key in ["target", "layer", "document"] {
                    assert!(remembered.get(key).is_none(), "last-used settings must not persist {key}");
                }
            }
        }
    }
}

/// Last-used colour records remain editable in their own space; incompatible spaces and malformed settings fall back to neutral.
#[test]
fn last_tone_settings_preserve_compatible_colour_controls_and_reject_bad_settings() {
    use photocraft_engine::adjust_params::{default_for, from_params, to_params};
    for mode in MODES {
        for kind in ["levels", "curves"] {
            let command = format!("image.adjustments.{kind}");
            let key = photocraft_engine::adjust_params::channel_keys(photocraft_engine::adjust_params::default_space(mode))[0];
            let record = if kind == "levels" { json!({"gamma":2.0}) } else { json!([[0, 255], [255, 0]]) };
            let mut supplied = json!({});
            supplied[key] = record;
            let compatible = to_params(&from_params(kind, &supplied, None, mode).unwrap());
            let wrong_mode = if mode == ColorMode::Rgb { ColorMode::Lab } else { ColorMode::Rgb };
            let incompatible = to_params(&default_for(kind, wrong_mode).unwrap());
            let invalid = if kind == "levels" { json!({"gamma":"invalid"}) } else { json!({"gray":[[0,0]]}) };
            for (last, expected) in [
                (compatible.clone(), compatible),
                (incompatible, to_params(&default_for(kind, mode).unwrap())),
                (invalid, to_params(&default_for(kind, mode).unwrap())),
            ] {
                let app = fixture(mode, SampleType::U8, ChannelTarget::Color(0));
                let original = app.session.active().unwrap().doc.clone();
                let mut h = harness(app);
                remember_last(&h, &command, last.clone());
                let id = adjust_dialog::open_last(h.state_mut(), &command).unwrap();
                h.run_steps(4);
                // Main captures the selected native component in each modal. Recall preserves
                // that address alongside the settings, while the remembered settings stay address-free.
                let mut expected = expected;
                expected["target"] = json!({"colors":[0]});
                assert_eq!(params(&h), expected, "{mode:?} {kind}: {last}");
                cancel(&mut h, id);
                assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
                assert_eq!(remembered(&h, &command), last, "Cancel preserves remembered settings");
            }
        }
    }
}

/// Grayscale layer pixels expose one curve too, so recalling an RGB dialog must not introduce hidden channel edits.
#[test]
fn last_tone_settings_on_grayscale_pixels_drop_hidden_colour_records() {
    for kind in ["levels", "curves"] {
        let mut app = fixture(ColorMode::Grayscale, SampleType::U8, ChannelTarget::Color(0));
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
        let command = format!("image.adjustments.{kind}");
        let last = if kind == "levels" { json!({"gamma":2.0,"red":{"gamma":3.0}}) } else { json!({"points":[[0,255],[255,0]],"red":[[0,255],[255,0]]}) };
        let mut h = harness(app);
        remember_last(&h, &command, last);
        let id = adjust_dialog::open_last(h.state_mut(), &command).unwrap();
        h.run_steps(4);
        let fields = params(&h);
        // The recalled colour record is dropped: absent, or neutral where the parameters carry one.
        if kind == "levels" {
            assert!(fields.get("red").is_none_or(|red| red["gamma"] == 1.0), "{fields}");
            assert_eq!(fields["gamma"], 2.0);
        } else {
            assert!(fields.get("red").is_none_or(|red| *red == json!([[0.0, 0.0], [255.0, 255.0]])), "{fields}");
            assert_eq!(fields["points"], json!([[0.0, 255.0], [255.0, 0.0]]));
        }
        cancel(&mut h, id);
    }
}

/// Build a filtered smart object with a selected filter mask, optionally still the implicit reveal-all mask.
fn filter_mask_fixture(mode: ColorMode, depth: SampleType, stored: bool) -> Box<PhotocraftApp> {
    let (session, id) = filter_mask_session(mode, depth, stored);
    let mut app = app_of(session);
    app.ui.filter_mask_target = Some(id.0);
    app
}

/// [`filter_mask_fixture`]'s session and the smart object, whose filter mask the Layers panel targets.
fn filter_mask_session(mode: ColorMode, depth: SampleType, stored: bool) -> (Session, photocraft_doc::LayerId) {
    let mut session = fixture_session(mode, depth, ChannelTarget::Color(0));
    session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    session.execute("filter.blur.gaussianBlur", json!({"radius":1})).unwrap();
    let id = session.active().unwrap().active_layer.unwrap();
    if stored {
        session.execute("edit.fill", json!({"target":"filterMask","contents":"color","color":"#404040"})).unwrap();
    }
    (session, id)
}

/// New upstream filter masks use the same scalar controls, picker and preserved targeting: both dialogs with a stored
/// and an implicit mask on an RGB document, and a grayscale document once. Every mode and depth is the engine's
/// `filter_mask_picker_samples_and_edits_only_the_filter_mask` (see [`RGB_8`]).
#[test]
fn filter_mask_tone_dialogs_preserve_the_target_and_sample_its_own_levels() {
    let mut h = bench();
    let mut cases = Vec::new();
    for kind in ["levels", "curves"] {
        for stored in [false, true] {
            cases.push((RGB_8, kind, stored));
        }
    }
    cases.push(((ColorMode::Grayscale, SampleType::U16), "levels", true));
    for ((mode, depth), kind, stored) in cases {
        let (session, id) = filter_mask_session(mode, depth, stored);
        next_case(&mut h, &session, Some(id.0));
        let st = h.state().session.active().unwrap();
        let layer = st.active_layer.unwrap();
        let expected_name = format!("{} Filter Mask", st.doc.layer(layer).unwrap().name);
        let before = st.doc.clone();
        open_here(&mut h, kind);
        assert_eq!(names(&h), [expected_name], "{mode:?} {depth:?} {kind}");
        assert_eq!(params(&h)["target"], "filterMask");
        let id = top(&h);
        arm(&mut h, "white");
        sample_at(&mut h, 4.0, 4.0);
        assert!(picker_armed(&h), "the picker stays armed on {mode:?} {depth:?} stored={stored}");
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc), "sampling is read-only");
        assert_eq!(params(&h)["target"], "filterMask");
        let white = if stored { 64.0 } else { 255.0 };
        let sampled = if kind == "levels" {
            params(&h)["inWhite"].as_f64().unwrap()
        } else {
            params(&h)["points"].as_array().unwrap().last().unwrap()[0].as_f64().unwrap()
        };
        assert!((sampled - white).abs() < 1.0, "{mode:?} {depth:?} {kind} stored={stored}: sampled {sampled}, want {white}");
        confirm(&mut h, id).unwrap();
        let after = &h.state().session.active().unwrap().doc;
        let LayerContent::Smart(old) = &before.layer(layer).unwrap().content else { panic!("smart fixture") };
        let LayerContent::Smart(new) = &after.layer(layer).unwrap().content else { panic!("smart result") };
        assert_eq!(new.smart_filters, old.smart_filters, "the adjustment edits the mask rather than adding a filter");
        assert_eq!(new.source, old.source);
        assert_eq!(after.channels, before.channels);
        assert_eq!(after.layer(layer).unwrap().mask, before.layer(layer).unwrap().mask);
        assert!(new.filter_mask.as_ref().unwrap().surface.sample_channel(8, 8, 0) > 0.99);
    }
}

/// A recorded alpha address takes precedence over the currently selected filter mask.
#[test]
fn action_alpha_target_overrides_the_filter_mask_panel_target() {
    for mode in MODES {
        for kind in ["levels", "curves"] {
            let mut app = filter_mask_fixture(mode, SampleType::U8, false);
            let before = app.session.active().unwrap().doc.clone();
            let values = if kind == "levels" { json!({"target":{"channel":0},"gamma":2}) } else { json!({"target":{"channel":0},"points":[[0,255],[255,0]]}) };
            add_action(&mut app, "Alpha over filter mask", format!("image.adjustments.{kind}"), values, true);
            let mut h = harness(app);
            play(&mut h, "Alpha over filter mask");
            assert_eq!(names(&h), ["Saved sky"]);
            assert_eq!(params(&h)["target"], json!({"channel":0}));
            let id = top(&h);
            confirm(&mut h, id).unwrap();
            assert!(value(h.state(), ChannelTarget::Alpha(0)) > 0.4);
            assert_eq!(h.state().session.active().unwrap().doc.layers, before.layers);
        }
    }
}

/// Explicit Curves points beat the legacy alias, while malformed pencil maps stay errors.
/// Canceling a valid alias dialog retains the original document and recorded Action verbatim.
#[test]
fn colour_set_action_gray_alias_precedence_and_cancel() {
    for mode in MODES {
        let raw = json!({"target":{"colors":[0,2]},"points":[[0,20],[255,240]],"gray":"ignored"});
        let mut h = colour_set_action(mode, SampleType::U8, "curves", raw.clone());
        assert!(params(&h).get("gray").is_none());
        assert_eq!(params(&h)["points"], json!([[0.0, 20.0], [255.0, 240.0]]), "{mode:?}");
        let original = h.state().session.active().unwrap().doc.clone();
        let id = top(&h);
        cancel(&mut h, id);
        assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        assert_eq!(recorded_params(&h), raw);
        let mut h = colour_set_action(mode, SampleType::U8, "curves", json!({"target":{"colors":[0,2]},"gray":[0,127,255],"pencil":true}));
        let original = h.state().session.active().unwrap().doc.clone();
        let id = top(&h);
        assert!(confirm(&mut h, id).is_err());
        assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
    }
}

/// A resolved Lab Action stores the combined and native Lightness settings independently when recalled.
#[test]
fn colour_set_action_gray_alias_lab_recall_keeps_both_records() {
    for kind in ["levels", "curves"] {
        let raw = if kind == "levels" {
            json!({"target":{"colors":[0,2]},"gray":{"gamma":2},"lightness":{"gamma":1.543217}})
        } else {
            json!({"target":{"colors":[0,2]},"gray":[[0,255],[255,0]],"lightness":[[0,30.123456],[255,200.654321]]})
        };
        let mut h = colour_set_action(ColorMode::Lab, SampleType::U8, kind, raw);
        let expected = params(&h);
        let id = top(&h);
        confirm(&mut h, id).unwrap();
        h.state_mut().session.execute("channel.target", json!({"colors":[0,2]})).unwrap();
        adjust_dialog::open_last(h.state_mut(), &format!("image.adjustments.{kind}")).unwrap();
        h.run_steps(4);
        let recalled = params(&h);
        for key in if kind == "levels" { &["gamma", "lightness", "a", "b"][..] } else { &["points", "lightness", "a", "b"][..] } {
            assert_eq!(recalled[*key], expected[*key], "{kind}: {key}");
        }
    }
}

/// Scalar alias normalization must consume smoothing exactly once, just like colour sets.
#[test]
fn scalar_action_gray_alias_does_not_repeat_pencil_smoothing() {
    let mut app = fixture(ColorMode::Lab, SampleType::F32, ChannelTarget::Alpha(0));
    let raw = json!({"target":{"channel":0},"gray":[[0,255],[100,30],[255,0]],"pencil":true,"smooth":2});
    add_action(&mut app, "Smoothed scalar", "image.adjustments.curves", raw.clone(), true);
    let mut h = harness(app);
    play(&mut h, "Smoothed scalar");
    assert!(params(&h).get("smooth").is_none());
    let st = h.state().session.active().unwrap();
    let mut oracle = Session::new();
    oracle.add_document((*st.doc).clone(), None);
    oracle.select_layer(st.active_layer.unwrap()).unwrap();
    oracle.execute("image.adjustments.curves", raw).unwrap();
    let id = top(&h);
    confirm(&mut h, id).unwrap();
    assert_eq!(h.state().session.active().unwrap().doc.channels, oracle.active().unwrap().doc.channels);
}

/// Missing and malformed recorded targets must not display document-colour controls or
/// histograms. The original address remains available for errors and Cancel leaves pixels intact.
#[test]
fn review_unresolved_action_targets_do_not_fall_back_to_colour_controls() {
    for target in [json!({"channel":999}), json!({"color":0,"channel":0}), json!({"colors":[999]}), json!("quickMask")] {
        for kind in ["levels", "curves"] {
            let mut app = fixture(ColorMode::Lab, SampleType::U16, ChannelTarget::Color(0));
            add_action(&mut app, "Missing target", format!("image.adjustments.{kind}"), json!({"target":target}), true);
            let mut h = harness(app);
            play(&mut h, "Missing target");
            assert!(names(&h).is_empty(), "invalid target must not show a colour dropdown: {target}");
            assert_eq!(params(&h)["target"], target);
            assert!(h.query_by_label("The adjustment target is missing or unavailable.").is_some());
            let original = h.state().session.active().unwrap().doc.clone();
            let id = top(&h);
            assert!(confirm(&mut h, id).is_err());
            assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        }
    }
}

/// Malformed Action step parameter shapes fail before opening an editable neutral dialog.
#[test]
fn review_malformed_action_params_cannot_open_a_neutral_adjustment() {
    for value in [json!([]), json!([1]), json!(42), json!("bad"), json!(true)] {
        let mut app = fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Alpha(0));
        let before = app.session.active().unwrap().doc.clone();
        add_action(&mut app, "Bad params", "image.adjustments.levels", value, true);
        let result = app.run("actions.play", json!({"action":"Bad params"})).unwrap();
        assert!(!app.dialog_open(), "bad params must fail the step instead of becoming defaults");
        assert!(result.get("failed").is_some() || result.get("error").is_some(), "{result}");
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }
    for value in [Value::Null, json!({})] {
        let mut app = fixture(ColorMode::Rgb, SampleType::U8, ChannelTarget::Alpha(0));
        add_action(&mut app, "Default params", "image.adjustments.levels", value, true);
        let result = app.run("actions.play", json!({"action":"Default params"})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(app.dialog_count(), 1);
    }
}

/// Recalling a precise Action's accepted settings must retain their parsed tone values and
/// produce the same native plane as directly applying those settings a second time.
#[test]
fn review_precise_action_settings_survive_last_used_recall() {
    for mode in MODES {
        for kind in ["levels", "curves"] {
            let mut raw = if kind == "levels" {
                json!({"gray":{"gamma":1.234567,"inBlack":8.123456}})
            } else {
                json!({"gray":[[0,0],[100.123456,200.654321],[255,255]]})
            };
            raw["target"] = json!({"channel":0});
            let mut h = colour_set_action(mode, SampleType::F32, kind, raw);
            let expected = params(&h);
            let id = top(&h);
            confirm(&mut h, id).unwrap();
            let st = h.state().session.active().unwrap();
            let mut oracle = Session::new();
            oracle.add_document((*st.doc).clone(), None);
            oracle.select_layer(st.active_layer.unwrap()).unwrap();
            oracle.execute(&format!("image.adjustments.{kind}"), expected.clone()).unwrap();
            let id = adjust_dialog::open_last(h.state_mut(), &format!("image.adjustments.{kind}")).unwrap();
            h.run_steps(4);
            let parse = |p: &Value| photocraft_engine::adjust_params::from_params(kind, p, None, ColorMode::Grayscale).unwrap();
            assert_eq!(parse(&params(&h)), parse(&expected), "{mode:?} {kind}");
            confirm(&mut h, id).unwrap();
            assert_eq!(h.state().session.active().unwrap().doc.channels, oracle.active().unwrap().doc.channels);
        }
    }
}

/// Build distinct active and addressed masks, retaining the active layer. The session holds the document, the id is
/// the addressed layer's.
fn addressed_mask_session(mode: ColorMode, depth: SampleType, filter: bool) -> (Session, photocraft_doc::LayerId) {
    let mut session = if filter { filter_mask_session(mode, depth, true).0 } else { fixture_session(mode, depth, ChannelTarget::Alpha(0)) };
    let mut other = session.active().unwrap().doc.layers[0].clone();
    other.id = Layer::new("id", other.content.clone()).id;
    other.name = "Receiver".into();
    let mut mask = LayerMask::reveal_all();
    mask.surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    mask.surface.fill_rect(session.active().unwrap().doc.bounds(), &[0.75]);
    if filter {
        let LayerContent::Smart(sm) = &mut other.content else { panic!("smart") };
        sm.filter_mask = Some(mask);
    } else {
        other.mask = Some(mask);
    }
    let id = other.id;
    session
        .edit("Other mask", |doc, _| {
            doc.layers.push(other);
            Ok(())
        })
        .unwrap();
    (session, id)
}

/// Named/id mask Actions show, sample, preview and edit the addressed layer's own mask without
/// switching the selected layer. The matrix includes native raster and Smart Filter masks.
#[test]
fn review_mask_action_context_matches_the_addressed_layer() {
    let mut h = bench();
    // Both dialogs with both mask kinds on an RGB document, and a CMYK and a Lab document once. Resolving the address is
    // the engine's (`review_picker_reads_the_addressed_mask_layer` runs every selector form in every mode and depth);
    // here every form runs on the first layer-mask and filter-mask cases, the name form on the rest.
    let mut cases = Vec::new();
    for kind in ["levels", "curves"] {
        for filter in [false, true] {
            cases.push((RGB_8, kind, filter));
        }
    }
    cases.extend([((ColorMode::Cmyk, SampleType::U16), "curves", true), ((ColorMode::Lab, SampleType::F32), "levels", false)]);
    for (case, ((mode, depth), kind, filter)) in cases.into_iter().enumerate() {
        let (source, layer) = addressed_mask_session(mode, depth, filter);
        let selectors = [json!("Receiver"), json!(layer.0), json!({"name":"Receiver"})];
        for selector in selectors.into_iter().take(if case < 2 { 3 } else { 1 }) {
            next_case(&mut h, &source, None);
            let command = format!("image.adjustments.{kind}");
            let target = if filter { "filterMask" } else { "mask" };
            let raw = json!({"layer":selector,"target":target});
            add_action(h.state_mut(), "Other mask", command.clone(), raw, true);
            let original = h.state().session.active().unwrap().doc.clone();
            let active = h.state().session.active().unwrap().active_layer;
            let expected = photocraft_engine::mask_view_cmds::viewed_mask(original.layer(layer).unwrap(), filter).unwrap().surface.sample_channel(8, 8, 0);
            play(&mut h, "Other mask");
            assert_eq!(names(&h), [if filter { "Receiver Filter Mask" } else { "Receiver Mask" }]);
            let id = top(&h);
            arm(&mut h, "white");
            sample_at(&mut h, 4.0, 4.0);
            let p = params(&h);
            let want = photocraft_engine::adjust_params::adjustment_eyedropper(
                kind,
                &json!({}),
                photocraft_engine::adjust_params::CurvesEyedropper::White,
                [expected; 3],
                ColorMode::Grayscale,
            )
            .unwrap();
            let got = photocraft_engine::adjust_params::from_params(kind, &p, None, ColorMode::Grayscale).unwrap();
            assert_eq!(got, want, "{mode:?} {depth:?} {kind} {target}");
            let preview = filter_dialog::preview_document(&original, active, &command, &p, 1).unwrap();
            confirm(&mut h, id).unwrap();
            let doc = &h.state().session.active().unwrap().doc;
            assert_eq!(doc.layers, preview.layers);
            assert_eq!(doc.layers[0], original.layers[0]);
            assert_eq!(doc.channels, original.channels);
            assert!(photocraft_engine::mask_view_cmds::viewed_mask(doc.layer(layer).unwrap(), filter).unwrap().surface.sample_channel(8, 8, 0) > 0.99);
            assert_eq!(h.state().session.active().unwrap().active_layer, active);
        }
    }
}

/// Repairing a missing Action address re-seeds its recorded alias without losing modal context.
#[test]
fn review_repaired_action_target_seeds_recorded_controls() {
    for (kind, raw) in [
        ("levels", json!({"target":{"channel":999},"gray":{"gamma":2}})),
        ("curves", json!({"target":{"channel":999},"gray":[[0,255],[255,0]],"lightness":[[0,0],[255,255]]})),
    ] {
        let mut h = colour_set_action(ColorMode::Lab, SampleType::U16, kind, raw.clone());
        assert!(names(&h).is_empty());
        assert_eq!(params(&h)["gray"], raw["gray"], "unresolved aliases retain their original meaning");
        let id = top(&h);
        let before = h.state().session.active().unwrap().doc.clone();
        set(&mut h, id, "target", json!({"channel":0}));
        assert_eq!(top(&h), id, "the repaired dialog is the same dialog");
        assert_eq!(names(&h), ["Saved sky"]);
        assert!(params(&h).get("gray").is_none());
        if kind == "levels" {
            assert_eq!(params(&h)["gamma"], 2.0);
            edit_gamma(&mut h, "1.0");
            confirm(&mut h, id).unwrap();
            assert_eq!(h.state().session.active().unwrap().doc.channels, before.channels);
        } else {
            assert_eq!(params(&h)["points"], json!([[0.0, 255.0], [255.0, 0.0]]));
            let mut oracle = Session::new();
            oracle.add_document((*before).clone(), None);
            let mut repaired = raw;
            repaired["target"] = json!({"channel":0});
            oracle.execute("image.adjustments.curves", repaired).unwrap();
            confirm(&mut h, id).unwrap();
            assert_eq!(h.state().session.active().unwrap().doc.channels, oracle.active().unwrap().doc.channels);
        }
    }
}

/// Record Again resolves a recorded scalar target before seeding defaults from another panel target.
#[test]
fn record_again_uses_recorded_alpha_settings_and_matches_its_preview() {
    for mode in MODES {
        for kind in ["levels", "curves"] {
            let mut app = fixture(mode, SampleType::U16, ChannelTarget::Color(0));
            let mut recorded = photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for(kind, mode).unwrap());
            for key in ["red", "green", "blue", "cyan", "magenta", "yellow", "black", "lightness", "a", "b"] {
                if recorded.get(key).is_some() {
                    recorded[key] = if kind == "levels" { json!({"gamma":4}) } else { json!([[0, 255], [255, 255]]) };
                }
            }
            recorded["target"] = json!({"channel": 0});
            if kind == "levels" {
                recorded["gamma"] = json!(2);
            } else {
                recorded["points"] = json!([[0, 255], [255, 0]]);
            }
            add_action(&mut app, "Recorded alpha", format!("image.adjustments.{kind}"), recorded, false);
            app.ui.actions.panel.selected = Some(ActionSelection::Step(0, 0));
            let original = app.session.active().unwrap().doc.clone();
            let mut h = harness(app);
            h.run_steps(3);
            let reply = control(
                &mut h,
                "ui.menu.invoke",
                json!({"id": craft::ui::runtime::commands::CommandRef::from(craft::automation::ui::actions::ActionsPanelCommands::RecordAgain).id}),
            );
            assert_eq!(reply["ok"], true, "{reply}");
            assert_eq!(names(&h), ["Saved sky"], "{mode:?} {kind}");
            let values = params(&h);
            assert_eq!(values["target"], json!({"channel": 0}));
            let neutral = photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for(kind, ColorMode::Grayscale).unwrap());
            for key in ["red", "green", "blue", "cyan", "magenta", "yellow", "black", "lightness", "a", "b"] {
                assert_eq!(values.get(key), neutral.get(key), "{mode:?} {kind}: hidden colour record {key}");
            }
            let id = top(&h);
            let state = h.state().session.active().unwrap();
            let preview = filter_dialog::preview_document(&state.doc, state.active_layer, &format!("image.adjustments.{kind}"), &values, 1).unwrap();
            assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
            let reply = control(&mut h, "ui.dialog.confirm", json!({"dialog": id}));
            assert_eq!(reply["ok"], true, "{reply}");
            let after = &h.state().session.active().unwrap().doc;
            assert_eq!(preview.channels[0].surface, after.channels[0].surface);
            let expected = if kind == "levels" { 0.5 } else { 0.75 };
            assert!((value(h.state(), ChannelTarget::Alpha(0)) - expected).abs() < 0.01, "{mode:?} {kind}");
            assert_eq!(after.layers, original.layers);
            assert_eq!(after.quick_mask, original.quick_mask);
            assert_eq!(recorded_params(&h)["target"], json!({"channel": 0}));
        }
    }
}
