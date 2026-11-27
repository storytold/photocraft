//! Live strokes paint on a thread of their own, so a heavy brush never holds up the frame: the
//! canvas hands the new pen points over, waits a little for them (`LIVE_STROKE_WAIT`), and shows
//! the newest step the stroke has painted. While the thread is behind, points that arrive meanwhile
//! are painted together as one step, and the cursor, panels and pen input keep their frame rate.
//!
//! Each step is the stroke's own copy of the document, as when it painted on the UI thread, so the
//! canvas shows and refreshes it the same way (`LiveStroke::since`).
//!
//! A stroke whose dabs the GPU draws (`gpu_dabs`) makes only the dabs here: each step is the
//! rectangle they reach, and the thread lists what the canvas has to do with them, in order, as
//! [`GpuWork`]: stamp these dabs, then ask for the coverage tiles that went calm. Only the UI
//! thread reaches the compositor, so the canvas takes that work with the steps (`catch_up`,
//! `take_gpu_work`) and does it before it draws them. The thread itself polls the read-backs and
//! feeds the stroke what arrived, and at the release hands the canvas the rest of the work and
//! waits for it before it makes the commit (`Finishing::take_gpu_work`).

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;
use web_time::Instant;

use photocraft_engine::paint::StrokePoint;
use photocraft_geom::Rect;
use photocraft_gpu::GpuDab;

use super::gpu_dabs::gpu_dab;
use super::strokes::EngineStroke;

/// What painting one batch of points gave: the rectangle it changed, or why it failed.
pub(super) type Step = photocraft_engine::Result<Rect>;

/// Pushes a coverage tile stays untouched before a GPU stroke reads it back during the stroke
/// (the head of the stroke is still changing).
const CALM_PUSHES: u64 = 4;

/// How often the thread looks at read-backs in flight while no points arrive.
const READBACK_POLL: Duration = Duration::from_millis(3);

/// How long the thread waits at the release for the canvas to do the work it handed over, before
/// it gives up and the commit renders the stroke itself.
const HAND_OVER_WAIT: Duration = Duration::from_secs(10);

/// Blocking polls the end of a GPU stroke makes for its read-backs (one waits for all of them).
const FINAL_POLLS: usize = 4;

/// A coverage tile read back from the GPU: its key, its values and the epoch it was asked at.
pub(super) type Coverage = ((i32, i32), Vec<f32>, u64);

/// Where a GPU stroke's coverage tiles come back from (`MaskReadback` polled with the device).
pub(super) trait CoverageSource: Send + Sync {
    /// The tiles that arrived, oldest request first; with `block`, waits for all that were asked.
    fn poll(&self, block: bool) -> Vec<Coverage>;
    /// Requests whose tiles `poll` hasn't returned yet.
    fn in_flight(&self) -> usize;
}

/// What the canvas does for a GPU stroke on the UI thread, which alone reaches the compositor.
/// The work comes in the order it must be done: a read-back request follows the stamps of every
/// dab pushed before it, so the copies see them.
pub(super) enum GpuWork {
    /// Stamp these dabs into the stroke's mask.
    Dabs(Vec<GpuDab>),
    /// Ask for these coverage tiles, as of this epoch (the stroke's push count).
    Read(Vec<(i32, i32)>, u64),
}

/// A stroke whose dabs the GPU draws.
pub(super) struct GpuMode {
    /// The document the canvas shows for the stroke: the live document whose colour layer's mask
    /// is the GPU's (`LiveStroke::gpu_preview`). The stroke's own copy of the document holds no
    /// stroke pixels.
    pub(super) doc: Arc<photocraft_doc::Document>,
    pub(super) source: Arc<dyn CoverageSource>,
}

/// The stroke as last painted, for the canvas to show.
#[derive(Clone)]
pub(super) struct Shown {
    pub(super) doc: Arc<photocraft_doc::Document>,
    /// See `EngineStroke::wants_time`.
    pub(super) wants_time: bool,
    /// The Brush's smoothing string (`photocraft_engine::brush_cmds::LiveStroke::smoothing_string`).
    pub(super) smoothing_string: Option<([f64; 2], [f64; 2])>,
}

impl Shown {
    fn of(stroke: &EngineStroke, gpu: Option<&GpuMode>) -> Self {
        let smoothing_string = match stroke {
            EngineStroke::Brush(l) => l.smoothing_string(),
            _ => None,
        };
        Self { doc: gpu.map_or_else(|| stroke.doc().clone(), |g| g.doc.clone()), wants_time: stroke.wants_time(), smoothing_string }
    }
}

struct State {
    /// Points handed over that the stroke hasn't painted yet.
    pending: Vec<StrokePoint>,
    /// The stroke, while nothing is painting with it.
    stroke: Option<EngineStroke>,
    /// Steps painted since the canvas last took them, and what the last one shows.
    steps: Vec<Step>,
    shown: Shown,
    /// A step failed: later points are dropped and `shown` stays the last good step, as when the
    /// canvas stopped feeding the stroke after its error.
    failed: bool,
    /// The canvas let go of the stroke: the thread ends.
    closed: bool,
    /// The canvas wants the stroke finished and taken (`PaintThread::finish_with`): once what is
    /// handed over is painted, the thread runs it and ends.
    finish: Option<Finish>,
    /// GPU strokes: the work of the steps in `steps`, not yet taken by the canvas.
    gpu_work: Vec<GpuWork>,
    /// GPU strokes at the release: the work left, handed to the canvas, until it takes it.
    gpu_final: Option<Vec<GpuWork>>,
    /// GPU strokes at the release: whether the canvas did the work it was handed.
    gpu_served: Option<bool>,
    /// GPU strokes at the release: the canvas let go of the stroke without doing it.
    gpu_abort: bool,
    /// Tests: the thread holds a step it took until this is cleared (see `PaintThread::hold`).
    #[cfg(test)]
    held: bool,
    /// Tests: the next step panics (see `PaintThread::panic_next_step`).
    #[cfg(test)]
    panic_next: bool,
}

/// What the thread does once it has painted everything: the points that came with the release, then
/// the canvas's function on the steps and the stroke.
struct Finish {
    rest: Vec<StrokePoint>,
    job: Box<dyn FnOnce(Vec<Step>, Option<EngineStroke>) + Send>,
    /// Wakes the canvas: the result is there, or a GPU stroke has handed it work.
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl State {
    fn idle(&self) -> bool {
        self.stroke.is_some() && self.pending.is_empty()
    }
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    gpu: Option<GpuMode>,
}

/// Paint `pts` on `stroke`. A GPU stroke makes the dabs (its step is the rectangle they reach) and
/// the work for the canvas: stamp them, then ask for the tiles that went calm.
fn paint_step(stroke: &mut EngineStroke, pts: &[StrokePoint]) -> (Step, Vec<GpuWork>) {
    match stroke {
        EngineStroke::Brush(live) if live.is_gpu() => gpu_step(live, pts),
        _ => (stroke.push(pts), Vec::new()),
    }
}

fn gpu_step(live: &mut photocraft_engine::brush_cmds::LiveStroke, pts: &[StrokePoint]) -> (Step, Vec<GpuWork>) {
    let dabs = match live.push_dabs(pts) {
        Ok(dabs) => dabs.iter().map(gpu_dab).collect::<Vec<GpuDab>>(),
        Err(e) => return (Err(e), Vec::new()),
    };
    let damage = dabs.iter().fold(Rect::EMPTY, |r, d| r.union(&d.rect)).intersect(&live.doc.bounds());
    // After this push's dabs: the request is queued behind their stamps.
    let (keys, epoch) = live.calm_keys(CALM_PUSHES);
    let mut work = vec![GpuWork::Dabs(dabs)];
    if !keys.is_empty() {
        work.push(GpuWork::Read(keys, epoch));
    }
    (Ok(damage), work)
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Paint the pending points, if the stroke is free.
    fn paint_pending(&self) {
        let (pts, stroke) = {
            let mut st = self.lock();
            if st.pending.is_empty() {
                return;
            }
            let Some(stroke) = st.stroke.take() else { return };
            (std::mem::take(&mut st.pending), stroke)
        };
        #[cfg(test)]
        self.test_hooks();
        let mut stroke = stroke;
        let (step, work) = paint_step(&mut stroke, &pts);
        let mut st = self.lock();
        if step.is_ok() {
            st.shown = Shown::of(&stroke, self.gpu.as_ref());
        } else {
            st.failed = true;
            st.pending.clear();
        }
        st.steps.push(step);
        st.gpu_work.extend(work);
        st.stroke = Some(stroke);
        drop(st);
        self.changed.notify_all();
    }

    /// Tests: wait while the step is held (the stroke is busy with it), then panic if asked to.
    #[cfg(test)]
    fn test_hooks(&self) {
        let mut st = self.lock();
        while st.held && !st.closed {
            st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
        let panic = std::mem::take(&mut st.panic_next);
        drop(st);
        assert!(!panic, "a step panicked on the live stroke's thread (test)");
    }

    /// Paint the release's points as one more step, take the stroke and the steps the canvas hasn't
    /// taken, and run the finishing job on them. `false` if there is nothing to finish yet.
    fn run_finish(&self) -> bool {
        let finish = {
            let mut st = self.lock();
            if !st.idle() {
                return false;
            }
            match st.finish.take() {
                Some(f) => f,
                None => return false,
            }
        };
        #[cfg(test)]
        self.test_hooks();
        let (mut steps, stroke, failed, mut work) = {
            let mut st = self.lock();
            (std::mem::take(&mut st.steps), st.stroke.take(), st.failed, std::mem::take(&mut st.gpu_work))
        };
        let stroke = stroke.map(|mut stroke| {
            if !failed && !finish.rest.is_empty() {
                let (step, more) = paint_step(&mut stroke, &finish.rest);
                steps.push(step);
                work.extend(more);
            }
            stroke
        });
        // A stroke that failed is rendered again by its command: nothing to read back for it.
        let stroke = match stroke {
            Some(stroke) if !failed && steps.iter().all(Result::is_ok) => Some(self.finish_gpu(stroke, work, finish.wake.as_ref())),
            other => other,
        };
        (finish.job)(steps, stroke);
        true
    }

    /// A GPU stroke at the release: end its dabs, hand the canvas all the work it hasn't done (the
    /// last stamps and a request for every tile still unread), wait for it, then collect the
    /// coverage of those tiles and feed it to the stroke. The commit is made from it. Stays as it
    /// is if the canvas doesn't do the work (the stroke then refuses to commit, and its command
    /// renders it instead).
    fn finish_gpu(&self, mut stroke: EngineStroke, mut work: Vec<GpuWork>, wake: &(dyn Fn() + Send + Sync)) -> EngineStroke {
        if let (Some(gpu), EngineStroke::Brush(live)) = (self.gpu.as_ref(), &mut stroke) {
            let last: Vec<GpuDab> = live.finish_dabs().iter().map(gpu_dab).collect();
            let (keys, epoch) = live.unfed_keys();
            work.push(GpuWork::Dabs(last));
            work.push(GpuWork::Read(keys, epoch));
            if self.hand_over(work, wake) {
                for _ in 0..FINAL_POLLS {
                    for (key, values, tag) in gpu.source.poll(true) {
                        live.feed_coverage(key, &values, tag);
                    }
                    if gpu.source.in_flight() == 0 {
                        break;
                    }
                }
            }
        }
        stroke
    }

    /// Give the canvas `work` and wait until it has done it. `false` if it said it couldn't, let
    /// go of the stroke, or didn't answer in `HAND_OVER_WAIT`.
    fn hand_over(&self, work: Vec<GpuWork>, wake: &(dyn Fn() + Send + Sync)) -> bool {
        self.lock().gpu_final = Some(work);
        self.changed.notify_all();
        wake();
        let deadline = Instant::now().checked_add(HAND_OVER_WAIT);
        let mut st = self.lock();
        let served = loop {
            if let Some(ok) = st.gpu_served {
                break ok;
            }
            let left = deadline.map_or(Duration::MAX, |d| d.saturating_duration_since(Instant::now()));
            if st.gpu_abort || st.closed || left.is_zero() {
                break false;
            }
            // Wake now and then to notice a canvas that went away.
            let (guard, _) = self.changed.wait_timeout(st, left.min(Duration::from_millis(50))).unwrap_or_else(PoisonError::into_inner);
            st = guard;
        };
        st.gpu_final = None;
        served
    }

    /// Is a GPU stroke's read-back in flight?
    fn reading(&self) -> bool {
        self.gpu.as_ref().is_some_and(|g| g.source.in_flight() > 0)
    }

    /// Feed a GPU stroke the coverage tiles that arrived. Runs on the thread, between its steps, so
    /// the stroke is here to take them.
    fn feed(&self) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        if !self.reading() {
            return;
        }
        let arrived = gpu.source.poll(false);
        if arrived.is_empty() {
            return;
        }
        if let Some(EngineStroke::Brush(live)) = self.lock().stroke.as_mut() {
            for (key, values, tag) in arrived {
                live.feed_coverage(key, &values, tag);
            }
        }
    }

    fn run(&self) {
        loop {
            {
                let mut st = self.lock();
                while st.pending.is_empty() && st.finish.is_none() && !st.closed {
                    if self.reading() {
                        // No points to wake the thread: look at the read-back again soon.
                        st = self.changed.wait_timeout(st, READBACK_POLL).unwrap_or_else(PoisonError::into_inner).0;
                        break;
                    }
                    st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
                }
                if st.closed {
                    return;
                }
            }
            self.feed();
            self.paint_pending();
            if self.run_finish() {
                return;
            }
        }
    }
}

/// A live stroke painting on its own thread (or inline, where there are no threads: the web).
pub(super) struct PaintThread {
    shared: Arc<Shared>,
    /// `None`: painting inline, in `push`.
    thread: Option<JoinHandle<()>>,
    shown: Shown,
    /// GPU strokes: work the canvas took with its steps and hasn't done yet (`take_gpu_work`).
    gpu_taken: Vec<GpuWork>,
    /// `finish_with` handed the stroke on: dropping this leaves the thread to finish it.
    finishing: bool,
}

/// What a [`Finishing`] has to give.
pub(super) enum Wait<R> {
    Ready(R),
    /// Not done yet.
    Pending,
    /// The thread stopped without a result (a panic), or the result was already taken.
    Lost,
}

struct Slot<R> {
    value: Mutex<Option<R>>,
    ready: Condvar,
}

/// A stroke finishing on its thread (`PaintThread::finish_with`). Dropping it doesn't wait.
pub(super) struct Finishing<R> {
    slot: Arc<Slot<R>>,
    /// `None`: finished inline.
    thread: Option<JoinHandle<()>>,
    /// A GPU stroke: its thread hands the canvas work at the release (`take_gpu_work`).
    gpu: Option<Arc<Shared>>,
}

impl<R> Drop for Finishing<R> {
    fn drop(&mut self) {
        // The canvas let go of a GPU stroke: its thread doesn't wait for work nobody will do.
        if let Some(shared) = &self.gpu {
            shared.lock().gpu_abort = true;
            shared.changed.notify_all();
        }
    }
}

impl<R> Finishing<R> {
    /// A GPU stroke at the release: the work its thread handed over (the last stamps and the
    /// requests for the coverage tiles still unread), once it has, to be done in order on the UI
    /// thread and answered with `gpu_served`.
    pub(super) fn take_gpu_work(&self) -> Option<Vec<GpuWork>> {
        self.gpu.as_ref()?.lock().gpu_final.take()
    }

    /// Tell a GPU stroke's thread whether the canvas did the work `take_gpu_work` gave it.
    pub(super) fn gpu_served(&self, ok: bool) {
        if let Some(shared) = &self.gpu {
            shared.lock().gpu_served = Some(ok);
            shared.changed.notify_all();
        }
    }

    /// The result if it is there, without waiting.
    pub(super) fn poll(&mut self) -> Wait<R> {
        self.wait(Duration::ZERO)
    }

    /// Wait up to `budget` for the result.
    pub(super) fn wait(&mut self, budget: Duration) -> Wait<R> {
        let deadline = Instant::now().checked_add(budget);
        let mut value = self.slot.value.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            // The thread stores the result before it ends: look at whether it ended first.
            let ended = self.thread.as_ref().is_none_or(JoinHandle::is_finished);
            if let Some(r) = value.take() {
                return Wait::Ready(r);
            }
            if ended {
                return Wait::Lost;
            }
            let left = deadline.map_or(Duration::MAX, |d| d.saturating_duration_since(Instant::now()));
            if left.is_zero() {
                return Wait::Pending;
            }
            // Wake now and then to notice a thread that stopped without a word.
            let (guard, _) = self.slot.ready.wait_timeout(value, left.min(Duration::from_millis(50))).unwrap_or_else(PoisonError::into_inner);
            value = guard;
        }
    }

    /// Wait for the result; `None` if the thread stopped without one.
    pub(super) fn wait_forever(&mut self) -> Option<R> {
        match self.wait(Duration::MAX) {
            Wait::Ready(r) => Some(r),
            Wait::Pending | Wait::Lost => None,
        }
    }
}

impl PaintThread {
    pub(super) fn start(stroke: EngineStroke) -> Self {
        Self::spawn(stroke, None)
    }

    /// Start a stroke whose dabs the GPU draws (`gpu_dabs`): `stroke` is a `LiveStroke::begin_gpu`
    /// one, whose first dabs the canvas has stamped. `None` without a thread of its own: the
    /// release hands the thread's work to the canvas and waits for it, which inline painting
    /// can't.
    pub(super) fn start_gpu(stroke: EngineStroke, gpu: GpuMode) -> Option<Self> {
        let thread = Self::spawn(stroke, Some(gpu));
        thread.thread.is_some().then_some(thread)
    }

    fn spawn(stroke: EngineStroke, gpu: Option<GpuMode>) -> Self {
        let shown = Shown::of(&stroke, gpu.as_ref());
        let state = State {
            pending: Vec::new(),
            stroke: Some(stroke),
            steps: Vec::new(),
            shown: shown.clone(),
            failed: false,
            closed: false,
            finish: None,
            gpu_work: Vec::new(),
            gpu_final: None,
            gpu_served: None,
            gpu_abort: false,
            #[cfg(test)]
            held: false,
            #[cfg(test)]
            panic_next: false,
        };
        let shared = Arc::new(Shared { state: Mutex::new(state), changed: Condvar::new(), gpu });
        let thread = if cfg!(target_arch = "wasm32") {
            None
        } else {
            let worker = shared.clone();
            // Without a thread (the system refused one) the stroke paints inline, as it used to.
            std::thread::Builder::new().name("live stroke".into()).spawn(move || worker.run()).ok()
        };
        Self { shared, thread, shown, gpu_taken: Vec::new(), finishing: false }
    }

    /// Hand points over to be painted (none after a failed step).
    pub(super) fn push(&mut self, pts: Vec<StrokePoint>) {
        if pts.is_empty() {
            return;
        }
        {
            let mut st = self.shared.lock();
            if st.failed {
                return;
            }
            st.pending.extend(pts);
        }
        if self.thread.is_some() {
            self.shared.changed.notify_all();
        } else {
            self.shared.paint_pending();
        }
    }

    /// Wait up to `wait` for everything handed over to be painted; returns the steps painted since
    /// the last call, oldest first, and updates what `shown` shows.
    pub(super) fn catch_up(&mut self, wait: Duration) -> Vec<Step> {
        let (shown, steps, work) = {
            let mut st = self.wait_idle(wait);
            (st.shown.clone(), std::mem::take(&mut st.steps), std::mem::take(&mut st.gpu_work))
        };
        self.shown = shown;
        // Together with the steps, so the canvas shows no step before it has done its work.
        self.gpu_taken.extend(work);
        steps
    }

    /// A GPU stroke: the work of the steps `catch_up` returned, in the order to do it: stamp the
    /// dabs, then ask for the tiles. The canvas does all of it before it draws those steps.
    pub(super) fn take_gpu_work(&mut self) -> Vec<GpuWork> {
        std::mem::take(&mut self.gpu_taken)
    }

    /// Is the stroke still painting points it was given, or holding steps `catch_up` hasn't taken?
    /// (A step can finish just after a frame's wait ran out; the next frame must collect it.)
    pub(super) fn busy(&self) -> bool {
        let st = self.shared.lock();
        !st.steps.is_empty() || (!st.idle() && self.thread.as_ref().is_some_and(|t| !t.is_finished()))
    }

    pub(super) fn shown(&self) -> &Shown {
        &self.shown
    }

    /// Wait for everything handed over to be painted; the steps since the last `catch_up` and the
    /// stroke itself (`None` if its thread stopped mid-step).
    pub(super) fn finish(mut self) -> (Vec<Step>, Option<EngineStroke>) {
        let steps = self.catch_up(Duration::MAX);
        let stroke = self.shared.lock().stroke.take();
        (steps, stroke)
    }

    /// Hand the stroke over to finish on its thread, without waiting: it paints what it was given,
    /// then `rest` as one more step (nothing after a failed step), and runs `f` on the steps the
    /// canvas hasn't taken and the stroke (`None` if the thread stopped mid-step). `wake` is called
    /// once the result is there. Inline (no thread) it all happens before this returns.
    pub(super) fn finish_with<R: Send + 'static>(
        mut self,
        rest: Vec<StrokePoint>,
        f: impl FnOnce(Vec<Step>, Option<EngineStroke>) -> R + Send + 'static,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Finishing<R> {
        let slot = Arc::new(Slot { value: Mutex::new(None), ready: Condvar::new() });
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let job = {
            let (slot, wake) = (slot.clone(), wake.clone());
            Box::new(move |steps, stroke| {
                let r = f(steps, stroke);
                *slot.value.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
                slot.ready.notify_all();
                wake();
            })
        };
        self.shared.lock().finish = Some(Finish { rest, job, wake });
        self.finishing = true;
        let thread = self.thread.take();
        if thread.is_some() {
            self.shared.changed.notify_all();
        } else {
            self.shared.paint_pending();
            self.shared.run_finish();
        }
        let gpu = self.shared.gpu.as_ref().map(|_| self.shared.clone());
        Finishing { slot, thread, gpu }
    }

    /// Tests: from now on the thread holds the next step it takes, as a slow brush would, until
    /// the returned function lets it go (from any thread).
    #[cfg(test)]
    pub(super) fn hold(&self) -> impl FnOnce() + Send + 'static + use<> {
        self.shared.lock().held = true;
        let shared = self.shared.clone();
        move || {
            shared.lock().held = false;
            shared.changed.notify_all();
        }
    }

    /// Tests: the next step panics on the thread, which then stops with the stroke.
    #[cfg(test)]
    pub(super) fn panic_next_step(&self) {
        self.shared.lock().panic_next = true;
    }

    /// Tests: is the thread still running? (It keeps the shared state alive until it ends.)
    #[cfg(test)]
    pub(super) fn thread_alive(&self) -> impl Fn() -> bool + use<> {
        let weak = Arc::downgrade(&self.shared);
        move || weak.strong_count() > 0
    }

    /// Look at the stroke once it has painted everything handed over.
    #[cfg(test)]
    pub(super) fn peek<R>(&self, f: impl FnOnce(&EngineStroke) -> R) -> Option<R> {
        self.wait_idle(Duration::MAX).stroke.as_ref().map(f)
    }

    fn wait_idle(&self, wait: Duration) -> MutexGuard<'_, State> {
        let deadline = Instant::now().checked_add(wait);
        let mut st = self.shared.lock();
        // A stopped thread (inline painting never waits) has nothing more to give.
        while !st.idle() && self.thread.as_ref().is_some_and(|t| !t.is_finished()) {
            let left = deadline.map_or(Duration::MAX, |d| d.saturating_duration_since(Instant::now()));
            if left.is_zero() {
                break;
            }
            // Wake now and then to notice a thread that stopped without a word.
            let (guard, _) = self.shared.changed.wait_timeout(st, left.min(Duration::from_millis(50))).unwrap_or_else(PoisonError::into_inner);
            st = guard;
        }
        st
    }
}

impl Drop for PaintThread {
    fn drop(&mut self) {
        // The thread finishes the step it is on, if any, and ends; nothing waits for it. One handed
        // over to finish (`finish_with`) is left to do that.
        if self.finishing {
            return;
        }
        self.shared.lock().closed = true;
        self.shared.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;
    use photocraft_engine::brush_cmds::LiveStroke;
    use serde_json::json;

    #[test]
    fn a_stroke_painted_on_the_thread_in_any_batches_equals_one_painted_inline() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 120, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "paint"})).unwrap();
        let begin = || {
            let p = json!({"points": [[20, 60, 1.0]], "brush": {"size": 40, "hardness": 0.3, "smoothing": {"amount": 0.4}}, "seed": 3});
            EngineStroke::Brush(Box::new(LiveStroke::begin(&s, &p).unwrap()))
        };
        let pts: Vec<StrokePoint> = (1..=60).map(|i| StrokePoint::new(20.0 + 6.0 * f64::from(i), 60.0 + 20.0 * (f64::from(i) / 9.0).sin(), 1.0)).collect();

        // Never waiting, so the thread merges points that arrive while it paints into one step.
        let mut thread = PaintThread::start(begin());
        assert!(thread.thread.is_some(), "a thread of its own");
        let mut steps = Vec::new();
        for chunk in pts.chunks(3) {
            thread.push(chunk.to_vec());
            steps.extend(thread.catch_up(Duration::ZERO));
        }
        let (rest, painted) = thread.finish();
        steps.extend(rest);
        assert!(!steps.is_empty() && steps.iter().all(Result::is_ok), "every step painted");
        let painted = painted.expect("the stroke comes back");

        let mut inline = begin();
        inline.push(&pts).unwrap();
        let layer = s.active().unwrap().active_layer.unwrap();
        let surface = |e: &EngineStroke| e.doc().layer(layer).unwrap().surface().unwrap().clone();
        let (a, b) = (surface(&painted), surface(&inline));
        let differ = (0..120).flat_map(|y| (0..400).map(move |x| (x, y))).filter(|&(x, y)| a.rgba(x, y) != b.rgba(x, y)).count();
        assert_eq!(differ, 0, "pixels differ from the inline stroke");
        assert!(b.rgba(200, 60 + (20.0 * (30.0f64 / 9.0).sin()) as i32)[3] > 0.5, "the stroke is there");
    }

    #[test]
    fn a_painted_step_keeps_the_stroke_busy_until_the_canvas_takes_it() {
        // The step finishes after the frame stopped waiting: the canvas must ask for another frame.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
        let p = json!({"points": [[20, 50, 1.0]], "brush": {"size": 20}});
        let mut thread = PaintThread::start(EngineStroke::Brush(Box::new(LiveStroke::begin(&s, &p).unwrap())));
        thread.push(vec![StrokePoint::new(120.0, 50.0, 1.0)]);
        thread.peek(|_| ());
        assert!(thread.busy(), "a painted step nobody took yet");
        assert_eq!(thread.catch_up(Duration::ZERO).len(), 1);
        assert!(!thread.busy());
    }

    #[test]
    fn after_a_failed_step_the_stroke_paints_nothing_more_and_shows_its_last_good_step() {
        // The Mixer Brush keeps its last good preview after an error (`take_steps`); points queued
        // behind the failed batch must not paint over it.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "paint"})).unwrap();
        let p = json!({"points": [[30, 50]], "size": 16});
        let mixer = photocraft_engine::brush_cmds::mixer_live::LiveMixer::begin(&s, &p).unwrap();
        let mut thread = PaintThread::start(EngineStroke::Mixer(Box::new(mixer)));
        let good = thread.shown().doc.clone();
        // More samples than a live Mixer stroke takes, then one it would take.
        thread.push(vec![StrokePoint::new(40.0, 50.0, 1.0); 70_000]);
        thread.peek(|_| ());
        thread.push(vec![StrokePoint::new(150.0, 50.0, 1.0)]);
        let steps = thread.catch_up(Duration::MAX);
        assert!(matches!(steps.as_slice(), [Err(_)]), "only the failed step");
        assert!(Arc::ptr_eq(&thread.shown().doc, &good), "the last good step stays shown");
        assert!(!thread.busy());
    }

    #[test]
    fn finishing_on_the_thread_gives_the_commit_of_finishing_here() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 300, "height": 100, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "paint"})).unwrap();
        let begin = || {
            let p = json!({"points": [[20, 50, 1.0]], "brush": {"size": 24, "hardness": 0.4}, "seed": 5});
            EngineStroke::Brush(Box::new(LiveStroke::begin(&s, &p).unwrap()))
        };
        let pts: Vec<StrokePoint> = (1..=40).map(|i| StrokePoint::new(20.0 + 6.0 * f64::from(i), 50.0 + 15.0 * (f64::from(i) / 7.0).sin(), 1.0)).collect();
        let (head, rest) = pts.split_at(30);

        let mut here = PaintThread::start(begin());
        here.push(pts.clone());
        let Some(EngineStroke::Brush(painted)) = here.finish().1 else { panic!("the stroke comes back") };
        let expected = painted.into_commit().unwrap();

        let woken = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = woken.clone();
        let mut thread = PaintThread::start(begin());
        thread.push(head.to_vec());
        let mut finishing = thread.finish_with(
            rest.to_vec(),
            |steps, stroke| match stroke {
                Some(EngineStroke::Brush(b)) => Some((steps.len(), b.into_commit().unwrap())),
                _ => None,
            },
            move || flag.store(true, std::sync::atomic::Ordering::SeqCst),
        );
        let (steps, got) = finishing.wait_forever().flatten().expect("the thread's result");
        assert!(steps >= 1, "the release's points are a step of their own");
        // The wake comes right after the result is handed over, so it can trail this wait a moment.
        let start = Instant::now();
        while !woken.load(std::sync::atomic::Ordering::SeqCst) {
            assert!(start.elapsed() < Duration::from_secs(10), "the canvas is woken once the result is there");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(got.bounds, expected.bounds);
        let r = expected.bounds;
        let differ =
            (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).map(move |x| (x, y))).filter(|&(x, y)| got.surface.rgba(x, y) != expected.surface.rgba(x, y)).count();
        assert_eq!(differ, 0, "pixels differ from the commit made here");
    }
    /// Coverage that never comes: a GPU stroke without a GPU.
    struct NoCoverage;

    impl CoverageSource for NoCoverage {
        fn poll(&self, _block: bool) -> Vec<Coverage> {
            Vec::new()
        }

        fn in_flight(&self) -> usize {
            0
        }
    }

    /// A GPU stroke's thread on a document: the stroke, the document it shows and the first dabs.
    fn gpu_thread() -> (PaintThread, Arc<photocraft_doc::Document>) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 120, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "paint"})).unwrap();
        let p = json!({"brush": {"size": 12, "hardness": 0.3, "flow": 0.6}, "color": "#d02060", "opacity": 0.8, "smoothing": 0.0, "target": "pixels", "points": [[20, 60, 1.0]]});
        let (live, _first) = LiveStroke::begin_gpu(&s, "paint.stroke", &p).unwrap();
        let doc = live.gpu_preview().unwrap().doc;
        let mode = GpuMode { doc: doc.clone(), source: Arc::new(NoCoverage) };
        (PaintThread::start_gpu(EngineStroke::Brush(Box::new(live)), mode).expect("a thread of its own"), doc)
    }

    fn move_to(i: u32) -> Vec<StrokePoint> {
        vec![StrokePoint::new(20.0 + 8.0 * f64::from(i), 60.0 + 20.0 * (f64::from(i) / 5.0).sin(), 1.0)]
    }

    #[test]
    fn a_gpu_strokes_work_comes_out_in_order_and_each_request_follows_the_dabs_of_its_epoch() {
        let (mut thread, doc) = gpu_thread();
        assert!(Arc::ptr_eq(&thread.shown().doc, &doc), "the canvas shows the document whose mask the GPU holds");
        let (mut steps, mut work) = (Vec::new(), Vec::new());
        for i in 1..=40 {
            thread.push(move_to(i));
            steps.extend(thread.catch_up(Duration::MAX));
            work.extend(thread.take_gpu_work());
            assert!(Arc::ptr_eq(&thread.shown().doc, &doc), "every step shows that document");
        }
        assert!(steps.len() == 40 && steps.iter().all(Result::is_ok), "a step per push");
        let (mut dabs_items, mut reads, mut offset) = (0i64, 0usize, None);
        for item in &work {
            match item {
                GpuWork::Dabs(_) => dabs_items += 1,
                GpuWork::Read(keys, epoch) => {
                    assert!(!keys.is_empty(), "no empty requests");
                    // The epoch is the push count: the request was made right after the dabs of
                    // the push it names, so exactly as many `Dabs` come before it.
                    let o = *epoch as i64 - dabs_items;
                    assert_eq!(*offset.get_or_insert(o), o, "epochs follow the dabs they were asked after");
                    reads += 1;
                }
            }
        }
        assert_eq!(dabs_items, 40, "the dabs of every step come out, step by step");
        assert!(reads > 0, "tiles that went calm are asked for during the stroke");
        assert!(thread.take_gpu_work().is_empty(), "work comes out once");
    }

    #[test]
    fn a_gpu_stroke_waits_at_the_release_for_the_canvas_to_do_its_work_and_then_commits() {
        for answer in [true, false] {
            let (mut thread, _doc) = gpu_thread();
            for i in 1..=20 {
                thread.push(move_to(i));
            }
            let mut finishing = thread.finish_with(move_to(21), |steps, stroke| (steps.len(), stroke.is_some()), || {});
            let start = Instant::now();
            let work = loop {
                if let Some(work) = finishing.take_gpu_work() {
                    break work;
                }
                assert!(start.elapsed() < Duration::from_secs(10), "the thread hands the canvas its work at the release");
                std::thread::sleep(Duration::from_millis(1));
            };
            // The release's own dabs, then the request for every tile not yet read back.
            let [.., GpuWork::Dabs(_), GpuWork::Read(keys, _)] = work.as_slice() else { panic!("the work ends with the last dabs and a request") };
            assert!(!keys.is_empty());
            let (steps, kept) = {
                assert!(matches!(finishing.poll(), Wait::Pending), "no result before the canvas answered");
                finishing.gpu_served(answer);
                finishing.wait_forever().expect("the result follows the canvas's answer")
            };
            assert!(steps >= 1 && kept, "answer {answer}: the stroke comes back");
        }
    }

    #[test]
    fn a_gpu_stroke_whose_canvas_lets_go_at_the_release_ends_its_thread() {
        let (thread, _doc) = gpu_thread();
        let alive = thread.thread_alive();
        let finishing = thread.finish_with(Vec::new(), |_, _| (), || {});
        let start = Instant::now();
        while finishing.take_gpu_work().is_none() {
            assert!(start.elapsed() < Duration::from_secs(10), "the thread hands the canvas its work");
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(finishing);
        while alive() {
            assert!(start.elapsed() < Duration::from_secs(10), "the thread doesn't wait for a canvas that is gone");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
