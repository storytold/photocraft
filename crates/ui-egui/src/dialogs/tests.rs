use std::sync::{Arc, Mutex};

use craft::host::{AppInfo, Host, MemorySections};
use craft::settings::Remembered;
use craft::ui::contract::Bind;
use craft::ui::dialog::{PromptAnswer, PromptChoice, StandardPrompt};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use serde_json::json;

use super::*;

/// A filter dialog over a 256×256 layer, with the preview the canvas computed for it on screen.
fn filter_dialog_with_preview(background: bool) -> (Box<PhotocraftApp>, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = background;
    app.run("file.new", serde_json::json!({"width": 256, "height": 256, "background": "#3366cc"})).unwrap();
    let id = crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
    let st = app.session.active().unwrap();
    let result = crate::filter_dialog::preview_document(&st.doc, st.active_layer, "filter.blur.gaussianBlur", &serde_json::json!({"radius": 4.0}), 1);
    let key = crate::filter_dialog::FilterPreviewKey {
        doc: st.doc.id,
        revision: st.revision,
        sources: Vec::new(),
        dialog: id,
        open: app.framework.dialogs.open_serial(id).unwrap_or(0),
        active: st.active_layer,
        target: st.channel_view.target,
        command: "filter.blur.gaussianBlur".into(),
        params: serde_json::json!({"radius": 4.0}),
        k: 1,
        tool_colors: (app.session.tools.foreground, app.session.tools.background),
        scratch: crate::scratch_reads::generation(),
    };
    app.filter_preview = Some(crate::filter_dialog::FilterPreview { key, result: result.map(std::sync::Arc::new), committing: false });
    (app, id)
}

#[test]
fn a_filters_preview_stays_on_screen_until_its_job_lands() {
    // OK dropped the preview and the filter ran as a background job, so the canvas flashed
    // the unfiltered image until the job finished.
    let (mut app, id) = filter_dialog_with_preview(true);
    let revision = app.session.active().unwrap().revision;
    testing::confirm(&mut app, id).unwrap();
    let doc = app.session.active().unwrap().doc.id;
    let job = app.session.job_on(doc).expect("the blur runs as a background job").id;
    assert!(matches!(crate::canvas::committed_filter_preview(&mut app, 0), Some(Some((1, _, [256, 256])))), "the preview is shown");
    // The job lands: the document changes and the preview goes.
    app.session.wait_job(job).unwrap();
    assert_ne!(app.session.active().unwrap().revision, revision);
    assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), Some(None));
    assert!(app.filter_preview.is_none());

    // A cancelled job leaves the document as it was, so its preview goes too.
    let (mut app, id) = filter_dialog_with_preview(true);
    testing::confirm(&mut app, id).unwrap();
    let doc = app.session.active().unwrap().doc.id;
    let job = app.session.job_on(doc).unwrap().id;
    assert!(matches!(crate::canvas::committed_filter_preview(&mut app, 0), Some(Some(_))));
    assert!(app.session.cancel_job(job));
    assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), Some(None));
    assert!(app.filter_preview.is_none());

    // Inline (no background jobs): the filter has already landed, nothing is held.
    let (mut app, id) = filter_dialog_with_preview(false);
    testing::confirm(&mut app, id).unwrap();
    assert!(app.filter_preview.is_none());
    assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), None);
}

#[test]
fn file_info_keeps_multiline_metadata_on_confirm_and_reopen() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "file.fileInfo", json!({})).unwrap();
    let id = app.framework.dialogs.stack().top().expect("File Info opens");
    for key in ["description", "copyright"] {
        app.framework.dialogs.set_json_field(&ctx, id, key, json!("First line\nSecond line")).unwrap();
    }
    confirm(&mut app, &ctx, id).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.fileInfo", json!({})).unwrap();
    let id = app.framework.dialogs.stack().top().expect("File Info reopens");
    let dialog = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("a parameter dialog");
    for key in ["description", "copyright"] {
        assert_eq!(dialog.fields().get(key), Some(&FieldValue::Text("First line\nSecond line".into())), "{key} comes back with its line break");
    }
}

/// A dialog with one colour row.
fn tint(fields: &mut DialogFields) -> ParamDialog<'_> {
    ParamDialog::new("test.tint", "Tint colour", vec![Param::new("tint", "Tint", ParamKind::color(None))]).values(Bind::bound(fields))
}

#[test]
fn a_color_row_opens_the_color_picker_as_a_child_and_its_ok_answers_the_row() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let tools = (app.session.tools.foreground, app.session.tools.background);
    let mut h =
        egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).with_context(app.ctx().clone()).build_ui_state(|ui, app| run(app, ui), app);
    let red = FieldValue::Color([1.0, 0.0, 0.0]);
    let parent = params::open_param(h.state_mut(), "test.tint", DialogFields::from_pairs([("tint", red.clone())]), serde_json::Value::Null, tint);
    h.run_steps(4);

    // The row's colour button asks for a colour: the picker opens over the dialog.
    h.get_by_role_and_label(egui::accesskit::Role::ColorWell, "Tint").click();
    h.run_steps(4);
    let picker = h.state().framework.dialogs.stack().top().expect("a dialog is open");
    assert_ne!(picker, parent, "the Color Picker is a dialog of its own");
    assert_eq!(h.state().framework.dialogs.stack().parent(picker), Some(parent), "it opens as the dialog's child");

    // The dialog's value moves on while the picker is open; the picker's OK answers the row with the colour it was opened with.
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, parent, "tint", serde_json::to_value(FieldValue::Color([0.0, 1.0, 0.0])).unwrap()).unwrap();
    testing::confirm(h.state_mut(), picker).unwrap();
    h.run_steps(4);
    let dialog = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(parent).expect("the dialog stays open");
    assert_eq!(dialog.fields().get("tint"), Some(&red), "the picker's colour reached the row");
    assert_eq!((h.state().session.tools.foreground, h.state().session.tools.background), tools, "the tool colours stay as they were");
}

#[test]
fn the_one_time_move_stores_owned_dialog_values_once_and_leaves_engine_read_keys() {
    let host = Host::builder(AppInfo { id: "photocraft", name: "PhotoCraft", version: "0" }).settings(Arc::new(MemorySections::default())).build();
    let mut prefs = photocraft_engine::prefs::Preferences::default();
    let dialogs = &mut prefs.dialogs;
    // Generic: the old key under `photocraft.`, nulls dropped.
    dialogs.insert("filter.gaussianBlur".into(), json!({"radius": 4.0, "preview": null}));
    // Routed: a library crate names the slot.
    dialogs.insert("shape.create.ellipse".into(), json!({"default": {"width": 3}}));
    // The Fill mode is stored as the layers panel's key.
    dialogs.insert("edit.fill".into(), json!({"mode": "colorBurn"}));
    // Kept: the engine reads these, and a don't-show-again entry is told by its key.
    dialogs.insert("file.print".into(), json!({"copies": 2}));
    dialogs.insert("ui.layersPanelOptions".into(), json!({"useDefaultMasksOnFillLayers": true}));
    dialogs.insert("file.export.saveForWebLegacy".into(), json!({"format": "jpeg"}));
    dialogs.insert(live_shape_prompt::PREF.into(), json!({"skip": true}));

    move_memories(&mut prefs, &host);

    let table = |scope: &str| Remembered::<toml::Table>::new(scope).recall(&host);
    assert_eq!(table("photocraft.filter.gaussianBlur").expect("moved").get("radius"), Some(&toml::Value::Float(4.0)));
    assert!(!table("photocraft.filter.gaussianBlur").unwrap().contains_key("preview"), "a null drops");
    assert!(table("craft_draw.shape.create.ellipse").is_some(), "a routed key lands under its library scope");
    assert!(table("photocraft.shape.create.ellipse").is_none());
    assert_eq!(table("photocraft.edit.fill").unwrap().get("mode"), Some(&toml::Value::String(photocraft_color::BlendMode::ColorBurn.key().to_string())));
    let mut left: Vec<&str> = prefs.dialogs.keys().map(String::as_str).collect();
    left.sort_unstable();
    // The Content-Aware Fill entry an empty recorded step reads is given to every profile, empty here (#14254).
    let mut kept =
        vec!["file.print", "ui.layersPanelOptions", "file.export.saveForWebLegacy", live_shape_prompt::PREF, photocraft_engine::atn_cmds::CAF_WORKSPACE_PREFS];
    kept.sort_unstable();
    assert_eq!(left, kept, "only the kept entries stay in the preferences");
    assert_eq!(prefs.dialogs.get(photocraft_engine::atn_cmds::CAF_WORKSPACE_PREFS), Some(&json!({})));

    // A second run finds nothing to move: a value changed since is not overwritten.
    Remembered::<toml::Table>::new("photocraft.filter.gaussianBlur").store(&host, &toml::Table::new());
    move_memories(&mut prefs, &host);
    assert_eq!(table("photocraft.filter.gaussianBlur"), Some(toml::Table::new()));
}

/// The Content-Aware Fill settings an empty recorded step reads stay in the preferences; the workspace's own memory
/// gets them only while it has none (#14254).
#[test]
fn the_move_keeps_the_content_aware_fill_settings_and_never_replaces_the_workspaces() {
    let host = Host::builder(AppInfo { id: "photocraft", name: "PhotoCraft", version: "0" }).settings(Arc::new(MemorySections::default())).build();
    let key = photocraft_engine::atn_cmds::CAF_WORKSPACE_PREFS;
    let workspace = Remembered::<toml::Table>::new(params::memory_scope(key));
    let output = |table: Option<toml::Table>| table.and_then(|t| t.get("output").cloned());
    let mut prefs = photocraft_engine::prefs::Preferences::default();
    prefs.dialogs.insert(key.into(), json!({"output": "current"}));
    move_memories(&mut prefs, &host);
    assert_eq!(prefs.dialogs.get(key), Some(&json!({"output": "current"})), "the engine's copy stays");
    assert_eq!(output(workspace.recall(&host)), Some(toml::Value::String("current".into())), "an empty workspace memory gets it");

    prefs.dialogs.insert(key.into(), json!({"output": "new"}));
    move_memories(&mut prefs, &host);
    assert_eq!(output(workspace.recall(&host)), Some(toml::Value::String("current".into())), "the workspace's own memory wins");
    assert_eq!(prefs.dialogs.get(key), Some(&json!({"output": "new"})));
}

struct MarkA;
struct MarkB;

static HITS: Mutex<Vec<(&'static str, PromptChoice)>> = Mutex::new(Vec::new());

fn row_a(_: &mut PhotocraftApp, _: &egui::Context, answer: &mut PromptAnswer) -> bool {
    answer.tag::<MarkA>().is_some_and(|_| {
        HITS.lock().unwrap().push(("a", answer.choice));
        true
    })
}

fn row_b(_: &mut PhotocraftApp, _: &egui::Context, answer: &mut PromptAnswer) -> bool {
    answer.tag::<MarkB>().is_some_and(|_| {
        HITS.lock().unwrap().push(("b", answer.choice));
        true
    })
}

/// A custom prompt answers a programmatic OK only through a button that takes it.
fn with_buttons(prompt: StandardPrompt) -> StandardPrompt {
    prompt.button(PromptChoice::Proceed, "OK").button(PromptChoice::Cancel, "Cancel")
}

#[test]
fn a_prompt_answer_reaches_its_own_entry_once_and_a_host_cancel_reaches_none() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    // The rows are listed with B first: A's answer must pass B's row and still reach A's.
    let rows: &[PromptRow] = &[row_b, row_a];
    let hits = || HITS.lock().unwrap().clone();

    // A dialog can be answered once a pass has shown it.
    let first = app.framework.dialogs.open(with_buttons(StandardPrompt::custom("A")).tag(MarkA));
    testing::show(&mut app);
    assert!(app.framework.dialogs.confirm_by_id(&ctx, first));
    take_prompts_from(&mut app, &ctx, rows);
    take_prompts_from(&mut app, &ctx, rows);
    assert_eq!(hits(), vec![("a", PromptChoice::Proceed)], "A's answer reached A's row, once");

    // A prompt opened over another of its type replaces it: the replaced one's host Cancel acts on nothing.
    app.framework.dialogs.open(with_buttons(StandardPrompt::custom("A again")).tag(MarkA));
    let second = app.framework.dialogs.open(with_buttons(StandardPrompt::custom("B")).tag(MarkB));
    testing::show(&mut app);
    take_prompts_from(&mut app, &ctx, rows);
    assert_eq!(hits().len(), 1, "a host Cancel reaches no row");
    assert!(app.framework.dialogs.confirm_by_id(&ctx, second));
    take_prompts_from(&mut app, &ctx, rows);
    assert_eq!(hits(), vec![("a", PromptChoice::Proceed), ("b", PromptChoice::Proceed)], "the loop went on past the host Cancel");

    // A prompt's own Cancel button answers `Ok` with `Cancel`, and its row acts on it.
    let third = app.framework.dialogs.open(with_buttons(StandardPrompt::custom("B")).tag(MarkB));
    app.framework.dialogs.get_mut_by_id::<StandardPrompt>(third).unwrap().set_choice(PromptChoice::Cancel);
    testing::show(&mut app);
    assert!(app.framework.dialogs.confirm_by_id(&ctx, third));
    take_prompts_from(&mut app, &ctx, rows);
    assert_eq!(hits().last(), Some(&("b", PromptChoice::Cancel)));
}
