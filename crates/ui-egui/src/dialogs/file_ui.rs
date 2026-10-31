//! Front ends of the File-menu commands added with slices: the Save for Web (Legacy) dialog
//! (Original / Optimized / 2-Up / 4-Up previews, GIF / PNG-8 / PNG-24 / JPEG / WBMP settings,
//! image size, per-slice output), the Print dialog (page preview with position, scale, marks;
//! colour handling), and form dialogs for Contact Sheet II, Create Droplet, Statistics, Script
//! Events Manager, Package and Paths to Illustrator. Every OK runs the engine command with the
//! dialog's fields, so the control channel can drive them with `ui.dialog.set` / `confirm`.

use std::sync::Arc;

use photocraft_engine::web_cmds::{self, Optimized, WebSettings};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;
use crate::theme::Tokens;

mod print;
mod web;

pub use print::open_print;
pub(crate) use print::print_status;
#[cfg(test)]
use print::{drag_print_position, driver_paper_is_wide, print_params, print_preview_image, printer_options, take_turned_page, turn_driver};
use print::{print_body, print_confirm};
pub use web::open_web;
#[cfg(test)]
use web::{WEB_COMMAND, WebCache, WebProxy, advance_preview, download_for_web, variants, web_frame_document};
use web::{web_body, web_confirm};

fn no_params(p: &Value) -> bool {
    p.as_object().is_none_or(|o| o.is_empty())
}

fn default_dir(app: &PhotocraftApp) -> String {
    app.session.active().and_then(|d| d.path.as_deref()).and_then(|p| p.rfind(['/', '\\']).map(|i| p[..i].to_string())).unwrap_or_else(|| ".".into())
}

/// A packaged macOS app does not start in a useful writable working directory. A saved
/// document keeps its own folder; otherwise prefer the user's Documents folder.
fn droplet_default_dir(app: &PhotocraftApp) -> String {
    let dir = default_dir(app);
    if dir != "." {
        return dir;
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        let documents = home.join("Documents");
        return if documents.is_dir() { documents } else { home }.to_string_lossy().into_owned();
    }
    dir
}

/// The folder publication dialogs (Export for Publication, Make Figure) offer for an unsaved
/// document, on every platform: the user's Documents folder, else their home folder. The
/// working directory ("." — `/` for a Finder-launched app) is a last resort only without either.
fn user_default_dir(app: &PhotocraftApp) -> String {
    let dir = default_dir(app);
    if dir != "." {
        return dir;
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|h| !h.is_empty());
    match home.map(std::path::PathBuf::from) {
        Some(home) => {
            let documents = home.join("Documents");
            if documents.is_dir() { documents } else { home }.to_string_lossy().into_owned()
        }
        None => dir,
    }
}

fn doc_stem(app: &PhotocraftApp) -> String {
    app.session.active().map(|d| d.doc.name.rsplit_once('.').map_or(d.doc.name.clone(), |(a, _)| a.to_string())).unwrap_or_else(|| tl!("Untitled").into())
}

/// `fields` with the values the command remembered from its last run (same types only).
fn with_remembered(app: &PhotocraftApp, command: &str, mut fields: Value) -> Value {
    if let (Some(Value::Object(saved)), Some(f)) = (app.session.prefs().dialogs.get(command), fields.as_object_mut()) {
        for (k, v) in saved {
            if f.get(k).is_some_and(|cur| std::mem::discriminant(cur) == std::mem::discriminant(v)) {
                f.insert(k.clone(), v.clone());
            }
        }
    }
    fields
}

/// A generic form dialog for `command` (rendered by `view_cmds::form_body`).
fn form(app: &mut PhotocraftApp, command: &str, label: &str, fields: Value, choices: Value) -> Value {
    let mut f = Map::new();
    f.insert("__command".into(), json!(command));
    f.insert("__label".into(), json!(label));
    f.insert("__form".into(), json!(true));
    f.insert("__choices".into(), choices);
    if let Value::Object(m) = fields {
        f.extend(m);
    }
    json!({"dialog": app.ui.open_dialog(DialogKind::Command, f)})
}

/// File › Scripts › Browse… lists Photoshop's script types (matching its filter, #2452) plus the
/// JSON/text/droplet files the engine parses; selecting a Photoshop script gets the engine's
/// "not Photoshop ExtendScript" error instead of hiding those files from the picker.
const SCRIPT_EXTENSIONS: &[&str] = &["psjs", "ccjs", "jsx", "js", "jsxbin", "json", "txt", "pcdroplet"];

/// Menu items fronted here (dialogs before the engine command). `None` when not ours.
pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !no_params(params) {
        return None;
    }
    let dir = default_dir(app);
    let stem = doc_stem(app);
    Some(Ok(match id {
        "file.export.saveForWebLegacy" => {
            app.session.active()?;
            json!({"dialog": open_web(app)})
        }
        "file.print" => {
            app.session.active()?;
            json!({"dialog": open_print(app)})
        }
        "file.printOneCopy" if photocraft_engine::print_cmds::last_print(&app.session).is_none() => {
            app.session.active()?;
            json!({"dialog": open_print(app)})
        }
        "figure.fromCollage" => {
            app.session.active()?;
            let r = form(
                app,
                id,
                tl!("Prepare Collage for Publication"),
                with_remembered(
                    app,
                    id,
                    json!({"rows": 0, "columns": 0, "width": "double", "widthMm": 175.0, "dpi": 300, "gutterMm": 1.0, "gutterColor": "#ffffff", "labels": "ABC", "labelBold": true, "labelPosition": "topLeft", "labelSizePt": 12.0, "labelColor": "#ffffff", "labelOutline": true}),
                ),
                json!({"width": ["single", "oneAndHalf", "double", "custom"], "labels": ["ABC", "abc", "(A)", "(a)", "A)", "a)", "none"], "labelPosition": ["topLeft", "topRight", "bottomLeft", "bottomRight"]}),
            );
            if let Some(f) = r.get("dialog").and_then(Value::as_u64).and_then(|d| app.ui.dialogs.iter_mut().find(|x| x.id == d)) {
                f.fields.insert(
                    "__labels".into(),
                    json!({
                        "rows": "Rows (0 = detect)", "columns": "Columns (0 = detect)", "labelBold": "Bold labels",
                    "ABC": "A, B, C (capital)", "abc": "a, b, c (small)", "(A)": "(A), (B), (C)", "(a)": "(a), (b), (c)",
                    "A)": "A), B), C)", "a)": "a), b), c)", "width": "Figure width", "single": "Single column (85 mm)",
                        "oneAndHalf": "1.5 column (120 mm)", "double": "Double column (175 mm)", "custom": "Custom (mm below)",
                        "widthMm": "Custom width (mm)", "dpi": "Resolution (dpi)", "gutterMm": "Separator width (mm)",
                        "gutterColor": "Separator color (#rrggbb)", "labels": "Panel labels", "none": "No labels", "labelPosition": "Label corner",
                        "topLeft": "Top left", "topRight": "Top right", "bottomLeft": "Bottom left", "bottomRight": "Bottom right",
                        "labelSizePt": "Label size (pt)", "labelColor": "Label color (#rrggbb)", "labelOutline": "Outline labels for contrast",
                    }),
                );
            }
            r
        }
        "figure.make" => {
            let folder = app
                .session
                .active()
                .and_then(|st| st.path.as_deref())
                .and_then(|p| std::path::Path::new(p).parent())
                // A relative path has an empty parent: treat it as no folder.
                .filter(|d| !d.as_os_str().is_empty())
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_else(|| user_default_dir(app));
            let r = form(
                app,
                id,
                tl!("Make Figure"),
                with_remembered(
                    app,
                    id,
                    json!({"files": folder, "layout": "rows", "columns": 2, "width": "double", "widthMm": 175.0, "dpi": 300, "gutterMm": 1.0, "gutterColor": "#ffffff", "labels": "abc", "labelBold": true, "labelPosition": "topLeft", "labelSizePt": 12.0, "labelColor": "#ffffff", "labelOutline": true}),
                ),
                json!({"layout": ["rows", "grid"], "width": ["single", "oneAndHalf", "double", "custom"], "labels": ["abc", "ABC", "(a)", "(A)", "a)", "A)", "none"], "labelPosition": ["topLeft", "topRight", "bottomLeft", "bottomRight"]}),
            );
            if let Some(f) = r.get("dialog").and_then(Value::as_u64).and_then(|d| app.ui.dialogs.iter_mut().find(|x| x.id == d)) {
                f.fields.insert(
                    "__labels".into(),
                    json!({
                        "files": "Folder of images (placed in name order)", "labelBold": "Bold labels",
                    "ABC": "A, B, C (capital)", "abc": "a, b, c (small)", "(A)": "(A), (B), (C)", "(a)": "(a), (b), (c)",
                    "A)": "A), B), C)", "a)": "a), b), c)", "layout": "Layout", "rows": "Rows: same height, no cropping",
                        "grid": "Grid: equal cells", "columns": "Columns", "width": "Figure width", "single": "Single column (85 mm)",
                        "oneAndHalf": "1.5 column (120 mm)", "double": "Double column (175 mm)", "custom": "Custom (mm below)",
                        "widthMm": "Custom width (mm)", "dpi": "Resolution (dpi)", "gutterMm": "Gutter between panels (mm)",
                        "gutterColor": "Gutter color (#rrggbb)", "labels": "Panel labels", "none": "No labels", "labelPosition": "Label corner",
                        "topLeft": "Top left", "topRight": "Top right", "bottomLeft": "Bottom left", "bottomRight": "Bottom right",
                        "labelSizePt": "Label size (pt)", "labelColor": "Label color (#rrggbb)", "labelOutline": "Outline labels for contrast",
                    }),
                );
            }
            r
        }
        "file.export.publication" => {
            let st = app.session.active()?;
            // Beside the document, else Documents: never "." (the app's working directory, `/`
            // when launched from the Finder).
            let folder = st
                .path
                .as_deref()
                .and_then(|p| std::path::Path::new(p).parent())
                .filter(|d| !d.as_os_str().is_empty())
                .map(|d| d.to_string_lossy().into_owned());
            let folder = folder.unwrap_or_else(|| user_default_dir(app));
            let mut fields = with_remembered(app, id, json!({"tiff": true, "png": true, "dpi": 300, "width": "keep", "widthMm": 85.0, "folder": folder}));
            // The name states the resolution the dialog opens with (a remembered 600 included).
            let dpi = fields["dpi"].as_f64().unwrap_or(300.0).round();
            fields["name"] = json!(format!("{stem}_{dpi}dpi"));
            let r = form(app, id, tl!("Export for Publication"), fields, json!({"width": ["keep", "single", "oneAndHalf", "double", "custom"]}));
            if let Some(f) = r.get("dialog").and_then(Value::as_u64).and_then(|d| app.ui.dialogs.iter_mut().find(|x| x.id == d)) {
                f.fields.insert(
                    "__labels".into(),
                    json!({
                        "tiff": "TIFF (LZW, lossless)", "png": "PNG (lossless)", "dpi": "Resolution (dpi)", "width": "Print width",
                        "keep": "Keep pixel size", "single": "Single column (85 mm)", "oneAndHalf": "1.5 column (120 mm)",
                        "double": "Double column (175 mm)", "custom": "Custom (mm below)", "widthMm": "Custom width (mm)",
                        "folder": "Folder", "name": "File name",
                    }),
                );
            }
            r
        }
        "file.export.exportPreferences" => return crate::prefs_ui::invoke(app, ctx, "edit.preferences.export", &json!({})),
        "file.automate.contactSheetII" => form(
            app,
            id,
            tl!("Contact Sheet II"),
            json!({"input": dir, "units": "inches", "width": 8.0, "height": 10.0, "resolution": 300.0, "mode": "rgb", "depth": 8, "columns": 5, "rows": 6, "placeAcrossFirst": true, "autoSpacing": true, "rotateForBestFit": false, "caption": true, "font": photocraft_text::fonts::DEFAULT_FAMILY, "fontSize": 12.0, "flatten": false}),
            json!({"units": ["inches", "cm", "pixels"], "mode": ["rgb", "gray", "cmyk", "lab"]}),
        ),
        "file.automate.createDroplet" => {
            let dir = droplet_default_dir(app);
            let Some(action) = crate::actions::selected_action(app) else {
                return Some(Err("record an action in the Actions panel first".into()));
            };
            let steps = crate::actions::action_steps(action);
            let name = action.name.clone();
            form(
                app,
                id,
                tl!("Create Droplet"),
                json!({"path": format!("{dir}/{name}.pcdroplet"), "name": name, "steps": steps, "output": format!("{dir}/droplet-output"), "format": "same"}),
                json!({"format": ["same", "png", "jpg", "psd", "tiff"]}),
            )
        }
        "file.scripts.statistics" => json!({"dialog": crate::stack_dialog::open_statistics(app)}),
        "file.scripts.browse" => {
            return Some(app.pick_file_bytes_filtered(SCRIPT_EXTENSIONS, |app, name, bytes| {
                let r = app.run("file.scripts.browse", json!({"name": name, "script": String::from_utf8_lossy(&bytes)}));
                if r.is_ok() {
                    let template = "Ran script {name}";
                    app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &name)]);
                }
                r
            }));
        }
        "file.scripts.scriptEventsManager" => {
            let enabled = app.session.prefs().script_events.enabled;
            let events: Vec<&str> = photocraft_engine::automate_cmds::EVENTS.iter().map(|e| e.0).collect();
            form(app, id, "Script Events Manager", json!({"enabled": enabled, "event": "openDocument", "script": "", "name": ""}), json!({"event": events}))
        }
        "file.package" => form(app, id, "Package", json!({"dir": dir}), json!({})),
        "file.export.pathsToIllustrator" => {
            let mut names = vec!["all".to_string()];
            if let Some(d) = app.session.active() {
                if d.doc.work_path.is_some() {
                    names.push("work".into());
                }
                names.extend(d.doc.paths.iter().map(|p| p.name.clone()));
            }
            form(app, id, "Export Paths to File", json!({"path": format!("{dir}/{stem}.ai"), "paths": "all"}), json!({"paths": names}))
        }
        _ => return None,
    }))
}

/// Checked state of toggle items owned here.
pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    match id {
        "view.lockSlices" => Some(app.session.file_menu.slices_locked),
        "file.generate.imageAssets" => Some(app.session.active().is_some_and(|d| app.session.file_menu.image_assets.contains(&d.doc.id))),
        _ => None,
    }
}

/// Dialogs whose body and confirm live here.
pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key("__web") || f.contains_key("__print")
}

pub fn dialog_width(f: &Map<String, Value>) -> Option<f32> {
    if f.contains_key("__web") {
        Some(900.0)
    } else if f.contains_key("__print") {
        Some(720.0)
    } else {
        None
    }
}

pub fn ok_label(f: &Map<String, Value>) -> Option<&'static str> {
    if f.contains_key("__web") {
        Some(tl!("Save…"))
    } else if f.contains_key("__print") {
        Some(tl!("Print"))
    } else {
        None
    }
}

/// The body of Save for Web or Print (dialog `id`).
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, id: u64) {
    if f.contains_key("__web") {
        web_body(app, ui, f);
    } else {
        print_body(app, ui, f, id);
    }
}

pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    if f.contains_key("__web") { web_confirm(app, f) } else { print_confirm(app, f) }
}

fn params(f: &Map<String, Value>) -> Value {
    Value::Object(f.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect())
}

fn s(f: &Map<String, Value>, k: &str, d: &str) -> String {
    f.get(k).and_then(Value::as_str).unwrap_or(d).to_string()
}
fn n(f: &Map<String, Value>, k: &str, d: f64) -> f64 {
    f.get(k).and_then(Value::as_f64).unwrap_or(d)
}
fn b(f: &Map<String, Value>, k: &str, d: bool) -> bool {
    f.get(k).and_then(Value::as_bool).unwrap_or(d)
}

// ---------- Save for Web ----------

#[path = "file_ui/web_preview.rs"]
mod web_preview;

fn dropdown_str(ui: &mut egui::Ui, id: &str, name: &str, f: &mut Map<String, Value>, key: &str, opts: &[(&str, &str)], w: f32) {
    let mut v = s(f, key, opts.first().map_or("", |o| o.0));
    let o: Vec<(String, &str)> = opts.iter().map(|(k, l)| (k.to_string(), *l)).collect();
    if crate::widgets::dropdown(ui, id, name, &mut v, &o, w) {
        f.insert(key.into(), json!(v));
    }
}

fn check(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str, d: bool) {
    let mut v = b(f, key, d);
    crate::widgets::checkbox(ui, &mut v, label);
    f.insert(key.into(), json!(v));
}

fn number(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str, range: std::ops::RangeInclusive<f32>, unit: &str, d: f64) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!(&label)).color(t.text_dim));
        let mut v = n(f, key, d) as f32;
        crate::widgets::value_field(ui, &mut v, range, unit, 64.0).accessible_name(tl!(&label));
        f.insert(key.into(), json!(v.round()));
    });
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod web_playback_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    fn animated_app(depth: u32) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 24, "depth": depth})).unwrap();
        app.run("timeline.create", json!({"duration": 3, "fps": 2})).unwrap();
        app.run("layer.videoLayers.newBlankVideoLayer", json!({})).unwrap();
        for (frame, color) in ["#ff0000", "#00ff00", "#0000ff"].iter().enumerate() {
            app.run("timeline.setFrame", json!({"frame":frame})).unwrap();
            app.run("edit.fill", json!({"color":color})).unwrap();
        }
        app.run("timeline.setFrame", json!({"frame":0})).unwrap();
        app
    }

    fn harness(app: PhotocraftApp, fields: Map<String, Value>) -> Harness<'static, (PhotocraftApp, Map<String, Value>)> {
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 800.0)).with_step_dt(0.1).build_ui_state(
            |ui, state: &mut (PhotocraftApp, Map<String, Value>)| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                web_body(&mut state.0, ui, &mut state.1);
            },
            (app, fields),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(3);
        h
    }

    /// WebP (#6391): Quality until Lossless is ticked; 4-Up halves a lossy quality like JPEG's.
    #[test]
    fn webp_settings_show_quality_until_lossless() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
        let id = open_web(&mut app);
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("format".into(), json!("webp"));
        assert_eq!(fields["lossless"], false);
        let mut h = harness(app, fields);
        assert!(h.query_all_by_label("Quality").next().is_some());
        h.get_by_label("Lossless").click();
        h.run_steps(2);
        assert_eq!(h.state().1["lossless"], true);
        assert!(h.query_all_by_label("Quality").next().is_none(), "lossless has no quality");

        let v = variants(&json!({"format": "webp", "quality": 80}));
        assert_eq!((v[1]["format"].as_str(), v[1]["quality"].as_f64(), v[2]["quality"].as_f64()), (Some("webp"), Some(40.0), Some(20.0)));
        let v = variants(&json!({"format": "webp", "lossless": true}));
        assert_eq!((v[1]["format"].as_str(), v[2]["format"].as_str()), (Some("png8"), Some("gif")));
    }

    /// #9174: Save for Web reopens on the preview tab chosen last, as Photoshop's does; 2-Up at
    /// first, and for a remembered value it doesn't know.
    #[test]
    fn save_for_web_reopens_on_the_last_preview_tab() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
        let id = open_web(&mut app);
        let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["__view"], "2up");
        let mut h = harness(app, fields);
        use egui_kittest::kittest::Queryable;
        h.get_by_label("4-Up").click();
        h.run_steps(2);
        assert_eq!(h.state().1["__view"], "4up");
        let (mut app, _) = h.into_state();
        let id = open_web(&mut app);
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["__view"], "4up");
        app.ui.web_view = Some("6up".into());
        let id = open_web(&mut app);
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["__view"], "2up");
    }

    #[test]
    fn animated_save_for_web_offers_playback() {
        let mut app = animated_app(8);
        let id = open_web(&mut app);
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("__view".into(), json!("original"));
        fields.insert("format".into(), json!("gif"));
        let mut h = harness(app, fields);
        assert!(h.query_by_label("Play").is_some(), "an animated document needs a preview playback control");
        let original = h.state().0.session.active().unwrap().doc.clone();
        let revision = h.state().0.session.active().unwrap().revision;
        let history = h.state().0.session.active().unwrap().history.past_len();
        h.get_by_label("Play").click();
        h.run_steps(2);
        assert_eq!(h.state().1["__web_playing"], true);
        h.run_steps(6);
        assert_ne!(h.state().1["__web_frame"], 0);
        h.get_by_label("Pause").click();
        h.run_steps(2);
        assert_eq!(h.state().1["__web_playing"], false);
        let paused = h.state().1["__web_frame"].clone();
        h.run_steps(20);
        assert_eq!(h.state().1["__web_frame"], paused);
        h.get_by_label("Play").click();
        h.run_steps(2);
        assert_eq!(h.state().1["__web_playing"], true);
        assert_eq!(h.state().1["__web_frame"], paused, "resuming must not count time spent paused");
        h.run_steps(6);
        assert_ne!(h.state().1["__web_frame"], paused, "resuming must advance the preview again");
        let st = h.state().0.session.active().unwrap();
        assert!(Arc::ptr_eq(&st.doc, &original));
        assert_eq!(st.revision, revision);
        assert_eq!(st.history.past_len(), history);
        assert!(params(&h.state().1).as_object().unwrap().keys().all(|k| !k.starts_with("__")));
        crate::dialogs::cancel(&mut h.state_mut().0, id).unwrap();
        let next = open_web(&mut h.state_mut().0);
        let fields = &h.state_mut().0.ui.dialog_mut(next).unwrap().fields;
        assert!(!fields.contains_key("__web_playing"));
        assert!(!fields.contains_key("__web_frame"));
    }

    #[test]
    fn frame_previews_preserve_source_and_match_colors_across_depths() {
        for depth in [8, 16, 32] {
            let app = animated_app(depth);
            let source = app.session.active().unwrap().doc.clone();
            for (frame, expected) in [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]].iter().enumerate() {
                let preview = web_frame_document(&source, Some(frame)).unwrap();
                let (proxy, _, _) = web_cmds::web_document(&preview, &json!({"width":16,"height":12}), &WebSettings::default()).unwrap();
                let pixels = photocraft_compose::flatten(&proxy);
                for (actual, expected) in pixels.px[0].iter().zip(expected) {
                    assert!((actual - expected).abs() < 0.002, "depth={depth} frame={frame}");
                }
            }
            assert_eq!(source.timeline.as_ref().unwrap().current, 0);
            assert_eq!(photocraft_compose::flatten(&source).px[0], [1.0, 0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn four_up_caches_follow_the_local_frame() {
        let mut app = animated_app(8);
        let doc_id = app.session.active().unwrap().doc.id.0;
        let id = open_web(&mut app);
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("__view".into(), json!("4up"));
        fields.insert("format".into(), json!("gif"));
        let mut h = harness(app, fields);
        for frame in [1, 2, 0] {
            h.state_mut().1.insert("__web_frame".into(), json!(frame));
            h.run_steps(2);
            for pane in 0usize..4 {
                let proxy: (String, WebProxy) = h.ctx.data(|d| d.get_temp(egui::Id::unique(("web-proxy", doc_id, pane)))).unwrap();
                assert!(proxy.1.0[0][frame] > 0.99, "pane {pane} must show frame {frame}");
                let cache: WebCache = h.ctx.data(|d| d.get_temp(egui::Id::unique(("web-pane", doc_id, pane)))).unwrap();
                assert!(cache.0.contains(&format!("Some({frame})")));
            }
        }
    }
    #[test]
    fn playback_refreshes_indexed_frames_without_resetting_palette_or_navigation() {
        let mut app = animated_app(8);
        let doc_id = app.session.active().unwrap().doc.id.0;
        let id = open_web(&mut app);
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("__view".into(), json!("optimized"));
        fields.insert("format".into(), json!("gif"));
        fields.insert("__zoom".into(), json!(2.0));
        let mut h = harness(app, fields);
        // Set the custom palette after the dialog has initialized its format defaults.
        let expected_palette = json!(["#ff0000", "#00ff00", "#0000ff"]);
        h.state_mut().1.insert("colorTable".into(), expected_palette.clone());
        h.run_steps(2);
        assert_eq!(h.state().1["colorTable"], expected_palette);
        let key = egui::Id::unique(("web-pane", doc_id, 1usize)).with("indexed-source");
        let encoded = |ctx: &egui::Context| ctx.data(|d| d.get_temp::<(String, Arc<Optimized>)>(key)).unwrap().1;
        let first = encoded(&h.ctx);
        h.state_mut().1.insert("colorTableEdits".into(), json!({"0":"#ffff00"}));
        h.run_steps(2);
        assert!(Arc::ptr_eq(&first, &encoded(&h.ctx)), "palette edits reuse the current frame's indices");
        let palette = h.state().1["colorTable"].clone();
        let center = h.state().1["__center"].clone();
        let mut previous = first;
        for (frame, rgba) in [(1, [0, 255, 0, 255]), (2, [0, 0, 255, 255]), (0, [255, 0, 0, 255])] {
            h.state_mut().1.insert("__web_frame".into(), json!(frame));
            h.run_steps(2);
            let next = encoded(&h.ctx);
            assert!(!Arc::ptr_eq(&previous, &next), "a changed frame needs fresh indexed pixels");
            assert_eq!(&next.preview[..4], &rgba);
            assert_eq!(h.state().1["colorTable"], palette);
            assert_eq!(h.state().1["colorTableEdits"], json!({"0":"#ffff00"}));
            assert_eq!(h.state().1["__zoom"], 2.0);
            assert_eq!(h.state().1["__center"], center);
            assert_eq!(h.state().0.session.active().unwrap().doc.timeline.as_ref().unwrap().current, 0);
            previous = next;
        }
    }

    #[test]
    fn still_documents_have_no_playback_controls() {
        for duration in [None, Some(1)] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width":32,"height":24})).unwrap();
            if let Some(duration) = duration {
                app.run("timeline.create", json!({"duration":duration,"fps":2})).unwrap();
            }
            let id = open_web(&mut app);
            let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
            let h = harness(app, fields);
            assert!(h.query_by_label("Play").is_none());
            assert!(!h.state().1.contains_key("__web_playing"));
        }
    }

    #[test]
    fn preview_keeps_current_frame_edits_without_storing_into_source() {
        let mut app = animated_app(8);
        app.run("edit.fill", json!({"color":"#ffff00"})).unwrap();
        let source = app.session.active().unwrap().doc.clone();
        let edited = web_frame_document(&source, Some(0)).unwrap();
        let other = web_frame_document(&source, Some(1)).unwrap();
        assert_eq!(photocraft_compose::flatten(&edited).px[0], [1.0, 1.0, 0.0, 1.0]);
        assert_eq!(photocraft_compose::flatten(&other).px[0], [0.0, 1.0, 0.0, 1.0]);
        assert!(Arc::ptr_eq(&source, &app.session.active().unwrap().doc));
        assert_eq!(source.timeline.as_ref().unwrap().current, 0);
    }

    #[test]
    fn optimized_gif_preview_matches_each_frame_at_every_depth() {
        let settings = WebSettings::from_params(&json!({"format":"gif","colors":16}), "preview").unwrap();
        for depth in [8, 16, 32] {
            let app = animated_app(depth);
            for (frame, expected) in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]].iter().enumerate() {
                let doc = web_frame_document(&app.session.active().unwrap().doc, Some(frame)).unwrap();
                let (proxy, _, _) = web_cmds::web_document(&doc, &json!({"width":16,"height":12}), &settings).unwrap();
                let pixels = photocraft_compose::flatten(&proxy);
                let optimized = web_cmds::optimize(&pixels.px, 16, photocraft_geom::Rect::new(0, 0, 16, 12), &settings, None, None, None, 72.0, true).unwrap();
                assert_eq!(&optimized.preview[..4], expected, "depth={depth} frame={frame}");
            }
        }
    }
}

#[cfg(test)]
mod web_clock_tests {
    use super::*;
    #[test]
    fn frames_follow_elapsed_time_and_keep_fraction() {
        assert_eq!(advance_preview(0, 3, 2.0, 0.0, 0.49), (0, 0.0));
        assert_eq!(advance_preview(0, 3, 2.0, 0.0, 0.75), (1, 0.5));
        assert_eq!(advance_preview(1, 3, 2.0, 0.5, 1.0), (2, 1.0));
    }
    #[test]
    fn slow_frames_catch_up_and_loop() {
        assert_eq!(advance_preview(2, 3, 2.0, 0.0, 1.0), (1, 1.0));
        let (frame, last) = advance_preview(0, 3, 2.0, 0.0, 6.2);
        assert_eq!(frame, 0);
        assert!((last - 6.0).abs() < 1e-10);
    }
    #[test]
    fn resumed_clock_excludes_pause_time() {
        assert_eq!(advance_preview(1, 3, 2.0, 20.0, 20.25), (1, 20.0));
        assert_eq!(advance_preview(1, 3, 2.0, 20.0, 20.5), (2, 20.5));
    }
    #[test]
    fn fractional_frame_rates_work() {
        assert_eq!(advance_preview(0, 3, 0.5, 0.0, 2.0), (1, 2.0));
    }
    #[test]
    fn malformed_or_changed_timeline_is_bounded() {
        assert_eq!(advance_preview(usize::MAX, 0, f32::NAN, 0.0, 1.0), (0, 1.0));
        assert_eq!(advance_preview(usize::MAX - 2, usize::MAX, 2.0, 0.0, 1.0), (0, 1.0));
        assert_eq!(advance_preview(2, 3, 2.0, 10.0, 9.0), (2, 9.0));
    }
}
