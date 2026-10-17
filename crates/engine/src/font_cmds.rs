//! Font files (#3393): `type.registerFonts` adds a font file or folder to the process-wide text
//! engine ([`photocraft_text::shared`]), so `type.fonts`, type layers and PSD imports in every
//! session of the process can use fonts that are neither bundled nor installed.

use serde_json::{Value, json};

use crate::{CommandSpec, EngineError, Result};
pub use photocraft_text::RegisteredFonts;

/// The command id. Automation runs it through its read root instead of the host path.
pub const REGISTER: &str = "type.registerFonts";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: REGISTER.into(), msg: msg.into() }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: REGISTER,
        label: "Register Fonts",
        menu: &[],
        shortcut: None,
        params: r##"{"path":str (a .ttf/.otf/.ttc/.otc file, or a folder searched 8 levels deep for them; over automation, under the read root)} → {"families":[str],"files":n,"skipped":[{"file":str,"error":str}]}; for the whole process, no history step"##,
        enabled: |_| Ok(()),
        // Like `type.fonts`: no document changes, so nothing to undo or record in an action.
        journal: false,
        run: |_, p| Ok(result_json(&register_path_param(path_param(p)?)?)),
    }]
}

/// The `path` parameter of `type.registerFonts`: a non-blank string.
pub fn path_param(p: &Value) -> Result<&str> {
    p.get("path").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| bad("`path` must name a font file or folder"))
}

#[cfg(not(target_arch = "wasm32"))]
fn register_path_param(path: &str) -> Result<RegisteredFonts> {
    register_font_path(std::path::Path::new(path))
}

#[cfg(target_arch = "wasm32")]
fn register_path_param(_: &str) -> Result<RegisteredFonts> {
    Err(EngineError::Other("registering fonts from a path is not available on the web".into()))
}

/// Registers a font file, or every font file in a folder, in the shared text engine
/// ([`photocraft_text::FontDb::register_font_path`], default limits). `type.registerFonts` and
/// the CLI's `--font-dir` use it.
#[cfg(not(target_arch = "wasm32"))]
pub fn register_font_path(path: &std::path::Path) -> Result<RegisteredFonts> {
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    engine.fonts.register_font_path(path, photocraft_text::FontFileLimits::DEFAULT).map_err(font_error)
}

/// Registers font files already read, `(name, bytes or why they couldn't be read)`, in the shared
/// text engine ([`photocraft_text::FontDb::register_font_files`], default limits): automation
/// reads them through its read root. `source` names them in errors, as the client gave it.
pub fn register_font_files(source: &str, files: Vec<(String, std::result::Result<Vec<u8>, String>)>) -> Result<RegisteredFonts> {
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    engine.fonts.register_font_files(source, files, photocraft_text::FontFileLimits::DEFAULT).map_err(font_error)
}

/// The `type.registerFonts` result.
pub fn result_json(r: &RegisteredFonts) -> Value {
    let skipped: Vec<Value> = r.skipped.iter().map(|(file, error)| json!({"file": file, "error": error})).collect();
    json!({"families": r.families, "files": r.files, "skipped": skipped})
}

fn font_error(e: photocraft_text::FontFileError) -> EngineError {
    match e {
        photocraft_text::FontFileError::Io { .. } => EngineError::Other(e.to_string()),
        _ => bad(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use std::path::{Path, PathBuf};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("photocraft-font-cmds-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A font file of its own family: the text engine is shared by every test in the process.
    fn write_font(path: &Path, family: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, photocraft_testkit::fonts::renamed_font(family).unwrap()).unwrap();
    }

    fn register(s: &mut Session, path: &Path) -> Result<Value> {
        s.execute(REGISTER, json!({ "path": path.to_string_lossy() }))
    }

    fn listed(s: &mut Session, family: &str) -> bool {
        s.execute("type.fonts", json!({})).unwrap()["families"].as_array().unwrap().iter().any(|f| f == family)
    }

    #[test]
    fn registered_fonts_are_listed_without_changing_the_document() {
        let dir = temp_dir("listed");
        write_font(&dir.join("brand/Listed.ttf"), "PC3393 Engine Listed");
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let revision = s.active().unwrap().revision;
        assert!(!listed(&mut s, "PC3393 Engine Listed"));

        let r = register(&mut s, &dir).unwrap();
        assert_eq!(r, json!({"families": ["PC3393 Engine Listed"], "files": 1, "skipped": []}));
        assert!(listed(&mut s, "PC3393 Engine Listed"));
        let faces = s.execute("type.fonts", json!({"family": "PC3393 Engine Listed"})).unwrap();
        assert_eq!(faces["faces"].as_array().map(Vec::len), Some(1), "{faces}");
        let st = s.active().unwrap();
        assert_eq!(st.revision, revision, "no document change");
        assert!(!st.history.can_undo(), "no history step");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registering_fonts_is_not_recorded_in_an_action() {
        let dir = temp_dir("record");
        write_font(&dir.join("Record.ttf"), "PC3393 Engine Record");
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("actions.record", json!({"name": "PC3393 fonts"})).unwrap();
        register(&mut s, &dir.join("Record.ttf")).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let stopped = s.execute("actions.stop", json!({})).unwrap();
        assert_eq!(stopped["steps"], 1, "only the new layer is recorded: {stopped}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_type_layer_in_a_registered_family_is_no_longer_missing_its_font() {
        let dir = temp_dir("layer");
        write_font(&dir.join("Layer.otf"), "PC3393 Engine Layer");
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 32})).unwrap();
        s.execute("type.create", json!({"x": 2, "y": 20, "text": "Brand", "font": "PC3393 Engine Layer", "size": 12})).unwrap();
        let missing = |s: &Session| crate::type_extra_cmds::missing_fonts(&s.active().unwrap().doc);
        assert_eq!(missing(&s), ["PC3393 Engine Layer"]);
        register(&mut s, &dir).unwrap();
        assert!(missing(&s).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_read_elsewhere_register_and_errors_name_the_given_source() {
        let bytes = photocraft_testkit::fonts::renamed_font("PC3393 Engine Bytes").unwrap();
        let r = register_font_files("fonts/brand", vec![("Bytes.ttf".into(), Ok(bytes)), ("gone.ttf".into(), Err("access denied".into()))]).unwrap();
        assert_eq!(r.families, ["PC3393 Engine Bytes"]);
        assert_eq!(r.skipped, [("gone.ttf".to_string(), "access denied".to_string())]);
        assert!(listed(&mut Session::new(), "PC3393 Engine Bytes"));
        let e = register_font_files("fonts/empty", Vec::new()).unwrap_err();
        assert!(matches!(&e, EngineError::BadParams { .. }) && e.to_string().contains("`fonts/empty`"), "{e}");
    }

    #[test]
    fn bad_params_missing_paths_non_fonts_and_empty_folders_are_errors() {
        let dir = temp_dir("errors");
        let mut s = Session::new();
        for p in [json!({}), json!({"path": null}), json!({"path": 5}), json!({"path": ["a.ttf"]}), json!({"path": ""}), json!({"path": "  "})] {
            assert!(matches!(s.execute(REGISTER, p.clone()), Err(EngineError::BadParams { cmd, .. }) if cmd == REGISTER), "{p}");
        }
        std::fs::write(dir.join("notes.txt"), b"not a font").unwrap();
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        for (path, says) in [("missing.ttf", "no font file or folder"), ("notes.txt", "not a font file"), ("empty", "no font files")] {
            let e = register(&mut s, &dir.join(path)).unwrap_err();
            assert!(matches!(&e, EngineError::BadParams { .. }) && e.to_string().contains(says), "{path}: {e}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
