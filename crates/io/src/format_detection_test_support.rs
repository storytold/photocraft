//! Observe the real detector boundary in tests without changing production routing.

thread_local! {
    pub(crate) static COMPRESSED_DETECTOR_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn invoke_format_detector(name: &str, bytes: &[u8]) -> Option<photocraft_formats::Format> {
    // Count after routing sanitizes input, so removing that policy fails the tests.
    if super::import_compression(bytes).is_some() {
        COMPRESSED_DETECTOR_CALLS.with(|calls| calls.set(calls.get() + 1));
    }
    photocraft_formats::detect(name, bytes)
}
