//! File-wide, nonrefundable native generation work, independent of retained storage.
use super::{IoError, Result, work_budget::WorkBudget};
use photocraft_raster::Interrupt;
use std::sync::{Arc, Mutex};

// Four-byte component equivalents represent eight GiB without an overflowing
// usize byte constant on 32-bit targets.
const LIMIT: usize = 1usize << 31;
#[derive(Clone)]
pub(super) struct Work(Arc<Mutex<State>>);
struct State {
    remaining: usize,
    rejections: usize,
}
impl Default for Work {
    fn default() -> Self {
        Self::with_units_internal(LIMIT)
    }
}
impl Work {
    fn with_units_internal(remaining: usize) -> Self {
        Self(Arc::new(Mutex::new(State { remaining, rejections: 0 })))
    }
    pub(super) fn charge_bytes(&self, bytes: usize, ctl: &Interrupt<'_>) -> Result<()> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let units = bytes / 4 + usize::from(!bytes.is_multiple_of(4));
        let mut state = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !WorkBudget::new(&mut state.remaining).exhaust_on_reject(units) {
            state.rejections = state.rejections.saturating_add(1);
            return Err(IoError::XcfNativeWorkLimit);
        }
        Ok(())
    }
    pub(super) fn refuse_unpriced<T>(&self, reason: &'static str, ctl: &Interrupt<'_>) -> Result<T> {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let mut state = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.remaining = 0;
        state.rejections = state.rejections.saturating_add(1);
        Err(IoError::XcfNativeWorkUnpriced(reason))
    }
    pub(super) fn remaining_bytes(&self) -> u64 {
        let state = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.remaining as u64).saturating_mul(4)
    }
    pub(super) fn rejections(&self) -> usize {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).rejections
    }
    #[cfg(test)]
    pub(super) fn with_units(units: usize) -> Self {
        Self::with_units_internal(units)
    }
    #[cfg(test)]
    pub(super) fn remaining_units(&self) -> usize {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remaining
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rounded_boundary_is_shared_nonrefundable_and_refusal_exhausts() {
        let work = Work::with_units(3);
        work.charge_bytes(1, &Interrupt::NONE).unwrap();
        let fork = work.clone();
        fork.charge_bytes(8, &Interrupt::NONE).unwrap();
        assert_eq!(work.remaining_units(), 0);
        assert!(matches!(work.charge_bytes(1, &Interrupt::NONE), Err(IoError::XcfNativeWorkLimit)));
        assert_eq!(fork.rejections(), 1);
        let work = Work::with_units(2);
        assert!(work.charge_bytes(usize::MAX, &Interrupt::NONE).is_err());
        assert_eq!(work.remaining_units(), 0);
        assert_eq!(Work::default().remaining_bytes(), 8u64 << 30);
    }
    #[test]
    fn cancellation_precedes_admission() {
        let work = Work::with_units(3);
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let is_cancelled = || cancel.load(std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(work.charge_bytes(4, &Interrupt::cancel_only(&is_cancelled)), Err(IoError::Cancelled)));
        assert_eq!(work.remaining_units(), 3);
    }
}
