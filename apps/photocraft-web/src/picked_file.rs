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
}
