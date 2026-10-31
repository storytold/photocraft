//! TIFF Options: the prompt File › Save As shows before writing a layered TIFF, as Photoshop
//! does when "Ask Before Saving Layered TIFF Files" is on in Preferences › File Handling.
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered: with the layers (Photoshop layer data in the file), or flattened ("Discard
//! Layers and Save a Copy"). "Don't show again" clears the preference. Cancel drops the save.

use crate::{ExportSettings, PhotocraftApp};

/// A save waiting on the TIFF Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the document goes.
    pub path: String,
    /// "Discard Layers and Save a Copy".
    pub discard_layers: bool,
    /// Clears `ask_before_saving_layered_tiff` when the save goes ahead.
    pub dont_ask_again: bool,
}

/// `true` when `path` is a TIFF name (`.tif` / `.tiff`).
pub fn is_tiff_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && (ext.eq_ignore_ascii_case("tif") || ext.eq_ignore_ascii_case("tiff")))
}

/// Whether saving the active document to `path` should go through the prompt: a TIFF that
/// would carry layer data, with the preference on.
pub fn wants_prompt(app: &PhotocraftApp, path: &str) -> bool {
    if !is_tiff_path(path) || !app.session.prefs().file_handling.ask_before_saving_layered_tiff {
        return false;
    }
    app.session.active().is_some_and(|st| photocraft_engine::file_cmds::tiff_would_write_layers(&st.doc))
}

/// Parks the save behind the prompt.
pub fn park(app: &mut PhotocraftApp, path: String) -> Result<(), String> {
    let doc = app.active_doc_id()?;
    app.tiff_options = Some(Prompt { doc, path, discard_layers: false, dont_ask_again: false });
    Ok(())
}

/// The TIFF Options body: the title, the note, the two checkboxes and the button row.
/// It mirrors in a right-to-left UI: the title and the note read from the right, and the
/// button row hugs the reading end.
fn body(ui: &mut egui::Ui, discard: &mut bool, dont_ask: &mut bool, ok: &mut bool, cancel: &mut bool) {
    ui.set_max_width(420.0);
    crate::text::direction::window_body(ui, |ui| {
        ui.label(egui::RichText::new(tl!("TIFF Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        ui.label(tl!("Including layers will increase file size."));
        ui.add_space(8.0);
        crate::widgets::checkbox(ui, discard, tl!("Discard Layers and Save a Copy"));
        crate::widgets::checkbox(ui, dont_ask, tl!("Don't show again"));
        ui.add_space(12.0);
        ui.with_layout(crate::text::direction::trailing(), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            *ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            *cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.tiff_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let (mut discard, mut dont_ask) = (p.discard_layers, p.dont_ask_again);
    let modal = egui::Modal::new(egui::Id::unique("tiff-options")).show(ctx, |ui| {
        body(ui, &mut discard, &mut dont_ask, &mut ok, &mut cancel);
    });
    cancel |= modal.should_close();
    p.discard_layers = discard;
    p.dont_ask_again = dont_ask;
    // A focused button takes Enter as its click without consuming the key: Enter on Cancel cancels.
    if ok || (!cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))) {
        let Some(p) = app.tiff_options.take() else { return };
        if p.dont_ask_again {
            app.session.edit_prefs(|prefs| prefs.file_handling.ask_before_saving_layered_tiff = false);
        }
        let settings = ExportSettings { tiff_layers: !p.discard_layers, ..app.save_settings() };
        let step = serde_json::json!({"path": p.path, "tiffLayers": !p.discard_layers});
        let written = app.with_document(p.doc, |app| app.write_document(p.path, &settings, p.discard_layers));
        if written.is_ok() {
            // An action being recorded keeps the Save As with this answer (#2032).
            app.session.journal.push(("file.saveAs".into(), step));
        }
        match written {
            Err(e) => {
                app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                app.ui.status_error = true;
            }
            Ok(Some(_)) if !p.discard_layers => crate::discard_ui::saved_document(app, ctx, p.doc),
            // A background save releases the close prompt when it finishes (`jobs_ui`).
            Ok(_) => {}
        }
    } else if cancel {
        app.tiff_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The harness state: the dialog's column and the four answers.
    struct State {
        column: egui::Rect,
        discard: bool,
        dont_ask: bool,
        ok: bool,
        cancel: bool,
    }

    impl Default for State {
        fn default() -> Self {
            Self { column: egui::Rect::ZERO, discard: false, dont_ask: false, ok: false, cancel: false }
        }
    }

    /// A harness drawing the TIFF Options body in a 500×300 window.
    fn body_harness() -> egui_kittest::Harness<'static, State> {
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(500.0, 300.0))
            .with_context_setup(|ctx| crate::PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
            .build_ui_state(
                |ui, s: &mut State| {
                    body(ui, &mut s.discard, &mut s.dont_ask, &mut s.ok, &mut s.cancel);
                    s.column = ui.max_rect();
                },
                State::default(),
            )
    }

    #[test]
    fn tiff_names() {
        assert!(is_tiff_path("a.tif"));
        assert!(is_tiff_path("dir/sub/a.TIFF"));
        assert!(is_tiff_path("C:\\x\\y.tiff"));
        assert!(!is_tiff_path("a.tif.psd"));
        assert!(!is_tiff_path(".tif"));
        assert!(!is_tiff_path("tif"));
        assert!(!is_tiff_path("a.png"));
    }

    /// The real TIFF Options modal in a harness, with the area it draws in.
    fn options_modal() -> (egui_kittest::Harness<'static, PhotocraftApp>, egui::Rect) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
        park(&mut app, "t.tif".into()).unwrap();
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .with_context_setup(|ctx| crate::PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
            .build_ui_state(|ui, app| show(app, ui.ctx()), app);
        h.run_steps(2);
        let win = h.ctx.memory(|m| m.area_rect(egui::Id::unique("tiff-options"))).expect("the modal");
        (h, win)
    }

    /// The modal mirrors in a right-to-left UI: the button group hugs the reading end, and the
    /// dialog keeps its width — the mirrored body must not grow the frame.
    #[test]
    fn the_modal_mirrors_and_keeps_its_width() {
        use egui_kittest::kittest::Queryable;
        let (ltr, ltr_win) = {
            let _ltr = crate::text::direction::direction_scope(crate::text::direction::Dir::Ltr);
            options_modal()
        };
        let ltr_ok = ltr.get_by_label("OK").rect();
        let (rtl, rtl_win) = {
            let _rtl = crate::text::direction::direction_scope(crate::text::direction::Dir::Rtl);
            options_modal()
        };
        let rtl_ok = rtl.get_by_label("OK").rect();
        assert!(ltr_ok.right() > ltr_win.center().x, "LTR: the button group hugs the right: {ltr_ok:?} in {ltr_win:?}");
        assert!(rtl_ok.left() < rtl_win.center().x, "RTL: the button group hugs the left: {rtl_ok:?} in {rtl_win:?}");
        assert!((rtl_win.width() - ltr_win.width()).abs() < 1.0, "the modal keeps its width: {ltr_win:?} vs {rtl_win:?}");
    }

    /// The body's geometry in one direction: the column, the title and the two buttons.
    struct BodyGeometry {
        column: egui::Rect,
        title: egui::Rect,
        ok: egui::Rect,
        cancel: egui::Rect,
    }

    /// The TIFF Options body drawn in `dir`, with the geometry it produced.
    fn body_geometry(dir: crate::text::direction::Dir) -> BodyGeometry {
        use egui_kittest::kittest::Queryable;
        let _dir = crate::text::direction::direction_scope(dir);
        let mut h = body_harness();
        h.run();
        BodyGeometry {
            column: h.state().column,
            title: h.get_by_label("TIFF Options").rect(),
            ok: h.get_by_label("OK").rect(),
            cancel: h.get_by_label("Cancel").rect(),
        }
    }

    /// The left-to-right body is unchanged: the title reads from the left, and the button row
    /// hugs the right edge with the default button at the row's end.
    #[test]
    fn ltr_the_body_reads_from_the_left() {
        let g = body_geometry(crate::text::direction::Dir::Ltr);
        assert!((g.title.left() - g.column.left()).abs() < 2.0, "the title reads from the left: {:?} in {:?}", g.title, g.column);
        assert!((g.ok.right() - g.column.right()).abs() < 2.0, "the button row hugs the right edge: {:?} in {:?}", g.ok, g.column);
        assert!(g.cancel.right() < g.ok.left(), "the default button ends the row: {:?} {:?}", g.ok, g.cancel);
    }

    /// The right-to-left body mirrors: the title reads from the right, and the button row hugs
    /// the reading end with the default button leading the row.
    #[test]
    fn rtl_the_body_reads_from_the_right() {
        let g = body_geometry(crate::text::direction::Dir::Rtl);
        assert!((g.title.right() - g.column.right()).abs() < 2.0, "the title reads from the right: {:?} in {:?}", g.title, g.column);
        assert!((g.ok.left() - g.column.left()).abs() < 2.0, "the button row hugs the reading end: {:?} in {:?}", g.ok, g.column);
        assert!(g.ok.right() < g.cancel.left(), "the default button leads the row: {:?} {:?}", g.ok, g.cancel);
    }

    /// The mirrored body keeps its content span: the reading-end alignment must not stretch the
    /// title or the button row.
    #[test]
    fn the_mirrored_body_keeps_its_content_span() {
        let ltr = body_geometry(crate::text::direction::Dir::Ltr);
        let rtl = body_geometry(crate::text::direction::Dir::Rtl);
        assert!((ltr.title.width() - rtl.title.width()).abs() < 1.0, "the title keeps its width: {:?} vs {:?}", ltr.title, rtl.title);
        let ltr_span = ltr.ok.union(ltr.cancel).width();
        let rtl_span = rtl.ok.union(rtl.cancel).width();
        assert!((ltr_span - rtl_span).abs() < 1.0, "the button row keeps its span: {ltr_span} vs {rtl_span}");
    }
}
