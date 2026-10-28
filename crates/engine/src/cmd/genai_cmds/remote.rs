//! Running a planned fill: one job per variation, all submitted side by side and polled together
//! from the fill's background job, with Cancel looked at between polls. A job the service turns
//! down for its rate limit is started again after a pause; the others carry on.

use crate::genai::{AiErrorKind, AiFailure, FillJob, FillPlan, JobPoll};
use crate::jobs::JobCtx;
use crate::{EngineError, Result};

/// Pauses before starting a job again after the service said it is over its rate limit, in
/// milliseconds, when it didn't say how long to wait. Then the error stands.
#[cfg(not(test))]
const BUSY_PAUSES_MS: [u64; 3] = [4_000, 8_000, 16_000];
#[cfg(test)]
const BUSY_PAUSES_MS: [u64; 3] = [5, 10, 20];
/// Longest wait the service may ask for before a retry, in seconds.
const MAX_RETRY_AFTER_S: u64 = 60;
/// How often running jobs are polled, in milliseconds.
#[cfg(not(test))]
const POLL_MS: u64 = 250;
#[cfg(test)]
const POLL_MS: u64 = 2;
/// The longest a fill waits for its jobs before giving up, in seconds.
const TIMEOUT_S: u64 = 15 * 60;

/// Why running the jobs stopped without a result.
#[derive(Debug)]
pub(super) enum Stopped {
    /// The person cancelled it.
    Cancelled,
    /// Every variation failed: the first failure.
    Failed(AiFailure),
}

impl From<Stopped> for EngineError {
    fn from(s: Stopped) -> EngineError {
        match s {
            Stopped::Cancelled => EngineError::Cancelled,
            Stopped::Failed(f) => EngineError::Other(f.message),
        }
    }
}

/// One variation's job.
struct Slot {
    job: Option<Box<dyn FillJob>>,
    /// Starts so far.
    starts: usize,
    /// Start again (after a rate limit) once this many polls have passed.
    wait_polls: u64,
    percent: u8,
    done: bool,
}

/// How long to wait before trying a rate-limited job again (in polls), or `None` once it has been
/// tried enough.
fn retry_wait(f: &AiFailure, starts: usize) -> Option<u64> {
    if f.kind != AiErrorKind::RateLimited {
        return None;
    }
    let pause = BUSY_PAUSES_MS.get(starts.saturating_sub(1))?;
    let ms = f.retry_after_secs.map_or(*pause, |s| s.min(MAX_RETRY_AFTER_S).saturating_mul(1000).max(*pause));
    Some(ms.div_ceil(POLL_MS).max(1))
}

/// Run `count` jobs of `plan` side by side and return the pictures they made (the first of each).
/// Variations that fail are dropped as long as one succeeds.
pub(super) fn run_jobs(ctx: &JobCtx, plan: &dyn FillPlan, count: usize) -> std::result::Result<Vec<Vec<u8>>, Stopped> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (ctx, plan, count);
        Err(Stopped::Failed(AiFailure::new(AiErrorKind::Unsupported, "Generative Fill needs the desktop app")))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        run_native(ctx, plan, count)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_native(ctx: &JobCtx, plan: &dyn FillPlan, count: usize) -> std::result::Result<Vec<Vec<u8>>, Stopped> {
    let model = plan.disclosure().model.clone();
    let waiting = if count == 1 { format!("Waiting for {model}") } else { format!("Waiting for {model} ({count} variations)") };
    ctx.progress(0.15, &waiting);
    let mut slots: Vec<Slot> = (0..count.max(1)).map(|_| Slot { job: None, starts: 0, wait_polls: 0, percent: 0, done: false }).collect();
    let mut images = Vec::new();
    let mut first_error: Option<AiFailure> = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(TIMEOUT_S);
    loop {
        if ctx.cancelled() {
            cancel_all(&mut slots);
            return Err(Stopped::Cancelled);
        }
        if std::time::Instant::now() >= deadline {
            cancel_all(&mut slots);
            return Err(Stopped::Failed(AiFailure::new(AiErrorKind::Network, "the AI service took too long to answer")));
        }
        for slot in slots.iter_mut().filter(|s| !s.done) {
            if slot.job.is_none() {
                if slot.wait_polls > 0 {
                    slot.wait_polls -= 1;
                    continue;
                }
                slot.starts += 1;
                match plan.start() {
                    Ok(job) => slot.job = Some(job),
                    Err(f) => settle(slot, f, &mut first_error, ctx),
                }
                continue;
            }
            let polled = slot.job.as_mut().map(|j| j.poll());
            match polled {
                Some(Ok(JobPoll::Working(p))) => slot.percent = p.unwrap_or(slot.percent).min(100),
                Some(Ok(JobPoll::Done(found))) => {
                    slot.done = true;
                    slot.job = None;
                    match found.into_iter().next() {
                        Some(img) => images.push(img),
                        None => {
                            first_error.get_or_insert_with(|| AiFailure::new(AiErrorKind::Provider, "the AI service returned no picture"));
                        }
                    }
                }
                Some(Ok(JobPoll::Cancelled)) => {
                    slot.done = true;
                    slot.job = None;
                }
                Some(Ok(JobPoll::Failed(f)) | Err(f)) => {
                    slot.job = None;
                    settle(slot, f, &mut first_error, ctx);
                }
                None => {}
            }
        }
        if slots.iter().all(|s| s.done) {
            break;
        }
        let mean = slots.iter().map(|s| if s.done { 100.0 } else { f32::from(s.percent) }).sum::<f32>() / slots.len() as f32;
        ctx.progress(0.15 + 0.7 * mean / 100.0, &waiting);
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
    }
    match (images.is_empty(), first_error) {
        (true, Some(e)) => Err(Stopped::Failed(e)),
        (true, None) => Err(Stopped::Cancelled),
        _ => Ok(images),
    }
}

/// A job that couldn't start or failed: wait and start it again when the service was busy,
/// else it is done and its failure kept (the first one is what the person sees).
#[cfg(not(target_arch = "wasm32"))]
fn settle(slot: &mut Slot, f: AiFailure, first_error: &mut Option<AiFailure>, ctx: &JobCtx) {
    match retry_wait(&f, slot.starts) {
        Some(polls) => {
            ctx.progress(0.15, "The AI service is busy; trying again");
            slot.wait_polls = polls;
        }
        None => {
            slot.done = true;
            first_error.get_or_insert(f);
        }
    }
}

/// Ask every running job to stop. One the service can't stop runs on; its result is dropped.
#[cfg(not(target_arch = "wasm32"))]
fn cancel_all(slots: &mut [Slot]) {
    for slot in slots {
        if let Some(job) = slot.job.as_mut() {
            let _ = job.cancel();
        }
        slot.job = None;
    }
}

/// `Result` for code that turns a [`Stopped`] into an engine error.
pub(super) fn into_result<T>(r: std::result::Result<T, Stopped>) -> Result<T> {
    r.map_err(EngineError::from)
}
