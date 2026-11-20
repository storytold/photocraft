use craft::behavior::CanvasView;
use craft::behavior::kurbo::Size;
use craft::theme::variant::Tone;
use craft::ui::bars::{Bar, BarKind, StatusItem, StatusMessage, StatusProgress, StatusReadout, StatusValues};
use craft::ui::contract::{Bind, MemoProps, MemoView, Outbox, ShowContainer as _};
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

/// Every input of the status bar.
#[derive(MemoProps)]
struct StatusProps<'a> {
    values: StatusValues,
    item: StatusItem,
    document: Option<Size>,
    /// The active document's main canvas as its last pass showed it; a document not yet shown has no readout.
    view: Option<CanvasView>,
    message: Option<StatusMessage>,
    render_path: Option<StatusMessage>,
    redraw: Option<Progress>,
    job: Option<Progress>,
    out: Outbox<'a, StatusEvent>,
}

/// What a run of the status bar reports.
enum StatusEvent {
    /// The readout's item was picked.
    Item(StatusItem),
    /// The zoom field edited the view.
    View(CanvasView),
    /// The zoom field cut an entry to its range.
    InvalidZoom(InvalidZoom),
    /// The job's Cancel was clicked.
    CancelJob,
}

/// The status bar, in the shell's status slot: the zoom readout of the active document and the latest status
/// message, and at the far end the render-path note, the background redraw and the background jobs' progress with
/// Cancel (#210). One view holds the whole bar, so a frame where none of it changed replays it.
pub fn status_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let job = crate::jobs_ui::status_job(app);
    let mut sent = Vec::new();
    let props = StatusProps {
        values: chrome_ui::status_values(app),
        item: chrome_ui::status_item(&app.ui.chrome),
        document: app.session.active().map(|st| Size::new(st.doc.size.width as f64, st.doc.size.height as f64)),
        view: crate::canvas::active_view(app),
        message: (!app.ui.status.is_empty())
            .then(|| StatusMessage::new(app.ui.status.clone()).tone(if app.ui.status_error { Tone::Warn } else { Tone::Neutral })),
        render_path: chrome_ui::render_path_label(app)
            .map(|(label, tip, command)| StatusMessage::new(label).tone(Tone::Warn).command(CommandRef::new(command)).tooltip(tip)),
        redraw: crate::background_redraw::progress(app),
        job: job.as_ref().map(|(_, progress)| progress.clone()),
        out: Outbox::new(&mut sent),
    };
    let _ = MemoView::new("status-bar", props, status_view).show(ui);
    let mut invalid_zoom = None;
    for event in sent {
        match event {
            StatusEvent::Item(picked) => chrome_ui::set_status_item(&mut app.ui.chrome, picked),
            StatusEvent::View(new) => {
                if let Some(i) = app.session.active_index() {
                    let ppp = app.canvas_ppp();
                    app.ui.views[i].set_from(&new, ppp);
                }
            }
            StatusEvent::InvalidZoom(invalid) => invalid_zoom = Some(invalid),
            StatusEvent::CancelJob => {
                if let Some((id, _)) = &job {
                    crate::jobs_ui::cancel(app, *id);
                }
            }
        }
    }
    chrome_ui::zoom_alert(app, invalid_zoom);
}

/// The status bar's view.
fn status_view(props: &mut StatusProps<'_>, ui: &mut egui::Ui) {
    let mut cancelled = false;
    let mut picked = None;
    let mut invalid_zoom = None;
    let mut edited = None;
    Bar::new("", BarKind::Status)
        .trailing(|ui, _| {
            if let Some(shown) = &props.render_path {
                shown.show(ui);
            }
            if let Some(progress) = &props.redraw {
                progress.widget().show(ui);
            }
            if let Some(progress) = &props.job {
                cancelled |= progress.widget().show(ui).cancelled;
            }
        })
        .show(ui, |ui, _| {
            match (props.document, props.view) {
                (Some(size), Some(view)) => {
                    let out = StatusReadout::new(Bind::given(view), size, props.item, &props.values).show(ui);
                    picked = out.item.or(picked);
                    invalid_zoom = out.invalid_zoom.or(invalid_zoom.take());
                    edited = out.view.value().copied().or(edited);
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
        });
    if let Some(picked) = picked {
        props.out.send(StatusEvent::Item(picked));
    }
    if let Some(view) = edited {
        props.out.send(StatusEvent::View(view));
    }
    if let Some(invalid) = invalid_zoom {
        props.out.send(StatusEvent::InvalidZoom(invalid));
    }
    if cancelled {
        props.out.send(StatusEvent::CancelJob);
    }
}
