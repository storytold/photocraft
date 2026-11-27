//! The main window's document tab strip: one framework `DocumentTabs` over the docked documents and the files still
//! opening, the tab menu, and the title a document's tab and canvas show.

use craft::ui::contract::overlay::Anchor;
use craft::ui::contract::{Bind, BodyCx, Entry, Remember as _, Trigger};
use craft::ui::menu::Menu;
use craft::ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent, DocumentTabsOutput};
use egui::{Pos2, Response};
use photocraft_doc::DocId;
use photocraft_engine::jobs::JobId;

use super::{PhotocraftApp, fmt_zoom, mode_label};
use crate::file_open::DropTo;
use serde_json::json;

/// A tab of the document strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DocTab {
    /// An open document.
    Doc(DocId),
    /// A file still opening in the background (#210): its job.
    Opening(JobId),
}

/// The tab menu while it is open: the document it was asked for and where it hangs. It names the document, not its
/// place on the strip, so a reorder while the menu is open cannot point it at another tab.
#[derive(Clone, Debug, Default)]
struct TabMenu(Option<(DocId, Pos2)>);

/// The index of document `id` in the session's document list.
pub(crate) fn document_index(app: &PhotocraftApp, id: DocId) -> Option<usize> {
    app.session.documents().iter().position(|st| st.doc.id == id)
}

/// The body of the canvas panel's tabs region: the strip, what the user did on it, and the tab menu a right click
/// asked for.
pub(super) fn strip(app: &mut PhotocraftApp, ui: &mut egui::Ui, cx: &mut BodyCx<'_>) {
    let ctx = ui.ctx().clone();
    let out = tabs(app, ui);
    let response = out.response.clone();
    // A floating document window's title bar dropped on the strip docks it (`document_windows`).
    if response.rect.is_positive() {
        app.dock_drop_rect = Some(response.rect);
    }
    if let Some(menu) = apply(app, &ctx, out) {
        cx.keep::<TabMenu>().0 = Some(menu);
    }
    tab_menu(app, ui, cx, &response);
}

/// The tab strip: a tab per docked document, in document order, then one per file still opening. The tab of the
/// document the main canvas shows is selected, or the opening file whose progress is shown.
pub(super) fn tabs(app: &PhotocraftApp, ui: &mut egui::Ui) -> DocumentTabsOutput<DocTab> {
    let docs = app.session.documents();
    let mut tabs: Vec<DocumentTab<DocTab>> = docked_document_indices(app)
        .into_iter()
        .filter_map(|i| {
            let st = docs.get(i)?;
            let zoom = app.ui.views.get(i).map_or(100.0, |v| v.zoom * 100.0);
            Some(DocumentTab::new(DocTab::Doc(st.doc.id), tab_title_at(app, i, zoom)).modified(st.is_dirty()))
        })
        .collect();
    tabs.extend(
        crate::jobs_ui::open_tabs(app)
            .into_iter()
            .map(|(job, name, fraction)| DocumentTab::new(DocTab::Opening(job), name).progress(fraction).close_label(tl!("Cancel opening"))),
    );
    let selected = app.jobs.focus.map(DocTab::Opening).or_else(|| main_canvas_document(app).and_then(|i| docs.get(i)).map(|st| DocTab::Doc(st.doc.id)));
    let mut strip =
        DocumentTabs::new(tl!("Documents"), &tabs).reorderable(true).tear_off(true).large(app.session.prefs().workspace.large_tabs).takes_files(true);
    if let Some(tab) = selected {
        strip = strip.selected(tab);
    }
    strip.show(ui)
}

/// Applies what the user did on the strip this pass. The strip reports only the input its own pass received, so a
/// second pass of a frame applies nothing twice. Returns the tab menu a right click asked for.
pub(super) fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, out: DocumentTabsOutput<DocTab>) -> Option<(DocId, Pos2)> {
    let mut menu = None;
    // A Move-tool drag over a tab turns into a layer transfer to that document (`layer_transfer`).
    if let Some(DocTab::Doc(id)) = out.hovered
        && crate::layer_transfer::pointer_if_armed(app, ctx).is_some()
        && let Some(i) = document_index(app, id)
    {
        crate::layer_transfer::over_tab(app, ctx, i);
    }
    for event in out.events {
        match event {
            DocumentTabsEvent::Selected(tab) | DocumentTabsEvent::Picked(tab) | DocumentTabsEvent::Activated(tab) => show_tab(app, tab),
            DocumentTabsEvent::CloseRequested(DocTab::Doc(id)) => close_document(app, ctx, id),
            DocumentTabsEvent::CloseRequested(DocTab::Opening(job)) => {
                let _ = crate::jobs_ui::cancel(app, job);
            }
            DocumentTabsEvent::Reordered { documents, before, .. } => reorder(app, ctx, &documents, before),
            DocumentTabsEvent::TornOff { documents, at, .. } => {
                for tab in documents {
                    if let DocTab::Doc(id) = tab {
                        run(app, ctx, "window.arrange.floatInWindow", json!({"document": id.0, "position": [at.x, at.y]}));
                    }
                }
            }
            DocumentTabsEvent::FilesDropped { files, before, .. } => {
                // The files open before the tab they were dropped in front of, or after every document.
                let slot = match before {
                    Some(DocTab::Doc(id)) => document_index(app, id),
                    _ => None,
                }
                .unwrap_or(app.session.documents().len());
                app.open_dropped(ctx, files, DropTo::Slot(slot));
            }
            DocumentTabsEvent::ContextMenu(request) => {
                if let DocTab::Doc(id) = request.key {
                    menu = Some((id, request.at));
                }
            }
            _ => {}
        }
    }
    menu
}

/// The guarded close shared by the tab strip and a tiled document's title.
pub(super) fn close_document(app: &mut PhotocraftApp, ctx: &egui::Context, id: DocId) {
    if let Some(index) = document_index(app, id) {
        let _ = crate::menus::invoke(app, ctx, "file.close", json!({"document": index}));
    }
}

/// A chosen tab: a document tab shows that document, an opening tab its progress.
fn show_tab(app: &mut PhotocraftApp, tab: DocTab) {
    match tab {
        DocTab::Doc(id) => {
            if let Some(i) = document_index(app, id) {
                app.session.set_active(i);
                app.jobs.focus = None;
            }
        }
        DocTab::Opening(job) => app.jobs.focus = Some(job),
    }
}

/// A dragged document tab now sits before `before` (last when `None` or an opening file's tab): `document.move` to
/// the matching place in the document list, floating documents keeping theirs.
fn reorder(app: &mut PhotocraftApp, ctx: &egui::Context, moved: &[DocTab], before: Option<DocTab>) {
    let Some(from) = moved.iter().find_map(|tab| match tab {
        DocTab::Doc(id) => document_index(app, *id),
        DocTab::Opening(_) => None,
    }) else {
        return;
    };
    let visible = docked_document_indices(app);
    let remaining: Vec<usize> = visible.iter().copied().filter(|&d| d != from).collect();
    let before = match before {
        Some(DocTab::Doc(id)) => document_index(app, id),
        _ => None,
    };
    let to = before.and_then(|b| remaining.iter().position(|&d| d == b)).unwrap_or(remaining.len());
    let target = move_target_after_filter(&visible, from, to, app.session.documents().len());
    if target != from {
        run(app, ctx, "document.move", json!({"document": from, "to": target}));
    }
}

/// Runs a tab command through the menus' guarded path; a refusal goes to the status line.
fn run(app: &mut PhotocraftApp, ctx: &egui::Context, id: &'static str, params: serde_json::Value) {
    if let Err(e) = crate::menus::invoke(app, ctx, id, params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The tab menu for the document a right click (or the menu key) named, at the place the request gave. It closes
/// when an entry runs, when it is dismissed and when its document is gone.
fn tab_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, cx: &mut BodyCx<'_>, strip: &Response) {
    let Some((id, at)) = cx.keep::<TabMenu>().0 else { return };
    let Some(index) = document_index(app, id) else {
        cx.keep::<TabMenu>().0 = None;
        return;
    };
    let count = app.session.documents().len();
    let has_path = app.session.documents()[index].path.is_some();
    let items = tab_context_items(id, index, count, has_path);
    let mut entries = Vec::with_capacity(items.len() + TAB_MENU_SEPARATORS_AFTER.len());
    for (row, (label, _, _, enabled)) in items.iter().enumerate() {
        entries.push(Entry::new(row, tl!(*label)).enabled(*enabled));
        if TAB_MENU_SEPARATORS_AFTER.contains(&row) {
            entries.push(Entry::separator());
        }
    }
    let mut open = true;
    let picked =
        Menu::new(tl!("Document Tab")).opened_by(strip, Trigger::Manual).anchor(Anchor::Point(at)).open(Bind::bound(&mut open)).pick(ui, &entries).picked;
    if !open || picked.is_some() {
        cx.keep::<TabMenu>().0 = None;
    }
    if let Some((_, command, params, _)) = picked.and_then(|row| items.into_iter().nth(row)) {
        run(app, ui.ctx(), command, params);
    }
}

/// Rows of the document tab menu after which a separator is drawn: after Move to New Window and
/// after Open Document…, as in the reference (#11403).
pub(crate) const TAB_MENU_SEPARATORS_AFTER: [usize; 2] = [4, 6];

pub(crate) type TabMenuItem = (&'static str, &'static str, serde_json::Value, bool);

/// All tab actions go through the same guarded commands as the menu bar, including the
/// unsaved-changes prompt. The clicked tab's document `doc` (at `index` in the session's list now) is explicit even when
/// another document is active: the file commands name it by its index, Move to New Window by its id.
/// Order and separators follow the reference application (#11403); see [`TAB_MENU_SEPARATORS_AFTER`].
pub(crate) fn tab_context_items(doc: DocId, index: usize, count: usize, has_path: bool) -> [TabMenuItem; 8] {
    [
        ("Consolidate All to Here", "window.arrange.consolidateAllToTabs", json!({}), count > 1),
        ("Close", "file.close", json!({"document": index}), true),
        ("Close All", "file.closeAll", json!({}), true),
        ("Close Others", "file.closeOthers", json!({"document": index}), count > 1),
        ("Move to New Window", "window.arrange.floatInWindow", json!({"document": doc.0}), true),
        ("New Document…", "file.new", json!({}), true),
        ("Open Document…", "file.open", json!({}), true),
        (crate::menus::reveal_label(), "file.revealInFinder", json!({"document": index}), has_path),
    ]
}

/// Photoshop's tab title: "name @ 50% (Layer 1, RGB/8)" at `zoom` percent, with "(Layer 1, Layer Mask/8)" when the
/// mask is targeted and "(Quick Mask/8)" in Quick Mask mode; the Background layer's name is omitted. The tab's
/// modified mark shows unsaved changes, so the text carries none.
pub(crate) fn tab_title_at(app: &PhotocraftApp, i: usize, zoom: f32) -> String {
    use photocraft_engine::channel_cmds::ChannelTarget;
    let Some(st) = app.session.documents().get(i) else { return String::new() };
    // Quick Mask mode targets the mask, not the layer (see `paint_target`): painting and Free
    // Transform change the red overlay, so the tab says so, as Photoshop's does.
    let quick_mask = st.doc.quick_mask.is_some() && st.channel_view.target == ChannelTarget::Composite;
    let active_layer = st.active_layer.and_then(|id| st.doc.layer(id)).filter(|l| !quick_mask && !st.doc.is_background(l.id));
    let mask = app.session.active_index() == Some(i) && app.ui.mask_target && active_layer.is_some_and(|l| l.mask.is_some());
    let model = if quick_mask {
        tl!("Quick Mask").to_string()
    } else if mask {
        tl!("Layer Mask").to_string()
    } else {
        mode_label(&st.doc).to_string()
    };
    let layer = active_layer.map(|l| format!("{}, ", l.name)).unwrap_or_default();
    // A save in progress: "name @ 100% (Layer 1, RGB/16) - Saving 45%", as Photoshop does (#3389).
    let saving = crate::jobs_ui::save_progress(app, st.doc.id).map(|s| format!(" - {s}")).unwrap_or_default();
    format!("{} @ {}% ({layer}{model}/{}){saving}", titled_name(app, st), fmt_zoom(zoom), st.doc.depth.bits())
}

/// The document's name as its tab and window title show it: "© name" when File Info marks it
/// Copyrighted, as Photoshop does (#13821).
pub(crate) fn titled_name(app: &PhotocraftApp, st: &photocraft_engine::DocState) -> String {
    let copyrighted = app.doc_queries.copyrighted(&app.session, st);
    format!("{}{}", if copyrighted { "\u{a9} " } else { "" }, st.doc.name)
}

/// The documents with a tab in the main window, in document order: every one not floating in its own window.
pub(super) fn docked_document_indices(app: &PhotocraftApp) -> Vec<usize> {
    app.session.documents().iter().enumerate().filter(|(_, st)| !crate::view_cmds::document_is_floating(app, st.doc.id)).map(|(i, _)| i).collect()
}

/// The document the main window's single canvas shows: the active one, or the first docked one while the active
/// document floats.
pub(super) fn main_canvas_document(app: &PhotocraftApp) -> Option<usize> {
    let active = app.session.active()?;
    if !crate::view_cmds::document_is_floating(app, active.doc.id) {
        return app.session.active_index();
    }
    docked_document_indices(app).into_iter().next()
}

/// Convert a final tab index in the visible (docked-only) strip to the document list index,
/// keeping hidden floating documents in their existing relative order.
pub(super) fn move_target_after_filter(visible: &[usize], from: usize, to: usize, document_count: usize) -> usize {
    let remaining: Vec<usize> = visible.iter().copied().filter(|&doc| doc != from).collect();
    match remaining.get(to) {
        Some(&doc) if doc > from => doc - 1,
        Some(&doc) => doc,
        None => document_count.saturating_sub(1),
    }
}

#[cfg(test)]
#[path = "../file_info_copyright_title_tests.rs"]
mod file_info_copyright_title_tests;
