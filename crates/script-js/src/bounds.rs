//! How much a run may ask of the app: host calls and output are counted, and past either bound
//! the run is stopped with a named error. The app holds its side to the same bounds, so a host
//! that miscounts gains nothing (`isolate::supervisor`).

/// Most host calls (output lines, dialogs and commands) one run may make.
pub const MAX_CALLS: usize = 1 << 19;
/// Most bytes of output (`$.writeln` lines) one run may send to the app. The app shows only its
/// latest lines, but every line is sent and handled, so their total is bounded here.
pub const MAX_OUTPUT_BYTES: usize = 32 << 20;

/// The calls and output a run has made so far.
#[derive(Debug, Default)]
pub struct Bounds {
    calls: usize,
    output: usize,
    /// Why the run must stop, once a bound was crossed. Every later call fails with it.
    overflow: Option<String>,
}

impl Bounds {
    /// Count one host call, with the bytes of output it writes: `Err` with the reason once the
    /// run has made more than it may.
    ///
    /// # Errors
    /// The named error of the bound crossed, now or by an earlier call.
    pub fn call(&mut self, output: usize) -> Result<(), String> {
        if let Some(m) = &self.overflow {
            return Err(m.clone());
        }
        self.calls += 1;
        self.output = self.output.saturating_add(output);
        let crossed = if self.calls > MAX_CALLS {
            Some(format!("The script was stopped: it made more than {MAX_CALLS} host calls (output lines, dialogs and commands)."))
        } else if self.output > MAX_OUTPUT_BYTES {
            Some(format!("The script was stopped: it wrote more than {} MiB of output.", MAX_OUTPUT_BYTES >> 20))
        } else {
            None
        };
        match crossed {
            Some(m) => Err(self.overflow.insert(m).clone()),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crossed_bound_stays_crossed() {
        let mut b = Bounds::default();
        assert!(b.call(MAX_OUTPUT_BYTES).is_ok());
        let m = b.call(1).unwrap_err();
        assert!(m.contains("MiB of output"), "{m}");
        assert_eq!(b.call(0), Err(m), "every later call fails the same way");
        let mut b = Bounds::default();
        for _ in 0..MAX_CALLS {
            assert!(b.call(0).is_ok());
        }
        assert!(b.call(0).unwrap_err().contains("host calls"));
    }
}
