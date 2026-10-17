//! Remove confirmed missing recent files without doing filesystem work on the UI thread.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::PhotocraftApp;

#[derive(Default)]
pub(crate) struct Runtime {
    pending: Option<(Vec<String>, Receiver<Vec<String>>)>,
    checked: Vec<String>,
    next_check: Option<Instant>,
}

fn missing_paths(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| {
            // Permission and transient I/O errors do not prove a file was deleted.
            std::fs::metadata(path).is_err_and(|err| err.kind() == std::io::ErrorKind::NotFound)
        })
        .cloned()
        .collect()
}

fn apply(app: &mut PhotocraftApp, checked: &[String], missing: &[String]) {
    // A newer list may include a file recreated or reopened while the worker was checking it.
    if app.ui.recent_files != checked {
        return;
    }
    for path in missing {
        app.remove_recent(path);
    }
}

pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let rt = &mut app.prefs_rt.recent_files;
    if let Some((_, rx)) = &rt.pending {
        match rx.try_recv() {
            Ok(missing) => {
                let Some((checked, _)) = rt.pending.take() else { return };
                apply(app, &checked, &missing);
            }
            Err(TryRecvError::Disconnected) => rt.pending = None,
            Err(TryRecvError::Empty) => return,
        }
    }
    let rt = &mut app.prefs_rt.recent_files;
    let now = Instant::now();
    if rt.checked == app.ui.recent_files && rt.next_check.is_some_and(|next| now < next) {
        ctx.request_repaint_after(Duration::from_secs(30));
        return;
    }
    rt.checked = app.ui.recent_files.clone();
    rt.next_check = Some(now + Duration::from_secs(30));
    if rt.checked.is_empty() {
        return;
    }
    ctx.request_repaint_after(Duration::from_secs(30));
    let paths = rt.checked.clone();
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    match std::thread::Builder::new().name("recent-files".into()).spawn(move || {
        let _ = tx.send(missing_paths(&paths));
        ctx.request_repaint();
    }) {
        Ok(_) => rt.pending = Some((rt.checked.clone(), rx)),
        Err(err) => log::warn!("Could not check recent files: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_recent_files_leave_both_lists_and_preferences() {
        let dir = std::env::temp_dir().join(format!(
            "photocraft-recents-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let kept = dir.join("kept.png");
        let deleted = dir.join("deleted.psd");
        std::fs::write(&kept, b"present").unwrap();
        std::fs::write(&deleted, b"present").unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent(kept.to_str().unwrap());
        app.push_recent(deleted.to_str().unwrap());
        let checked = app.ui.recent_files.clone();
        std::fs::remove_file(&deleted).unwrap();
        let ctx = egui::Context::default();
        tick(&mut app, &ctx);
        let (snapshot, rx) = app.prefs_rt.recent_files.pending.take().unwrap();
        assert_eq!(snapshot, checked);
        let missing = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(missing, vec![deleted.to_string_lossy().to_string()]);
        apply(&mut app, &checked, &missing);
        assert_eq!(app.ui.recent_files, vec![kept.to_string_lossy().to_string()]);
        assert_eq!(app.session.prefs().file_handling.recent_files, app.ui.recent_files);
        std::fs::remove_file(kept).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn stale_recent_check_does_not_remove_a_newer_list() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent("missing.psd");
        let checked = app.ui.recent_files.clone();
        app.push_recent("new.png");
        let before = app.ui.recent_files.clone();
        apply(&mut app, &checked, &checked);
        assert_eq!(app.ui.recent_files, before);
    }
}
