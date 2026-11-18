//! The timeout around GPU initialization, with test-controlled work instead of a driver.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use super::{InitWait, run_with_timeout};

const LONG: Duration = Duration::from_secs(60);

#[test]
fn work_that_finishes_in_time_returns_its_result() {
    assert!(matches!(run_with_timeout("t_in_time", LONG, || 7), Ok(7)));
}

#[test]
fn a_timed_out_request_counts_as_running_until_its_work_finishes() {
    let (release, released) = mpsc::channel::<()>();
    let res = run_with_timeout("t_abandoned", Duration::from_millis(10), move || released.recv().is_ok());
    let Err(InitWait::TimedOut(pending)) = res else { panic!("held work must time out") };
    assert!(!pending.is_finished());
    assert!(!pending.wait(Duration::from_millis(50)), "the wait returned while the work was still held");

    release.send(()).unwrap();
    assert!(pending.wait(LONG), "the work finished but the wait never saw it");
    assert!(pending.is_finished());
}

/// A result that arrives after the timeout is dropped on the worker before it counts as
/// finished: a context's device must be gone before a waiter lets the process exit.
#[test]
fn an_unclaimed_result_is_dropped_before_the_request_finishes() {
    struct SlowDrop(Arc<AtomicBool>);
    impl Drop for SlowDrop {
        fn drop(&mut self) {
            std::thread::sleep(Duration::from_millis(50));
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let (release, released) = mpsc::channel::<()>();
    let result_dropped = Arc::clone(&dropped);
    let res = run_with_timeout("t_unclaimed", Duration::from_millis(10), move || {
        let _ = released.recv();
        SlowDrop(result_dropped)
    });
    let Err(InitWait::TimedOut(pending)) = res else { panic!("held work must time out") };

    release.send(()).unwrap();
    assert!(pending.wait(LONG));
    assert!(dropped.load(Ordering::SeqCst), "the request finished before its unclaimed result was dropped");
}
