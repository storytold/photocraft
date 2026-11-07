//! Plug-ins limited to the ones the app brings (App Store guideline 2.5.2) hold below any menu:
//! whatever runs the commands (a menu, a script's or an action's steps, a control client) can't
//! install a plug-in from data or a file, load a folder of them, or have the Plug-ins preference
//! folder load one. Its own test binary: the policy is process-wide, as the plug-in registry is.

use photocraft_engine::{Session, plugin_cmds};
use serde_json::json;

const INVERT: &[u8] = include_bytes!("../../plugins/tests/fixtures/invert.wasm");
const INVERT_ID: &str = "org.photocraft.example.invert";

fn b64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    out
}

fn installed(s: &mut Session) -> bool {
    let list = s.execute("plugin.list", json!({})).unwrap();
    list["plugins"].as_array().unwrap().iter().any(|p| p["id"] == INVERT_ID)
}

#[test]
fn bundled_only_refuses_every_way_of_adding_a_plug_in() {
    let dir = std::env::temp_dir().join(format!("photocraft-bundled-plugins-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("invert.wasm");
    std::fs::write(&file, INVERT).unwrap();
    let folder = dir.to_string_lossy().into_owned();

    plugin_cmds::set_bundled_only(true);
    let mut s = Session::new();
    for (id, params) in [
        ("plugin.install", json!({"data": b64(INVERT)})),
        ("plugin.install", json!({"path": file.to_string_lossy()})),
        ("plugin.reload", json!({"path": folder})),
    ] {
        let e = s.execute(id, params.clone()).unwrap_err().to_string();
        assert!(e.contains(plugin_cmds::BUNDLED_ONLY), "{id} {params}: {e}");
    }
    // The Plug-ins preference folder isn't loaded either.
    s.execute("prefs.set", json!({"path": "plugIns.additionalPluginsFolder", "value": folder})).unwrap();
    s.execute("prefs.set", json!({"path": "plugIns.useAdditionalPluginsFolder", "value": true})).unwrap();
    assert!(!installed(&mut s), "nothing was added");

    // Without the policy the same calls add it.
    plugin_cmds::set_bundled_only(false);
    s.execute("plugin.install", json!({"data": b64(INVERT)})).unwrap();
    assert!(installed(&mut s));
    std::fs::remove_dir_all(&dir).unwrap();
}
