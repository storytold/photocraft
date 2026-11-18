use super::*;
use craft::automation::actions::{Action, ActionStep};
use craft::behavior::value::{EditEvent, Mixed};
use craft::ui::contract::ReorderMove;
use craft::ui::primitives::textures::{ThumbRender, ThumbRequest, ThumbSource};

fn app_with(mode: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 24, "mode": mode})).unwrap();
    app
}

fn alpha_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.channels.iter().map(|c| c.name.clone()).collect()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The `Masked` layer with a reveal-all mask and one alpha channel, in a 64×48 document.
fn masked_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({"name": "Masked"})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    // White, so loading it as a selection selects something.
    app.run("channel.new", json!({"fill": "white"})).unwrap();
    app
}

/// The command of `command` for the targeted row, through the framework's own command-to-event map.
fn fire(app: &mut PhotocraftApp, ctx: &egui::Context, command: ChannelsCommands) {
    assert!(run_fired(app, ctx, &CommandRef::from(command)));
}

fn enabled(app: &PhotocraftApp, command: ChannelsCommands) -> bool {
    let wanted = CommandRef::from(command);
    command_states(app).into_iter().find(|(r, _)| *r == wanted).is_some_and(|(_, s)| s.is_enabled())
}

/// The `channel` field of the newest open dialog.
fn newest_dialog_channel(app: &PhotocraftApp) -> Value {
    let open = app.framework.dialogs.open_dialogs().pop().expect("a dialog is open");
    crate::dialogs::params::captured(app, open.key.as_deref().expect("a keyed dialog"))["channel"].clone()
}

/// The thumbnail revision of a colour key follows the pixels and Show Channels in Color, not the selection (#4518).
#[test]
fn channel_thumbs_follow_their_inputs() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    let request = ThumbRequest::for_cell(28.0, 1.0);
    let revision = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        ChannelThumbs::new(app, st).revision(&Row::Color(0), &request)
    };
    let first = revision(&app);
    app.session.execute("prefs.set", json!({"values": {"interface.showChannelsInColor": true}})).unwrap();
    let in_color = revision(&app);
    assert_ne!(in_color, first, "the tint is part of the picture");
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 20})).unwrap();
    assert_eq!(revision(&app), in_color, "a marquee changes no pixels");
    app.run("select.deselect", json!({})).unwrap();
    assert_eq!(revision(&app), in_color, "neither does Deselect");
    app.run("edit.fill", json!({"color": "#336699"})).unwrap();
    assert_ne!(revision(&app), in_color, "a fill changes them");
}

/// The C and K pictures of rich black 100/100/100/0 show its own inks: full cyan and no black, not the RGB
/// thumbnail separated again (#8359).
#[test]
fn cmyk_channel_thumbs_upload_the_document_inks() {
    let mut app = app_with("cmyk");
    app.run("tools.setColors", json!({"foregroundCmyk": [1.0, 1.0, 1.0, 0.0]})).unwrap();
    app.run("edit.fillForeground", json!({})).unwrap();
    let st = app.session.active().unwrap();
    let source = ChannelThumbs::new(&app, st);
    let request = ThumbRequest::for_cell(56.0, 1.0);
    let centre = |key: Row| {
        let ThumbRender::Ready(img) = source.render(&key, &request) else { panic!("{key:?} renders at once") };
        img[(img.size[0] / 2, img.size[1] / 2)]
    };
    assert_eq!(centre(Row::Color(0)), egui::Color32::from_gray(0), "cyan is full ink");
    assert_eq!(centre(Row::Color(3)), egui::Color32::from_gray(255), "rich black has no K");
}

/// Each event runs its engine command; a reorder is one history step and counts alpha channels only.
#[test]
fn channel_events_run_their_commands() {
    let mut app = masked_app();
    let ctx = app.ctx().clone();
    app.run("channel.new", json!({"name": "Alpha 2", "fill": "white"})).unwrap();
    app.run("channel.new", json!({"name": "Alpha 3", "fill": "white"})).unwrap();

    // Target: an alpha, the layer mask, then the composite.
    target(&mut app, Row::Alpha(0), false);
    assert_eq!(app.session.active().unwrap().channel_view.target, ChannelTarget::Alpha(0));
    target(&mut app, Row::LayerMask, false);
    assert!(app.ui.mask_target);
    target(&mut app, Row::Composite, false);
    assert!(!app.ui.mask_target, "the composite targets the pixels again");

    // Visibility: the step's value is what the eye becomes. A colour eye off, then on.
    let color_shown = |app: &PhotocraftApp, k| app.session.active().unwrap().channel_view.color_visible(k);
    assert!(color_shown(&app, 1));
    apply_visibility(&mut app, Row::Color(1), false);
    assert!(!color_shown(&app, 1), "the eye is off");
    apply_visibility(&mut app, Row::Color(1), true);
    assert!(color_shown(&app, 1), "and on again");
    // The mask row's eye: the overlay on, then the mask view off.
    let mask_view = |app: &PhotocraftApp| photocraft_engine::mask_view_cmds::current(app.session.active().unwrap()).map(|v| v.mode);
    assert_eq!(mask_view(&app), None);
    apply_visibility(&mut app, Row::LayerMask, true);
    assert_eq!(mask_view(&app), Some(photocraft_engine::mask_view_cmds::MaskViewMode::Overlay));
    apply_visibility(&mut app, Row::LayerMask, false);
    assert_eq!(mask_view(&app), None);

    // Load as a selection: the targeted alpha is white.
    target(&mut app, Row::Alpha(0), false);
    fire(&mut app, &ctx, ChannelsCommands::LoadSelection);
    assert!(app.session.active().unwrap().doc.selection.is_some());

    // Reorder: Alpha 3 before Alpha 1. The rows are RGB, R, G, B, mask, A1, A2, A3, so the slot is 5 of the rows
    // that remain, and no moving row stands before it: `channel.move` takes index 0.
    let before = steps(&app);
    let mv = ReorderMove { key: Row::Alpha(2), keys: vec![Row::Alpha(2)], to: 5, copy: false, parent: None };
    apply(&mut app, &ctx, ChannelsEvent::Reorder(EditEvent::set(Mixed::Same(mv.clone()), mv, "Move".into(), None)));
    assert_eq!(alpha_names(&app), ["Alpha 3", "Alpha 1", "Alpha 2"]);
    assert_eq!(steps(&app), before + 1, "one history step");

    // Drops on the footer, through the event's own path: what each button does to each row.
    assert_eq!(alpha_names(&app), ["Alpha 3", "Alpha 1", "Alpha 2"]);
    apply_footer_drop(&mut app, ChannelsFooterButton::Delete, &[Row::Alpha(1)]);
    assert_eq!(alpha_names(&app), ["Alpha 3", "Alpha 2"], "an alpha is deleted");
    apply_footer_drop(&mut app, ChannelsFooterButton::Delete, &[Row::Color(1)]);
    assert_eq!(alpha_names(&app).len(), 2, "colour channels stay");
    apply_footer_drop(&mut app, ChannelsFooterButton::New, &[Row::Color(1)]);
    assert_eq!(alpha_names(&app).last().map(String::as_str), Some("Green copy"), "a colour row duplicates into an alpha");
    apply_footer_drop(&mut app, ChannelsFooterButton::New, &[Row::LayerMask]);
    assert_eq!(alpha_names(&app).last().map(String::as_str), Some("Masked Mask copy"), "the mask's copy names its layer");
    app.run("select.deselect", json!({})).unwrap();
    apply_footer_drop(&mut app, ChannelsFooterButton::LoadSelection, &[Row::Alpha(0)]);
    assert!(app.session.active().unwrap().doc.selection.is_some(), "a dropped alpha loads");
    // Footer clicks act on the targeted row: Save Selection adds a channel, Delete removes the targeted one.
    let count = alpha_names(&app).len();
    apply_footer(&mut app, &ctx, ChannelsFooterButton::SaveSelection, &[], Mods::default());
    assert_eq!(alpha_names(&app).len(), count + 1, "the selection is saved");
    apply_footer(&mut app, &ctx, ChannelsFooterButton::Delete, &[Row::Alpha(0)], Mods::default());
    assert_eq!(alpha_names(&app).len(), count, "the footer's trash deletes the targeted alpha");

    // The mask row on the trash asks first and deletes nothing.
    apply_footer_drop(&mut app, ChannelsFooterButton::Delete, &[Row::LayerMask]);
    assert!(crate::dialogs::alerts::prompt_tagged::<crate::mask_trash_prompt::MaskTrash>(&app).is_some(), "the prompt is open");
    assert!(masked_layer(&app).is_some(), "nothing deleted yet");
}

/// While recording, a colour row click saves as Photoshop's component select.
#[test]
fn recording_a_colour_row_click_saves_as_photoshop_select() {
    for (mode, k, name) in [("rgb", 0, "red"), ("cmyk", 0, "cyan"), ("lab", 0, "lightness")] {
        let mut app = app_with(mode);
        app.run("actions.record", json!({"name": "Click"})).unwrap();
        target(&mut app, Row::Color(k), false);
        app.run("actions.stop", json!({})).unwrap();
        let recorded = app.session.actions.list.actions().iter().find(|a| a.name == "Click").unwrap().steps.clone();
        assert_eq!(recorded, vec![ActionStep::new("channel.target", json!({"channel": {"color": name}, "extend": false}))], "{mode}");
    }
}

/// With the mask row targeted while the engine target is the composite, the footer loads the mask; a colour or alpha
/// target loads its own reference.
#[test]
fn channel_footer_follows_the_shown_target() {
    let mut app = masked_app();
    let ctx = app.ctx().clone();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
    app.run("layer.layerMask.hideAll", json!({})).unwrap();
    app.run("layer.layerMask.revealSelection", json!({})).unwrap();
    app.run("channel.target", json!({"channel": "composite"})).unwrap();
    app.ui.mask_target = true;
    let sample = |app: &PhotocraftApp, x, y| app.session.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(x, y, 0);
    let targeted = rows(&app).into_iter().find(|r| r.targeted).unwrap().key;
    assert_eq!(targeted, Row::LayerMask);
    run_drop(&mut app, targeted, ChannelsFooterButton::LoadSelection);
    assert_eq!((sample(&app, 8, 8), sample(&app, 48, 8)), (1.0, 0.0), "the mask row wins over the composite engine target");
    app.ui.mask_target = false;
    app.run("channel.target", json!({"channel": 0})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    run_drop(&mut app, Row::Alpha(0), ChannelsFooterButton::LoadSelection);
    let alpha_at = app.session.active().unwrap().doc.channels[0].surface.sample_channel(48, 8, 0);
    assert!(app.session.active().unwrap().doc.selection.is_some());
    assert_eq!(sample(&app, 48, 8), alpha_at, "the alpha target loads its own channel, not the mask");
    // Options of the mask row open the display options.
    apply(&mut app, &ctx, ChannelsEvent::Options(Row::LayerMask));
    assert_eq!(newest_dialog_channel(&app), json!("mask"));
}

/// A colour channel click recorded into an action saves as Photoshop's component select and targets the same
/// channel as the index did.
#[test]
fn recorded_colour_clicks_save_as_component_selects() {
    use photocraft_doc::ColorMode;
    for (mode, color_mode, count) in [("rgb", ColorMode::Rgb, 3), ("cmyk", ColorMode::Cmyk, 4), ("lab", ColorMode::Lab, 3)] {
        for k in 0..count {
            let mut s = photocraft_engine::Session::new();
            s.execute("file.new", json!({"width": 2, "height": 1, "mode": mode})).unwrap();
            let by_index = s.execute("channel.target", json!({"channel": Row::Color(k).reference(), "extend": false})).unwrap();
            s.execute("channel.target", json!({"channel": "composite"})).unwrap();
            let params = json!({"channel": Row::Color(k).target_reference(color_mode), "extend": false});
            assert_eq!(s.execute("channel.target", params.clone()).unwrap(), by_index, "{mode}/{k}");
            s.actions.list.push(Action::new("Click", vec![ActionStep::new("channel.target", params)])).unwrap();
            let saved = s.execute("actions.save", json!({"action": "Click"})).unwrap_or_else(|e| panic!("{mode}/{k}: {e}"));
            assert_eq!(saved["skipped"], json!([]));
            let loaded = s.execute("actions.load", json!({"data": saved["data"]})).unwrap();
            assert_eq!(loaded["unsupported"], json!([]), "{mode}/{k}");
            let name = photocraft_engine::channel_cmds::color_names(color_mode)[k].to_ascii_lowercase();
            assert_eq!(s.actions.list.actions().last().unwrap().steps, vec![ActionStep::new("channel.target", json!({"channel": {"color": name}}))]);
        }
    }
    // Rows the colour names can't express keep their reference.
    assert_eq!(Row::Color(0).target_reference(ColorMode::Grayscale), Row::Color(0).reference());
    assert_eq!(Row::Alpha(1).target_reference(ColorMode::Rgb), json!(1));
    assert_eq!(Row::QuickMask.target_reference(ColorMode::Rgb), json!("composite"));
}

/// The commands' states and what they run follow the targeted row.
#[test]
fn channel_commands_follow_the_target() {
    use ChannelsCommands::{ChannelOptions, Delete, Duplicate, MergeSpot};

    // A targeted spot channel: duplicate, options, merge and delete are all there.
    let mut app = app_with("rgb");
    let ctx = app.ctx().clone();
    app.run("channel.newSpot", json!({"name": "Ink", "fromSelection": false})).unwrap();
    app.run("channel.target", json!({"channel": 0})).unwrap();
    for command in [Duplicate, Delete, MergeSpot, ChannelOptions] {
        assert!(enabled(&app, command), "{command:?} acts on the targeted spot channel");
    }
    fire(&mut app, &ctx, ChannelOptions);
    assert_eq!(newest_dialog_channel(&app), json!(0));
    fire(&mut app, &ctx, Duplicate);
    assert_eq!(alpha_names(&app), ["Ink", "Ink copy"]);
    fire(&mut app, &ctx, Delete);
    assert_eq!(alpha_names(&app), ["Ink copy"], "the targeted channel is deleted");
    app.run("channel.target", json!({"channel": {"color": 1}})).unwrap();
    assert!(!enabled(&app, Delete), "colour channels aren't deleted here");
    fire(&mut app, &ctx, Duplicate);
    assert_eq!(alpha_names(&app), ["Ink copy", "Green copy"]);

    // The gray composite is its one channel; the RGB composite has nothing to duplicate.
    assert!(!enabled(&app_with("rgb"), Duplicate));
    let mut gray = app_with("grayscale");
    assert!(enabled(&gray, Duplicate));
    fire(&mut gray, &ctx, Duplicate);
    assert_eq!(alpha_names(&gray), ["Gray copy"]);

    // Quick Mask and the layer mask duplicate; the mask's copy is "<layer> Mask copy".
    let mut quick = app_with("rgb");
    quick.run("select.editInQuickMaskMode", json!({})).unwrap();
    fire(&mut quick, &ctx, Duplicate);
    assert_eq!(quick.session.active().unwrap().doc.channels.len(), 1);
    let mut mask = app_with("rgb");
    mask.run("layer.new.layer", json!({"name": "Masked"})).unwrap();
    mask.run("layer.layerMask.revealAll", json!({})).unwrap();
    mask.run("channel.target", json!({"channel": "mask"})).unwrap();
    assert!(!enabled(&mask, Delete), "Delete Channel waits on the layer mask row; the footer trash is its delete");
    fire(&mut mask, &ctx, Duplicate);
    assert_eq!(alpha_names(&mask), ["Masked Mask copy"]);

    // Channel Options in Quick Mask take the targeted alpha, else Quick Mask's own.
    let mut options = app_with("rgb");
    options.run("channel.new", json!({"name": "Alpha 1"})).unwrap();
    options.run("select.editInQuickMaskMode", json!({})).unwrap();
    fire(&mut options, &ctx, ChannelOptions);
    assert_eq!(newest_dialog_channel(&options), json!("quickMask"));
    options.run("channel.target", json!({"channel": 0})).unwrap();
    fire(&mut options, &ctx, ChannelOptions);
    assert_eq!(newest_dialog_channel(&options), json!(0));

    // Several colour channels targeted: nothing to duplicate.
    let mut several = app_with("rgb");
    several.run("channel.target", json!({"channel": {"color": 0}})).unwrap();
    several.run("channel.target", json!({"channel": {"color": 1}, "extend": true})).unwrap();
    assert!(!enabled(&several, Duplicate));

    // No document: nothing acts on a row.
    let none = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for command in [Duplicate, Delete, MergeSpot, ChannelOptions] {
        assert!(!enabled(&none, command), "{command:?} waits for a document");
    }

    // New Spot Channel… and New Channel… open their dialogs.
    let mut app = app_with("rgb");
    fire(&mut app, &ctx, ChannelsCommands::NewSpotChannel);
    assert!(app.dialog_open());
    let mut app = app_with("rgb");
    fire(&mut app, &ctx, ChannelsCommands::NewChannel);
    assert!(app.dialog_open());
    let id = app.framework.dialogs.open_dialogs().pop().unwrap().id;
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    assert_eq!(alpha_names(&app).len(), 1, "OK in New Channel adds a channel");
}

/// Merge Spot Channel prints the targeted spot channel into the image and removes it: a solid blue ink over the left
/// half turns those pixels blue and leaves the rest white.
#[test]
fn panel_menu_merges_the_targeted_spot_channel() {
    let mut app = app_with("rgb");
    let ctx = app.ctx().clone();
    app.run("channel.new", json!({"name": "Alpha 1"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 24})).unwrap();
    app.run("channel.newSpot", json!({"name": "Ink", "color": "#0000ff", "solidity": 100})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("channel.target", json!({"channel": 1})).unwrap();
    let pixel = |app: &PhotocraftApp, x: usize, y: usize| {
        let doc = &app.session.active().unwrap().doc;
        let comp = photocraft_compose::render(doc, doc.bounds());
        let p = comp.px[y * doc.size.width as usize + x];
        [p[0], p[1], p[2]].map(|v| (v * 255.0).round() as u8)
    };
    assert_eq!((pixel(&app, 4, 4), pixel(&app, 24, 4)), ([255, 255, 255], [255, 255, 255]), "the ink is not in the image yet");
    fire(&mut app, &ctx, ChannelsCommands::MergeSpot);
    assert_eq!(alpha_names(&app), ["Alpha 1"], "the spot channel is merged, the alpha kept");
    assert_eq!(pixel(&app, 4, 4), [0, 0, 255], "the ink is printed where the channel had it");
    assert_eq!(pixel(&app, 24, 4), [255, 255, 255], "and nowhere else");
}

/// Merge Channels… asks first, as in Photoshop: the dialog picks the mode and the document for each channel; nothing
/// merges until OK. With params the command runs directly.
#[test]
fn panel_menu_merge_channels_opens_its_dialog() {
    let mut app = app_with("grayscale");
    let ctx = app.ctx().clone();
    for _ in 0..2 {
        app.run("file.new", json!({"width": 32, "height": 24, "mode": "grayscale"})).unwrap();
    }
    fire(&mut app, &ctx, ChannelsCommands::MergeChannels);
    assert_eq!(app.session.documents().len(), 3, "nothing is merged before OK");
    assert!(app.dialog_open());
    let id = app.framework.dialogs.open_dialogs().pop().unwrap().id;
    app.framework.dialogs.set_json_field(&ctx, id, "mode", json!("lab")).unwrap();
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    let docs = app.session.documents();
    assert_eq!(docs.len(), 1, "the sources close");
    assert_eq!(docs[0].doc.mode, photocraft_doc::ColorMode::Lab, "in the mode picked");

    let mut app = app_with("grayscale");
    for _ in 0..2 {
        app.run("file.new", json!({"width": 32, "height": 24, "mode": "grayscale"})).unwrap();
    }
    crate::menus::invoke(&mut app, &ctx, "channel.merge", json!({"mode": "rgb"})).unwrap();
    assert!(!app.dialog_open() && app.session.documents().len() == 1, "explicit params merge directly");
}

/// Two grayscale documents fill no mode: Merge Channels… is greyed and the dispatcher says why instead of opening
/// the dialog; a third one enables it.
#[test]
fn merge_channels_waits_for_three_grayscale_documents() {
    let mut app = app_with("grayscale");
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 32, "height": 24, "mode": "grayscale"})).unwrap();
    let why = crate::menus::invoke(&mut app, &ctx, "channel.merge", json!({})).unwrap_err();
    assert!(why.contains("three") && !app.dialog_open(), "{why}");
    assert!(!enabled(&app, ChannelsCommands::MergeChannels), "two documents are not enough");
    app.run("file.new", json!({"width": 32, "height": 24, "mode": "grayscale"})).unwrap();
    assert!(enabled(&app, ChannelsCommands::MergeChannels), "three are");
}
