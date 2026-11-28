//! Window › Swatches: the swatch library (`photocraft_engine::presets::swatches`) in the framework's
//! `SwatchesPanel`: groups as folders, flat touching squares with name tooltips, a context menu (Rename, Delete,
//! Move to) and the footer.
//!
//! As in Photoshop, clicking a swatch sets the foreground colour (and recolours selected type or
//! fills the selected shape layers with a shape, Pen or path selection tool);
//! Alt- or Ctrl/Cmd-clicking sets the background colour. The panel menu adds New Swatch, New Group,
//! Import/Replace (`.aco`, `.act`, `.ase`), Export (`.aco`) and Export for Exchange (`.ase`) and Reset. The panel
//! reports `SwatchEdit`s; [`apply_edit`] maps each to one `swatches.*` engine command.

use photocraft_engine::color_cmds::ColorExt;
use photocraft_engine::presets::PresetsExt;
use std::ffi::OsStr;

use serde_json::{Value, json};

use craft::behavior::Selection;
use craft::cms::convert::CmykConverter;
use craft::color_ui::palette::{SwatchColor, SwatchPaint};
use craft::color_ui::ui::PickedColor;
use craft::color_ui::ui::{SwatchAt, SwatchEdit, SwatchKey, SwatchesCommand, SwatchesMenu, SwatchesPanel, SwatchesWords};
use craft::theme::Srgba;
use craft::theme::variant::Tone;
use craft::ui::bars::StatusMessage;
use craft::ui::choice::Swatch;
use craft::ui::contract::{Bind, CommandSet, Show as _};
use craft::ui::menu::PanelRow;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::patterns::presets::{PresetFileRequest, PresetWords};
use craft::ui::runtime::commands::{CommandRef, CommandState};
use craft::ui::runtime::env::Label;

use crate::PhotocraftApp;
use crate::dialogs::params::{self, ParamFamily};

/// Runs engine command `cmd`; a failure goes to the status line and gives `None`.
fn run(app: &mut PhotocraftApp, cmd: &str, params: Value) -> Option<Value> {
    match app.run(cmd, params) {
        Ok(v) => Some(v),
        Err(e) => {
            if e != "cancelled" {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            None
        }
    }
}

/// Set the foreground (or background) colour from a swatch.
pub fn use_swatch(app: &mut PhotocraftApp, name: &str, background: bool) {
    let target = if background { "background" } else { "foreground" };
    if run(app, "swatches.use", json!({"swatch": name, "target": target})).is_some() && !background {
        // Clicking a swatch recolours the selected characters (#364), or the selected type and
        // shape layers whatever the tool, as in Photoshop (#9238).
        crate::apply_color::swatch_picked(app);
    }
}

/// The converter that shows CMYK swatches and the colour models behind them: the working CMYK profile's.
/// `CmykConverter::new` shares the transforms of a profile, so building it per use costs a lookup.
fn cmyk_converter(app: &PhotocraftApp) -> Option<CmykConverter> {
    CmykConverter::new(app.session.color().working(photocraft_doc::ColorMode::Cmyk)).ok()
}

/// The words of the panel and of its file entries, from the context's environment, so the names a menu command makes
/// and the status lines follow the interface language.
fn words(ctx: &egui::Context) -> (std::sync::Arc<SwatchesWords>, std::sync::Arc<PresetWords>) {
    let env = craft::ui::runtime::env::Env::of_root(ctx);
    (env.words::<SwatchesWords>(), env.words::<PresetWords>())
}

/// The Swatches panel body.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let foreground = Srgba::from_array(app.session.tools.foreground);
    let converter = cmyk_converter(app);
    let out = {
        let mut panel = SwatchesPanel::new(&app.session.presets().swatches, Bind::bound(&mut app.ui.swatches), foreground);
        if let Some(converter) = &converter {
            panel = panel.cmyk(converter);
        }
        panel.show(ui)
    };
    if let Some((at, _, background)) = out.used {
        apply_used(app, at, background);
    }
    for edit in out.edits {
        apply_edit(app, edit);
    }
}

/// A swatch was clicked: select it and set the foreground, or the background with Alt or Cmd/Ctrl held.
pub(crate) fn apply_used(app: &mut PhotocraftApp, at: SwatchAt, background: bool) {
    let name = app.session.presets().swatches.get(at.group).and_then(|g| g.entries.get(at.index)).map(|e| e.name.clone());
    if let Some(name) = name {
        app.ui.swatches.selection = Selection::single(SwatchKey::Swatch(name.clone()));
        use_swatch(app, &name, background);
    }
}

/// The engine's colour parameter of `color`: a hex string for RGB, the model and its values (percent for CMYK) for
/// CMYK and Lab, the colour shown in sRGB for the others.
fn color_param(color: &SwatchColor, converter: Option<&CmykConverter>) -> Value {
    match color {
        SwatchColor::Cmyk(c) => json!({"model": "cmyk", "values": c.map(|v| v * 100.0)}),
        SwatchColor::Lab(l) => json!({"model": "lab", "values": l}),
        SwatchColor::Rgb(c) => json!(Srgba::from_rgb_array(*c).to_hex()),
        // Without a usable working profile the document's default coated one shows them.
        other => json!(other.to_srgb(converter.unwrap_or_else(|| CmykConverter::coated())).to_hex()),
    }
}

/// Runs one edit the panel resolved.
pub(crate) fn apply_edit(app: &mut PhotocraftApp, edit: SwatchEdit) {
    match edit {
        // PhotoCraft's swatches hold colours: the panel offers no gradient, pattern or None swatch to add.
        SwatchEdit::Add { group, name, paint: SwatchPaint::Color(color), .. } => new_swatch(app, group, name, &color),
        SwatchEdit::NewGroup { name, .. } => new_group(app, name),
        SwatchEdit::Rename { swatch, name, .. } => {
            if run(app, "swatches.rename", json!({"swatch": swatch, "name": name})).is_some() {
                // The renamed swatch stays selected under its new name.
                let (old, new) = (SwatchKey::Swatch(swatch), SwatchKey::Swatch(name));
                let selection = &mut app.ui.swatches.selection;
                for key in selection.items.iter_mut().chain(selection.lead.as_mut()).chain(selection.anchor.as_mut()) {
                    if *key == old {
                        *key = new.clone();
                    }
                }
            }
        }
        SwatchEdit::RenameGroup { group, name, .. } => {
            run(app, "swatches.renameGroup", json!({"group": group, "name": name}));
        }
        SwatchEdit::Delete { swatch, .. } => {
            if run(app, "swatches.delete", json!({"swatch": swatch})).is_some() {
                let gone = SwatchKey::Swatch(swatch);
                let selection = &mut app.ui.swatches.selection;
                selection.items.retain(|key| *key != gone);
                selection.lead = selection.lead.take().filter(|key| *key != gone);
                selection.anchor = selection.anchor.take().filter(|key| *key != gone);
            }
        }
        SwatchEdit::DeleteGroup { group, .. } => {
            run(app, "swatches.deleteGroup", json!({"group": group}));
        }
        SwatchEdit::MoveTo { swatch, group, at, .. } => {
            run(app, "swatches.move", json!({"swatch": swatch, "to": group, "position": at}));
        }
        SwatchEdit::MoveGroup { group, to, .. } => {
            run(app, "swatches.moveGroup", json!({"group": group, "to": to}));
        }
        SwatchEdit::Reset => {
            run(app, "swatches.reset", json!({}));
        }
        // A file's groups import through `import_bytes`, which has the engine parse the bytes: one parse path.
        _ => {}
    }
}

/// New Swatch: the colour named and selected in its group, then renamed in place (Photoshop asks for the name).
fn new_swatch(app: &mut PhotocraftApp, group: Option<String>, name: String, color: &SwatchColor) {
    let converter = cmyk_converter(app);
    let mut params = json!({"name": name, "color": color_param(color, converter.as_ref())});
    if let Some(group) = group {
        params["group"] = json!(group);
    }
    if let Some(r) = run(app, "swatches.add", params) {
        let name = r["name"].as_str().unwrap_or_default().to_string();
        app.ui.swatches.selection = Selection::single(SwatchKey::Swatch(name.clone()));
        app.ui.swatches.rename = Some(SwatchKey::Swatch(name));
    }
}

fn new_group(app: &mut PhotocraftApp, name: String) {
    if let Some(r) = run(app, "swatches.newGroup", json!({"name": name})) {
        let group = r["group"].as_str().unwrap_or_default().to_string();
        app.ui.swatches.rename = Some(SwatchKey::Group(group));
    }
}

/// Command params for a picked file: a real path keeps the journal small; browser files arrive
/// as bytes.
pub(crate) fn source(name: &str, bytes: &[u8]) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    if std::path::Path::new(name).is_absolute() && std::path::Path::new(name).is_file() {
        return json!({"path": name});
    }
    let _ = name;
    json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})
}

/// Import an `.aco`/`.act`/`.ase` file's bytes (File › Open, drag and drop, the panel menu).
pub fn import_bytes(app: &mut PhotocraftApp, name: &str, bytes: &[u8], replace: bool) -> Result<Value, String> {
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).filter(|s| !s.trim().is_empty());
    let mut p = source(name, bytes);
    p["mode"] = json!(if replace { "replace" } else { "append" });
    // Bytes without a path (the browser, drag and drop) are sniffed by content, and `.act` has
    // no signature: one starting 00 01 or 00 02 reads as `.aco`. Its file name decides.
    if std::path::Path::new(name).extension().and_then(OsStr::to_str).is_some_and(|e| e.eq_ignore_ascii_case("act")) {
        p["format"] = json!("act");
    }
    if let Some(stem) = stem {
        p["group"] = json!(stem);
    }
    let r = app.run("swatches.import", p)?;
    let n = r["count"].as_u64().unwrap_or(0);
    let warnings: Vec<String> = r.get("warnings").and_then(|w| serde_json::from_value(w.clone()).ok()).unwrap_or_default();
    let shown = crate::file_open::display_name(name);
    app.ui.status_error = false;
    let imported = crate::i18n::fmt(
        &crate::i18n::trn(crate::i18n::current(), n, "Imported {n} swatch from {file}", "Imported {n} swatches from {file}"),
        &[("file", &shown)],
    );
    app.ui.status = match warnings.first() {
        Some(warning) => {
            let template = "{message} (notes: {n}; {warning})";
            crate::i18n::fmt(tl!(template), &[("message", &imported), ("n", &warnings.len().to_string()), ("warning", warning)])
        }
        None => imported,
    };
    // The imported swatches are what the user looks for next.
    let _ = crate::panel_docking::show_panel(app, "swatches", Some(true));
    Ok(r)
}

/// The status line shows a framework status message.
fn show_status(app: &mut PhotocraftApp, status: &StatusMessage) {
    app.ui.status = status.text.as_str().to_owned();
    app.ui.status_error = status.tone == Tone::Bad;
}

/// Answers a file dialog a menu command asked for through the app's pickers: a pick to read imports through the engine,
/// a pick to write saves the export and says so in the status line.
fn answer_file(app: &mut PhotocraftApp, request: PresetFileRequest) {
    let started = match request {
        PresetFileRequest::Open { extensions, replace, .. } => {
            let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
            app.pick_file_bytes_filtered(&extensions, move |app, name, bytes| import_bytes(app, &name, &bytes, replace))
        }
        PresetFileRequest::Save { ref file_name, .. } => {
            let file_name = file_name.clone();
            // The opaque export report belongs to this save, not a later request. The picker never
            // calls back on Cancel or an unavailable dialog, so dropping it also drops the report.
            let mut pending = std::mem::take(&mut app.ui.swatches);
            app.ui.swatches.browser = pending.browser.clone();
            app.ui.swatches.selection = pending.selection.clone();
            app.ui.swatches.rename = pending.rename.clone();
            app.pick_save(&file_name, move |app, path| {
                let PresetFileRequest::Save { bytes, .. } = &request else { return Err(crate::file_dialog::UNEXPECTED.into()) };
                let write = app.services.write.as_mut().ok_or("no writer configured")?;
                write(&path, bytes)?;
                let shown = crate::file_open::display_name(&path);
                let env = craft::ui::runtime::env::Env::of_root(app.ctx());
                let (swatch_words, preset_words) = words(app.ctx());
                let status =
                    request.saved(&shown).and_then(|event| SwatchesPanel::apply_file(&mut pending, &event, &swatch_words, &preset_words, env.format()).status);
                if let Some(status) = status {
                    show_status(app, &status);
                } else {
                    app.ui.status_error = false;
                    app.ui.status = crate::i18n::fmt(tl!("Exported to {file}"), &[("file", &shown)]);
                }
                Ok(json!({"path": path}))
            })
        }
        _ => Ok(Value::Null),
    };
    if let Err(e) = started {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The Swatches entries of the panel menu ("…").
pub(crate) fn panel_rows(_app: &PhotocraftApp) -> Vec<PanelRow> {
    SwatchesMenu::rows()
}

/// The state of each Swatches command: both exports wait for a swatch.
pub(crate) fn command_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    SwatchesMenu::states(&app.session.presets().swatches).into_iter().map(|(command, state)| (CommandRef::from(command), state)).collect()
}

/// [`command_states`], derived from the presets the swatches live in.
pub(crate) fn source_states(app: &PhotocraftApp, name: &'static str) -> std::sync::Arc<crate::app::framework::SourceStates> {
    crate::app::framework::derived_states(app, name, &app.session.presets().revision(), || command_states(app))
}

/// Runs a fired command of the Swatches menu; false for another set's command.
pub(crate) fn run_fired(app: &mut PhotocraftApp, ctx: &egui::Context, command: &CommandRef) -> bool {
    let Some(command) = SwatchesCommand::from_ref(command) else { return false };
    let foreground = Srgba::from_array(app.session.tools.foreground);
    let (swatch_words, _) = words(ctx);
    let out = SwatchesMenu::run_command(&app.session.presets().swatches, &mut app.ui.swatches, command, foreground, &swatch_words);
    if let Some(edit) = out.edit {
        apply_edit(app, edit);
    }
    if let Some(request) = out.file {
        answer_file(app, request);
    }
    if let Some(status) = &out.status {
        show_status(app, status);
    }
    true
}

/// The key of the Color Picker's Color Swatch Name dialog (#16745).
const SWATCH_NAME: &str = "colorPicker.swatchName";

/// The Color Swatch Name dialog's result handler.
pub(crate) const SWATCH_NAME_FAMILY: ParamFamily =
    ParamFamily { check: Some(check_swatch_name), ..ParamFamily::new(|key| key == SWATCH_NAME, run_swatch_name) };

/// The Color Picker's Add to Swatches: asks for the name first in the Color Swatch Name dialog (#16745), which shows
/// the colour and offers the first free "Swatch N" selected. The colour is kept in the model the user typed it in
/// (CMYK or Lab values kept by the picker), else RGB. The picker stays open.
pub fn add_from_picker(app: &mut PhotocraftApp, picked: PickedColor) {
    let color = match picked.model {
        Some(m) if m.model == "cmyk" => json!({"model": "cmyk", "values": &m.values[..4]}),
        Some(m) if m.model == "lab" => json!({"model": "lab", "values": &m.values[..3]}),
        _ => json!(picked.color.to_hex()),
    };
    let taken = |name: &str| app.session.presets().swatches.iter().any(|g| g.entries.iter().any(|e| e.name == name));
    let template = "Swatch {n}";
    let name = (1u32..).map(|n| crate::i18n::fmt(tl!(template), &[("n", &n.to_string())])).find(|name| !taken(name)).unwrap_or_default();
    let chip = picked.color;
    params::open_param_unsaved(app, SWATCH_NAME, DialogFields::new(), json!({"color": color}), move |fields: &mut DialogFields| {
        let rows = vec![
            Param::custom("swatch", Label::Empty, move |ui, _fields, _cx| Swatch::new(Label::from(tl!("Swatch color")), chip).show(ui).response).full_width(),
            Param::new("name", tl!("Name:"), ParamKind::text(name.clone())).select_on_focus(),
        ];
        ParamDialog::command(SWATCH_NAME, tl!("Color Swatch Name"), rows).values(Bind::bound(fields))
    });
}

/// Keeps the dialog open on a blank name, which `swatches.add` refuses.
fn check_swatch_name(_app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    match result.fields.text("name") {
        Some(name) if !name.trim().is_empty() => Ok(()),
        _ => Err(Label::from(tl!("Enter a name for the swatch."))),
    }
}

/// OK: adds the colour the dialog opened with under the typed name and selects it.
fn run_swatch_name(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let color = params::captured(app, SWATCH_NAME)["color"].clone();
    if let Some(r) = run(app, "swatches.add", json!({"color": color, "name": result.fields.text("name")})) {
        let name = r["name"].as_str().unwrap_or_default().to_string();
        app.ui.status_error = false;
        let template = "Added swatch {name}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &name)]);
        app.ui.swatches.selection = Selection::single(SwatchKey::Swatch(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Tool;

    fn app() -> Box<PhotocraftApp> {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    /// Bytes without a path (the browser) go by the file name: an `.act` whose first color is
    /// 00 01 xx looks like an `.aco` header by content.
    #[test]
    fn an_act_from_bytes_imports_by_its_file_name() {
        let mut app = app();
        let mut colors = vec![[10u8, 20, 30]; 256];
        colors[0] = [0, 1, 5];
        let bytes = craft_psd::act::write(&colors, None, None).unwrap();
        let r = import_bytes(&mut app, "Palette.act", &bytes, true).unwrap();
        assert_eq!((r["format"].as_str(), r["count"].as_u64()), (Some("act"), Some(256)));
    }

    /// #5568: clicking a swatch with a shape tool and a shape layer selected fills the shape with it, one undo
    /// step each, a gradient fill included.
    #[test]
    fn swatch_click_fills_the_selected_shape() {
        use photocraft_doc::{Fill, LayerContent, LayerId};
        let mut app = app();
        app.session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        let id = LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": {"gradient": {}}})).unwrap()["layer"].as_u64().unwrap());
        app.ui.tool = crate::state::Tool::Rectangle;
        let fill = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(id).unwrap().content {
            LayerContent::Shape(sh) => sh.fill.clone(),
            _ => panic!("not a shape layer"),
        };
        let steps = |app: &PhotocraftApp| app.session.active().unwrap().history.entries().len();
        let before = steps(&app);
        let names: Vec<String> = app.session.presets().swatches[0].entries.iter().take(2).map(|s| s.name.clone()).collect();
        use_swatch(&mut app, &names[0], false);
        let fg = app.session.tools.foreground;
        assert_eq!(fill(&app), Some(Fill::Solid(photocraft_doc::Color::rgba(fg[0], fg[1], fg[2], 1.0))), "{} filled the shape", names[0]);
        assert_eq!(steps(&app), before + 1);
        use_swatch(&mut app, &names[1], false);
        let fg = app.session.tools.foreground;
        assert_eq!(fill(&app), Some(Fill::Solid(photocraft_doc::Color::rgba(fg[0], fg[1], fg[2], 1.0))), "{} filled the shape", names[1]);
        assert_eq!(steps(&app), before + 2, "each swatch click is its own undo step");
    }

    /// Using a swatch while characters are selected recolours them, not just the foreground.
    #[test]
    fn clicking_a_swatch_recolours_selected_type() {
        let mut app = app();
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let id = app.run("type.create", json!({"text": "Hello world", "size": 40, "x": 20, "y": 100, "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        app.ui.tool = Tool::Type;
        app.ui.text_edit = Some(crate::state::TextEdit { layer: id, document: None, caret: 0, anchor: 5, session: "s".into(), created: false, mask: false });
        use_swatch(&mut app, "Red", false);
        let red = [230.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0, 1.0];
        assert_eq!(app.session.tools.foreground, red);
        let st = app.session.active().unwrap();
        let Some(photocraft_doc::LayerContent::Text(t)) = st.doc.layer(photocraft_doc::LayerId(id)).map(|l| &l.content) else { panic!("type layer") };
        let runs = t.char_runs();
        assert_eq!(runs[0].len, 5, "the selection is its own run");
        assert_eq!(runs[0].style.color.to_rgba8(), [230, 40, 40, 255]);
        assert_eq!(runs[1].style.color.to_rgba8(), [255, 255, 255, 255]);
    }

    /// The panel reports Alt as `background`: the background is set, the swatch selected, the foreground kept.
    #[test]
    fn alt_click_sets_the_background_and_selects() {
        let mut app = app();
        app.session.tools.background = [0.0, 0.0, 1.0, 1.0];
        apply_used(&mut app, SwatchAt::new(0, 0), true);
        assert_eq!(app.session.tools.background, [0.0, 0.0, 0.0, 1.0], "Black");
        assert_eq!(app.session.tools.foreground, [0.0, 0.0, 0.0, 1.0], "foreground untouched (default black)");
        assert_eq!(app.ui.swatches.selection, Selection::single(SwatchKey::Swatch("Black".into())));
        // The footer's trash deletes the selection.
        let n = app.session.presets().swatches[0].entries.len();
        apply_edit(&mut app, SwatchEdit::delete("Black".into()));
        assert_eq!(app.session.presets().swatches[0].entries.len(), n - 1);
        assert_eq!(app.ui.swatches.selection, Selection::default());
    }

    #[test]
    fn new_swatch_from_the_foreground_and_picker() {
        let mut app = app();
        app.session.tools.foreground = [0.5, 0.25, 1.0, 1.0];
        let color = SwatchColor::Rgb([0.5, 0.25, 1.0]);
        apply_edit(&mut app, SwatchEdit::add(Some("Dark".into()), "Color Swatch 1".into(), color));
        let dark = app.session.presets().swatches.iter().find(|g| g.name == "Dark").unwrap();
        assert_eq!(dark.entries.last().unwrap().name, "Color Swatch 1", "into the selected swatch's group");
        assert_eq!(app.ui.swatches.rename, Some(SwatchKey::Swatch("Color Swatch 1".into())), "renamed in place");
        // The picker's Add to Swatches asks for the name first (#16745); OK adds it.
        add_from_picker(&mut app, PickedColor::cmyk(Srgba::rgb(0.0, 1.0, 1.0), [100.0, 0.0, 0.0, 0.0]));
        crate::dialogs::testing::show(&mut app);
        let id = app.framework.dialogs.open_dialogs().last().expect("the Color Swatch Name dialog").id;
        crate::dialogs::testing::confirm(&mut app, id).unwrap();
        let s = app.session.presets().swatches[0].entries.last().unwrap();
        assert_eq!(s.color(), Some(&photocraft_engine::presets::swatches::SwatchColor::Cmyk([1.0, 0.0, 0.0, 0.0])));
    }

    /// The menu's file requests go to the host file service: a read imports (a group named by the file, or a
    /// replace), an export is written as `.ase` bytes.
    #[test]
    fn import_and_export_through_the_services() {
        let aco = craft_psd::aco::write(&[craft_psd::aco::AcoSwatch { name: "Teal".into(), color: craft_psd::aco::AcoColor::Rgb([0, 32768, 32768]) }]).unwrap();
        let files: std::sync::Arc<dyn craft::host::Files> = std::sync::Arc::new(craft::host::ScriptedFiles::answering([
            craft::host::Picked::Contents(vec![("Ocean.aco".into(), aco.clone())]),
            craft::host::Picked::Contents(vec![("Ocean.aco".into(), aco)]),
            craft::host::Picked::Paths(vec!["Swatches.ase".into()]),
        ]));
        // The host takes its file service when the app is made.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { files: Some(files), ..Default::default() });
        let ctx = app.ctx().clone();
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, Vec<u8>)>::new()));
        let sink = written.clone();
        app.services.write = Some(Box::new(move |path, bytes| {
            sink.lock().unwrap().push((path.to_owned(), bytes.to_vec()));
            Ok(())
        }));
        assert!(run_fired(&mut app, &ctx, &CommandRef::from(SwatchesCommand::Import)));
        app.poll_file_dialog();
        assert_eq!(app.session.presets().swatches.last().unwrap().name, "Ocean");
        assert!(run_fired(&mut app, &ctx, &CommandRef::from(SwatchesCommand::Replace)));
        app.poll_file_dialog();
        assert_eq!(app.session.presets().swatches.len(), 1, "replace");
        assert!(run_fired(&mut app, &ctx, &CommandRef::from(SwatchesCommand::ExportExchange)));
        app.poll_file_dialog();
        let written = written.lock().unwrap();
        assert_eq!(written.len(), 1, "one file written");
        assert!(written[0].1.starts_with(b"ASEF"));
        assert!(!run_fired(&mut app, &ctx, &CommandRef::new("not.swatches")));
    }

    /// #3658: moving a group header reorders the folders in the library, and the order persists.
    #[test]
    fn dragging_a_group_header_reorders_and_persists() {
        let mut app = app();
        let names: Vec<String> = app.session.presets().swatches.iter().map(|g| g.name.clone()).collect();
        assert!(names.len() >= 2, "the default library has folders: {names:?}");
        let last = names.last().unwrap().clone();
        let rev = app.session.presets().revision();
        apply_edit(&mut app, SwatchEdit::move_group(last.clone(), 0));
        let order: Vec<String> = app.session.presets().swatches.iter().map(|g| g.name.clone()).collect();
        assert_eq!(order[0], last, "the dragged folder moved to the front: {order:?}");
        assert_eq!(order[1], names[0]);
        assert_ne!(app.session.presets().revision(), rev, "the library saves its new order");
    }

    type ExportWrites = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

    // Original swatches generated here; no external palette or reference-app data.
    fn export_fixture(answers: Vec<craft::host::Picked>, fail_write: bool, lossy: bool) -> (Box<PhotocraftApp>, ExportWrites) {
        use craft::color_ui::palette::{SwatchEntry, SwatchGroup, SwatchKind};
        let files: std::sync::Arc<dyn craft::host::Files> = std::sync::Arc::new(craft::host::ScriptedFiles::answering(answers));
        let written: ExportWrites = Default::default();
        let sink = written.clone();
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                files: Some(files),
                write: Some(Box::new(move |path, bytes| {
                    sink.lock().unwrap().push((path.to_owned(), bytes.to_vec()));
                    if fail_write { Err("export destination is read-only".into()) } else { Ok(()) }
                })),
                ..Default::default()
            },
        );
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let mut red = SwatchEntry::new("Red", SwatchColor::Rgb([1.0, 0.0, 0.0]));
        let teal = SwatchEntry::new("Teal", SwatchColor::Rgb([0.0, 0.5, 0.5]));
        app.session.presets_mut().swatches = if lossy {
            red.kind = SwatchKind::Spot;
            vec![SwatchGroup { name: "First".into(), entries: vec![red] }, SwatchGroup { name: "Second".into(), entries: vec![teal] }]
        } else {
            vec![SwatchGroup { name: "First".into(), entries: vec![red, teal] }]
        };
        app.ui.swatches.selection = Selection::single(SwatchKey::Swatch("Red".into()));
        app.ui.swatches.rename = Some(SwatchKey::Swatch("Red".into()));
        app.ui.status = "Previous status".into();
        app.ui.status_error = false;
        (app, written)
    }

    fn export_request(app: &mut PhotocraftApp, command: SwatchesCommand) -> PresetFileRequest {
        let (words, _) = words(app.ctx());
        let foreground = Srgba::from_array(app.session.tools.foreground);
        let output = SwatchesMenu::run_command(&app.session.presets().swatches, &mut app.ui.swatches, command, foreground, &words);
        assert!(output.edit.is_none() && output.status.is_none(), "asking for a file neither edits nor reports success");
        output.file.expect("the nonempty library can export")
    }

    #[test]
    fn swatch_save_reports_every_loss_after_writing_and_keeps_newer_panel_state() {
        use craft::color_ui::palette::{SwatchFormat, SwatchNote, write_swatches};
        let (mut app, written) = export_fixture(vec![craft::host::Picked::Paths(vec!["Saved.aco".into()])], false, true);
        let library = app.session.presets().swatches.clone();
        let document = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.entries();
        let colors = (app.session.tools.foreground, app.session.tools.background);
        let panel = app.ui.swatches.clone();
        let (_, report) = write_swatches(&library, SwatchFormat::Aco).unwrap();
        assert!(report.notes.contains(&SwatchNote::GroupsFlattened) && report.notes.contains(&SwatchNote::KindsDropped(1)));
        let env = craft::ui::runtime::env::Env::of_root(app.ctx());
        let (swatch_words, _) = words(app.ctx());
        let lines = SwatchesPanel::note_lines(&report, true, &swatch_words, env.format());
        let request = export_request(&mut app, SwatchesCommand::Export);
        answer_file(&mut app, request);
        assert!(written.lock().unwrap().is_empty(), "the deferred picker has not written yet");
        assert_eq!(app.ui.status, "Previous status", "loss notes wait for a successful write");
        assert_eq!(app.ui.swatches, panel, "starting the save retains the public panel state, without its pending report");
        // A save completes asynchronously: its old state must not replace newer UI choices.
        app.ui.swatches.browser.zoom += 0.125;
        app.ui.swatches.selection = Selection::single(SwatchKey::Group("Second".into()));
        app.ui.swatches.rename = Some(SwatchKey::Swatch("Teal".into()));
        let newer_panel = app.ui.swatches.clone();
        app.poll_file_dialog();
        assert_eq!(written.lock().unwrap().len(), 1);
        for line in lines {
            assert!(app.ui.status.contains(&line), "the saved status lost note {line:?}: {}", app.ui.status);
        }
        assert!(!app.ui.status_error, "a saved warning is not a write failure");
        assert_eq!(app.ui.swatches, newer_panel, "Saved only consumes its captured report");
        assert_eq!(app.session.presets().swatches, library);
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &document));
        assert_eq!(app.session.active().unwrap().history.entries(), history);
        assert_eq!((app.session.tools.foreground, app.session.tools.background), colors);
    }

    #[test]
    fn cancelled_or_failed_swatch_saves_leave_no_report_for_a_late_completion() {
        let cases = [
            (craft::host::Picked::Cancelled, false, "Previous status", 0),
            (craft::host::Picked::Unavailable("picker unavailable".into()), false, "picker unavailable", 0),
            (craft::host::Picked::Paths(vec!["Read-only.aco".into()]), true, "export destination is read-only", 1),
        ];
        for (answer, fail_write, status, attempts) in cases {
            let (mut app, written) = export_fixture(vec![answer], fail_write, true);
            let library = app.session.presets().swatches.clone();
            let document = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.entries();
            let panel = app.ui.swatches.clone();
            let request = export_request(&mut app, SwatchesCommand::Export);
            // Hostile late acknowledgement control, not an actual successful write.
            let late = request.saved("Not-written.aco").unwrap();
            answer_file(&mut app, request);
            app.poll_file_dialog();
            assert_eq!(written.lock().unwrap().len(), attempts);
            assert_eq!(app.ui.status, status);
            assert_eq!(app.ui.status_error, attempts != 0 || status != "Previous status");
            assert_eq!(app.ui.swatches, panel);
            let env = craft::ui::runtime::env::Env::of_root(app.ctx());
            let (swatch_words, preset_words) = words(app.ctx());
            let stale = SwatchesPanel::apply_file(&mut app.ui.swatches, &late, &swatch_words, &preset_words, env.format());
            assert!(stale.report.is_none() && stale.status.is_none(), "an abandoned save must not leave success notes in the live state");
            assert_eq!(app.session.presets().swatches, library);
            assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &document));
            assert_eq!(app.session.active().unwrap().history.entries(), history);
        }
    }

    #[test]
    fn successful_aco_and_ase_saves_keep_their_bytes_and_clean_status() {
        use craft::color_ui::palette::{SwatchGroup, read_swatches};
        fn color_count(groups: &[SwatchGroup]) -> usize {
            groups.iter().flat_map(|group| &group.entries).filter(|entry| entry.color().is_some()).count()
        }
        for (command, file) in [(SwatchesCommand::Export, "Saved.aco"), (SwatchesCommand::ExportExchange, "Saved.ase")] {
            let (mut app, written) = export_fixture(vec![craft::host::Picked::Paths(vec![file.into()])], false, false);
            let library = app.session.presets().swatches.clone();
            let document = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.entries();
            let panel = app.ui.swatches.clone();
            let request = export_request(&mut app, command);
            answer_file(&mut app, request);
            app.poll_file_dialog();
            let written = written.lock().unwrap();
            assert_eq!(written.len(), 1);
            assert_eq!(written[0].0, file);
            let (back, _) = read_swatches(&written[0].1, file).unwrap();
            assert_eq!(color_count(&back), color_count(&library));
            assert_eq!(app.ui.status, crate::i18n::fmt(tl!("Exported to {file}"), &[("file", file)]));
            assert!(!app.ui.status_error);
            assert_eq!(app.ui.swatches, panel);
            assert_eq!(app.session.presets().swatches, library);
            assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &document));
            assert_eq!(app.session.active().unwrap().history.entries(), history);
        }
    }
}
