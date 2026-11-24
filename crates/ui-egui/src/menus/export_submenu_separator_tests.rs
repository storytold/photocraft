//! File › Export: Export Preferences… sits in its own group, between two separators (#13801).

use super::*;

/// The File › Export entries as the menu bar shows them, `---` for a separator.
fn export_ids() -> Vec<String> {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    menu_rows(&app).into_iter().filter(|r| r.path == ["File", "Export"]).map(|r| if r.label == "---" { "---".to_string() } else { r.command.id }).collect()
}

#[test]
fn export_preferences_is_alone_between_separators() {
    let ids = export_ids();
    let at = ids.iter().position(|id| id == "file.export.exportPreferences").expect("Export Preferences is in File › Export");
    assert!(at >= 2, "{ids:?}");
    assert_eq!(ids[at - 1], "---", "a separator above Export Preferences: {ids:?}");
    assert_eq!(ids.get(at + 1).map(String::as_str), Some("---"), "the separator below Export Preferences stays: {ids:?}");
    assert_eq!(ids[..at - 1], ["file.export.quickExportAsPng", "file.export.exportAs"], "the first group keeps Quick Export and Export As: {ids:?}");
}
