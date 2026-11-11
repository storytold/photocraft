//! The redraw thread: a sweep's blocks ([`super::sweep`]) composited off the UI thread.
//!
//! On the UI thread a sweep draws only what fits a frame's budget (~24 ms), and the GPU waits
//! between frames: on a 1.1-gigapixel storyboard it was busy for about 15% of a first draw. Here
//! a thread takes the blocks as fast as the GPU and the compositor allow, while the UI thread
//! keeps to input and painting and shows each block as it lands.
//!
//! The UI thread [`publish`]es the document it shows, every frame before it draws anything; the
//! thread reads it once it holds the compositor ([`GpuCanvas::composite_lent`]), so a block always
//! shows the newest version its render could see. A block the thread can't draw goes back to the
//! UI thread's sweep, which draws it as before. Off with `PHOTOCRAFT_SWEEP_ON_UI_THREAD` (to
//! compare), on the web, and in unit tests, which step sweeps frame by frame.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use photocraft_doc::{DocId, Document};
use photocraft_geom::Rect;

use crate::PhotocraftApp;
use crate::gpu_canvas::GpuCanvas;

/// Blocks per job while an edit draws itself (a stroke or a drag): few, so that edit's own
/// render waits little for the compositor.
pub(crate) const JOB_BLOCKS: usize = 3;
/// Blocks per job otherwise. Each render costs about 7.5 ms whatever its size (planning the
/// document, the prefix cache check, setup) against about 2.6 ms a block on the storyboard this
/// was measured on, so three blocks a job spent half the first redraw on that; nine cut it to
/// about a third.
pub(crate) const JOB_BLOCKS_IDLE: usize = 9;
/// Jobs queued per document at a time, so the thread always has the next one.
pub(crate) const IN_FLIGHT: usize = 2;

/// The document each canvas shows, by document id, and whether its texture stores the sRGB
/// encoding.
type Latest = Arc<Mutex<HashMap<u64, (Arc<Document>, bool)>>>;

/// Blocks of one document for the thread to draw.
struct Job {
    key: u64,
    ticket: u64,
    blocks: Vec<Rect>,
}

/// A finished job, back on the UI thread.
pub(crate) struct Done {
    pub(crate) doc: DocId,
    pub(crate) ticket: u64,
    pub(crate) blocks: Vec<Rect>,
    pub(crate) result: Result<photocraft_gpu::Stats, photocraft_gpu::Unsupported>,
}

/// The running thread: where jobs go, where they come back, and the documents it draws.
pub(crate) struct RedrawThread {
    tx: Sender<Job>,
    rx: Receiver<Done>,
    latest: Latest,
    next: u64,
}

/// Whether sweeps may run on the redraw thread: not on the web, not in unit tests (they step
/// sweeps frame by frame), and not with `PHOTOCRAFT_SWEEP_ON_UI_THREAD` set.
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !cfg!(target_arch = "wasm32") && !cfg!(test) && std::env::var_os("PHOTOCRAFT_SWEEP_ON_UI_THREAD").is_none())
}

impl RedrawThread {
    /// No threads on the web: the sweep draws on the UI thread.
    #[cfg(target_arch = "wasm32")]
    fn spawn(_: GpuCanvas, _: egui::Context) -> Option<Self> {
        None
    }

    /// Start the thread; `None` when it can't start (the sweeps then stay on the UI thread).
    #[cfg(not(target_arch = "wasm32"))]
    fn spawn(gpu: GpuCanvas, ctx: egui::Context) -> Option<Self> {
        let (tx, jobs) = std::sync::mpsc::channel::<Job>();
        let (done, rx) = std::sync::mpsc::channel::<Done>();
        let latest: Latest = Arc::default();
        let shared = latest.clone();
        let spawned = std::thread::Builder::new().name("photocraft-redraw".into()).spawn(move || {
            // Ends when the app drops its end of the channel.
            while let Ok(job) = jobs.recv() {
                let pick = || shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&job.key).cloned();
                let result = gpu.composite_lent(pick, &job.blocks);
                if done.send(Done { doc: DocId(job.key), ticket: job.ticket, blocks: job.blocks, result }).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        match spawned {
            Ok(_) => {
                log::info!("canvas: redrawing in blocks on the redraw thread");
                Some(RedrawThread { tx, rx, latest, next: 0 })
            }
            Err(e) => {
                log::warn!("canvas: could not start the redraw thread ({e}); redrawing on the UI thread");
                None
            }
        }
    }

    /// Queue `blocks` of document `doc`; returns the job's ticket, `None` when the thread is gone.
    pub(crate) fn send(&mut self, doc: DocId, blocks: Vec<Rect>) -> Option<u64> {
        self.next += 1;
        self.tx.send(Job { key: doc.0, ticket: self.next, blocks }).ok()?;
        Some(self.next)
    }

    /// The jobs finished since the last call, and whether the thread is still there (it isn't
    /// once it has stopped, and the jobs it had then never come back).
    pub(crate) fn finished(&self) -> (Vec<Done>, bool) {
        let mut done = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(d) => done.push(d),
                Err(std::sync::mpsc::TryRecvError::Empty) => return (done, true),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return (done, false),
            }
        }
    }
}

/// The redraw thread, started the first time a sweep needs it; `None` where it is off or
/// couldn't start.
pub(crate) fn thread<'a>(app: &'a mut PhotocraftApp, gpu: &GpuCanvas, ctx: &egui::Context) -> Option<&'a mut RedrawThread> {
    if !enabled() || app.redraw_thread_failed {
        return None;
    }
    if app.redraw_thread.is_none() {
        app.redraw_thread = RedrawThread::spawn(gpu.clone(), ctx.clone());
        app.redraw_thread_failed = app.redraw_thread.is_none();
    }
    app.redraw_thread.as_mut()
}

/// Tell the redraw thread which version of document `doc` its canvas shows now, and whether the
/// texture stores the sRGB encoding, while a sweep of it runs (else forget it). Called every frame
/// before the canvas draws anything itself, and before jobs are sent.
pub(crate) fn publish(app: &PhotocraftApp, doc: &Arc<Document>, encode_srgb: bool) {
    let Some(t) = app.redraw_thread.as_ref() else { return };
    let mut latest = t.latest.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let live: Vec<u64> = app.session.documents().iter().map(|d| d.doc.id.0).collect();
    latest.retain(|k, _| live.contains(k));
    // Kept only while a sweep of it runs: a version held here shares its tiles with the next, so
    // a brush stroke copies every tile it touches rather than painting into it.
    if super::sweep::running(app, doc.id) {
        latest.insert(doc.id.0, (doc.clone(), encode_srgb));
    } else {
        latest.remove(&doc.id.0);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "redraw_thread_tests.rs"]
mod tests;
