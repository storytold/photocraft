//! Maintenance guard only: this does not validate native Windows DPI behavior.

#[test]
fn winit_carries_the_windows_11_suggested_rectangle_patch() {
    let manifest = include_str!("../../../Cargo.toml");
    let lock = include_str!("../../../Cargo.lock");
    let handler = include_str!("../../../vendor/winit/src/platform_impl/windows/event_loop.rs").replace("\r\n", "\n");
    let build_check = include_str!("../../../vendor/winit/src/platform_impl/windows/dark_mode.rs");

    let patches = manifest.split("[patch.crates-io]").nth(1).expect("Cargo patch table");
    assert!(patches.contains("winit = { path = \"vendor/winit\" }"));
    let winit = lock.split("[[package]]").find(|package| package.contains("name = \"winit\"")).expect("locked winit");
    assert!(winit.contains("version = \"0.30.13\""));
    assert!(!winit.contains("source ="), "winit must resolve to the local patch");
    assert!(!winit.contains("checksum ="));
    assert!(handler.contains(concat!(
        "if !WIN10_BUILD_VERSION.is_some_and(|build| build < 22000) {\n",
        "                new_outer_rect = suggested_rect;\n",
        "            } else {"
    )));
    assert!(build_check.contains("pub(crate) static WIN10_BUILD_VERSION:"));
}
