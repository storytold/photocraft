//! The Crop tool's saved presets: New Crop Preset… and Delete Crop Preset… at the end of the
//! preset list of the options bar and the task bar.
//!
//! A crop preset is a tool preset of the `"crop"` tool (`tool.presets.*`, persisted with the
//! preferences like every tool preset), so it shows in the preset list and in Window › Tool
//! Presets alike. Its options hold `{"toolOptions": {"crop": {"ratio": .., "size": ..}}}`, the
//! same shape as [`CropSettings`]; presets saved under the older `crop_*` keys are read through
//! [`crate::tool_memory::upgrade`].
//!
//! - New Crop Preset… opens a hosted parameter dialog with one Name (suggested by
//!   [`CropSettings::preset_name`]); OK saves the ratio and size fields (`tool.presets.new`).
//!   Nothing opens when there is nothing to save.
//! - Delete Crop Preset… lists the saved crop presets in a multi-select list; OK deletes the
//!   selected ones (`tool.presets.edit` delete). Nothing opens with no saved crop preset.

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::draw::ui::crop::{CropSettings, CropUserPreset};
use craft::ui::contract::{Bind, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::form_dialogs::{DialogFields, FormDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};
use std::borrow::Cow;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The tool name crop presets are saved under.
pub const TOOL: &str = "crop";
/// The longest preset name (the engine's limit for tool preset names).
pub const MAX_NAME: usize = 255;
/// The key of the New Crop Preset dialog.
const NEW_KEY: &str = "newCropPreset";
/// The key of the Delete Crop Preset dialog.
const DELETE_KEY: &str = "photocraft.deleteCropPreset";

/// An already translated text as a label.
fn label(text: impl ToString) -> Label {
    Label::from(text.to_string())
}

/// The saved crop presets, in saved order. A preset whose `toolOptions` hold no readable crop
/// settings is skipped, so an index into this list is the index the preset list and
/// `CropSettings::choose` use: every caller passes this same list.
pub fn user_presets(app: &PhotocraftApp) -> Vec<CropUserPreset> {
    app.session
        .presets
        .tool_presets
        .iter()
        .filter(|p| Tool::from_name(&p.tool) == Some(Tool::Crop))
        .filter_map(|p| {
            let mut options = p.options.get("toolOptions")?.clone();

            crate::tool_memory::upgrade(&mut options);
            let crop = options.get("crop").filter(|c| c.get("ratio").is_some())?;
            let settings: CropSettings = serde_json::from_value(crop.clone()).ok()?;
            Some(CropUserPreset::new(p.name.clone(), settings.ratio, settings.size))
        })
        .collect()
}

/// Choose a saved crop preset: its fields fill the options bar (`tool.presets.select`, the same
/// as clicking it in the Tool Presets panel).
pub fn select(app: &mut PhotocraftApp, name: &str) {
    crate::preset_panels::select_tool_preset(app, name);
}

/// Shows a failed command in the status line.
fn report(app: &mut PhotocraftApp, result: Result<(), String>) {
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The trimmed name a New Crop Preset would save (1–[`MAX_NAME`] characters). The one name rule:
/// the dialog's OK, `check` and `run` use it.
fn valid_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.chars().count() <= MAX_NAME).then(|| name.to_string())
}

/// The New Crop Preset dialog over its values: one Name; OK waits for a valid one.
fn new_dialog(fields: &mut DialogFields) -> ParamDialog<'_> {
    let ok = valid_name(fields.text("name").unwrap_or_default()).is_some();
    ParamDialog::new(NEW_KEY, tl!("New Crop Preset"), vec![Param::new("name", tl!("Name"), ParamKind::text(""))]).ok_enabled(ok).values(Bind::bound(fields))
}

/// Open New Crop Preset with the name made from the current options. `None` (nothing opens) when
/// there is nothing to save; an open one is reused.
pub fn open_new(app: &mut PhotocraftApp) -> Option<DialogId> {
    let suggested = app.ui.tool_options.crop.preset_name(&crate::crop_ui::words(app), &crate::crop_ui::units(app))?;
    let fields = DialogFields::from_pairs([("name", suggested)]);
    Some(crate::dialogs::params::open_param(app, NEW_KEY, fields, Value::Null, new_dialog))
}

/// Whether the parameter dialog `key` is New Crop Preset (its `ParamFamily` row).
pub fn owns(key: &str) -> bool {
    key == NEW_KEY
}

/// Refuses an OK without a usable name.
pub fn check(_app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    valid_name(result.fields.text("name").unwrap_or_default()).map(|_| ()).ok_or_else(|| label(tl!("Enter a name for the crop preset")))
}

/// New Crop Preset's OK: saves the ratio and size fields of the options under the name.
pub fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let Some(name) = valid_name(result.fields.text("name").unwrap_or_default()) else { return };
    let crop = &app.ui.tool_options.crop;
    let options = json!({"toolOptions": {"crop": {"ratio": crop.ratio, "size": crop.size}}});
    let outcome = app.run("tool.presets.new", json!({"name": name, "tool": TOOL, "options": options})).map(|_| ());
    report(app, outcome);
}

/// The saved names as the display order of the Delete dialog's list, keyed by name.
struct Names<'a>(&'a [String]);

impl DisplayOrder<String> for Names<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> String {
        self.0[at].clone()
    }

    fn index_of(&self, key: &String) -> Option<usize> {
        self.0.iter().position(|n| n == key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|n| Cow::Borrowed(n.as_str()))
    }
}

impl ItemSource<String> for Names<'_> {}

/// The Delete Crop Preset dialog: a multi-select list of the saved crop presets.
pub struct DeleteCropPreset {
    names: Vec<String>,
    selected: Selection<String>,
}

impl StackDialog for DeleteCropPreset {
    type Result = Vec<String>;

    fn id(&self) -> DialogId {
        dialog_id(DELETE_KEY)
    }

    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<Vec<String>>> {
        let DeleteCropPreset { names, selected } = self;
        let shown = FormDialog::new(label(tl!("Delete Crop Preset")), Bind::bound(selected)).ok_reason(label(tl!("Select the crop presets to delete"))).show(
            ui,
            |ui, _form, selected| {
                Text::new(label(tl!("Select the crop presets to delete:"))).show(ui);
                List::<String>::new(label(tl!("Crop presets"))).max_rows(8).selection(Bind::bound(&mut *selected)).show_items(
                    ui,
                    &Names(names),
                    &mut |key, _| ListRow::new(key.clone()),
                );
                !selected.items.is_empty()
            },
        );
        match (shown.accepted, shown.closed) {
            (Some(picked), _) => Some(DialogResult::Ok(picked.items)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<Vec<String>> {
        (!self.selected.items.is_empty()).then(|| self.selected.items.clone())
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({"selected": self.selected.items}))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if field != "selected" {
            return Err(format!("the Delete Crop Preset dialog has no field `{field}`"));
        }
        let picked: Vec<String> = value
            .as_array()
            .and_then(|a| a.iter().map(|v| v.as_str().map(str::to_string)).collect())
            .ok_or_else(|| format!("field `selected` takes a list of preset names, got {value}"))?;
        // Names that are no saved preset are dropped, as the list could not select them.
        let items: Vec<String> = self.names.iter().filter(|n| picked.contains(n)).cloned().collect();
        self.selected = Selection::new(items, None, None);
        ctx.request_repaint();
        Ok(())
    }
}

/// Open Delete Crop Preset. `None` when no crop preset is saved; an open one is reused.
pub fn open_delete(app: &mut PhotocraftApp) -> Option<DialogId> {
    let names: Vec<String> = user_presets(app).into_iter().map(|p| p.name).collect();
    if names.is_empty() {
        return None;
    }
    let dialog = DeleteCropPreset { names, selected: Selection::default() };
    Some(match app.framework.dialogs.open_unless_open(dialog) {
        Ok(id) => id,
        Err(open) => open.id(),
    })
}

/// Runs the answer of the Delete dialog (the `TAKERS` entry): deletes the selected presets.
pub fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some(DialogResult::Ok(names)) = app.framework.dialogs.take::<DeleteCropPreset>() else { return };
    let outcome = app.run("tool.presets.edit", json!({"action": "delete", "preset": names})).map(|_| ());
    report(app, outcome);
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::draw::ui::crop::{CropLength, CropRatio, ResolutionUnit};
    use craft::ui::runtime::env::UnitKind;
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
    use photocraft_engine::presets::tools::ToolPreset;

    fn app_over(files: &std::sync::Arc<craft::settings::testing::MemoryFiles>) -> PhotocraftApp {
        let mut doc = Document::with_background("crop", Size::new(400, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        doc.resolution_dpi = 72.0;
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);

        let services = crate::testing::services_over(files);
        let mut app = PhotocraftApp::new(s, services);
        app.ui.tool = Tool::Crop;
        app
    }

    fn names(app: &PhotocraftApp) -> Vec<String> {
        user_presets(app).into_iter().map(|p| p.name).collect()
    }

    /// OK on dialog `id`; the passes after it deliver the held answer to the family that runs it.
    fn confirm(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId) -> Result<(), String> {
        let answered = crate::dialogs::confirm(app, ctx, id);
        settle(app);
        answered
    }

    /// Runs the dialog passes a held answer needs to reach its family.
    fn settle(app: &mut PhotocraftApp) {
        for _ in 0..2 {
            crate::dialogs::testing::show(app);
        }
    }

    /// Sets a field of the open dialog `id` as `ui.dialog.set` does.
    fn set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, field: &str, value: Value) -> Result<(), String> {
        app.framework.dialogs.set_json_field(ctx, id, field, value)
    }

    /// New Crop Preset → saved → kept across a restart → listed → choosing it fills the options;
    /// an old-format preset still lists and applies; junk options are refused; Delete removes it.
    #[test]
    fn save_reload_select_and_delete() {
        let files = std::sync::Arc::new(craft::settings::testing::MemoryFiles::new());
        let mut app = app_over(&files);
        let ctx = app.ctx().clone();
        // Nothing to save and nothing saved: neither dialog opens.
        assert_eq!(open_new(&mut app), None);
        assert_eq!(open_delete(&mut app), None);

        let o = &mut app.ui.tool_options.crop;
        o.ratio = CropRatio::Size;
        o.size.width = Some(CropLength::new(6.0, UnitKind::Inches));
        o.size.height = Some(CropLength::new(9.0, UnitKind::Inches));
        o.size.resolution = "240".into();
        o.size.resolution_unit = ResolutionUnit::PixelsPerInch;
        let id = open_new(&mut app).unwrap();
        assert_eq!(open_new(&mut app), Some(id), "one dialog at a time");
        // Bad names are refused and save nothing.
        for bad in ["", "   ", &"x".repeat(MAX_NAME + 1)] {
            set(&mut app, &ctx, id, "name", json!(bad)).unwrap();
            assert!(confirm(&mut app, &ctx, id).is_err(), "{bad:?}");
        }
        assert!(names(&app).is_empty());
        set(&mut app, &ctx, id, "name", json!("  Poster  ")).unwrap();
        confirm(&mut app, &ctx, id).unwrap();
        assert_eq!(names(&app), ["Poster"]);

        // A ratio preset too.
        app.ui.tool_options.crop.ratio = CropRatio::sides(3.0, 2.0);
        let id = open_new(&mut app).unwrap();
        confirm(&mut app, &ctx, id).unwrap();
        assert_eq!(names(&app), ["Poster", "3 : 2"]);

        // An old-format preset lists and applies; junk options are not listed.
        app.session.presets.tool_presets.push(ToolPreset { name: "Old".into(), tool: "crop".into(), options: json!({"toolOptions": {"crop_ratio": "2:5"}}) });
        app.session.presets.tool_presets.push(ToolPreset { name: "Junk".into(), tool: "crop".into(), options: json!({"toolOptions": {"crop": {"ratio": 5}}}) });
        assert_eq!(names(&app), ["Poster", "3 : 2", "Old"]);
        crate::prefs_ui::save_preferences(&mut app).unwrap();

        // Restart over the same store.
        let mut fresh = app_over(&files);
        let ctx = fresh.ctx().clone();
        assert_eq!(names(&fresh), ["Poster", "3 : 2", "Old"]);
        select(&mut fresh, "Poster");
        let o = &fresh.ui.tool_options.crop;
        assert_eq!(o.ratio, CropRatio::Size);
        assert_eq!(
            (o.size.width, o.size.height, o.size.resolution.as_str()),
            (Some(CropLength::new(6.0, UnitKind::Inches)), Some(CropLength::new(9.0, UnitKind::Inches)), "240")
        );
        select(&mut fresh, "Old");
        assert_eq!(fresh.ui.tool_options.crop.ratio, CropRatio::sides(2.0, 5.0));

        // Delete the selected one; names that are no preset are ignored.
        let id = open_delete(&mut fresh).unwrap();
        set(&mut fresh, &ctx, id, "selected", json!(["Poster", "not a preset"])).unwrap();
        assert!(set(&mut fresh, &ctx, id, "selected", json!(42)).is_err());
        set(&mut fresh, &ctx, id, "selected", json!(["Poster", "not a preset"])).unwrap();
        confirm(&mut fresh, &ctx, id).unwrap();
        assert_eq!(names(&fresh), ["3 : 2", "Old"]);
        // Other tool presets are untouched.
        assert!(fresh.session.presets.tool_presets.iter().any(|p| p.name == "Soft Round 100 px"));
    }

    #[test]
    fn control_channel_opens_the_dialogs() {
        use crate::control::{ControlRequest, Reply, handle};
        let files = std::sync::Arc::new(craft::settings::testing::MemoryFiles::new());
        let mut app = app_over(&files);
        let ctx = app.ctx().clone();
        let call = |app: &mut PhotocraftApp, m: &str, p: Value| {
            let (r, _rx) = ControlRequest::new(m, p);
            match handle(app, &ctx, &r) {
                Reply::Done(v) => v,
                _ => Value::Null,
            }
        };
        assert_eq!(call(&mut app, "ui.dialog.open", json!({"kind": "newCropPreset"}))["ok"], false);
        assert_eq!(call(&mut app, "ui.dialog.open", json!({"kind": "deleteCropPreset"}))["ok"], false);
        app.ui.tool_options.crop.ratio = CropRatio::sides(16.0, 10.0);
        let id = call(&mut app, "ui.dialog.open", json!({"kind": "newCropPreset"}))["result"]["dialog"].clone();
        assert_eq!(call(&mut app, "ui.dialog.set", json!({"dialog": id, "field": "name", "value": "Wide"}))["ok"], true);
        assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
        settle(&mut app);
        assert_eq!(names(&app), ["Wide"]);
        let id = call(&mut app, "ui.dialog.open", json!({"kind": "deleteCropPreset"}))["result"]["dialog"].clone();
        call(&mut app, "ui.dialog.set", json!({"dialog": id, "field": "selected", "value": ["Wide"]}));
        assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
        settle(&mut app);
        assert!(names(&app).is_empty());
    }
}
