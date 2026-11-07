#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Read;

/// A print server that sends its reply a few bytes at a time, so the header search and the chunk
/// walk run across reads the way they do on a slow socket.
struct Trickle<'a> {
    data: &'a [u8],
    step: usize,
}

impl Read for Trickle<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.step.min(buf.len()).min(self.data.len());
        let (head, rest) = self.data.split_at(n);
        buf[..n].copy_from_slice(head);
        self.data = rest;
        Ok(n)
    }
}

// Raw bytes from the print server (HTTP headers, Content-Length or chunks, the IPP reply) through
// everything the client does with them: never panic. The first byte picks how many bytes each
// read returns.
fuzz_target!(|data: &[u8]| {
    let Some((&step, data)) = data.split_first() else { return };
    let mut stream = Trickle { data, step: usize::from(step).max(1) };
    let _ = photocraft_ipp::fuzz_reply(&mut stream);
});
