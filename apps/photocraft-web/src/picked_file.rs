//! Admit a browser file from its immutable size before invoking the whole-file reader.
use std::future::Future;

/// Keep the size check before creating the read future, shared by single and multiple selection.
pub async fn read<F, Fut>(name: String, size: f64, max: Option<u64>, read: F) -> Result<(String, Vec<u8>), String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Vec<u8>>,
{
    if let Some(max) = max
        && (!size.is_finite() || size < 0.0 || size > max as f64)
    {
        return Err(format!("{name}: the file is larger than {max} bytes or its size is invalid"));
    }
    Ok((name, read().await))
}

/// Preflight every immutable browser size before creating the future that reads any selection.
/// The optional staging budget is separate from the per-file cap enforced by [`read`].
pub async fn read_contents<F, Fut>(sizes: impl IntoIterator<Item = f64>, max: Option<u64>, read: F) -> Result<Vec<(String, Vec<u8>)>, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<(String, Vec<u8>)>, String>>,
{
    if let Some(max) = max {
        let total = sizes.into_iter().try_fold(0u64, |sum, size| {
            if !size.is_finite() || size < 0.0 || size.fract() != 0.0 || size >= u64::MAX as f64 {
                return None;
            }
            sum.checked_add(size as u64)
        });
        if total.is_none_or(|total| total > max) {
            return Err(format!("the selected files are larger than {max} bytes or their total size is invalid"));
        }
    }
    read().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::task::{Context, Poll, Waker};

    /// Tiny metadata-only cases prove oversize refusal does not even invoke the payload reader.
    #[test]
    fn size_limit_precedes_the_browser_read_and_preserves_uncapped_reads() {
        for (size, max, allowed) in [
            (4.0, Some(4), true),
            (5.0, Some(4), false),
            (0.0, Some(0), true),
            (f64::NAN, Some(4), false),
            (f64::INFINITY, Some(4), false),
            (-1.0, Some(4), false),
            (5.0, None, true),
        ] {
            let called = Cell::new(false);
            let bytes = if size == 0.0 { Vec::new() } else { vec![1, 2, 3, 4] };
            let future = read("picked.atn".into(), size, max, || {
                assert!(allowed, "an oversized file must not be read");
                called.set(true);
                std::future::ready(bytes.clone())
            });
            let mut future = std::pin::pin!(future);
            let Poll::Ready(result) = future.as_mut().poll(&mut Context::from_waker(Waker::noop())) else { panic!("an immediate read") };
            assert_eq!(called.get(), allowed);
            assert_eq!(result.is_ok(), allowed);
            if allowed {
                let (name, actual) = result.unwrap();
                assert_eq!(name, "picked.atn");
                assert_eq!(actual, bytes);
            } else {
                assert!(result.unwrap_err().contains("picked.atn"));
            }
        }
    }

    /// Check all metadata before even creating the read future, including a late excess and
    /// overflowing totals. Huge sizes here are numbers only; the reader returns four tiny bytes.
    #[test]
    fn contents_budget_preflights_the_entire_selection_before_any_read() {
        let cases = [
            (vec![2.0, 2.0], Some(4), true),
            (vec![2.0, 3.0], Some(4), false),
            (vec![0.0, 0.0], Some(0), true),
            (vec![], Some(0), true),
            (vec![(512u64 << 20) as f64], Some(512 << 20), true),
            (vec![(512u64 << 20) as f64, 1.0], Some(512 << 20), false),
            (vec![2f64.powi(63), 2f64.powi(63)], Some(u64::MAX), false),
            (vec![2f64.powi(64)], Some(u64::MAX), false),
            (vec![f64::NAN], Some(4), false),
            (vec![f64::INFINITY], Some(4), false),
            (vec![-1.0], Some(4), false),
            (vec![0.5], Some(4), false),
            (vec![f64::NAN, f64::INFINITY], None, true),
        ];
        for (sizes, max, allowed) in cases {
            let called = Cell::new(false);
            let files = vec![("first.atn".into(), vec![1, 2]), ("last.atn".into(), vec![3, 4])];
            let future = read_contents(sizes, max, || {
                assert!(allowed, "no file can be read from a refused selection");
                called.set(true);
                std::future::ready(Ok(files.clone()))
            });
            let mut future = std::pin::pin!(future);
            let Poll::Ready(result) = future.as_mut().poll(&mut Context::from_waker(Waker::noop())) else { panic!("an immediate read") };
            assert_eq!(called.get(), allowed);
            assert_eq!(result.is_ok(), allowed);
            if allowed {
                assert_eq!(result.unwrap(), files, "selection order and payloads are unchanged");
            } else {
                assert!(result.unwrap_err().contains("selected files"));
            }
        }
    }
}
