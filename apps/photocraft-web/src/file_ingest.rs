//! Admission for user-selected browser files, before allocating their contents.
//!
//! This bounds raw input buffers, not decoded documents. One batch owns the reader until its
//! bytes have been consumed by the UI; further selections/drops get an actionable busy error.

use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub(crate) struct Admission {
    busy: Arc<AtomicBool>,
    max_files: usize,
    max_bytes: usize,
}

impl Default for Admission {
    fn default() -> Self {
        // Browser-specific acquisition policy: leave room for the JS copy and decoding in
        // wasm32 memory. Larger documents can still be opened with the desktop application.
        Self { busy: Arc::default(), max_files: 32, max_bytes: 256 * 1024 * 1024 }
    }
}

pub(crate) struct Lease(Arc<AtomicBool>);

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(crate) trait Source {
    fn name(&self) -> String;
    fn size(&self) -> Option<f64>;
    fn read(self) -> impl Future<Output = Result<Vec<u8>, String>>;
}

pub(crate) struct Batch<S> {
    files: Vec<(S, usize)>,
    lease: Lease,
}

pub(crate) struct Delivery {
    pub files: Vec<(String, Vec<u8>)>,
    pub lease: Lease,
}

pub(crate) struct RoutedDelivery {
    pub open: Vec<(String, Vec<u8>)>,
    pub place: Vec<(String, Vec<u8>)>,
    pub lease: Lease,
}

impl Delivery {
    /// Keep the admission lease across both queues until the UI consumes them.
    pub fn route(self, places: &[bool]) -> Result<RoutedDelivery, String> {
        if places.len() != self.files.len() {
            return Err("Dropped file destinations do not match the admitted files.".into());
        }
        let mut routed = RoutedDelivery { open: Vec::new(), place: Vec::new(), lease: self.lease };
        for (file, &place) in self.files.into_iter().zip(places) {
            if place {
                routed.place.push(file);
            } else {
                routed.open.push(file);
            }
        }
        Ok(routed)
    }
}

impl Admission {
    /// Synchronous: reject the entire batch before spawning a reader or allocating contents.
    pub fn admit<S: Source>(&self, files: Vec<S>) -> Result<Batch<S>, String> {
        if files.is_empty() || files.len() > self.max_files {
            return Err(format!("Choose between 1 and {} files at a time in the browser.", self.max_files));
        }
        let mut total = 0usize;
        let mut checked = Vec::with_capacity(files.len());
        for file in files {
            let size = file.size().ok_or("Cannot determine the selected file's size; no files were read.")?;
            if !size.is_finite() || size < 0.0 || size.fract() != 0.0 || size > self.max_bytes as f64 {
                return Err(self.too_large());
            }
            let size = size as usize;
            total = total.checked_add(size).filter(|n| *n <= self.max_bytes).ok_or_else(|| self.too_large())?;
            checked.push((file, size));
        }
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Files are still being opened. Wait for them to finish, then select or drop again.")?;
        Ok(Batch { files: checked, lease: Lease(self.busy.clone()) })
    }

    fn too_large(&self) -> String {
        format!("Selected files exceed the browser's {}-byte input limit. Choose fewer/smaller files or use PhotoCraft desktop.", self.max_bytes)
    }
}

impl<S: Source> Batch<S> {
    pub async fn read(self) -> Result<Delivery, String> {
        let mut files = Vec::with_capacity(self.files.len());
        for (file, size) in self.files {
            let name = file.name();
            let bytes = file.read().await.map_err(|e| format!("Could not read {name}: {e}"))?;
            if bytes.len() != size {
                return Err(format!("The size of {name} changed while reading; no files were opened."));
            }
            files.push((name, bytes));
        }
        Ok(Delivery { files, lease: self.lease })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::task::{Context, Poll, Waker};

    struct Fake {
        size: Option<f64>,
        bytes: Result<Vec<u8>, String>,
        reads: Arc<AtomicUsize>,
    }
    impl Source for Fake {
        fn name(&self) -> String {
            "tiny.png".into()
        }
        fn size(&self) -> Option<f64> {
            self.size
        }
        async fn read(self) -> Result<Vec<u8>, String> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.bytes
        }
    }
    fn source(size: f64, reads: &Arc<AtomicUsize>) -> Fake {
        Fake { size: Some(size), bytes: Ok(vec![7; 2]), reads: reads.clone() }
    }
    fn budget() -> Admission {
        Admission { max_files: 2, max_bytes: 4, ..Default::default() }
    }
    fn ready<F: Future>(f: F) -> F::Output {
        let waker = Waker::noop();
        match std::pin::pin!(f).as_mut().poll(&mut Context::from_waker(waker)) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("fake readers must be immediately ready"),
        }
    }

    #[test]
    fn drop_routing_preserves_destinations_and_holds_admission_until_consumed() {
        let budget = budget();
        let reads = Arc::default();
        let delivery = ready(budget.admit(vec![source(2.0, &reads), source(2.0, &reads)]).unwrap().read()).unwrap();
        let routed = delivery.route(&[true, false]).unwrap();
        assert_eq!(routed.place.len(), 1);
        assert_eq!(routed.open.len(), 1);
        assert_eq!(routed.place[0].1, vec![7; 2]);
        assert!(budget.admit(vec![source(2.0, &reads)]).is_err());
        let RoutedDelivery { open, place, lease } = routed;
        drop(open);
        drop(place);
        assert!(budget.admit(vec![source(2.0, &reads)]).is_err());
        drop(lease);
        let delivery = ready(budget.admit(vec![source(2.0, &reads)]).unwrap().read()).unwrap();
        assert!(delivery.route(&[]).is_err());
        assert!(budget.admit(vec![source(2.0, &reads)]).is_ok());
    }
    #[test]
    fn rejects_count_single_size_and_total_before_any_reads() {
        let reads = Arc::default();
        for sizes in [vec![], vec![0.0; 3], vec![5.0], vec![3.0, 2.0]] {
            assert!(budget().admit(sizes.into_iter().map(|n| source(n, &reads)).collect()).is_err());
        }
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn rejects_missing_non_finite_negative_and_fractional_metadata() {
        let reads = Arc::default();
        for size in [None, Some(f64::NAN), Some(f64::INFINITY), Some(-1.0), Some(0.5)] {
            let mut file = source(2.0, &reads);
            file.size = size;
            assert!(budget().admit(vec![file]).is_err());
        }
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn shared_picker_drop_budget_lasts_through_delivery() {
        let budget = budget();
        let drop_budget = budget.clone();
        let reads = Arc::default();
        let picker = budget.admit(vec![source(2.0, &reads), source(2.0, &reads)]).unwrap();
        assert!(drop_budget.admit(vec![source(2.0, &reads)]).is_err());
        let delivery = ready(picker.read()).unwrap();
        assert_eq!(delivery.files, vec![("tiny.png".into(), vec![7; 2]); 2]);
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        assert!(drop_budget.admit(vec![source(2.0, &reads)]).is_err());
        let Delivery { files, lease } = delivery;
        drop(files);
        drop(lease);
        assert!(drop_budget.admit(vec![source(2.0, &reads)]).is_ok());
    }

    #[test]
    fn cancellation_read_failure_and_size_mismatch_release_admission() {
        let budget = budget();
        let reads = Arc::default();
        drop(budget.admit(vec![source(2.0, &reads)]).unwrap());
        let mut failed = source(2.0, &reads);
        failed.bytes = Err("read denied".into());
        assert!(ready(budget.admit(vec![failed]).unwrap().read()).is_err());
        assert!(ready(budget.admit(vec![source(1.0, &reads)]).unwrap().read()).is_err());
        assert!(ready(budget.admit(vec![source(2.0, &reads)]).unwrap().read()).is_ok());
    }

    #[test]
    fn pending_read_cancellation_releases_and_does_not_start_next_file() {
        struct PendingSource(Arc<AtomicUsize>);
        impl Source for PendingSource {
            fn name(&self) -> String {
                "pending".into()
            }
            fn size(&self) -> Option<f64> {
                Some(1.0)
            }
            async fn read(self) -> Result<Vec<u8>, String> {
                self.0.fetch_add(1, Ordering::Relaxed);
                std::future::pending().await
            }
        }
        let budget = budget();
        let reads = Arc::new(AtomicUsize::new(0));
        let batch = budget.admit(vec![PendingSource(reads.clone()), PendingSource(reads.clone())]).unwrap();
        let mut future = Box::pin(batch.read());
        let waker = Waker::noop();
        assert!(future.as_mut().poll(&mut Context::from_waker(waker)).is_pending());
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        drop(future);
        assert!(budget.admit(vec![PendingSource(reads)]).is_ok());
    }
}
