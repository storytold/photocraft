//! Window control methods for multisession. Public API: `photocraft_ui_egui::control::methods() -> &'static [&'static str]`.
use photocraft_ui_egui::control::methods;

#[test]
fn ms_window_exposes_multisession_methods() {
    for m in ["engine.execute", "events.poll", "client.leave", "hub.probe"] {
        assert!(methods().contains(&m), "missing {m}");
    }
    // doc.inspect is retired (decision H7): the window must not advertise it.
    assert!(!methods().contains(&"doc.inspect"), "doc.inspect was retired (H7) but is still listed");
}
