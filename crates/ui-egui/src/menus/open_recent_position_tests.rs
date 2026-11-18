//! File menu: Open Recent comes directly after Open as Smart Object (#14053).

use super::*;

fn app_with(recent: &[&str]) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.recent_files = recent.iter().map(|s| (*s).to_string()).collect();
    app
}

/// A row's id, `---` for a separator.
fn id_of(row: &craft::menu::Row) -> String {
    if row.label == "---" { "---".to_string() } else { row.command.id.clone() }
}

#[test]
fn open_recent_follows_open_as_smart_object() {
    let app = app_with(&["/tmp/a.png"]);
    let rows = menu_rows(&app);
    let mut flat: Vec<String> = Vec::new();
    let mut seen_recent = false;
    for row in rows.iter().filter(|r| r.path.first().map(String::as_str) == Some("File")) {
        if row.path.len() == 2 && row.path[1] == "Open Recent" {
            if !seen_recent {
                flat.push("Open Recent".to_string());
                seen_recent = true;
            }
        } else if row.path.len() == 1 {
            flat.push(id_of(row));
        }
        if flat.len() >= 8 {
            break;
        }
    }
    assert_eq!(
        flat,
        vec!["file.new", "file.newFromClipboard", "file.open", "file.openAs", "file.openAsSmartObject", "Open Recent", "file.openExrParts", "---"]
    );
}

#[test]
fn open_recent_entries_are_unchanged() {
    let rp = vec!["File".to_string(), "Open Recent".to_string()];
    for (recent, expect) in [
        (vec![], vec![("file.clearRecent", "Clear Recent File List", false)]),
        (
            vec!["/tmp/a.png", "/dir/b.psd"],
            vec![
                ("file.openRecent.0", "a.png", true),
                ("file.openRecent.1", "b.psd", true),
                ("---", "---", false),
                ("file.clearRecent", "Clear Recent File List", true),
            ],
        ),
    ] {
        let app = app_with(&recent);
        let got: Vec<(String, String, bool)> = menu_rows(&app)
            .into_iter()
            .filter(|r| r.path == rp)
            .map(|r| if r.label == "---" { ("---".to_string(), "---".to_string(), false) } else { (r.command.id, r.label, r.enabled) })
            .collect();
        let want: Vec<(String, String, bool)> = expect.into_iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c)).collect();
        assert_eq!(got, want);
    }
}
