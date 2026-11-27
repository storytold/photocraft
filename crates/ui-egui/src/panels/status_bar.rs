use craft::behavior::CanvasView;
use craft::behavior::kurbo::Size;
use craft::theme::variant::Tone;
use craft::ui::bars::{Bar, BarKind, StatusItem, StatusMessage, StatusProgress, StatusReadout, StatusValues};
use craft::ui::contract::{Bind, MemoProps, MemoView, Outbox, ShowContainer as _};
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::text::InvalidZoom;

use super::{PhotocraftApp, chrome_ui};

// ----------------------------------------------------------------------------- status bar

/// A task's progress as the bar's trailing group shows it: its name, how much is done (`None` while it has reported
/// nothing), the cancel button's label, and why it can't be cancelled.
#[derive(Clone, Debug, PartialEq, MemoProps)]
pub(crate) struct Progress {
    pub label: String,
    pub fraction: Option<f32>,
    pub cancel: Option<String>,
    pub refused: Option<String>,
}

impl Progress {
    fn widget(&self) -> StatusProgress {
        let mut progress = StatusProgress::new(self.label.clone(), self.fraction);
        if let Some(cancel) = &self.cancel {
            progress = progress.cancel(cancel.clone());
        }
        if let Some(refused) = &self.refused {
            progress = progress.cancel_refused(refused.clone());
        }
        progress
    }
}

/// Every input of the status bar's body: the readout and the status message.
#[derive(MemoProps)]
struct StatusProps<'a> {
    values: StatusValues,
    item: StatusItem,
    document: Option<Size>,
    /// The zoom of the active document's main canvas as its last pass showed it, the one view field the readout shows
    /// (a pan leaves it as it is); a document not yet shown has no readout.
    zoom: Option<f64>,
    message: Option<StatusMessage>,
    out: Outbox<'a, StatusEvent>,
}

/// Every input of the status bar's trailing group: the render-path note, the background redraw and the job.
#[derive(MemoProps)]
struct TrailingProps<'a> {
    render_path: Option<StatusMessage>,
    redraw: Option<Progress>,
    job: Option<Progress>,
    out: Outbox<'a, StatusEvent>,
}

/// What a run of the status bar reports.
enum StatusEvent {
    /// The readout's item was picked.
    Item(StatusItem),
    /// The zoom field set the zoom, about the view's centre.
    Zoom(f64),
    /// The zoom field cut an entry to its range.
    InvalidZoom(InvalidZoom),
    /// The job's Cancel was clicked.
    CancelJob,
}

/// The status bar, in the shell's status slot: the zoom readout of the active document and the latest status
/// message, and at the far end the render-path note, the background redraw and the background jobs' progress with
/// Cancel (#210). The slot replays while `region_versions::status_bar` holds; when it runs, the body and the trailing
/// group are a memoized view each, so a frame that changed only one runs only that one. The bar itself stays the slot
/// pane's direct child: it fills the slot's height and centres its content there, where a memoized view around it
/// would be moved down onto the slot row's line and clipped at the window's bottom edge.
pub fn status_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let job = crate::jobs_ui::status_job(app);
    let values = chrome_ui::status_values(app);
    let item = chrome_ui::status_item(&app.ui.chrome);
    let document = app.session.active().map(|st| Size::new(st.doc.size.width as f64, st.doc.size.height as f64));
    let zoom = crate::canvas::active_view(app).map(|view| view.zoom);
    let message =
        (!app.ui.status.is_empty()).then(|| StatusMessage::new(app.ui.status.clone()).tone(if app.ui.status_error { Tone::Warn } else { Tone::Neutral }));
    let render_path = chrome_ui::render_path_label(app)
        .map(|(label, tip, command)| StatusMessage::new(label).tone(Tone::Warn).command(CommandRef::new(command)).tooltip(tip));
    let redraw = crate::background_redraw::progress(app);
    let progress = job.as_ref().map(|(_, progress)| progress.clone());
    // The bar may run each group more than once in a pass (it measures the trailing group first); each run sends into
    // its group's outbox, and the last of each kind of event applies below.
    let (mut body_sent, mut trailing_sent) = (Vec::new(), Vec::new());
    Bar::new("", BarKind::Status)
        .trailing(|ui, _| {
            let props = TrailingProps { render_path: render_path.clone(), redraw: redraw.clone(), job: progress.clone(), out: Outbox::new(&mut trailing_sent) };
            let _ = MemoView::new("status-bar-trailing", props, trailing_view).show(ui);
        })
        .show(ui, |ui, _| {
            let props = StatusProps { values: values.clone(), item, document, zoom, message: message.clone(), out: Outbox::new(&mut body_sent) };
            let _ = MemoView::new("status-bar", props, body_view).show(ui);
        });
    let (mut picked, mut edited, mut invalid_zoom, mut cancelled) = (None, None, None, false);
    for event in body_sent.into_iter().chain(trailing_sent) {
        match event {
            StatusEvent::Item(item) => picked = Some(item),
            StatusEvent::Zoom(new) => edited = Some(new),
            StatusEvent::InvalidZoom(invalid) => invalid_zoom = Some(invalid),
            StatusEvent::CancelJob => cancelled = true,
        }
    }
    if let Some(picked) = picked {
        chrome_ui::set_status_item(&mut app.ui.chrome, picked);
    }
    if let Some(zoom) = edited
        && let Some(i) = app.session.active_index()
    {
        let ppp = app.canvas_ppp();
        app.ui.views[i].set_zoom(zoom, ppp);
    }
    if cancelled && let Some((id, _)) = &job {
        crate::jobs_ui::cancel(app, *id);
    }
    chrome_ui::zoom_alert(app, invalid_zoom);
}

/// The status bar's trailing group, in a row of its own centred down the bar's height, so its members stand on the
/// body's line: the bar's end region lays its children out from its top, and a memoized view's content is a layout
/// room the bar's row does not reach into (#16352).
fn trailing_view(props: &mut TrailingProps<'_>, ui: &mut egui::Ui) {
    let centred = ui.layout().with_cross_align(egui::Align::Center);
    ui.with_layout(centred, |ui| Row::new().show(ui, |ui, _| trailing_members(props, ui)));
}

/// The members of the status bar's trailing group: the render-path note, the background redraw and the job.
fn trailing_members(props: &mut TrailingProps<'_>, ui: &mut egui::Ui) {
    if let Some(shown) = &props.render_path {
        shown.show(ui);
    }
    if let Some(progress) = &props.redraw {
        progress.widget().show(ui);
    }
    if let Some(progress) = &props.job
        && progress.widget().show(ui).cancelled
    {
        props.out.send(StatusEvent::CancelJob);
    }
}

/// The status bar's body, in a row of its own: a memoized view's content is a layout room the bar's row does not reach
/// into, so without it the message would sit at the top of the bar instead of on the readout's line (#16352).
fn body_view(props: &mut StatusProps<'_>, ui: &mut egui::Ui) {
    Row::new().show(ui, |ui, _| body_members(props, ui));
}

/// The members of the status bar's body: the readout, or "No document", and the status message.
fn body_members(props: &mut StatusProps<'_>, ui: &mut egui::Ui) {
    match (props.document, props.zoom) {
        (Some(size), Some(zoom)) => {
            let out = StatusReadout::new(Bind::given(zoom), size, props.item, &props.values).show(ui);
            if let Some(picked) = out.item {
                props.out.send(StatusEvent::Item(picked));
            }
            if let Some(zoom) = out.zoom_edits.value() {
                props.out.send(StatusEvent::Zoom(*zoom));
            }
            if let Some(invalid) = out.invalid_zoom {
                props.out.send(StatusEvent::InvalidZoom(invalid));
            }
        }
        (Some(_), None) => {}
        (None, _) => {
            StatusMessage::new(tl!("No document")).show(ui);
        }
    }
    // A command run with no document open still reports here (#1567: File › Automate › Batch).
    if let Some(shown) = &props.message {
        shown.show(ui);
    }
}

/// The status bar of a floating document window: the readout of its own document `doc` (the zoom field, then the item
/// picked in the main status bar's menu, which picking here changes too), as the reference application's floating
/// document windows have along their bottom (#16415). `view` is the window's canvas as its last pass showed it; a window
/// not shown yet has an empty bar. Returns a view the zoom field set.
pub(crate) fn document_window_status(app: &mut PhotocraftApp, ui: &mut egui::Ui, doc: usize, view: Option<CanvasView>) -> Option<CanvasView> {
    let values = chrome_ui::status_values_of(app, doc);
    let item = chrome_ui::status_item(&app.ui.chrome);
    let size = app.session.documents().get(doc).map(|st| Size::new(st.doc.size.width as f64, st.doc.size.height as f64));
    let (mut picked, mut edited, mut invalid_zoom) = (None, None, None);
    Bar::new("", BarKind::Status).show(ui, |ui, _| {
        if let (Some(size), Some(view)) = (size, view) {
            let out = StatusReadout::new(Bind::given(view.zoom), size, item, &values).show(ui);
            picked = out.item;
            edited = out.zoom_edits.value().map(|zoom| {
                let mut zoomed = view;
                zoomed.zoom = *zoom;
                zoomed
            });
            invalid_zoom = out.invalid_zoom;
        }
    });
    if let Some(picked) = picked {
        chrome_ui::set_status_item(&mut app.ui.chrome, picked);
    }
    chrome_ui::zoom_alert(app, invalid_zoom);
    edited
}
