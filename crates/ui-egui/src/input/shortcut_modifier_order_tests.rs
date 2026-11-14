use super::pretty;

#[test]
fn modifier_order_matches_platform_convention() {
    let expected = if cfg!(target_os = "macos") { "⌘⌥⇧K" } else { "Alt+Shift+Ctrl+K" };

    assert_eq!(pretty("Cmd+Alt+Shift+K"), expected);
}
