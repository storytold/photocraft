//! The main window's catch-all drop target for files dragged in from the operating system: files dropped where no
//! other target takes them (the tab strip takes its own) open as documents, or are placed when dropped on the canvas.
//! Without it a drop on the Home screen, which hides the tab strip, reached no target at all (#16588).

use craft::behavior::{Input, Metrics, Sense};
use craft::ui::contract::{Access, Common, Component, Cx, MeasureCx, NoValue, OsFiles, Outcome, Pointer as _, Props as _, Role, Show as _};
use craft::ui::popover::WholeWindow;
use craft::ui::runtime::env::Label;
use craft::ui::style::{ComponentStyle, StyleCx};
use egui::{Rect, Response, Vec2};

use super::PhotocraftApp;
use crate::file_open::DropTo;

/// The style of the drop target: it paints nothing; the framework draws the drop mark around its rect.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct WindowFilesStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What the drop target reports: the files dropped on the window in this pass that no other target took.
#[derive(Debug)]
pub(crate) struct WindowFilesOutput {
    response: Response,
    dropped: Option<OsFiles>,
}

impl Outcome for WindowFilesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A target over the whole window for files from the operating system. It takes no pointer input and paints nothing.
#[derive(Component, craft::ui::contract::Props)]
pub(crate) struct WindowFiles {
    #[common]
    common: Common,
    dropped: Option<OsFiles>,
}

impl WindowFiles {
    fn new(label: impl Into<Label>) -> Self {
        WindowFiles { common: Common::new(), dropped: None }.label(label)
    }
}

impl Component for WindowFiles {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = WindowFilesStyle;
    type Output = WindowFilesOutput;

    fn style(_: &StyleCx<'_>, _: WindowFilesStyleKey) -> WindowFilesStyle {
        WindowFilesStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.dropped = cx.window_drop_target::<OsFiles, _>(&WholeWindow::new(cx.rect));
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> WindowFilesOutput {
        WindowFilesOutput { response: cx.response.clone(), dropped: self.dropped }
    }
}

/// Registers the target over `rect` (the document area, which the drop mark outlines) for this pass and returns the
/// files dropped on the window that no other target took.
pub(super) fn take(ui: &mut egui::Ui, rect: Rect) -> Option<OsFiles> {
    WindowFiles::new(tl!("Drop files to open them")).show_at(ui, rect).dropped
}

/// Opens or places `files`, by where the pointer is now ([`PhotocraftApp::drop_to`]): on the shown canvas they are
/// placed in the document, anywhere else they open. Run after the document area drew, so the canvas rect is this
/// pass's.
pub(super) fn open(app: &mut PhotocraftApp, ctx: &egui::Context, files: OsFiles) {
    let at = app.cursor_pos(ctx);
    let mut to = app.drop_to(ctx, at);
    // The files go to the document the canvas shows there, which the drop activates, as a press does: on a pane of
    // Window › Arrange's n-up or Tile arrangement that pane's document; on the single canvas while the active document
    // floats, the docked document it shows.
    if matches!(to, DropTo::Canvas { .. })
        && let Some(p) = at
        && let Some(shown) = match super::arranged_cells(app, app.last_canvas_rect) {
            Some(cells) => cells.into_iter().find(|(_, cell)| cell.contains(p)).map(|(pane, _)| pane),
            None => super::tabs::main_canvas_document(app),
        }
        && app.session.active_index() != Some(shown)
    {
        app.session.set_active(shown);
        to = app.drop_to(ctx, at);
    }
    app.open_dropped(ctx, files.files, to);
}

#[cfg(test)]
#[path = "window_drop/tests.rs"]
mod tests;
