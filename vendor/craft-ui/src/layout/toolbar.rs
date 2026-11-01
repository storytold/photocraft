/// Visible toolbar groups and the groups to present in an application-owned overflow menu.
///
/// Groups stay at their natural widths, in their original order. This helper does not paint,
/// hide commands, create a menu, or change widget identity. The caller must make every index in
/// `overflow` available through an enabled, focusable overflow control and its menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolbarFit {
    /// A leading sequence of `(input index, natural width)` pairs. Groups are never compressed.
    pub shown: Vec<(usize, f32)>,
    /// Remaining input indices, in order. Empty when the whole toolbar fits.
    pub overflow: Vec<usize>,
    /// Full requested overflow-control width when needed, otherwise zero.
    pub overflow_width: f32,
}

const MAX_GROUPS: usize = 4096;
const MAX_EXTENT: f32 = 1_000_000.0;

fn nonnegative_extent(value: f32) -> bool {
    value.is_finite() && (0.0..=MAX_EXTENT).contains(&value)
}

/// Fit indivisible toolbar groups and, when needed, a full-size overflow control.
///
/// `spacing` is included between adjacent visible groups and before the overflow control. The
/// result preserves a leading sequence instead of skipping a wide group to show later commands.
/// An empty input returns an empty fit. At most 4096 groups and dimensions at most one million
/// logical points are accepted; all dimensions must be finite, widths strictly positive, and
/// `available` and `spacing` nonnegative. Work and storage are bounded and linear in group count.
///
/// Returns `None` for invalid input or if overflow is needed but `available` is smaller than
/// `overflow_width`. The caller must then use an explicit alternative such as a scrollable row,
/// wrapping, or an existing application menu; do not silently hide the commands. A positive
/// overflow-control width is required even when the current groups happen to fit.
pub fn fit_toolbar(natural: &[f32], available: f32, spacing: f32, overflow_width: f32) -> Option<ToolbarFit> {
    if natural.len() > MAX_GROUPS
        || !nonnegative_extent(available)
        || !nonnegative_extent(spacing)
        || !nonnegative_extent(overflow_width)
        || overflow_width == 0.0
        || natural.iter().any(|&width| !nonnegative_extent(width) || width == 0.0)
    {
        return None;
    }
    let gaps = natural.len().saturating_sub(1);
    let total = natural.iter().map(|&width| f64::from(width)).sum::<f64>() + f64::from(spacing) * gaps as f64;
    if total <= f64::from(available) {
        return Some(ToolbarFit { shown: natural.iter().copied().enumerate().collect(), ..Default::default() });
    }
    if available < overflow_width {
        return None;
    }
    let budget = f64::from(available) - f64::from(overflow_width);
    let mut used = 0.0;
    let shown_count = natural
        .iter()
        .take_while(|&&width| {
            // Every shown group has a following gap: to the next group or to the overflow.
            let next = used + f64::from(width) + f64::from(spacing);
            if next <= budget {
                used = next;
                true
            } else {
                false
            }
        })
        .count();
    Some(ToolbarFit {
        shown: natural.iter().copied().take(shown_count).enumerate().collect(),
        overflow: (shown_count..natural.len()).collect(),
        overflow_width,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_fit_keeps_every_natural_width_and_needs_no_overflow() {
        assert_eq!(
            fit_toolbar(&[24.0, 36.0, 48.0], 120.0, 6.0, 20.0),
            Some(ToolbarFit { shown: vec![(0, 24.0), (1, 36.0), (2, 48.0)], overflow: vec![], overflow_width: 0.0 })
        );
        assert_eq!(fit_toolbar(&[], 0.0, 6.0, 20.0), Some(ToolbarFit::default()));
    }

    #[test]
    fn overflow_reserves_its_control_and_the_preceding_spacing() {
        assert_eq!(
            fit_toolbar(&[24.0, 36.0, 48.0], 96.0, 4.0, 20.0),
            Some(ToolbarFit { shown: vec![(0, 24.0), (1, 36.0)], overflow: vec![2], overflow_width: 20.0 })
        );
        // At 87 points the second group and its gap would overlap the overflow control.
        assert_eq!(
            fit_toolbar(&[24.0, 36.0, 48.0], 87.0, 4.0, 20.0),
            Some(ToolbarFit { shown: vec![(0, 24.0)], overflow: vec![1, 2], overflow_width: 20.0 })
        );
    }

    #[test]
    fn a_wide_first_group_is_not_compressed_or_skipped() {
        assert_eq!(
            fit_toolbar(&[200.0, 24.0, 24.0], 120.0, 4.0, 20.0),
            Some(ToolbarFit { shown: vec![], overflow: vec![0, 1, 2], overflow_width: 20.0 })
        );
        assert_eq!(fit_toolbar(&[200.0], 60.0, 4.0, 20.0), Some(ToolbarFit { shown: vec![], overflow: vec![0], overflow_width: 20.0 }));
    }

    #[test]
    fn tiny_rows_require_a_real_fallback_instead_of_an_inaccessible_button() {
        for available in [0.0, 1.0, 19.999] {
            assert_eq!(fit_toolbar(&[30.0, 30.0], available, 4.0, 20.0), None);
        }
        assert_eq!(fit_toolbar(&[30.0, 30.0], 20.0, 4.0, 20.0), Some(ToolbarFit { shown: vec![], overflow: vec![0, 1], overflow_width: 20.0 }));
        assert!(fit_toolbar(&[10.0], 10.0, 4.0, 20.0).unwrap().overflow.is_empty());
    }

    #[test]
    fn malformed_dimensions_and_excessive_group_counts_are_rejected() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, MAX_EXTENT + 1.0, f32::MAX] {
            assert_eq!(fit_toolbar(&[24.0], value, 4.0, 20.0), None);
            assert_eq!(fit_toolbar(&[24.0], 100.0, value, 20.0), None);
            assert_eq!(fit_toolbar(&[24.0], 100.0, 4.0, value), None);
            assert_eq!(fit_toolbar(&[value], 100.0, 4.0, 20.0), None);
        }
        assert_eq!(fit_toolbar(&[0.0], 100.0, 4.0, 20.0), None);
        assert_eq!(fit_toolbar(&[24.0], 100.0, 4.0, 0.0), None);
        assert!(fit_toolbar(&vec![24.0; MAX_GROUPS], MAX_EXTENT, 4.0, 20.0).is_some());
        assert_eq!(fit_toolbar(&vec![24.0; MAX_GROUPS + 1], MAX_EXTENT, 4.0, 20.0), None);
    }

    #[test]
    fn every_group_remains_reachable_and_every_accepted_fit_stays_within_budget() {
        let widths = [12.5, 28.0, 24.25, 58.0, 32.0];
        for available in [20.0, 31.5, 37.0, 55.0, 93.5, 156.0, 500.0, MAX_EXTENT] {
            for spacing in [0.0, 0.5, 4.0, 12.0] {
                let fit = fit_toolbar(&widths, available, spacing, 20.0).unwrap();
                let indices: Vec<_> = fit.shown.iter().map(|&(i, _)| i).chain(fit.overflow.iter().copied()).collect();
                assert_eq!(indices, [0, 1, 2, 3, 4]);
                for &(index, width) in &fit.shown {
                    assert_eq!(width.to_bits(), widths[index].to_bits());
                }
                let count = fit.shown.len() + usize::from(!fit.overflow.is_empty());
                let used = fit.shown.iter().map(|&(_, width)| f64::from(width)).sum::<f64>()
                    + f64::from(fit.overflow_width)
                    + count.saturating_sub(1) as f64 * f64::from(spacing);
                assert!(used <= f64::from(available), "{fit:?} uses {used} of {available}");
            }
        }
    }
}
