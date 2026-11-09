//! Preset files opened like documents (File › Open, drag-and-drop, the Brushes panel's
//! Import Brushes…, the Preset Manager's Load…): Photoshop brushes (`.abr`) go to the brush
//! library through `brush.presets.importAbr`, gradients (`.grd`) to the Gradients panel through
//! `gradient.presets.importGrd`, layer styles (`.asl`) to the Styles panel through
//! `style.presets.importAsl`, swatches (`.aco`, `.act`, `.ase`) to the Swatches panel through
//! `swatches.import`, patterns (`.pat`) to the Patterns panel through `pattern.import`, keyboard
//! shortcut sets (`.kys`) to Edit › Keyboard Shortcuts through
//! [`crate::kys_import`], instead of opening as documents. Import Brushes… also takes an image
//! file, which becomes a brush tip through `brush.presets.importImage` (TOOL-213-12).

use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Extensions handled here rather than by the document importer (`.aco`/`.act`/`.ase` swatches go to
/// the Swatches panel through [`crate::swatches_ui::import_bytes`]).
/// `.psp` is Photoshop's live `Keyboard Shortcuts.psp` (the same XML as a `.kys`); other `.psp`
/// settings files are refused with a clear message.
pub const PRESET_EXTS: &[&str] = &["abr", "grd", "asl", "pat", "aco", "act", "ase", "kys", "psp", "csh"];

fn ext(name: &str) -> String {
    std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Is `name` a preset file this module imports?
pub fn is_preset_file(name: &str) -> bool {
    PRESET_EXTS.contains(&ext(name).as_str())
}

/// Image files Import Brushes… offers besides `.abr`: each becomes a sampled brush tip.
pub const BRUSH_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "psd", "psb", "bmp", "gif", "webp", "tga"];

/// Brushes panel / brush preset picker › Import Brushes…: an `.abr` joins the brush library as a
/// group (as File › Open does), an image file becomes a new brush preset with that image as its tip.
pub fn import_brushes(app: &mut PhotocraftApp) -> Result<Value, String> {
    let exts: Vec<&str> = std::iter::once("abr").chain(BRUSH_IMAGE_EXTS.iter().copied()).collect();
    app.pick_file_bytes_filtered(&exts, |app, name, bytes| match open(app, &name, &bytes) {
        Some(r) => r.map(|()| Value::Null),
        None => import_brush_image(app, &name, &bytes),
    })
}

/// An image file as a brush tip: a new preset named after the file, made the current brush.
pub fn import_brush_image(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    let file = std::path::Path::new(name).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string());
    let mut p = source(name, bytes);
    if p.get("data").is_some() {
        p["fileName"] = json!(file);
    }
    let r = app.run("brush.presets.importImage", p)?;
    let preset = r.get("name").and_then(Value::as_str).unwrap_or_default();
    app.ui.status_error = false;
    app.ui.status = format!("Imported brush {preset} from {file}");
    // Show it: Brush Settings › Brushes tab.
    app.ui.panels.brush_settings = true;
    app.ui.brush_tab = 1;
    Ok(r)
}

/// Import from a gradient picker through the same preset command as File › Open.
pub(crate) fn import_gradients_dialog(app: &mut PhotocraftApp) {
    let result = app.pick_file_bytes_filtered(&["grd"], |app, name, bytes| {
        if ext(&name) != "grd" {
            return Err(tl!("Choose a gradient file (.grd).").into());
        }
        open(app, &name, &bytes).ok_or_else(|| tl!("Choose a gradient file (.grd).").to_string())??;
        Ok(Value::Null)
    });
    if let Err(error) = result
        && error != crate::file_dialog::CANCELLED
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

/// Command params for the file: a real path keeps the journal small; browser files arrive as bytes.
fn source(name: &str, bytes: &[u8]) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    if std::path::Path::new(name).is_absolute() && std::path::Path::new(name).is_file() {
        return json!({"path": name});
    }
    let _ = name;
    json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})
}

/// Import a custom-shape set from the Shapes panel using the normal preset route.
pub(crate) fn import_shapes_dialog(app: &mut PhotocraftApp) {
    let result = app.pick_file_bytes_filtered(&["csh"], |app, name, bytes| {
        if ext(&name) != "csh" {
            return Err(tl!("Choose a custom shape file (.csh).").into());
        }
        open(app, &name, &bytes).ok_or_else(|| tl!("Choose a custom shape file (.csh).").to_string())??;
        Ok(Value::Null)
    });
    if let Err(error) = result
        && error != crate::file_dialog::CANCELLED
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

/// Import a preset file; `None` when `name` isn't one.
pub fn open(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Option<Result<(), String>> {
    let (cmd, what) = match ext(name).as_str() {
        "abr" => ("brush.presets.importAbr", "brushes"),
        "grd" => ("gradient.presets.importGrd", "gradients"),
        "asl" => ("style.presets.importAsl", "styles"),
        "pat" => ("pattern.import", "patterns"),
        "csh" => ("shape.presets.importCsh", "shapes"),
        "aco" | "act" | "ase" => return Some(crate::swatches_ui::import_bytes(app, name, bytes, false).map(|_| ())),
        "kys" | "psp" => return Some(crate::kys_import::open_bytes(app, name, bytes).map(|_| ())),
        _ => return None,
    };
    // Trimmed like the engine trims `group`, so the clash check sees the name the import uses.
    let stem = std::path::Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| tl!("Imported").into());
    let mut p = source(name, bytes);
    p["group"] = json!(stem);
    if what == "brushes" {
        p["select"] = json!(true);
    }
    // A group of the same name with the user's presets: ask before importing (#10640).
    if matches!(what, "brushes" | "gradients") {
        let held = crate::preset_group_prompt::Held { cmd, what, stem, params: p };
        return Some(match crate::preset_group_prompt::intercept(app, held) {
            Ok(Some(held)) => run_import(app, held.cmd, held.what, &held.stem, held.params),
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        });
    }
    Some(run_import(app, cmd, what, &stem, p))
}

/// Run the import command for a preset file and report it in the status bar.
pub(crate) fn run_import(app: &mut PhotocraftApp, cmd: &str, what: &str, stem: &str, p: Value) -> Result<(), String> {
    app.run(cmd, p).map(|r| {
        // A background import (#210): `jobs_ui` reports the count when it lands.
        if r.get("pending").and_then(Value::as_bool) == Some(true) {
            app.ui.status_error = false;
            let template = "Importing {what} from {file}…";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("what", tl!(what)), ("file", stem)]);
            return;
        }
        let n = r.get("count").and_then(Value::as_u64).unwrap_or(0);
        let warnings: Vec<String> = r.get("warnings").and_then(|w| serde_json::from_value(w.clone()).ok()).unwrap_or_default();
        app.ui.status_error = false;
        // Shape import may suffix the group when that name is taken. Report the group that
        // was stored, which the background job path already does.
        let reported_group = r.get("group").and_then(Value::as_str).filter(|g| !g.is_empty());
        let (one, other, slot, shown) = if what == "shapes" {
            ("Imported {n} shape from {group}", "Imported {n} shapes from {group}", "group", reported_group.unwrap_or(stem))
        } else {
            let (one, other) = match what {
                "brushes" => ("Imported {n} brush from {file}", "Imported {n} brushes from {file}"),
                "styles" => ("Imported {n} style from {file}", "Imported {n} styles from {file}"),
                "patterns" => ("Imported {n} pattern from {file}", "Imported {n} patterns from {file}"),
                _ => ("Imported {n} gradient from {file}", "Imported {n} gradients from {file}"),
            };
            (one, other, "file", stem)
        };
        let imported = crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n, one, other), &[(slot, shown)]);
        // Name the group too when it isn't the file's: "Kit (2)" beside an existing "Kit" (#10640).
        let imported = match r.get("group").and_then(Value::as_str) {
            Some(group) if what != "shapes" && group != stem.trim() => {
                crate::i18n::fmt(tl!("{message} into “{group}”"), &[("message", &imported), ("group", group)])
            }
            _ => imported,
        };
        app.ui.status = match warnings.first() {
            Some(warning) => {
                let template = "{message} (notes: {n}; {warning})";
                crate::i18n::fmt(tl!(template), &[("message", &imported), ("n", &warnings.len().to_string()), ("warning", warning)])
            }
            None => imported,
        };
        if what == "brushes" {
            // Show them: Brush Settings › Brushes tab.
            app.ui.panels.brush_settings = true;
            app.ui.brush_tab = 1;
        }
        if what == "shapes" {
            app.ui.presets_ui.shapes = true;
        }
        if what == "styles" {
            app.ui.presets_ui.styles = true;
        }
    })
}

#[cfg(test)]
#[path = "../../psd/tests/common/csh_fixture.rs"]
mod csh_fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn recognises_preset_files() {
        assert!(is_preset_file("/a/b/Set.ABR"));
        assert!(is_preset_file("sunsets.grd"));
        assert!(is_preset_file("Comic Styles.ASL"));
        assert!(is_preset_file("Brand.ACO"));
        assert!(is_preset_file("Palette.ACT"));
        assert!(is_preset_file("exchange.ase"));
        assert!(is_preset_file("Debaser-ColourLibrary-Part6.PAT"));
        assert!(is_preset_file("Original.CSH"));
        assert!(!is_preset_file("photo.psd"));
        assert!(!is_preset_file("abr"));
    }

    #[test]
    fn csh_open_imports_presets_and_shows_shapes_without_opening_a_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        open(&mut app, "Original.CSH", &csh_fixture::triangle_csh()).unwrap().unwrap();
        assert!(app.session.active().is_none());
        assert!(app.ui.presets_ui.shapes);
        assert!(!app.ui.status_error);
        let imported = app.session.presets.shapes.iter().find(|g| g.name == "Original").unwrap();
        assert_eq!(imported.items.len(), 1);
        assert_eq!(imported.items[0].name, "Triangle");
        assert_eq!(app.ui.status, "Imported 1 shape from Original");
        open(&mut app, "Original.CSH", &csh_fixture::triangle_csh()).unwrap().unwrap();
        assert_eq!(app.ui.status, "Imported 1 shape from Original 1");
        assert!(app.session.presets.shapes.iter().any(|g| g.name == "Original 1"));
    }

    #[test]
    fn malformed_csh_open_preserves_the_library() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let before = app.session.presets.to_json(&app.session);
        assert!(open(&mut app, "broken.csh", b"cush").unwrap().is_err());
        assert_eq!(app.session.presets.to_json(&app.session), before);
        assert!(app.session.active().is_none());
    }

    #[test]
    #[ignore = "requires a GPU adapter; renders the CSH import workflow for visual inspection"]
    fn csh_panel_import_visual_evidence() {
        let _guard = photocraft_testkit::gpu_lock();
        let (dialog, requests) =
            crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::Contents("Original.csh".into(), csh_fixture::triangle_csh()))]);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let services = crate::Services { file_dialog: Some(dialog), ..Default::default() };
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
            app.set_wgpu(cc.wgpu_render_state.as_ref().unwrap().clone());
            app.run("file.new", json!({"width": 400, "height": 300, "background": "transparent"})).unwrap();
            // A user-cleared library makes the one imported fixture visible without scrolling.
            app.session.presets.shapes.clear();
            app.ui.presets_ui.shapes = true;
            app
        });
        h.run_steps(8);
        let destination = std::env::var("PHOTOCRAFT_CSH_EVIDENCE").unwrap_or_else(|_| "csh-evidence".into());
        std::fs::create_dir_all(&destination).unwrap();
        h.render().unwrap().save(std::path::Path::new(&destination).join("before-import.png")).unwrap();
        h.get_by_label("Import Shapes…").click();
        h.run_steps(12);
        assert!(matches!(requests.borrow().first(), Some(crate::file_dialog::FileDialogRequest::Open { extensions: Some(exts), .. }) if exts == &["csh"]));
        assert_eq!(h.state().session.presets.shapes[0].items[0].name, "Triangle");
        assert!(!h.state().ui.status_error);
        h.state_mut().run("shape.presets.place", json!({"preset": "Triangle", "rect": [60, 60, 280, 180], "fill": "#d85a32"})).unwrap();
        h.run_steps(8);
        h.render().unwrap().save(std::path::Path::new(&destination).join("imported-triangle.png")).unwrap();
    }

    /// Opening `Kit.abr` when the library has a "Kit" group of the user's asks first: Import as
    /// New Group (Enter) keeps it, Cancel imports nothing, Replace deletes it (#10640).
    #[test]
    fn reimporting_a_brush_file_asks_before_touching_its_group() {
        use photocraft_psd::abr::{LegacyBrush, LegacyTip, write_v12};
        let round = |name: &str| LegacyBrush {
            name: name.into(),
            spacing: 25,
            anti_alias: true,
            tip: LegacyTip::Computed { diameter: 9, hardness: 100, angle: 0, roundness: 100 },
        };
        let abr = write_v12(2, &[round("Chalk"), round("Ink")], true).unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let in_group = |app: &PhotocraftApp, g: &str| app.session.tools.presets.iter().filter(|p| p.group == g).count();
        let prompt = |app: &PhotocraftApp| app.ui.dialogs.iter().find(|d| crate::preset_group_prompt::owns(&d.fields)).map(|d| d.id);
        open(&mut app, "Kit.abr", &abr).unwrap().unwrap();
        assert_eq!(in_group(&app, "Kit"), 2);
        assert!(prompt(&app).is_none(), "a new group imports without asking");
        assert!(app.ui.status.ends_with("brushes from Kit"), "{}", app.ui.status);
        // The user edits one of its brushes.
        let edited = app.session.tools.presets.iter_mut().find(|p| p.group == "Kit").unwrap();
        edited.brush.size = 77.0;
        let edited = edited.name.clone();

        // Cancel imports nothing.
        open(&mut app, "Kit.abr", &abr).unwrap().unwrap();
        let id = prompt(&app).expect("the prompt is open");
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert_eq!((in_group(&app, "Kit"), in_group(&app, "Kit (2)")), (2, 0));
        assert!(app.held_preset_import.is_none());

        // A file name with a trailing space (`Kit .abr`) names the same group.
        open(&mut app, "Kit .abr", &abr).unwrap().unwrap();
        let id = prompt(&app).expect("the prompt is open for the trimmed name");
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert_eq!(in_group(&app, "Kit (2)"), 0);

        // The default answer imports beside it.
        open(&mut app, "Kit.abr", &abr).unwrap().unwrap();
        let id = prompt(&app).expect("the prompt is open");
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!((in_group(&app, "Kit"), in_group(&app, "Kit (2)")), (2, 2));
        // The status names the file and the group it landed in.
        assert!(app.ui.status.ends_with("from Kit into “Kit (2)”"), "{}", app.ui.status);
        let size = |app: &PhotocraftApp| app.session.tools.presets.iter().find(|p| p.name == edited).map(|p| p.brush.size);
        assert_eq!(size(&app), Some(77.0), "the edited brush is kept");

        // Replace deletes the group's brushes first.
        open(&mut app, "Kit.abr", &abr).unwrap().unwrap();
        let id = prompt(&app).expect("the prompt is open");
        if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) {
            d.fields.insert("choice".into(), json!("replace"));
        }
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!((in_group(&app, "Kit"), in_group(&app, "Kit (2)")), (2, 2));
        assert_eq!(size(&app), Some(9.0), "the file's own brush replaced the edited one");
        assert!(prompt(&app).is_none());
    }

    /// The same prompt guards a gradient group: `Fire.grd` again asks, Cancel imports nothing,
    /// Import as New Group adds "Fire (2)", Replace swaps "Fire" alone (#10640).
    #[test]
    fn reimporting_a_gradient_file_asks_before_touching_its_group() {
        use photocraft_psd::grd::{GrdColor, GrdGradient, GrdStop, write};
        let stop = |location: f32, color| GrdStop { location, midpoint: 0.5, color };
        let ember =
            GrdGradient { name: "Ember".into(), stops: vec![stop(0.0, GrdColor::Foreground), stop(1.0, GrdColor::Rgb([1.0, 0.0, 0.0]))], ..Default::default() };
        let grd = write(&[ember]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let groups = |app: &PhotocraftApp, g: &str| app.session.presets.gradients.iter().filter(|x| x.name == g).count();
        let prompt = |app: &PhotocraftApp| app.ui.dialogs.iter().find(|d| crate::preset_group_prompt::owns(&d.fields)).map(|d| d.id);
        open(&mut app, "Fire.grd", &grd).unwrap().unwrap();
        assert_eq!(groups(&app, "Fire"), 1);
        assert!(prompt(&app).is_none(), "a new group imports without asking");

        open(&mut app, "Fire.grd", &grd).unwrap().unwrap();
        let id = prompt(&app).expect("the prompt is open");
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert_eq!((groups(&app, "Fire"), groups(&app, "Fire (2)")), (1, 0));

        open(&mut app, "Fire.grd", &grd).unwrap().unwrap();
        let id = prompt(&app).expect("the prompt is open");
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!((groups(&app, "Fire"), groups(&app, "Fire (2)")), (1, 1));
        assert!(app.ui.status.ends_with("from Fire into “Fire (2)”"), "{}", app.ui.status);

        open(&mut app, "Fire.grd", &grd).unwrap().unwrap();
        let id = prompt(&app).expect("the prompt is open");
        if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) {
            d.fields.insert("choice".into(), json!("replace"));
        }
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!((groups(&app, "Fire"), groups(&app, "Fire (2)"), groups(&app, "Fire (3)")), (1, 1, 0));
    }

    /// Import Brushes… with a PNG: the image becomes the current brush's tip (TOOL-213-12).
    #[test]
    fn import_brushes_turns_an_image_into_a_brush_tip() {
        use photocraft_engine::paint::TipShape;
        let mut px = vec![255u8; 16 * 16];
        for y in 4..12 {
            for x in 6..9 {
                px[y * 16 + x] = 0;
            }
        }
        let img = photocraft_codecs::Image::from_u8(16, 16, photocraft_codecs::ChannelLayout::Gray, px).unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let (show, asked) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::Contents("Leaf.png".into(), png))]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() });
        import_brushes(&mut app).unwrap();
        app.poll_file_dialog(&egui::Context::default(), None);
        match asked.borrow().first() {
            Some(crate::file_dialog::FileDialogRequest::Open { extensions: Some(e), .. }) => {
                assert!(e.iter().any(|x| x == "abr") && e.iter().any(|x| x == "png"), "{e:?}")
            }
            other => panic!("expected a filtered open dialog, got {other:?}"),
        }
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(app.session.tools.current_preset.as_deref(), Some("Leaf"));
        let TipShape::Sampled(tip) = &app.session.tools.brush.tip else { panic!("sampled tip expected") };
        assert_eq!((tip.width, tip.height), (3, 8));
        assert!(app.ui.panels.brush_settings && app.ui.brush_tab == 1, "the Brushes tab shows the new brush");
    }
}
