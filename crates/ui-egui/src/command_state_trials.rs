//! Count trials for the command states the runtime reads (#15709): every source's states are a value derived from
//! what they read, so an idle frame builds none, an edit builds only the sources that read what it changed, and the
//! held states always equal a fresh build of every source.

use craft::behavior::geometry::CropBox;
use craft::behavior::kurbo::Rect as DocRect;
use craft::ui::runtime::commands::{CommandRef, CommandStates};
use craft::ui::runtime::measure::last_frame_derived_builds;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::{sources, states_id};
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The sources whose states follow the enable rules, which read the document, the session and the tool.
pub(crate) const RULES: [&str; 5] = ["align", "selection", "history", "layers", "channels"];

/// The app with one new document holding a pixel layer, settled.
fn harness() -> H {
    harness_of(|s| {
        s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
    })
}

/// The app with the session `open` makes, the Brush tool picked, settled.
fn harness_of(open: impl FnOnce(&mut photocraft_engine::Session) + 'static) -> H {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        open(&mut s);
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.ui.tool = Tool::Brush;
        app
    });
    h.run_steps(8);
    h
}

/// The sources the last frame built states for, in the framework's order.
fn built(h: &H) -> Vec<&'static str> {
    let builds = last_frame_derived_builds(&h.ctx);
    sources().map(|(name, _)| name).filter(|name| builds.iter().any(|(id, _)| *id == states_id(name))).collect()
}

/// Runs one frame after `change` and checks which sources it built.
fn builds_only(h: &mut H, what: &str, change: impl FnOnce(&mut PhotocraftApp), expected: &[&str]) {
    h.run_steps(2);
    change(h.state_mut());
    h.run_steps(1);
    let mut expected = expected.to_vec();
    expected.sort_by_key(|source| sources().position(|(name, _)| name == *source));
    assert_eq!(built(h), expected, "{what} builds only the sources that read it");
}

#[test]
fn an_idle_frame_builds_no_command_states() {
    let mut h = harness();
    for frame in 0..10 {
        h.run_steps(1);
        assert_eq!(built(&h), Vec::<&str>::new(), "idle frame {frame} builds no source");
    }
}

#[test]
fn an_edit_builds_only_the_sources_that_read_it() {
    let mut h = harness();
    builds_only(
        &mut h,
        "the brush texture",
        |app| {
            let on = app.session.tools.brush.texture.enabled;
            app.session.tools.brush.texture.enabled = !on;
        },
        &["brushSettings"],
    );
    builds_only(&mut h, "a tool", |app| crate::tool_memory::select(app, Tool::Hand), &[&["tool"][..], &RULES].concat());
    builds_only(&mut h, "the artboard placing mode", |app| app.ui.artboard_tool.adding = true, &["artboard"]);
    builds_only(&mut h, "the selected slice", |app| app.ui.slices.selected = Some(1), &["slice"]);
    // View › Lock Slices reads the lock, so the rule-holding sources build with the slice states.
    builds_only(&mut h, "the slice lock", |app| app.session.file_menu.slices_locked = true, &[&["slice"][..], &RULES].concat());
    builds_only(
        &mut h,
        "a new layer",
        |app| {
            app.run("layer.new.layer", json!({})).unwrap();
        },
        &[&["slice"][..], &RULES].concat(),
    );
}

#[test]
fn a_document_opening_builds_the_slice_and_artboard_states() {
    let mut h = harness_of(|_| {});
    h.run_steps(2);
    h.state_mut().run("file.new", json!({"width": 64, "height": 64})).unwrap();
    h.run_steps(1);
    let built = built(&h);
    assert!(built.contains(&"slice") && built.contains(&"artboard"), "a document opens: {built:?}");
    let add_new = CommandRef::from(craft::draw::ui::artboard::ArtboardCommand::AddNew);
    let artboard = h.state().framework.held_states.states.get(&add_new).cloned();
    assert!(artboard.is_some_and(|state| state.is_enabled()), "Add New Artboard is on with a document");
}

/// Checks that every held state is the one its source builds afresh: the sources are asked again through a new context,
/// whose derived store is empty, so each builds from the app's state as it is now.
fn held_is_fresh(h: &mut H, step: &str) {
    h.run_steps(2);
    let app = h.state_mut();
    let fresh = egui::Context::default();
    PhotocraftApp::setup_context(&fresh, Default::default());
    let own = std::mem::replace(&mut app.ctx, fresh);
    let built: Vec<_> = sources().map(|(name, states)| (name, states(app, name))).collect();
    app.ctx = own;
    let mut held = (*app.framework.held_states.states).clone();
    for (name, states) in built {
        for (command, state) in states.iter() {
            assert!(held.remove(command).as_ref() == Some(state), "after {step}, the {name} state of {command:?} is a fresh build");
        }
    }
    assert!(held == CommandStates::new(), "after {step}, every held state has a source");
}

#[test]
fn the_held_command_states_equal_a_fresh_build_after_each_step() {
    let mut h = harness();
    held_is_fresh(&mut h, "settling");
    crate::tool_memory::select(h.state_mut(), Tool::Move);
    held_is_fresh(&mut h, "a tool");
    h.state_mut().session.tools.brush.texture.enabled = true;
    held_is_fresh(&mut h, "the brush texture");
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    held_is_fresh(&mut h, "a new layer");
    h.state_mut().run("layer.layerMask.revealAll", json!({})).unwrap();
    h.state_mut().ui.mask_target = true;
    held_is_fresh(&mut h, "a mask target");
    crate::workspace_ui::save(h.state_mut(), "Trial").unwrap();
    held_is_fresh(&mut h, "a new workspace");
    crate::tool_memory::select(h.state_mut(), Tool::Crop);
    h.run_steps(2);
    h.state_mut().canvas_tools.crop.tool.crop_to(CropBox::new(DocRect::new(10.0, 10.0, 100.0, 60.0), 0.0));
    held_is_fresh(&mut h, "a pending crop");
    assert!(crate::crop_ui::pending(h.state()), "the crop is pending");
}

/// Merge Channels reads every open grayscale document, not only the active one: a change to a background document
/// moves its state in either direction (#16252).
#[test]
fn merge_channels_follows_a_change_to_a_background_document() {
    let mut h = harness_of(|s| {
        for _ in 0..3 {
            s.execute("file.new", json!({"width": 64, "height": 64, "mode": "grayscale"})).unwrap();
        }
        s.set_active(0);
    });
    let merge = |h: &mut H| {
        h.run_steps(2);
        let merge = CommandRef::from(craft::imaging::ui::channels::ChannelsCommands::MergeChannels);
        h.state().framework.held_states.states.get(&merge).is_some_and(|state| state.is_enabled())
    };
    assert!(merge(&mut h), "three grayscale documents of one size can merge");
    // A change to a background document, as a job or a script makes through `Session::document_mut`.
    let resize = |h: &mut H, side: u32| {
        let s = &mut h.state_mut().session;
        let Some(id) = s.documents().get(2).map(|st| st.doc.id) else { panic!("three documents are open") };
        let Some(st) = s.document_mut(id) else { panic!("the third document is open") };
        std::sync::Arc::make_mut(st.doc_mut()).size = photocraft_geom::Size::new(side, side);
    };
    resize(&mut h, 32);
    assert!(!merge(&mut h), "a background document of another size leaves two to merge");
    held_is_fresh(&mut h, "resizing a background document");
    resize(&mut h, 64);
    assert!(merge(&mut h), "and back at the same size, three again");
    held_is_fresh(&mut h, "resizing it back");
}
