//! Where a run may write: only where the user said it could.
//!
//! A script may save the document (`saveAs`, `exportDocument`) to a file the user chose in a
//! save dialog during this run, or into a folder the user chose in a folder dialog during this
//! run. Nothing else: not a path the script made up, not one from an earlier run. The app keeps
//! the grants and checks every write before running it, because on the desktop the script runs
//! in a separate, untrusted process.
//!
//! On the web a picker gives the script a name, not a path, so nothing is ever granted there.
//!
//! The check reads the path. On Windows a name with a `:` (an alternate data stream,
//! `photo.png:hidden`) is refused, since it is no file the user chose. Then, for a granted path
//! only (a script learns nothing of any other path), the file system is asked one thing: a target
//! that is a symbolic link (or, on Windows, another reparse point such as a junction) is refused,
//! even one the user picked, since writing it would write wherever it points. What remains is a
//! race: something else on the machine that replaces a granted file with a link between this
//! check and the write redirects the write. A script can't do that by itself (it has no other
//! way to touch the file system), so this stays outside what scripts can reach.

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
    /// Files this run has written into a chosen folder: it may replace its own.
    written: Vec<Vec<String>>,
}

/// The parts of an absolute path, or `None` for a relative one or one with `.` or `..` parts.
/// On Windows `/` and `\` both separate and a drive (`C:`) or share (`\\server`) starts it;
/// elsewhere only `/` separates, as a `\` is part of a file name there.
fn parts(path: &str) -> Option<Vec<String>> {
    parts_on(path, cfg!(windows))
}

/// [`parts`] as Windows (`windows`) or any other system reads `path`.
fn parts_on(path: &str, windows: bool) -> Option<Vec<String>> {
    if path.contains('\0') {
        return None;
    }
    let separators: &[char] = if windows { &['/', '\\'] } else { &['/'] };
    // The parts that name the root: a drive (`C:`), a share (`\\\\server\\share`), or none (`/`).
    let root = if windows {
        // `C:\x` is absolute; `C:x` is relative to that drive's current folder.
        let b = path.as_bytes();
        let drive = b.first().is_some_and(u8::is_ascii_alphabetic) && b.get(1) == Some(&b':') && matches!(b.get(2), Some(b'/' | b'\\'));
        if drive {
            1
        } else if path.starts_with("\\\\") || path.starts_with("//") {
            2
        } else {
            return None;
        }
    } else if path.starts_with('/') {
        0
    } else {
        return None;
    };
    let out: Vec<String> = path.split(separators).filter(|p| !p.is_empty()).map(str::to_owned).collect();
    // A root alone is no file or folder a script may be given.
    if out.len() <= root || out.iter().any(|p| p == "." || p == "..") {
        return None;
    }
    // Past the drive, a `:` names a stream of a file (or is no valid name): never a file to write.
    if windows && out.iter().skip(root).any(|p| p.contains(':')) {
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

/// Whether `path` is a symbolic link or another reparse point (a junction, on Windows) now.
/// Whether something is at `path` (a dangling link included).
fn exists(path: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::symlink_metadata(path).is_ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        false
    }
}

fn is_link(path: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::symlink_metadata(path).is_ok_and(|m| {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                // FILE_ATTRIBUTE_REPARSE_POINT
                m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0
            }
            #[cfg(not(windows))]
            m.file_type().is_symlink()
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        false
    }
}

/// Names compared exactly. Most Windows and macOS volumes ignore case, but not all (a
/// case-sensitive APFS volume, a case-sensitive NTFS folder), and there `Exports` and `exports`
/// are different folders: a path written otherwise than the user's choice is refused, which
/// costs a script nothing, since it has the path the user chose.
fn same(a: &str, b: &str) -> bool {
    a == b
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
        // The same choice made twice is kept once.
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
        self.chosen_file(path) || self.in_chosen_folder(path)
    }

    /// Whether `path` is a file the user chose in a save dialog (or that name with an extension).
    fn chosen_file(&self, path: &str) -> bool {
        let Some(p) = parts(path) else { return false };
        let Some((name, dir)) = p.split_last() else { return false };
        self.files.iter().any(|f| f.split_last().is_some_and(|(n, d)| same_parts(d, dir) && (same(n, name) || extended(n, name))))
    }

    /// Whether `path` is exactly a file the user chose in a save dialog: the one file the
    /// dialog asked about replacing, if it was there.
    fn chosen_exactly(&self, path: &str) -> bool {
        parts(path).is_some_and(|p| self.files.iter().any(|f| same_parts(f, &p)))
    }

    /// Whether `path` is right inside a folder the user chose.
    fn in_chosen_folder(&self, path: &str) -> bool {
        let Some(p) = parts(path) else { return false };
        let Some((_, dir)) = p.split_last() else { return false };
        self.folders.iter().any(|f| same_parts(f, dir))
    }

    /// Note a write command that succeeded: a file it wrote into a chosen folder is the run's
    /// own, which it may write again.
    pub fn wrote(&mut self, id: &str, params: &Value) {
        if !WRITES.contains(&id) || self.written.len() >= MAX_GRANTS {
            return;
        }
        let path = params.get("path").and_then(Value::as_str).unwrap_or("");
        if let Some(p) = parts(path)
            && !self.written.iter().any(|w| same_parts(w, &p))
        {
            self.written.push(p);
        }
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
        if !self.allows(path) {
            return Err("a script can only save to a file or folder you chose in this run (File.saveDialog() or Folder.selectDialog())".into());
        }
        // Only a granted path is looked at on disk: a script learns nothing of any other.
        if is_link(path) {
            return Err("a script can't save over a link: it would write wherever the link points".into());
        }
        // A chosen folder, or a chosen name the script adds an extension to, takes new files.
        // One that was there before (and not written by this run) is replaced only when the user
        // chose exactly it in a save dialog, which asked about replacing it.
        let own = parts(path).is_some_and(|p| self.written.iter().any(|w| same_parts(w, &p)));
        if !self.chosen_exactly(path) && !own && exists(path) {
            return Err(format!(
                "a script can't replace {}, which was already there: choose that file in a save dialog (File.saveDialog()) to replace it",
                path.rsplit(['/', '\\']).next().unwrap_or(path)
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// `path` (written with `/`) as an absolute path on this platform.
    fn abs(path: &str) -> String {
        if cfg!(windows) { format!("C:{path}") } else { path.to_owned() }
    }

    #[test]
    fn only_chosen_paths_are_writable() {
        let mut g = PathGrants::default();
        let save = json!({"path": abs("/home/a/out.png")});
        assert!(g.check("file.saveACopy", &save).is_err(), "nothing is granted before a dialog");
        assert!(g.check("session.inspect", &json!({})).is_ok(), "other commands are not paths");
        g.record(ModalKind::SaveFile, &json!(abs("/home/a/out.png")));
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
            let no = if no.starts_with('/') { abs(no) } else { no.to_owned() };
            assert!(!g.allows(&no), "{no}");
        }
        g.record(ModalKind::SaveFile, &json!(abs("/home/a/plain")));
        assert!(g.allows(&abs("/home/a/plain.jpg")), "a name typed without an extension takes the format's");
        assert!(!g.allows(&abs("/home/a/plain.x/y")));
        assert!(!g.allows(&abs("/home/a/plainer.jpg")));
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
    fn windows_paths_start_at_a_drive_root_or_a_share() {
        let win = |p: &str| parts_on(p, true);
        assert_eq!(win("C:\\Users\\a/out.png"), Some(vec!["C:".into(), "Users".into(), "a".into(), "out.png".into()]));
        assert!(win("\\\\server\\share\\x.png").is_some());
        // Relative, or a root alone.
        for relative in ["C:out.png", "C:", "out.png", "\\Users\\a.png", "1:\\x.png", "C:\\", "\\\\server\\share\\"] {
            assert_eq!(win(relative), None, "{relative}");
        }
        assert_eq!(parts_on("C:\\Users\\a.png", false), None, "only Windows has drives");
        assert_eq!(win("C:\\Exports\\photo.png:hidden"), None, "an alternate data stream is no file");
        assert_eq!(win("C:\\Exports\\photo.png::$DATA"), None);
    }

    /// A chosen folder takes new files and the run's own; a file that was there before is
    /// replaced only when the user chose it in a save dialog.
    #[test]
    fn a_chosen_folder_does_not_replace_a_file_it_already_held() {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!("photocraft-grants-existing-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old.png");
        std::fs::write(&old, b"before").unwrap();
        let path = |p: &std::path::Path| json!({ "path": p.to_str().unwrap() });
        let mut g = PathGrants::default();
        g.record(ModalKind::SelectFolder, &json!(dir.to_str().unwrap()));
        let refused = g.check("file.saveACopy", &path(&old)).unwrap_err();
        assert!(refused.contains("old.png") && refused.contains("save dialog"), "{refused}");
        // A new file is written, and written again: it is the run's own.
        let new = dir.join("new.png");
        assert_eq!(g.check("file.saveACopy", &path(&new)), Ok(()));
        std::fs::write(&new, b"run").unwrap();
        g.wrote("file.saveACopy", &path(&new));
        assert_eq!(g.check("file.saveACopy", &path(&new)), Ok(()));
        // Chosen in a save dialog, the old file may be replaced.
        g.record(ModalKind::SaveFile, &json!(old.to_str().unwrap()));
        assert_eq!(g.check("file.saveACopy", &path(&old)), Ok(()));
        drop(std::fs::remove_dir_all(&dir));
    }

    /// A name chosen without an extension takes one, but not one that names a file already
    /// there: the save dialog asked only about the name the user chose.
    #[test]
    fn a_chosen_name_with_an_added_extension_does_not_replace_a_file_already_there() {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!("photocraft-grants-extended-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Untitled.jpg"), b"before").unwrap();
        let path = |name: &str| json!({ "path": dir.join(name).to_str().unwrap() });
        let mut g = PathGrants::default();
        g.record(ModalKind::SaveFile, &json!(dir.join("Untitled").to_str().unwrap()));
        assert_eq!(g.check("file.saveACopy", &path("Untitled.png")), Ok(()), "a new file with an extension");
        let refused = g.check("file.saveACopy", &path("Untitled.jpg")).unwrap_err();
        assert!(refused.contains("Untitled.jpg") && refused.contains("save dialog"), "{refused}");
        drop(std::fs::remove_dir_all(&dir));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_a_chosen_folder_is_not_written() {
        let dir = std::env::temp_dir().join(format!("photocraft-grants-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let outside = dir.join("outside.txt");
        std::fs::write(&outside, "keep").unwrap();
        let link = dir.join("export.png");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let mut g = PathGrants::default();
        g.record(ModalKind::SelectFolder, &json!(dir.to_string_lossy()));
        let save = |p: &std::path::Path| g.check("file.saveACopy", &json!({"path": p.to_string_lossy()}));
        assert!(save(&dir.join("fresh.png")).is_ok(), "a new file in the chosen folder");
        assert!(save(&link).unwrap_err().contains("link"), "the link in it");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_path_differing_only_in_case_is_another_path() {
        let mut g = PathGrants::default();
        g.record(ModalKind::SelectFolder, &json!(abs("/Volumes/Work/Exports")));
        g.record(ModalKind::SaveFile, &json!(abs("/Volumes/Work/plain")));
        assert!(g.allows(&abs("/Volumes/Work/Exports/out.png")));
        assert!(!g.allows(&abs("/Volumes/Work/exports/out.png")), "on a case-sensitive volume that is another folder");
        assert!(!g.allows(&abs("/Volumes/Work/PLAIN.png")));
    }

    #[test]
    fn other_dialogs_and_relative_answers_grant_nothing() {
        let mut g = PathGrants::default();
        g.record(ModalKind::OpenFile, &json!(abs("/home/a/in.png")));
        g.record(ModalKind::Prompt, &json!(abs("/home/a/x.png")));
        g.record(ModalKind::SaveFile, &json!("x.png"));
        g.record(ModalKind::SaveFile, &json!(null));
        g.record(ModalKind::SelectFolder, &json!(abs("/")));
        assert!(!g.allows(&abs("/home/a/in.png")));
        assert!(!g.allows(&abs("/home/a/x.png")));
        assert!(!g.allows(&abs("/x.png")), "the root is not a folder a script may be given");
    }
}
