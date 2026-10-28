//! `photocraft -`: the image piped on standard input opens as a new document (#9468), as in
//! `cat image.png | photocraft -`, `photocraft - < image.png` or `wl-paste | photocraft -`. Only an
//! explicit `-` reads standard input: a launcher or desktop shortcut can hand the app a pipe that
//! never closes, and reading it unasked would hang the launch. The launch keeps its own process
//! (`single_instance::LaunchContext::stdin`): the bytes can't be handed to the running one as a path.

use photocraft_ui_egui::{PhotocraftApp, i18n};

/// Read the piped image to its end, before the window opens, at most
/// [`PIPE_LIMIT`](photocraft_format::read::PIPE_LIMIT) bytes. A terminal is refused rather than
/// waited on: nothing was piped in. The error is shown once the app is up ([`open`]).
pub(crate) fn read() -> Result<Vec<u8>, String> {
    use std::io::IsTerminal as _;
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Err("it is a terminal; pipe an image in, as in `photocraft - < image.png`".into());
    }
    photocraft_format::read::read_pipe(&mut stdin.lock(), photocraft_format::read::PIPE_LIMIT).map_err(|e| e.to_string())
}

/// Open what [`read`] returned as a new document. Its format comes from the bytes; it is named as
/// File › New names one (the first free "Untitled-N") and has no path, so File › Save asks where
/// to save it. A failure is shown as an error: a read as "Couldn't open standard input: …", a
/// decode under the document's name, as a background open reports it.
pub(crate) fn open(app: &mut PhotocraftApp, piped: Result<&[u8], &str>) {
    match piped {
        Err(e) => app.open_failed(i18n::t("standard input"), e),
        Ok(bytes) => {
            let name = photocraft_engine::file_cmds::untitled_name(i18n::t("Untitled"), |n| app.session.documents().iter().any(|d| d.doc.name == n));
            if let Err(e) = app.open_bytes(&name, bytes) {
                app.open_failed(&name, &e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;
    use photocraft_ui_egui::Services;

    fn app() -> PhotocraftApp {
        let services = Services {
            import: Some(Box::new(|name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        PhotocraftApp::new(Session::new(), services)
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, vec![90; (w * h * 4) as usize]).unwrap();
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()
    }

    /// #9468: piped bytes open by their content as an untitled document with no path, so Save
    /// asks where to save; a second one takes the next name.
    #[test]
    fn a_piped_image_opens_untitled_without_a_path() {
        let mut app = app();
        open(&mut app, Ok(&png(7, 4)));
        open(&mut app, Ok(&png(3, 2)));
        let docs = app.session.documents();
        assert_eq!(docs.len(), 2, "{}", app.ui.status);
        assert_eq!((docs[0].doc.name.as_str(), docs[0].doc.size.width, docs[0].doc.size.height), ("Untitled-1", 7, 4));
        assert_eq!(docs[1].doc.name, "Untitled-2");
        assert!(docs.iter().all(|d| d.path.is_none()), "nothing to save back to");
        assert!(!app.ui.status_error, "{}", app.ui.status);
    }

    /// #9468: an empty pipe (or any failed read) and bytes that aren't an image are errors in the
    /// status bar and a notice, with no document and no panic.
    #[test]
    fn empty_or_garbage_input_is_an_error_not_a_document() {
        let mut app = app();
        open(&mut app, Err("nothing was piped in"));
        assert!(app.ui.status_error && app.ui.status.contains("standard input: nothing was piped in"), "{}", app.ui.status);
        for garbage in [&b"plain text, not an image"[..], &[0u8; 64], &png(7, 4)[..24]] {
            app.ui.status.clear();
            app.ui.status_error = false;
            open(&mut app, Ok(garbage));
            assert!(app.ui.status_error && app.ui.status.contains("Untitled-1"), "{}", app.ui.status);
        }
        assert!(app.session.documents().is_empty());
    }
}
