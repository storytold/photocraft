//! Ctrl+Alt+Q is AltGr+Q, which types `@` on a German layout: it must not quit (#10917).

use egui::{Context, Event, Key, Modifiers, RawInput, ViewportCommand, ViewportId};

use craft::keymap::{Chord, Platform, egui::matches as key_matches};

use crate::PhotocraftApp;

/// One frame with Q pressed under `modifiers`; whether egui asked the window to close.
fn closes(ctx: &Context, modifiers: Modifiers) -> bool {
    let events = vec![Event::Key { key: Key::Q, physical_key: None, pressed: true, repeat: false, modifiers }];
    let mut out = ctx.run_ui(RawInput { events, ..Default::default() }, |_| {});
    out.textures_delta.clear();
    out.viewport_output.get(&ViewportId::ROOT).is_some_and(|v| v.commands.iter().any(|c| matches!(c, ViewportCommand::Close)))
}

#[test]
fn ctrl_alt_q_does_not_quit() {
    let ctx = Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::AppTheme::pro());
    let ctrl_alt = Modifiers { alt: true, ctrl: true, command: true, ..Default::default() };
    assert!(!closes(&ctx, ctrl_alt), "egui's quit no longer takes Ctrl+Alt+Q");
    assert!(!closes(&ctx, Modifiers::CTRL | Modifiers::COMMAND), "Ctrl+Q is left to File › Exit");
    assert!(closes(&ctx, Modifiers::MAC_CMD | Modifiers::COMMAND), "⌘Q still quits on the Mac");

    let exit = Chord::parse("Cmd+Q").unwrap();
    let platform = Platform::current();
    assert!(key_matches(&exit, Key::Q, &(Modifiers::CTRL | Modifiers::COMMAND), platform), "Ctrl+Q runs File › Exit");
    assert!(!key_matches(&exit, Key::Q, &ctrl_alt, platform), "Ctrl+Alt+Q doesn't");
}
