//! Native discovery may block in the OS resolver/driver. Never run it on the UI thread.
use photocraft_engine::print_cmds::RgbPrinters;
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

type Answer = Result<(Value, RgbPrinters), String>;
#[cfg(not(target_arch = "wasm32"))]
static DISCOVERING: AtomicBool = AtomicBool::new(false);

/// There is at most one discovery worker in the process, with no work queue. Dropping a
/// ticket discards its answer, NOT the OS call: a stuck resolver keeps the permit until it
/// returns. Reopening a dialog must not accumulate detached threads or reuse a stale answer.
pub(crate) struct PrinterDiscovery {
    rx: mpsc::Receiver<Answer>,
}

impl PrinterDiscovery {
    pub(super) fn start() -> Result<Self, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self::spawn(&DISCOVERING, || {
                let mut session = photocraft_engine::Session::new();
                let printers = session.execute("file.printers", serde_json::json!({})).map_err(|e| e.to_string())?;
                Ok((printers, photocraft_engine::print_cmds::rgb_printers(&session)))
            })
        }
        #[cfg(target_arch = "wasm32")]
        {
            Err("Printer discovery is not available in the browser".into())
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn spawn(busy: &'static AtomicBool, discover: impl FnOnce() -> Answer + Send + 'static) -> Result<Self, String> {
        if busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Err("Printer discovery is still running. Close and reopen Print after it finishes to try again.".into());
        }
        struct Permit(&'static AtomicBool);
        impl Drop for Permit {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let permit = Permit(busy);
        let (tx, rx) = mpsc::sync_channel(1);
        // Do not join on the UI thread; the permit outlives the dialog until the OS returns.
        let _worker = std::thread::Builder::new()
            .name("printer-discovery".into())
            .spawn(move || {
                let _permit = permit;
                let _ = tx.send(discover());
            })
            .map_err(|e| format!("Cannot start printer discovery: {e}"))?;
        Ok(Self { rx })
    }

    #[cfg(test)]
    pub(super) fn test_ticket() -> (mpsc::SyncSender<Answer>, Self) {
        let (tx, rx) = mpsc::sync_channel(1);
        (tx, Self { rx })
    }

    pub(super) fn poll(&self) -> Option<Answer> {
        match self.rx.try_recv() {
            Ok(answer) => Some(answer),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("Printer discovery stopped without a result".into())),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[test]
    fn closing_a_stalled_discovery_does_not_admit_more_workers_or_reuse_its_answer() {
        static BUSY: AtomicBool = AtomicBool::new(false);
        let (release, wait) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let ticket = PrinterDiscovery::spawn(&BUSY, move || {
            wait.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            finished.send(()).unwrap();
            Err("old dialog".into())
        })
        .unwrap();
        assert!(ticket.poll().is_none());
        drop(ticket);
        for _ in 0..100 {
            assert!(PrinterDiscovery::spawn(&BUSY, || panic!("must not run")).is_err());
        }
        release.send(()).unwrap();
        done.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while BUSY.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let next = PrinterDiscovery::spawn(&BUSY, || Err("new dialog".into())).unwrap();
        loop {
            if let Some(answer) = next.poll() {
                assert_eq!(answer.unwrap_err(), "new dialog");
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
}
