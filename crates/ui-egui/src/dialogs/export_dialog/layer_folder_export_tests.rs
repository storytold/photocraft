//! #14808: Layer › Export As with several files to write asks for a folder and writes one
//! `<layer name>.<ext>` per layer there: illegal characters become `_`, equal names are numbered
//! from the upper layer down, existing files are replaced (a planted link too, never followed).
//! One file keeps the save dialog with the layer's name.

use super::*;
use photocraft_doc::{Color, Size};

fn named(name: &str) -> Document {
    Document::with_background(name, Size::new(4, 4), ColorMode::Rgb, photocraft_doc::SampleType::U8, Color::WHITE)
}

#[test]
fn names_keep_spaces_replace_illegal_characters_and_number_duplicates_downwards() {
    assert_eq!(layer_file_stem("a/b:c*?"), "a_b_c__");
    assert_eq!(layer_file_stem("Artboard 1"), "Artboard 1");
    assert_eq!(layer_file_stem(r#"q<u>o"t|e\s"#), "q_u_o_t_e_s");
    // Bottom to top: three `Dup`s under `Top`. The upper `Dup` keeps the name.
    let docs = [named("Dup"), named("Dup"), named("Dup"), named("Top")];
    assert_eq!(layer_file_names(&docs, "png"), ["Dup-2.png", "Dup-1.png", "Dup.png", "Top.png"]);
    // Names that differ only in case are one file on a case-insensitive destination: the upper layer
    // keeps its name, the lower one is numbered.
    let docs = [named("Logo"), named("logo")];
    assert_eq!(layer_file_names(&docs, "png"), ["Logo-1.png", "logo.png"]);
    let docs = [named("logo"), named("LOGO"), named("Logo")];
    assert_eq!(layer_file_names(&docs, "png"), ["logo-2.png", "LOGO-1.png", "Logo.png"]);
    // Windows folds names by upper-casing: a final sigma and a sigma are one file there.
    let docs = [named("ς"), named("σ")];
    assert_eq!(layer_file_names(&docs, "png"), ["ς-1.png", "σ.png"]);
    // `K` and the Kelvin sign are one name on a volume that normalises (APFS); upper-casing alone keeps them apart.
    let docs = [named("K"), named("\u{212A}")];
    assert_eq!(layer_file_names(&docs, "png"), ["K-1.png", "\u{212A}.png"]);
    // A layer already called `Dup-1` keeps its name; the numbering skips it.
    let docs = [named("Dup"), named("Dup-1"), named("Dup")];
    assert_eq!(layer_file_names(&docs, "jpg"), ["Dup-2.jpg", "Dup-1.jpg", "Dup.jpg"]);
}

/// A planted link at the destination: the test needs Unix symlinks, and so do its helpers.
#[cfg(unix)]
mod planted_link {
    use super::*;
    use craft::host::{Picked, ScriptedFiles};
    use std::{cell::RefCell, rc::Rc};

    type Written = Rc<RefCell<Vec<String>>>;

    /// A document with filled layers `a/b:c*?`, `Dup` and `Dup` (bottom to top), plus an empty layer. Its file dialogs
    /// stay open until the test answers them.
    fn app() -> (Box<PhotocraftApp>, Vec<LayerId>, LayerId, Written, Arc<ScriptedFiles>) {
        let written = Written::default();
        let capture = written.clone();
        let files = Arc::new(ScriptedFiles::held());
        let services = crate::Services {
            files: Some(files.clone()),
            export: Some(Box::new(|doc, path, settings| {
                photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(move |path, _| {
                capture.borrow_mut().push(path.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
        app.run("layer.new.layer", json!({"name": "Blank"})).unwrap();
        let empty = app.session.active().unwrap().active_layer.unwrap();
        let mut ids = Vec::new();
        for (name, color) in [("a/b:c*?", "#ff0000"), ("Dup", "#00ff00"), ("Dup", "#0000ff")] {
            app.run("layer.new.layer", json!({"name": name})).unwrap();
            app.run("paint.bucket", json!({"x": 0, "y": 0, "color": color})).unwrap();
            ids.push(app.session.active().unwrap().active_layer.unwrap());
        }
        (app, ids, empty, written, files)
    }

    /// Layer › Export As on `ids` as PNG: Export asks for a destination, which the user answers with `path`. Returns the
    /// destination request.
    fn export_layers_to(app: &mut PhotocraftApp, files: &ScriptedFiles, ids: &[LayerId], path: &std::path::Path) -> craft::host::PickRequest {
        let ctx = app.ctx().clone();
        let id = open_layers(app, ids).unwrap();
        app.framework.dialogs.set_json_field(&ctx, id, "format", json!("png")).unwrap();
        crate::dialogs::confirm(app, &ctx, id).unwrap();
        let request = files.requests().last().cloned().expect("Export asks for a destination");
        files.answer(Picked::Paths(vec![path.to_path_buf()])).expect("the destination dialog is open");
        app.poll_file_dialog();
        // The dialog's next pass takes the answered path and exports.
        crate::dialogs::confirm(app, &ctx, id).unwrap();
        request
    }

    #[test]
    fn a_planted_link_is_replaced_not_followed() {
        let guard = tempfile::tempdir().unwrap();
        let dir = guard.path().to_path_buf();
        let link = dir.join("Dup.png");
        std::os::unix::fs::symlink(dir.join("victim"), &link).unwrap();
        let (mut app, ids, _, written, files) = app();
        export_layers_to(&mut app, &files, &ids, &dir);
        assert!(link.symlink_metadata().is_err(), "the link is gone before the write");
        assert!(written.borrow().contains(&link.to_string_lossy().into_owned()));
        assert!(!dir.join("victim").exists(), "nothing was written through the link");
    }
}
