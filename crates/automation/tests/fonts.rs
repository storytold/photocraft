//! `type.registerFonts` over automation (#3393): fonts are read under the read root, errors name
//! the path as the client sent it, and a step inside an action stays refused. Each test registers
//! its own family: the text engine is shared by the whole process.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use photocraft_automation::{AuthorizedWorkspace, Headless};
use photocraft_engine::actions_cmds::{Action, nested_failure};
use serde_json::{Value, json};

fn root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-automation-fonts-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn font(family: &str) -> Vec<u8> {
    photocraft_testkit::fonts::renamed_font(family).unwrap()
}

fn reading(root: &Path) -> Headless {
    Headless::with_workspace(AuthorizedWorkspace::new(Some(root), None).unwrap())
}

fn listed(h: &mut Headless, family: &str) -> bool {
    h.command_run("type.fonts", json!({})).unwrap()["families"].as_array().unwrap().iter().any(|f| f == family)
}

#[test]
fn a_headless_session_registers_fonts_from_the_read_root() {
    let root = root("headless");
    write(&root.join("fonts/brand/Root.ttf"), &font("PC3393 Automation Root"));
    write(&root.join("fonts/notes.txt"), b"not a font, not collected");
    write(&root.join("single/Single.otf"), &font("PC3393 Automation Single"));
    let mut h = reading(&root);

    let r = h.command_run("type.registerFonts", json!({"path": "fonts"})).unwrap();
    assert_eq!(r, json!({"families": ["PC3393 Automation Root"], "files": 1, "skipped": []}));
    assert!(listed(&mut h, "PC3393 Automation Root"));
    // A file, by an absolute path beneath the root.
    let absolute = root.join("single").join("Single.otf");
    let r = h.command_run("type.registerFonts", json!({"path": absolute.to_string_lossy()})).unwrap();
    assert_eq!(r["families"], json!(["PC3393 Automation Single"]));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn errors_name_the_requested_path_never_the_root() {
    let root = root("errors");
    write(&root.join("fonts/broken.ttf"), b"garbage");
    std::fs::create_dir_all(root.join("empty")).unwrap();
    let host = root.to_string_lossy().into_owned();
    let mut h = reading(&root);
    for (path, says) in [
        ("fonts/missing", "no font file or folder at `fonts/missing`"),
        ("empty", "no font files (.ttf, .otf, .ttc, .otc) in `empty`"),
        ("fonts", "no fonts could be read from `fonts`: broken.ttf: not a font file"),
        ("../escape", "automation path rejected"),
    ] {
        let error = h.command_run("type.registerFonts", json!({"path": path})).unwrap_err().to_string();
        assert!(error.contains(says) && !error.contains(&host), "{path}: {error}");
    }
    for params in [json!({}), json!({"path": 3}), json!({"path": "  "})] {
        let error = h.command_run("type.registerFonts", params.clone()).unwrap_err().to_string();
        assert!(error.contains("invalid parameters for `type.registerFonts`"), "{params}: {error}");
    }
    for mut session in [Headless::new(), Headless::with_workspace(AuthorizedWorkspace::new(None, Some(&root)).unwrap())] {
        let error = session.command_run("type.registerFonts", json!({"path": "fonts"})).unwrap_err().to_string();
        assert!(error.contains("read authority is absent"), "{error}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_step_inside_an_action_stays_refused_and_json_lines_reach_the_read_root() {
    let root = root("nested");
    write(&root.join("fonts/Nested.ttf"), &font("PC3393 Automation Nested"));
    let mut h = reading(&root);
    h.session.actions.list = vec![Action::new("Fonts", vec![("type.registerFonts".into(), json!({"path": "fonts"}))])];
    let played = h.command_run("actions.play", json!({"action": "Fonts"}));
    let refused = match &played {
        Err(error) => error.to_string(),
        Ok(result) => nested_failure("actions.play", result).unwrap_or_default(),
    };
    assert!(refused.contains("ambient filesystem paths"), "{played:?}");
    assert!(!listed(&mut h, "PC3393 Automation Nested"));

    let server = Mutex::new(h);
    let request = json!({"id": 1, "method": "engine.execute", "params": {"command": "type.registerFonts", "params": {"path": "fonts"}}});
    let mut out = Vec::new();
    photocraft_automation::rpc::serve_lines(&server, format!("{request}\n").as_bytes(), &mut out).unwrap();
    let reply: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["families"], json!(["PC3393 Automation Nested"]));
    let _ = std::fs::remove_dir_all(&root);
}
