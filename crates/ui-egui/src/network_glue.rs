//! Print ▸ Network printer (WS-Print or IPP): the app's part of `craft_ui::printing::network`. The model,
//! its rules and the widgets are craft-ui's; the printing is craft-print's WS-Print and IPP backends through
//! `photocraft_print::network`. This file supplies what a host supplies: the address check, the
//! worker that asks a printer what it lists, the worker that sends a job and follows it, the
//! mapping of craft-print's answers onto craft-ui's light types, and the wording.
//!
//! Built only with the `network-print` feature on a target with sockets (the `network_print` cfg).
//! All network work runs on its own threads; the UI thread only starts it and drains receivers.
//!
//! The document is the print-ready PDF the engine wrote for the page it laid out
//! (`photocraft_engine::print_cmds::prepare_network_print`). A printer that lists PDF gets it as it is;
//! one that lists only PWG raster gets the same page rendered by the engine at the resolution the registry
//! chose (the printer's listed one nearest 300 dpi), through `photocraft_print::network::RenderedPages`,
//! which is set on the job's own station so a later print can never reuse it.
//!
//! One press of Print is one tracked `PrintJob` (see `tracked`): it sends the document, follows the job,
//! retries only what is safe, and says "check the printer before sending again" when the outcome of a
//! send is unknown. craft-ui's job model is fed from the same worker.
//!
//! The dialog's wording goes through `tl!`, so every catalog that claims complete menus carries it (the
//! rows were machine-drafted and await a native speaker's review). The tracked print's status sentences
//! (`tracked`) are status messages, which stay English (docs/localization.md).

use std::sync::{Arc, Mutex, PoisonError, mpsc};

use craft_ui::printing::network::{self as net, Failure, FailureKind, JobHandle, Labels, PrinterSummary, QueryPrinter, Request, SendJob};
use photocraft_print::network::ipp::PrinterAttrs;
use photocraft_print::network::station::{self, Station};
use photocraft_print::network::wsd::PrinterElements;
use photocraft_print::network::{Duplex, Job, JobOptions, Orientation, PrintError, Quality, UserConfirmed};

mod caps;
mod tracked;
use tracked::{Command, Slot};

use photocraft_engine::print_cmds::NetworkPrint;
use photocraft_print::network::RenderedPages;

/// Any transport that can be shared with the worker threads (tests give an in-memory printer).
pub type Transport = photocraft_print::network::Transport;

/// The media types the dialog can produce: the print-ready PDF, or its sheets as PWG raster
/// (the registry converts through `PdfToRaster` when the printer lists no PDF).
pub(crate) const FORMATS: &[&str] = &["application/pdf", "image/pwg-raster"];

/// The printer last addressed: its registry (WS-Print and IPP backends) and, with the test env var, its
/// WS-Eventing listener, kept for the query and for the jobs that follow, keyed by the typed address.
pub(crate) type Stations = Arc<Mutex<Option<(String, Arc<Station>)>>>;

/// The station for `address`: the one already open for that text, else a new one. `PHOTOCRAFT_TEST_EVENTS_BIND`
/// (debug builds) turns the WS-Eventing listener on for a WS-Print printer.
fn station_for(stations: &Stations, address: &str, transport: Option<Transport>) -> Result<Arc<Station>, PrintError> {
    let mut slot = lock(stations);
    if let Some((typed, station)) = slot.as_ref()
        && typed == address
    {
        return Ok(Arc::clone(station));
    }
    let station = Arc::new(open_station(address, transport)?);
    *slot = Some((address.to_owned(), Arc::clone(&station)));
    Ok(station)
}

/// A new station for `address`. `PHOTOCRAFT_TEST_EVENTS_BIND` (debug builds) turns the WS-Eventing listener on
/// for a WS-Print printer.
fn open_station(address: &str, transport: Option<Transport>) -> Result<Station, PrintError> {
    let events = match station::events_from_env() {
        Some(Ok(config)) => Some(config),
        Some(Err(e)) => return Err(e),
        None => None,
    };
    Station::open(address, transport, events)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
const MAX_JOB_NAME: usize = 100;

/// The destination's state and the closures its workers are made from.
#[derive(Default)]
pub(crate) struct NetTasks {
    transport: Option<Transport>,
    pub(crate) state: net::State,
    query: Option<QueryPrinter>,
    pub(crate) stations: Stations,
    /// The tracked print of the last job: its status line, buttons and the small status row.
    pub(crate) tracked: Slot,
    /// The destination of the Print dialog is the network printer rather than the system's.
    pub(crate) selected: bool,
    ctx: Option<egui::Context>,
}

impl NetTasks {
    pub(crate) fn set_transport(&mut self, transport: Transport) {
        self.transport = Some(transport);
        self.query = None;
        *lock(&self.stations) = None;
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn transport(&self) -> Option<Transport> {
        self.transport.clone()
    }

    /// Draw the section; `sheets_ok`: the dialog's settings give something to print.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, sheets_ok: bool) -> net::Response {
        let (transport, ctx, stations) = (self.transport.clone(), ui.ctx().clone(), Arc::clone(&self.stations));
        self.ctx = Some(ctx.clone());
        let query = self.query.get_or_insert_with(|| query_printer(transport, Some(ctx), stations));
        let host = net::Host { validate: &validate, query, sheets_ok, formats: FORMATS };
        let response = self.state.show(ui, egui::Id::unique("network-printer"), &host, &labels());
        status_block(ui, &self.tracked, false);
        response
    }
}

/// The address checked against the network policy (a numeric loopback, link-local or private
/// address). The error is what the dialog shows under the field.
pub(crate) fn validate(text: &str) -> Result<(), Failure> {
    station::validate(text).map(|_| ()).map_err(|e| Failure::new(FailureKind::Policy, e.to_string()))
}

/// What went wrong, as craft-ui words it. `open`: the failure is the printer's address being refused.
fn failure(e: &PrintError, open: bool) -> Failure {
    tracked::failure_of(&photocraft_print::network::AttemptError::from(e), open)
}

fn summary(elements: PrinterElements) -> PrinterSummary {
    let caps = caps::wsd(&elements);
    let description = elements.description.unwrap_or_default();
    let status = elements.status.unwrap_or_default();
    PrinterSummary {
        name: description.name,
        color: description.color_supported,
        state: status.state,
        queued_jobs: status.queued_jobs,
        reasons: status.reasons,
        caps,
    }
}

fn repaint(ctx: &Option<egui::Context>) {
    if let Some(ctx) = ctx {
        ctx.request_repaint();
    }
}

/// What an IPP printer's attributes say, as craft-ui's light types (the keywords are the dialog's tokens).
fn ipp_summary(attrs: &PrinterAttrs) -> PrinterSummary {
    let state = match attrs.state {
        Some(3) => "Idle",
        Some(4) => "Processing",
        Some(5) => "Stopped",
        _ => "Unknown",
    };
    let mut reasons: Vec<String> = attrs.state_reasons.iter().filter(|r| r.as_str() != "none").cloned().collect();
    if attrs.accepting_jobs == Some(false) {
        reasons.push("not-accepting-jobs".to_owned());
    }
    PrinterSummary {
        name: attrs.name.clone().or_else(|| attrs.info.clone()).or_else(|| attrs.make_and_model.clone()).unwrap_or_default(),
        color: attrs.color_supported,
        state: state.to_owned(),
        queued_jobs: None,
        reasons,
        caps: caps::ipp(attrs),
    }
}

/// The worker that asks the printer at the (normalised) address what it is and what it lists, over the
/// protocol its address names (WS-Print or IPP). It sends no job.
pub(crate) fn query_printer(transport: Option<Transport>, ctx: Option<egui::Context>, stations: Stations) -> QueryPrinter {
    Box::new(move |address: &str| {
        let (tx, rx) = mpsc::channel();
        let (address, transport, ctx, stations) = (address.to_owned(), transport.clone(), ctx.clone(), Arc::clone(&stations));
        std::thread::spawn(move || {
            let result = match station_for(&stations, &address, transport) {
                Ok(station) => match station.protocol() {
                    station::Protocol::Wsd => station.wsd_info().map(summary).map_err(|e| failure(&e, false)),
                    station::Protocol::Ipp => station.ipp_info().map(|attrs| ipp_summary(&attrs)).map_err(|e| failure(&e, false)),
                }
                .inspect(|summary| station.set_display_name(&summary.name)),
                Err(e) => Err(failure(&e, true)),
            };
            let _ = tx.send(result);
            repaint(&ctx);
        });
        rx
    })
}

/// The job and its per-job options for the registry. `Photo` is craft-print's `Quality::Photo`: the
/// backend maps it to the printer's own quality when the printer lists one and says so when it does not.
fn options(request: &Request, job_name: &str) -> (Job, JobOptions) {
    let name: String = job_name.chars().filter(|c| !c.is_control()).take(MAX_JOB_NAME).collect();
    let mut job = Job::default();
    if !name.trim().is_empty() {
        job.title = name;
    }
    job.copies = request.copies.unwrap_or(1);
    job.duplex = request.sides.map(|s| match s {
        net::Sides::OneSided => Duplex::Off,
        net::Sides::LongEdge => Duplex::LongEdge,
        net::Sides::ShortEdge => Duplex::ShortEdge,
    });
    let mut options = JobOptions::new();
    if let Some(o) = request.orientation {
        options = options.with_orientation(match o {
            net::Orientation::Portrait => Orientation::Portrait,
            net::Orientation::Landscape => Orientation::Landscape,
            net::Orientation::ReversePortrait => Orientation::ReversePortrait,
            net::Orientation::ReverseLandscape => Orientation::ReverseLandscape,
        });
    }
    match request.quality {
        Some(net::Quality::Draft) => options = options.with_quality(Quality::Draft),
        Some(net::Quality::Normal) => options = options.with_quality(Quality::Normal),
        Some(net::Quality::High) => options = options.with_quality(Quality::High),
        Some(net::Quality::Photo) => options = options.with_quality(Quality::Photo),
        None => {}
    }
    options = caps::apply_request(options, request);
    if let Some(media) = &request.media_size {
        options = options.with_media(media.as_str());
    }
    if let Some(pages) = request.expected_pages {
        options = options.with_expected_pages(pages);
    }
    (job, options)
}

/// The worker that makes the tracked print of `print` and drives it (see [`tracked`]). Each call makes a
/// new print on a station of its own, with the person's `confirmed` (never `interactive()`: it is the
/// attestation of the ticked box, taken when Print was pressed).
pub(crate) fn send_job(
    transport: Option<Transport>,
    ctx: Option<egui::Context>,
    print: NetworkPrint,
    (stations, slot): (Stations, Slot),
    confirmed: UserConfirmed,
) -> SendJob {
    Box::new(move |address: &str, request: Request| {
        let (job, options) = options(&request, &print.title);
        // The name the printer gave in the query, for the status lines; a new station for the print.
        let name = lock(&stations).as_ref().filter(|(typed, _)| typed == address).map(|(_, s)| s.display_name()).unwrap_or_default();
        match open_station(address, transport.clone()) {
            Ok(station) => {
                station.set_display_name(&name);
                let page = print.clone();
                let render: Arc<photocraft_print::network::RenderFn> = Arc::new(move |spec| page.raster_pages(spec.dpi, spec.colour));
                station.set_converter(Arc::new(RenderedPages::new(Arc::clone(&print.pdf), render)));
                let tracked_print = station.job(print.pdf.as_ref().clone(), job, options, confirmed);
                tracked::spawn(Arc::new(station), tracked_print, &slot, ctx.clone())
            }
            Err(e) => {
                let (tx, rx) = mpsc::channel();
                let _ = tx.send(Err(failure(&e, true)));
                JobHandle::new(rx, Box::new(|| {}))
            }
        }
    })
}

/// TEST BUILD: the status of the tracked print (a line built from its `JobView`, the retry advice, the last
/// answer of a command) and the buttons that make sense for the state. `row`: the small row shown with the
/// dialog closed (adds Dismiss). The status lines are English (status messages); the buttons are translated.
pub(crate) fn status_block(ui: &mut egui::Ui, slot: &Slot, row: bool) -> bool {
    let Some(view) = lock(slot).as_ref().and_then(|t| t.view()) else { return false };
    let mut dismiss = false;
    ui.add_space(4.0);
    ui.label(egui::RichText::new(format!("{}: {}", view.view.title, tracked::status_text(&view.view, &view.name))).strong());
    if let Some(advice) = tracked::advice_text(view.advice, view.lifecycle, &view.name) {
        ui.label(advice);
    }
    if let Some(stale) = tracked::stale_text(view.stale, &view.name) {
        ui.label(stale);
    }
    if let Some(note) = &view.note {
        ui.label(egui::RichText::new(note).small());
    }
    if let Some(events) = &view.events {
        ui.label(egui::RichText::new(format!("WS-Eventing listener: {events}")).small().weak());
    }
    let buttons = tracked::buttons(&view);
    ui.horizontal_wrapped(|ui| {
        let guard = lock(slot);
        let Some(tracked) = guard.as_ref() else { return };
        let mut ask = |enabled: bool, label: &str, tip: &str, command: Command| {
            if ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(tip).clicked() {
                tracked.send(command);
            }
        };
        ask(buttons.cancel, tl!("Cancel print"), tl!("Ask the printer to cancel this print"), Command::Cancel);
        ask(buttons.retry, tl!("Retry"), tl!("Send this print again as a new attempt (check the printer first if it says so)"), Command::Retry);
        ask(buttons.reconcile, tl!("Reconcile"), tl!("Look at the printer's job lists (reads only) for a job from the earlier attempt"), Command::Reconcile);
        ask(buttons.accept, tl!("Accept found job"), tl!("Follow the job the printer already has instead of sending again"), Command::AcceptFound);
        ask(buttons.refresh, tl!("Check now"), tl!("Read the job's status now"), Command::Refresh);
        if row && ui.button(tl!("Dismiss")).clicked() {
            dismiss = true;
        }
    });
    if dismiss {
        *lock(slot) = None;
    }
    true
}

/// The wording of the destination, in the interface language. Templates keep their `{...}` holes.
pub(crate) fn labels() -> Labels<'static> {
    Labels {
        address: tl!("Printer address:"),
        address_hint: "http://192.168.1.50:5357/service  or  ipp://192.168.1.50:631/ipp/print",
        address_note: tl!("Only IP addresses on this network are accepted (private, link-local or loopback), not host names."),
        query_printer: tl!("Query printer"),
        querying: tl!("Asking the printer…"),
        query_first: tl!("Query the printer first to see what it supports."),
        printer_status: tl!("Printer status: {state}"),
        reasons: tl!("Reasons: {reasons}"),
        no_common_format: tl!("The printer accepts none of the formats this document can be sent in"),
        copies: tl!("Copies:"),
        sides: tl!("Two-sided:"),
        sides_off: tl!("Off"),
        sides_long_edge: tl!("Flip on long edge"),
        sides_short_edge: tl!("Flip on short edge"),
        orientation: tl!("Printer orientation:"),
        portrait: tl!("Portrait"),
        landscape: tl!("Landscape"),
        reverse_portrait: tl!("Reverse portrait"),
        reverse_landscape: tl!("Reverse landscape"),
        quality: tl!("Quality:"),
        draft: tl!("Draft"),
        normal: tl!("Normal"),
        high: tl!("High"),
        photo: tl!("Photo"),
        color: tl!("Colour:"),
        color_color: tl!("Colour"),
        color_monochrome: tl!("Grayscale"),
        resolution: tl!("Resolution:"),
        resolution_dpi: "{dpi} dpi",
        resolution_dpi_xy: "{x} x {y} dpi",
        auto: tl!("Auto"),
        unsupported_copies: tl!("This printer prints one copy at a time."),
        unsupported_sides: tl!("This printer only prints one-sided."),
        unsupported_orientation: tl!("This printer does not take an orientation setting."),
        unsupported_quality: tl!("This printer does not take a quality setting."),
        unsupported_color: tl!("This printer only prints in grayscale."),
        unsupported_resolution: tl!("This printer does not take a resolution setting."),
        unconfirmed_note: tl!("Not confirmed by the printer."),
        confirm: tl!("I confirm sending this document to this printer"),
        sending: tl!("Sending to {printer}…"),
        canceling: tl!("Canceling…"),
        cancel: tl!("Cancel"),
        sent: tl!("Sent to {printer}"),
        accepted: tl!("Accepted by {printer}"),
        stage_waiting: tl!("Waiting in the printer's queue"),
        stage_held: tl!("Held by the printer"),
        stage_printing: tl!("The printer is printing it"),
        stage_stopped: tl!("The printer paused the job; it needs attention"),
        sheets: tl!("Sheets printed: {n}"),
        sheets_progress: tl!("Sheets printed: {done} of {total}"),
        outcome_warnings: tl!("Printed with warnings"),
        outcome_errors: tl!("Printed with errors"),
        outcome_canceled: tl!("The job was canceled"),
        outcome_aborted: tl!("The printer stopped the job"),
        outcome_forgotten: tl!("The printer no longer lists the job; it most likely finished"),
        outcome_still_running: tl!("Still printing; the job continues on the printer"),
        outcome_accepted_unknown: tl!("Sent. The printer can't say whether it printed."),
        outcome_unknown_warning: tl!("The printer may have received the document. Check the printer before sending again."),
        failure_policy: tl!("The printer address was refused"),
        failure_transport: tl!("Couldn't reach the printer"),
        failure_protocol: tl!("The printer's reply couldn't be understood"),
        failure_fault: tl!("The printer refused the request"),
        failure_no_common_format: tl!("The printer accepts none of the formats this document can be sent in"),
        failure_unsupported: tl!("The printer doesn't support this setting"),
        failure_encode: tl!("The document couldn't be prepared"),
        failure_invalid: tl!("A setting isn't valid"),
        failure_stopped: tl!("The print job stopped unexpectedly"),
        failure_other: tl!("The print job stopped unexpectedly"),
    }
}

/// The one-line notice for a print that ended with the dialog closed, naming the printer by its display name
/// (nothing when it has none, nor an address): "<name>: The printer stopped the job".
fn notice_text(labels: &Labels<'_>, job: &net::JobView, name: &str) -> Option<String> {
    let who = tracked::printer_label(name);
    let named = who != "the printer";
    let prefix = |text: String| if named { format!("{who}: {text}") } else { text };
    match job {
        net::JobView::Finished(r) => Some(match labels.outcome_text(r.outcome) {
            Some(text) => prefix(text.to_owned()),
            None => crate::i18n::fmt(labels.sent, &[("printer", &tracked::printer_label(&r.printer))]),
        }),
        net::JobView::Failed(f) => {
            let mut text = prefix(format!("{}: {}", labels.failure_heading(f.kind), tracked::scrubbed(&f.detail, &who)));
            if f.outcome_unknown {
                text = format!("{text} {}", labels.outcome_unknown_warning);
            }
            Some(text)
        }
        _ => None,
    }
}

/// Whether a print that ended is told as an error: it failed, or the printer stopped it or finished it with errors.
fn ended_badly(job: &net::JobView) -> bool {
    use net::Outcome::{Aborted, CompletedWithErrors};
    matches!(job, net::JobView::Failed(_) | net::JobView::Finished(net::Report { outcome: Aborted | CompletedWithErrors, .. }))
}

impl NetTasks {
    /// Send the laid-out `print` to the queried printer, off the UI thread. It needs the person's
    /// confirmation, which it spends. Returns `true` when a job started.
    pub(crate) fn send(&mut self, print: NetworkPrint) -> bool {
        let address = self.state.draft.normalized_url();
        // The attestation comes from the ticked box and nothing else.
        let Ok(confirmed) = UserConfirmed::attest(Some(self.state.draft.confirmed)) else { return false };
        let media = print.media;
        let send = send_job(self.transport.clone(), self.ctx.clone(), print, (Arc::clone(&self.stations), Arc::clone(&self.tracked)), confirmed);
        // One page: the total that progress shows until the printer reports one.
        self.state.draft.expected_pages = Some(1);
        self.state.start_job(&send, validate(&address).is_ok(), true, FORMATS, media)
    }

    /// Collect finished work (each frame), also while the dialog is closed. Returns the one-line notice
    /// for a print that ended since the last call, naming the printer.
    pub(crate) fn poll(&mut self, ctx: &egui::Context) -> Option<String> {
        let was_running = self.state.draft.running();
        self.state.poll(ctx);
        if !was_running || self.state.draft.running() {
            return None;
        }
        let name = self.state.draft.printer().map(|p| p.name.clone()).unwrap_or_default();
        notice_text(&labels(), &self.state.draft.job, &name)
    }

    /// The small status row of the last tracked print, shown while the Print dialog is closed.
    pub(crate) fn show_status_row(&self, ctx: &egui::Context) {
        if lock(&self.tracked).is_none() {
            return;
        }
        let slot = Arc::clone(&self.tracked);
        egui::Window::new(tl!("Print status"))
            .id(egui::Id::unique("print-status-row"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                status_block(ui, &slot, true);
            });
    }
}

impl crate::PhotocraftApp {
    /// Use `transport` for network printing instead of the built-in HTTP one (tests; embedding).
    pub fn set_network_transport(&mut self, transport: Transport) {
        self.net_print.set_transport(transport);
    }

    /// Each frame: collect finished network work, tell the person how a print ended, and show the status
    /// row while no Print dialog is open.
    pub(crate) fn frame_network(&mut self, ctx: &egui::Context) {
        if let Some(message) = self.net_print.poll(ctx) {
            self.ui.status = message;
            self.ui.status_error = ended_badly(&self.net_print.state.draft.job);
        }
        if !self.ui.dialogs.iter().any(|d| crate::file_ui::owns(&d.fields)) {
            self.net_print.show_status_row(ctx);
        }
    }
}

#[cfg(test)]
mod tests;
