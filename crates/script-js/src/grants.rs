//! Where a run may write: only where the user said it could.
//!
//! A script may save the document (`saveAs`, `exportDocument`) to a file the user chose in a
//! save dialog during this run, or into a folder the user chose in a folder dialog during this
//! run. Nothing else: not a path the script made up, not one from an earlier run. The app keeps
//! the grants and checks every write before running it, because on the desktop the script runs
//! in a separate, untrusted process.
//!
//! On the web a picker gives the script a name, not a path, so nothing is ever granted there.

use serde_json::Value;

use crate::ModalKind;

/// The commands that write a file, with their path parameter.
const WRITES: &[&str] = &["file.saveACopy", "file.export.saveForWebLegacy"];
/// Most grants one run keeps.
const MAX_GRANTS: usize = 1024;

/// The paths the user chose for this run.
#[derive(Debug, Default, Clone)]
pub struct PathGrants {
    files: Vec<Vec<String>>,
    folders: Vec<Vec<String>>,
}

/// The parts of an absolute path, or `None` for a relative one or one with `.` or `..` parts.
/// On Windows `/` and `\` both separate and a drive (`C:`) or share (`\\server`) starts it;
/// elsewhere only `/` separates, as a `\` is part of a file name there.
fn parts(path: &str) -> Option<Vec<String>> {
    if path.contains('\0') {
        return None;
    }
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let absolute = if cfg!(windows) {
        let drive = path.as_bytes().get(1) == Some(&b':') && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
        drive || path.starts_with("\\\\") || path.starts_with("//")
    } else {
        path.starts_with('/')
    };
    if !absolute {
        return None;
    }
    let out: Vec<String> = path.split(separators).filter(|p| !p.is_empty()).map(str::to_owned).collect();
    if out.is_empty() || out.iter().any(|p| p == "." || p == "..") {
        return None;
    }
    Some(out)
}

/// Whether `name` is `chosen` with an extension added: a save picks its format's extension when
/// the user typed a name without one. A chosen name with an extension allows only itself, so a
/// script can't write `photo.psd` next to the `photo.jpg` the user picked.
fn extended(chosen: &str, name: &str) -> bool {
    if chosen.rfind('.').is_some_and(|i| i > 0) {
        return false;
    }
    let Some(ext) = name.get(chosen.len()..).and_then(|rest| rest.strip_prefix('.')) else { return false };
    same(name.get(..chosen.len()).unwrap_or(""), chosen) && (1..=5).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn same(a: &str, b: &str) -> bool {
    // Windows and macOS file systems usually ignore case.
    if cfg!(any(windows, target_os = "macos")) { a.eq_ignore_ascii_case(b) } else { a == b }
}

fn same_parts(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
}

impl PathGrants {
    /// Keep what the user chose in a dialog of `kind` whose answer is `answer`: a save dialog
    /// grants that file, a folder dialog that folder. Other dialogs grant nothing.
    pub fn record(&mut self, kind: ModalKind, answer: &Value) {
        let Some(path) = answer.as_str().and_then(parts) else { return };
        if self.files.len() + self.folders.len() >= MAX_GRANTS {
            return;
        }
        // A replayed dialog answers again: keep each grant once.
        let list = match kind {
            ModalKind::SaveFile => &mut self.files,
            ModalKind::SelectFolder => &mut self.folders,
            _ => return,
        };
        if !list.contains(&path) {
            list.push(path);
        }
    }

    /// Whether `path` may be written: a chosen file (or, when the chosen name had no extension,
    /// that name with one), or a file right inside a chosen folder.
    #[must_use]
    pub fn allows(&self, path: &str) -> bool {
        let Some(p) = parts(path) else { return false };
        let Some((name, dir)) = p.split_last() else { return false };
        let file = self.files.iter().any(|f| f.split_last().is_some_and(|(n, d)| same_parts(d, dir) && (same(n, name) || extended(n, name))));
        let folder = self.folders.iter().any(|f| same_parts(f, dir));
        file || folder
    }

    /// Check a command a run asks for: one that writes a file must name a granted path.
    ///
    /// # Errors
    /// The message for the script when the path was not chosen by the user.
    pub fn check(&self, id: &str, params: &Value) -> Result<(), String> {
        if !WRITES.contains(&id) {
            return Ok(());
        }
        let path = params.get("path").and_then(Value::as_str).unwrap_or("");
        if self.allows(path) {
            Ok(())
        } else {
            Err("a script can only save to a file or folder you chose in this run (File.saveDialog() or Folder.selectDialog())".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn only_chosen_paths_are_writable() {
        let mut g = PathGrants::default();
        let save = json!({"path": "/home/a/out.png"});
        assert!(g.check("file.saveACopy", &save).is_err(), "nothing is granted before a dialog");
        assert!(g.check("session.inspect", &json!({})).is_ok(), "other commands are not paths");
        g.record(ModalKind::SaveFile, &json!("/home/a/out.png"));
        assert!(g.check("file.saveACopy", &save).is_ok());
        for no in [
            "/home/a/out.jpg",
            "/home/a/out.png.psd",
            "/home/a/other.png",
            "/home/b/out.png",
            "/home/a/sub/out.png",
            "out.png",
            "/home/a/../a/out.png",
            "/home/a/./out.png",
            "",
        ] {
            assert!(!g.allows(no), "{no}");
        }
        g.record(ModalKind::SaveFile, &json!("/home/a/plain"));
        assert!(g.allows("/home/a/plain.jpg"), "a name typed without an extension takes the format's");
        assert!(!g.allows("/home/a/plain.x/y"));
        assert!(!g.allows("/home/a/plainer.jpg"));
    }

    #[cfg(not(windows))]
    #[test]
    fn a_backslash_is_part_of_a_name_outside_windows() {
        let mut g = PathGrants::default();
        g.record(ModalKind::SelectFolder, &json!("/home/a/Exports"));
        assert!(g.allows("/home/a/Exports/one.png"));
        assert!(!g.allows("/home/a/Exports\\one.png"), "that is a file in /home/a");
        g.record(ModalKind::SaveFile, &json!("C:\\Users\\a\\out.png"));
        assert!(!g.allows("C:\\Users\\a\\out.png"), "not an absolute path here");
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_take_either_separator() {
        let mut g = PathGrants::default();
        g.record(ModalKind::SelectFolder, &json!("C:\\Users\\a\\Exports\\"));
        assert!(g.allows("C:\\Users\\a\\Exports\\one.jpg"));
        assert!(g.allows("C:/Users/a/Exports/two.png"));
        assert!(!g.allows("C:\\Users\\a\\Exports\\deep\\one.jpg"), "only right inside the folder");
        assert!(!g.allows("C:\\Users\\a\\one.jpg"));
    }

    #[test]
    fn other_dialogs_and_relative_answers_grant_nothing() {
        let mut g = PathGrants::default();
        g.record(ModalKind::OpenFile, &json!("/home/a/in.png"));
        g.record(ModalKind::Prompt, &json!("/home/a/x.png"));
        g.record(ModalKind::SaveFile, &json!("x.png"));
        g.record(ModalKind::SaveFile, &json!(null));
        g.record(ModalKind::SelectFolder, &json!("/"));
        assert!(!g.allows("/home/a/in.png"));
        assert!(!g.allows("/home/a/x.png"));
        assert!(!g.allows("/x.png"), "the root is not a folder a script may be given");
    }
}
