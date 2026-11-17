//! Reads of the OS clipboard made on a worker thread and waited for a bounded time. Reading the
//! clipboard waits on the app that owns it: arboard's Wayland data-control read waits for that app
//! to send its data with no time limit, and its X11 read waits up to 4 s for each format it tries.
//! On Linux and the BSDs every paste read goes through a [`Loads`] worker with a clipboard handle of
//! its own, opened on the worker at its first read ([`opened_on_worker`]), so an owner or a display
//! server that never answers holds the worker, not the window. Built on every platform so it is
//! tested on every platform.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The longest a text or file-list read waits for the clipboard's owner to send its data.
pub(super) const LOAD_WAIT: Duration = Duration::from_secs(2);
/// The longest a bitmap read waits: the transfer and the decoding of the image both happen within
/// the read.
pub(super) const IMAGE_WAIT: Duration = Duration::from_secs(10);
/// What a read says once its worker has stopped.
pub(super) const GONE: &str = "the clipboard's reader stopped";
/// What a read says while an earlier one has no answer yet.
pub(super) const BUSY: &str = "the clipboard's owner hasn't answered an earlier read";

/// A read of the OS clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Request {
    Files,
    Image,
    Text,
}

/// What a [`Request`] read: `None` when the clipboard holds nothing in that format.
pub(super) enum Answer {
    Files(Result<Option<Vec<PathBuf>>, String>),
    Image(Result<Option<(u32, u32, Vec<u8>)>, String>),
    Text(Result<Option<String>, String>),
}

/// What a read says when it gets the answer to another request.
const MIXED: &str = "the clipboard answered another read";

impl Answer {
    /// The answer to `request` when it couldn't be read.
    pub(super) fn failed(request: Request, why: String) -> Self {
        match request {
            Request::Files => Self::Files(Err(why)),
            Request::Image => Self::Image(Err(why)),
            Request::Text => Self::Text(Err(why)),
        }
    }

    pub(super) fn files(self) -> Result<Option<Vec<PathBuf>>, String> {
        match self {
            Self::Files(files) => files,
            _ => Err(MIXED.into()),
        }
    }

    pub(super) fn image(self) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        match self {
            Self::Image(image) => image,
            _ => Err(MIXED.into()),
        }
    }

    pub(super) fn text(self) -> Result<Option<String>, String> {
        match self {
            Self::Text(text) => text,
            _ => Err(MIXED.into()),
        }
    }
}

/// How long a read of `request` waits for the clipboard's owner: [`IMAGE_WAIT`] for a bitmap,
/// [`LOAD_WAIT`] for text and file lists.
pub(super) fn wait_for(request: Request) -> Duration {
    match request {
        Request::Image => IMAGE_WAIT,
        Request::Files | Request::Text => LOAD_WAIT,
    }
}

/// `request` read on `reads`' worker, waiting `wait` at most. A read with no answer in that time,
/// or one that can't be sent, is the read's error, never an empty clipboard.
pub(super) fn read_on(reads: &mut Loads<Request, Answer>, request: Request, wait: Duration) -> Answer {
    reads.get(request, wait).unwrap_or_else(|why| Answer::failed(request, why))
}

/// A worker's `load` that opens its clipboard with `open` at its first request, on the worker, and
/// then answers each request with `read`. Opening a clipboard handle talks to the display server,
/// so a server that stops answering holds the worker and the read times out. An open that fails is
/// the request's answer (`failed`), and the next request tries to open it again.
pub(super) fn opened_on_worker<C, R, A>(
    mut open: impl FnMut() -> Result<C, String>,
    mut read: impl FnMut(&mut C, R) -> A,
    failed: impl Fn(R, String) -> A,
) -> impl FnMut(R) -> A {
    let mut clipboard = None;
    move |request| {
        let clipboard = match &mut clipboard {
            Some(clipboard) => clipboard,
            slot => match open() {
                Ok(opened) => slot.insert(opened),
                Err(why) => return failed(request, why),
            },
        };
        read(clipboard, request)
    }
}

/// How long dropping [`Loads`] waits for an unanswered read, after it was sent. Clones share the
/// value, so the worker can set it once it knows which backend its clipboard opened.
#[derive(Clone)]
pub(super) struct Finish(Arc<AtomicU64>);

impl Finish {
    pub(super) fn set(&self, finish: Duration) {
        self.0.store(u64::try_from(finish.as_millis()).unwrap_or(u64::MAX), Ordering::SeqCst);
    }

    fn get(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

impl From<Duration> for Finish {
    fn from(finish: Duration) -> Self {
        let shared = Self(Arc::default());
        shared.set(finish);
        shared
    }
}

/// Requests of type `R` answered with `A` on a worker thread, one at a time.
pub(super) struct Loads<R, A> {
    /// The worker's requests, each with the channel its answer is sent on. The worker stops once
    /// this is dropped.
    requests: Option<mpsc::Sender<(R, mpsc::Sender<A>)>>,
    /// Requests sent and not answered yet, counted down by the worker: while one is open, reads
    /// fail at once rather than queue behind it.
    open: Arc<AtomicUsize>,
    /// When the last request was sent.
    sent: Option<Instant>,
    /// Disconnected once the worker has let go of what its `load` holds.
    done: mpsc::Receiver<()>,
    /// How long after it was sent dropping `Loads` waits for an unanswered read to end.
    finish: Finish,
}

/// When dropping `Loads` stops waiting for its worker: `finish` after the unanswered read
/// `pending` was sent (at once if that time has passed), and with no read unanswered
/// [`LOAD_WAIT`] from `now` at most (an idle worker ends at once).
fn stop_by(pending: Option<Instant>, now: Instant, finish: Duration) -> Instant {
    pending.map_or(now + LOAD_WAIT, |sent| sent + finish)
}

impl<R: Send + 'static, A: Send + 'static> Loads<R, A> {
    /// A worker thread named `name` that answers each request with `load`. Dropping the result
    /// waits up to `finish` after an unanswered read was sent for that read to end, as `finish`
    /// stands at the drop. `None` when the worker can't start.
    pub(super) fn spawn(name: &str, finish: impl Into<Finish>, mut load: impl FnMut(R) -> A + Send + 'static) -> Option<Self> {
        let finish = finish.into();
        let (requests, asked) = mpsc::channel::<(R, mpsc::Sender<A>)>();
        let (alive, done) = mpsc::channel::<()>();
        let open = Arc::new(AtomicUsize::new(0));
        let answered = open.clone();
        let work = move || {
            while let Ok((request, reply)) = asked.recv() {
                let Ok(answer) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load(request))) else {
                    // A read that panicked stops the worker: its reader and later ones get GONE.
                    drop(asked);
                    answered.fetch_sub(1, Ordering::SeqCst);
                    break;
                };
                // Counted down before the answer is sent, so its reader can ask again at once.
                answered.fetch_sub(1, Ordering::SeqCst);
                // A read that timed out no longer waits: its late answer is dropped.
                let _ = reply.send(answer);
            }
            // `load` and the clipboard it holds are dropped first: dropping `Loads` waits for `alive`.
            drop(load);
            drop(alive);
        };
        std::thread::Builder::new().name(name.into()).spawn(work).ok()?;
        Some(Self { requests: Some(requests), open, sent: None, done, finish })
    }

    /// `request` sent to the worker; its answer comes on the channel returned. Err ([`BUSY`]) while
    /// an earlier request is still unanswered, and ([`GONE`]) once the worker has stopped.
    pub(super) fn ask(&mut self, request: R) -> Result<mpsc::Receiver<A>, String> {
        let requests = self.requests.as_ref().ok_or(GONE)?;
        if self.open.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err(BUSY.into());
        }
        let (reply, answer) = mpsc::channel();
        if requests.send((request, reply)).is_err() {
            self.open.fetch_sub(1, Ordering::SeqCst);
            return Err(GONE.into());
        }
        self.sent = Some(Instant::now());
        Ok(answer)
    }

    /// The answer to `request`, or why there is none, after `wait` at most.
    pub(super) fn get(&mut self, request: R, wait: Duration) -> Result<A, String> {
        wait_for_answer(&self.ask(request)?, wait)
    }
}

impl<R, A> Loads<R, A> {
    /// Whether a request sent is still unanswered.
    fn waiting(&self) -> bool {
        self.open.load(Ordering::SeqCst) > 0
    }

    /// Whether the worker has let go of what its `load` holds.
    fn ended(&self) -> bool {
        matches!(self.done.try_recv(), Err(mpsc::TryRecvError::Disconnected))
    }
}

/// The most workers [`forget_unanswered`] keeps with an unanswered read. Past it, the stalled reads
/// stay in use and later reads fail with [`BUSY`] until one of them ends.
pub(super) const MAX_RETIRED: usize = 4;

/// Takes `reads` out of use while a read on it is unanswered, without waiting for that read, so the
/// next read starts a new worker. The app calls this once its own copy has replaced what the
/// clipboard held: the unanswered read is of the old contents. The stalled worker goes to `retired`
/// until it ends, and dropping `retired` before the app's own handle still leaves that handle the
/// last one on X11 ([`Loads::spawn`]'s `finish`). Retired workers that have ended are let go; while
/// [`MAX_RETIRED`] have not, `reads` stays as it is. False when `reads` was kept with its read
/// unanswered: the app calls this again before its next read, until it is true.
pub(super) fn forget_unanswered<R, A>(reads: &mut Option<Loads<R, A>>, retired: &mut Vec<Loads<R, A>>) -> bool {
    retired.retain(|loads| !loads.ended());
    if !reads.as_ref().is_some_and(Loads::waiting) {
        return true;
    }
    if retired.len() >= MAX_RETIRED {
        return false;
    }
    if let Some(mut stalled) = reads.take() {
        // The worker stops once its unanswered read ends.
        drop(stalled.requests.take());
        retired.push(stalled);
    }
    true
}

/// The answer that comes on `answer` ([`Loads::ask`]), or why there is none, after `wait` at most.
fn wait_for_answer<A>(answer: &mpsc::Receiver<A>, wait: Duration) -> Result<A, String> {
    match answer.recv_timeout(wait) {
        Ok(answer) => Ok(answer),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!("the clipboard's owner didn't send its data within {} s", wait.as_secs())),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(GONE.into()),
    }
}

impl<R, A> Drop for Loads<R, A> {
    /// Stops the worker and waits until it has let go of what its `load` holds, until [`stop_by`]
    /// at most: on X11 the last arboard handle dropped hands what the app copied to the clipboard
    /// manager, and the app's own handle must be that one.
    fn drop(&mut self) {
        let pending = self.sent.filter(|_| self.open.load(Ordering::SeqCst) > 0);
        drop(self.requests.take());
        let end = stop_by(pending, Instant::now(), self.finish.get());
        // Nothing is sent on `done`: this ends when the worker drops `alive`, or at `end`.
        let _ = self.done.recv_timeout(end.saturating_duration_since(Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_that_answers_in_time_gives_its_answer() {
        let mut loads = Loads::spawn("test", Duration::ZERO, |n: u32| n * 2).unwrap();
        assert_eq!(loads.get(2, Duration::from_secs(600)), Ok(4));
        assert_eq!(loads.get(3, Duration::from_secs(600)), Ok(6));
    }

    #[test]
    fn an_owner_that_never_answers_holds_the_worker_not_the_caller() {
        let (release, gate) = mpsc::channel::<()>();
        let mut calls = 0;
        let mut loads = Loads::spawn("test", Duration::ZERO, move |()| {
            calls += 1;
            if calls == 1 {
                // The first owner sends nothing until released.
                let _ = gate.recv();
                return "stale".to_string();
            }
            "fresh".to_string()
        })
        .unwrap();
        assert!(loads.get((), Duration::ZERO).unwrap_err().contains("didn't send its data"), "the first read times out");
        // While it is unanswered, a read doesn't queue behind it.
        assert_eq!(loads.get((), Duration::ZERO), Err(BUSY.to_string()));
        release.send(()).unwrap();
        // Once the late answer is in, it is dropped and a new read gets its own answer.
        let text = loop {
            match loads.get((), Duration::from_secs(600)) {
                Ok(text) => break text,
                Err(why) => {
                    // The late answer isn't in yet.
                    assert_eq!(why, BUSY);
                    std::thread::yield_now();
                }
            }
        };
        assert_eq!(text, "fresh");
    }

    #[test]
    fn a_read_after_the_worker_stops_says_so() {
        let mut loads = Loads::spawn("test", Duration::ZERO, |()| ()).unwrap();
        // The worker ends when its requests stop: cut them off by hand.
        drop(loads.requests.take());
        assert_eq!(loads.get((), Duration::from_secs(600)), Err(GONE.to_string()));
    }

    #[test]
    fn a_panicking_read_stops_the_worker_and_later_reads_say_so() {
        let mut loads = Loads::spawn("test", Duration::ZERO, |()| -> u8 { panic!("reader bug") }).unwrap();
        assert_eq!(loads.get((), Duration::from_secs(600)), Err(GONE.to_string()));
        // The worker is gone, and the read that panicked is not left open.
        assert_eq!(loads.get((), Duration::from_secs(600)), Err(GONE.to_string()));
    }

    #[test]
    fn dropping_the_reads_waits_until_the_worker_lets_go_of_its_clipboard() {
        let clipboard = Arc::new(());
        let held = clipboard.clone();
        let mut loads = Loads::spawn("test", Duration::ZERO, move |()| Arc::strong_count(&held)).unwrap();
        assert_eq!(loads.get((), Duration::from_secs(600)), Ok(2));
        drop(loads);
        assert_eq!(Arc::strong_count(&clipboard), 1, "the worker's handle is gone before the app's");
    }

    #[test]
    fn dropping_the_reads_waits_for_an_unanswered_read_up_to_its_limit() {
        let now = Instant::now();
        let limit = Duration::from_secs(25);
        // No read unanswered: the idle worker ends at once, LOAD_WAIT is only the bound.
        assert_eq!(stop_by(None, now, limit), now + LOAD_WAIT);
        // A read sent 2 s ago that timed out: wait for it until 25 s after it was sent.
        assert_eq!(stop_by(Some(now - Duration::from_secs(2)), now, limit), now + Duration::from_secs(23));
        // Its limit already over, or no limit (data-control): no wait.
        assert!(stop_by(Some(now - Duration::from_secs(30)), now, limit) <= now);
        assert_eq!(stop_by(Some(now), now, Duration::ZERO), now);
    }

    #[test]
    fn a_copy_over_an_unanswered_read_lets_the_next_read_start_afresh() {
        let (release, gate) = mpsc::channel::<()>();
        // A long finish, as on X11: forgetting the stalled reads must not wait for it.
        let mut reads = Some(
            Loads::spawn("test", Duration::from_secs(600), move |()| {
                let _ = gate.recv();
            })
            .unwrap(),
        );
        let mut retired = Vec::new();
        // Bound after `reads` and `retired`, so a failing assertion releases the read before
        // either is dropped.
        let release = release;
        assert!(reads.as_mut().unwrap().get((), Duration::ZERO).is_err(), "the read times out");
        forget_unanswered(&mut reads, &mut retired);
        assert!(reads.is_none(), "the stalled reads are taken out of use");
        assert_eq!(retired.len(), 1, "and kept until their worker ends");
        drop(release);
        while !retired[0].ended() {
            std::thread::yield_now();
        }
        forget_unanswered(&mut reads, &mut retired);
        assert!(retired.is_empty(), "a worker that ended is let go");

        // With no finish, as on data-control, the stalled reads are kept too, and dropping them
        // doesn't wait.
        let (release, gate) = mpsc::channel::<()>();
        let mut reads = Some(
            Loads::spawn("test", Duration::ZERO, move |()| {
                let _ = gate.recv();
            })
            .unwrap(),
        );
        assert!(reads.as_mut().unwrap().get((), Duration::ZERO).is_err(), "the read times out");
        forget_unanswered(&mut reads, &mut retired);
        assert!(reads.is_none() && retired.len() == 1);
        drop(retired.pop());
        drop(release);

        let mut idle = Some(Loads::spawn("test", Duration::ZERO, |()| 1).unwrap());
        assert_eq!(idle.as_mut().unwrap().get((), Duration::from_secs(600)), Ok(1));
        forget_unanswered(&mut idle, &mut Vec::new());
        assert!(idle.is_some(), "reads with every answer in stay");
    }

    #[test]
    fn copies_over_unanswered_reads_keep_at_most_max_retired_stalled_workers() {
        let mut retired = Vec::new();
        // Declared after `retired`, so a failing assertion releases the reads before they drop.
        let mut releases = Vec::new();
        let mut stalled = || {
            let (release, gate) = mpsc::channel::<()>();
            releases.push(release);
            let mut reads = Loads::spawn("test", Duration::ZERO, move |()| {
                let _ = gate.recv();
            })
            .unwrap();
            assert!(reads.get((), Duration::ZERO).is_err(), "the read times out");
            Some(reads)
        };
        for kept in 1..=MAX_RETIRED {
            let mut reads = stalled();
            assert!(forget_unanswered(&mut reads, &mut retired));
            assert!(reads.is_none(), "copy {kept} takes the stalled reads out of use");
            assert_eq!(retired.len(), kept);
        }
        let mut reads = stalled();
        assert!(!forget_unanswered(&mut reads, &mut retired), "the app is told to try again");
        assert!(reads.is_some(), "past the limit the stalled reads stay in use");
        assert_eq!(reads.as_mut().unwrap().get((), Duration::ZERO), Err(BUSY.to_string()));
        assert_eq!(retired.len(), MAX_RETIRED);
        // Once a retired read ends, trying again takes the stalled reads out of use.
        drop(releases.remove(0));
        while !retired[0].ended() {
            std::thread::yield_now();
        }
        assert!(forget_unanswered(&mut reads, &mut retired));
        assert!(reads.is_none());
        assert_eq!(retired.len(), MAX_RETIRED);
        releases.clear();
    }

    #[test]
    fn the_clipboard_opens_on_the_worker_at_the_first_read_not_on_the_caller() {
        let (release, gate) = mpsc::channel::<()>();
        let opens = Arc::new(AtomicUsize::new(0));
        let counted = opens.clone();
        let load = opened_on_worker(
            move || {
                assert_eq!(std::thread::current().name(), Some("test"), "the clipboard opens on the worker");
                // The first open hangs until released, as on a display server that stops answering.
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    let _ = gate.recv();
                }
                Ok(())
            },
            |(), n: u32| Ok(n),
            |_, why| Err(why),
        );
        let mut loads = Loads::spawn("test", Duration::ZERO, load).unwrap();
        assert_eq!(opens.load(Ordering::SeqCst), 0, "starting the reads opens nothing");
        let why = loads.get(1, Duration::ZERO).unwrap_err();
        assert!(why.contains("didn't send its data"), "a hung open holds the worker, not the caller: {why}");
        release.send(()).unwrap();
        let answer = loop {
            match loads.get(2, Duration::from_secs(600)) {
                Ok(answer) => break answer,
                Err(why) => {
                    // The hung read hasn't ended yet.
                    assert_eq!(why, BUSY);
                    std::thread::yield_now();
                }
            }
        };
        assert_eq!(answer, Ok(2));
        assert_eq!(loads.get(3, Duration::from_secs(600)), Ok(Ok(3)));
        assert_eq!(opens.load(Ordering::SeqCst), 1, "later reads keep the handle");

        // An open that fails is the read's error, and the next read opens again.
        let mut tries = 0;
        let load = opened_on_worker(
            move || {
                tries += 1;
                if tries == 1 { Err("no backend".to_string()) } else { Ok(()) }
            },
            |(), n: u32| Ok(n),
            |_, why| Err(why),
        );
        let mut loads = Loads::spawn("test", Duration::ZERO, load).unwrap();
        assert_eq!(loads.get(1, Duration::from_secs(600)), Ok(Err("no backend".to_string())));
        assert_eq!(loads.get(2, Duration::from_secs(600)), Ok(Ok(2)));
    }

    #[test]
    fn the_worker_sets_how_long_dropping_the_reads_waits() {
        let finish = Finish::from(Duration::from_secs(25));
        let learned = finish.clone();
        let mut loads = Loads::spawn("test", finish, move |()| learned.set(Duration::ZERO)).unwrap();
        assert_eq!(loads.finish.get(), Duration::from_secs(25), "until the worker knows, the wait is the one given");
        assert_eq!(loads.get((), Duration::from_secs(600)), Ok(()));
        assert_eq!(loads.finish.get(), Duration::ZERO, "the wait the worker set is the one dropping uses");
    }

    #[test]
    fn text_and_file_lists_wait_2_s_and_bitmaps_10_s() {
        assert_eq!(wait_for(Request::Text), Duration::from_secs(2));
        assert_eq!(wait_for(Request::Files), Duration::from_secs(2));
        assert_eq!(wait_for(Request::Image), Duration::from_secs(10));
    }

    #[test]
    fn a_read_that_gets_no_answer_is_an_error_not_an_empty_clipboard() {
        let (release, gate) = mpsc::channel::<()>();
        let mut reads = Loads::spawn("test", Duration::ZERO, move |request| {
            let _ = gate.recv();
            Answer::Files(Ok(Some(vec![PathBuf::from(format!("{request:?}"))])))
        })
        .unwrap();
        let why = read_on(&mut reads, Request::Files, Duration::ZERO).files().unwrap_err();
        assert!(why.contains("didn't send its data within 0 s"), "{why}");
        assert_eq!(read_on(&mut reads, Request::Text, Duration::ZERO).text(), Err(BUSY.to_string()));
        drop(release);
    }

    #[test]
    fn an_answer_to_another_request_is_not_taken_for_this_one() {
        assert_eq!(Answer::Text(Ok(Some("x".into()))).files(), Err(MIXED.to_string()));
        assert_eq!(Answer::Files(Ok(None)).image(), Err(MIXED.to_string()));
        assert_eq!(Answer::Image(Ok(None)).text(), Err(MIXED.to_string()));
        assert_eq!(Answer::failed(Request::Image, "why".into()).image(), Err("why".to_string()));
    }
}
