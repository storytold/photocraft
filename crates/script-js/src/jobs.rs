//! The job queue of one attempt: promise reactions run before the attempt reports `Done`.
//!
//! Jobs run in FIFO order after the script body, in the attempt's own context, so a replay runs
//! the same jobs in the same order and meets the same taped calls. Every job is charged to the
//! budget through the same checkpoint as a host call, so an endless `.then` chain ends with the
//! time-limit error, and a pause or stop inside a job unwinds the whole attempt like anywhere else.
//!
//! Jobs that depend on timers or outside events (dynamic `import()`, `Atomics.waitAsync`) cannot
//! be replayed deterministically; queuing one fails the run instead of dropping its effects.
//! Finalization-registry cleanup depends on garbage collection timing, so it never runs, which
//! the language allows.
//!
//! At most [`MAX_QUEUED`] jobs wait at a time. Queuing more fails the run with a named error;
//! the extra jobs are dropped, never run, so the queue can't grow without bound between host
//! calls.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use boa_engine::job::{GenericJob, Job, JobExecutor, PromiseJob};
use boa_engine::{Context, JsResult};

use crate::runner::Stop;
use crate::{abort, natives};

/// Jobs that may wait at once.
pub const MAX_QUEUED: usize = 100_000;

fn overflow_message() -> String {
    format!("The script was stopped: it queued more than {MAX_QUEUED} promise reactions at once.")
}

enum Queued {
    Promise(PromiseJob),
    Generic(GenericJob),
}

#[derive(Default)]
pub(crate) struct Jobs {
    queue: RefCell<VecDeque<Queued>>,
    unsupported: Cell<bool>,
    overflowed: Cell<bool>,
}

impl Jobs {
    /// Drop the queue and stop the run: the next checkpoint, or the end of the attempt, reports
    /// the overflow.
    fn overflow(&self, ctx: &mut Context) {
        self.overflowed.set(true);
        self.queue.borrow_mut().clear();
        if let Some(shared) = ctx.get_data::<crate::runner::Shared>().cloned()
            && let Ok(mut st) = shared.try_borrow_mut()
            && st.stop.is_none()
        {
            st.stop = Some(Stop::Fault(overflow_message()));
        }
    }

    /// A job was queued that the run cannot perform.
    pub fn unsupported(&self) -> bool {
        self.unsupported.get()
    }
}

impl JobExecutor for Jobs {
    fn enqueue_job(self: Rc<Self>, job: Job, ctx: &mut Context) {
        if matches!(job, Job::PromiseJob(_) | Job::GenericJob(_)) && self.queue.borrow().len() >= MAX_QUEUED {
            self.overflow(ctx);
            return;
        }
        match job {
            Job::PromiseJob(j) => self.queue.borrow_mut().push_back(Queued::Promise(j)),
            Job::GenericJob(j) => self.queue.borrow_mut().push_back(Queued::Generic(j)),
            Job::FinalizationRegistryCleanupJob(_) => {}
            _ => self.unsupported.set(true),
        }
    }

    fn run_jobs(self: Rc<Self>, ctx: &mut Context) -> JsResult<()> {
        loop {
            if self.overflowed.get() {
                self.overflow(ctx);
                return Err(abort::uncatchable());
            }
            if self.unsupported.get() {
                return Ok(());
            }
            let Some(job) = self.queue.borrow_mut().pop_front() else { return Ok(()) };
            let result = natives::checkpoint(ctx).and_then(|()| match job {
                Queued::Promise(j) => j.call(ctx),
                Queued::Generic(j) => j.call(ctx),
            });
            if let Err(e) = result {
                self.queue.borrow_mut().clear();
                return Err(e);
            }
        }
    }
}
