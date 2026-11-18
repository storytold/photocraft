//! Checked admission of nonrefundable work, independent of retained storage.
//!
//! Units, initial limits, cancellation and error classification belong to each
//! domain. Borrowing the existing counter cannot restart it for another owner.

pub(super) struct WorkBudget<'a> {
    remaining: &'a mut usize,
}

impl<'a> WorkBudget<'a> {
    pub(super) fn new(remaining: &'a mut usize) -> Self {
        Self { remaining }
    }

    pub(super) fn can_charge(&self, units: usize) -> bool {
        *self.remaining >= units
    }

    /// A refused request does not spend the residual allowance.
    pub(super) fn keep_remaining(&mut self, units: usize) -> bool {
        let Some(left) = self.remaining.checked_sub(units) else { return false };
        *self.remaining = left;
        true
    }

    /// Domains that treat a failed attempt as exhausting their allowance use this.
    pub(super) fn exhaust_on_reject(&mut self, units: usize) -> bool {
        if self.keep_remaining(units) {
            return true;
        }
        *self.remaining = 0;
        false
    }
}

/// Admit related counters atomically in the supplied rejection order. No counter
/// is debited on refusal. A domain may then explicitly exhaust the failing one.
/// Fixed arrays require no allocation and cannot alias mutable counters.
pub(super) fn charge_all<const N: usize>(mut requests: [(&mut usize, usize); N]) -> std::result::Result<(), usize> {
    for (index, (remaining, units)) in requests.iter().enumerate() {
        if **remaining < *units {
            return Err(index);
        }
    }
    for (remaining, units) in &mut requests {
        // All counters passed admission, and exclusive borrows prevent changes
        // between that check and this debit. Saturation keeps this panic-free.
        **remaining = remaining.saturating_sub(*units);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_borrows_keep_spent_work_and_exact_boundary() {
        let mut left = 9;
        assert!(WorkBudget::new(&mut left).keep_remaining(4));
        assert!(WorkBudget::new(&mut left).keep_remaining(5));
        assert!(!WorkBudget::new(&mut left).keep_remaining(1));
        assert_eq!(left, 0);
        assert!(WorkBudget::new(&mut left).keep_remaining(0));
    }

    #[test]
    fn rejection_policy_is_explicit_and_hostile_requests_never_wrap() {
        let mut keep = 7;
        let mut exhaust = 7;
        assert!(!WorkBudget::new(&mut keep).keep_remaining(usize::MAX));
        assert!(!WorkBudget::new(&mut exhaust).exhaust_on_reject(usize::MAX));
        assert_eq!((keep, exhaust), (7, 0));
        assert!(WorkBudget::new(&mut keep).keep_remaining(7));
        let mut maximum = usize::MAX;
        assert!(WorkBudget::new(&mut maximum).keep_remaining(usize::MAX));
        assert_eq!(maximum, 0);
    }

    #[test]
    fn related_charges_reject_in_order_without_partial_debits() {
        let (mut local_visits, mut local_bytes, mut file_visits, mut file_bytes) = (2, 5, 3, 4);
        assert_eq!(charge_all([(&mut local_visits, 1), (&mut local_bytes, 5), (&mut file_visits, 1), (&mut file_bytes, 5)]), Err(3));
        assert_eq!((local_visits, local_bytes, file_visits, file_bytes), (2, 5, 3, 4));
        assert_eq!(charge_all([(&mut local_visits, 3), (&mut local_bytes, 6), (&mut file_visits, 4), (&mut file_bytes, 5)]), Err(0));
        assert_eq!((local_visits, local_bytes, file_visits, file_bytes), (2, 5, 3, 4));
        assert_eq!(charge_all([(&mut local_visits, 1), (&mut local_bytes, 4), (&mut file_visits, 1), (&mut file_bytes, 4)]), Ok(()));
        assert_eq!((local_visits, local_bytes, file_visits, file_bytes), (1, 1, 2, 0));
    }

    #[test]
    fn failed_storage_owner_cannot_refund_separate_work() {
        let mut work = 10;
        let mut retained = 20;
        let before_owner = retained;
        assert!(WorkBudget::new(&mut work).keep_remaining(6));
        retained -= 8;
        assert_eq!(retained, 12);
        // The parser's optional owner failed: only its storage snapshot returns.
        retained = before_owner;
        assert_eq!((retained, work), (20, 4));
        assert!(!WorkBudget::new(&mut work).keep_remaining(6));
    }
}
