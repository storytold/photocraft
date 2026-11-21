//! Temporary diagnostic lines for the Windows real-app run of #15549, #15556 and #15870 (draft
//! PR, never merged). Printed to stderr only when `PHOTOCRAFT_DIAG` is set.

use std::sync::OnceLock;

/// Whether `PHOTOCRAFT_DIAG` is set, read once.
pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PHOTOCRAFT_DIAG").is_some_and(|v| !v.is_empty()))
}

/// The pointer, focus and IME events of `events`, short: buttons and IME events in full, moves as
/// their count and first and last position.
pub(crate) fn events(events: &[egui::Event]) -> Option<String> {
    let mut parts = Vec::new();
    let mut moves = Vec::new();
    for event in events {
        match event {
            egui::Event::PointerMoved(p) => moves.push(*p),
            egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                parts.push(format!("button({:.1},{:.1} {button:?} pressed={pressed} mods={modifiers:?})", pos.x, pos.y));
            }
            egui::Event::PointerGone => parts.push("gone".into()),
            egui::Event::WindowFocused(f) => parts.push(format!("focused({f})")),
            egui::Event::Ime(ime) => {
                let len = match ime {
                    egui::ImeEvent::Commit(text) => format!(" commit_len={}", text.len()),
                    _ => String::new(),
                };
                parts.push(format!("ime({ime:?}{len})"));
            }
            _ => {}
        }
    }
    if let (Some(first), Some(last)) = (moves.first(), moves.last()) {
        parts.push(format!("moves(n={} first={:.1},{:.1} last={:.1},{:.1})", moves.len(), first.x, first.y, last.x, last.y));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Whether `json` differs from the value given last time (the text session's, pass by pass).
pub(crate) fn text_changed(json: &str) -> bool {
    static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if *last == json {
        return false;
    }
    json.clone_into(&mut last);
    true
}
