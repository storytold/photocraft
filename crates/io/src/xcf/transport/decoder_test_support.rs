//! Count real decoder construction only in test builds.

thread_local! {
    pub(crate) static DECODER_STARTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn compressed_reader<'a, 'bytes: 'a, 'ctl: 'a>(
    kind: crate::ImportCompression,
    input: &'a mut super::Input<'bytes, 'ctl>,
    policy: &'a std::cell::Cell<bool>,
) -> Box<dyn std::io::Read + 'a> {
    DECODER_STARTS.with(|starts| starts.set(starts.get() + 1));
    super::make_compressed_reader(kind, input, policy)
}
