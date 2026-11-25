//! The record of everything a run did outside the engine, in call order.
//!
//! The first attempt appends entries; every later attempt of the same run replays them. A
//! replayed call must ask for exactly what was recorded at that position (same kind, same
//! arguments); anything else is a divergence and fails the run instead of re-applying effects.
//!
//! The tape is bounded: past [`MAX_ENTRIES`] calls, [`MAX_BYTES`] of recorded data,
//! [`MAX_READINGS`] clock and `Math.random()` readings or [`MAX_OUTPUT_BYTES`] of output, the run
//! is stopped with a named error.
//! Output lines are kept as a hash and a length, not as text: replay only has to recognise them.

use std::hash::{DefaultHasher, Hash, Hasher};

use serde_json::Value;

/// Most host calls (output lines, dialogs and commands) one run may record.
pub const MAX_ENTRIES: usize = 1 << 19;
/// Most bytes of recorded calls, outcomes and readings one run may keep (an estimate).
pub const MAX_BYTES: usize = 64 << 20;
/// Most clock and `Math.random()` readings one run may record.
pub const MAX_READINGS: usize = 1 << 22;
/// Most bytes of output (`$.writeln` lines) one run may send to the app. Lines are kept on the
/// tape as a hash, so they do not count toward [`MAX_BYTES`]; the app shows only its latest
/// lines, but every line is sent and handled, so their total is bounded here.
pub const MAX_OUTPUT_BYTES: usize = 32 << 20;
/// Most bytes of ScriptUI window updates one run may send. Each update carries a whole window,
/// and a palette that shows progress sends one per step, so these are bounded apart from the
/// output lines.
pub const MAX_WINDOW_BYTES: usize = 256 << 20;
/// Bytes charged per entry for its bookkeeping, on top of its data.
const ENTRY_BYTES: usize = 64;
/// Longest divergence message, in characters.
const MAX_DIVERGENCE_CHARS: usize = 500;

/// What a call asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    /// A session command by id, with its JSON params.
    Command {
        /// Command name, e.g. `document.inspect`.
        id: String,
        /// JSON parameters sent with the command.
        params: Value,
    },
    /// A line written to the script output.
    Output(String),
    /// A ScriptUI window shown, updated (`Some`) or hidden (`None`) without waiting.
    Window {
        /// The window's control id.
        id: u32,
        /// Its controls, or `None` to hide it.
        window: Option<Value>,
    },
    /// A modal dialog (`alert`, `confirm`, `prompt`, a ScriptUI dialog, a file picker) with its arguments.
    Modal {
        /// Which dialog was opened.
        kind: crate::ModalKind,
        /// The script's arguments, as JSON (`[message, ...]`).
        args: Value,
    },
}

/// A call as the tape keeps it.
#[derive(Clone, Debug, PartialEq)]
pub enum Recorded {
    Command {
        id: String,
        params: Value,
    },
    /// An output line by its hash and length in bytes. Two lines that collide (a 2^-64 chance per
    /// pair) would replay as the same line, which only skips writing a line that was written.
    Output {
        hash: u64,
        len: usize,
    },
    Modal {
        kind: crate::ModalKind,
        args: Value,
    },
    /// A window update by its id, the hash of its controls and their length in bytes (see
    /// `Output`).
    Window {
        id: u32,
        hash: u64,
        len: usize,
    },
}

impl From<Call> for Recorded {
    fn from(call: Call) -> Self {
        match call {
            Call::Command { id, params } => Self::Command { id, params },
            Call::Output(line) => {
                // Fixed keys: the same line hashes the same on every attempt of the run.
                let mut h = DefaultHasher::new();
                line.hash(&mut h);
                Self::Output { hash: h.finish(), len: line.len() }
            }
            Call::Modal { kind, args } => Self::Modal { kind, args },
            Call::Window { id, window } => {
                let text = window.as_ref().map(Value::to_string).unwrap_or_default();
                let mut h = DefaultHasher::new();
                text.hash(&mut h);
                Self::Window { id, hash: h.finish(), len: text.len() }
            }
        }
    }
}

impl Recorded {
    fn bytes(&self) -> usize {
        match self {
            Self::Command { id, params } => id.len() + json_bytes(params),
            Self::Output { .. } | Self::Window { .. } => 16,
            Self::Modal { args, .. } => json_bytes(args),
        }
    }
}

/// Size of a JSON value as text: a fair estimate of what it keeps in memory.
fn json_bytes(v: &Value) -> usize {
    serde_json::to_vec(v).map_or(0, |b| b.len())
}

/// How a recorded call ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Returned this value.
    Return(Value),
    /// Failed with this message (seen by the script as a catchable `Error`).
    Throw(String),
    /// A modal dialog: `None` while the UI has not answered it yet.
    Modal(Option<Value>),
}

impl Outcome {
    fn bytes(&self) -> usize {
        match self {
            Self::Return(v) | Self::Modal(Some(v)) => json_bytes(v),
            Self::Throw(m) => m.len(),
            Self::Modal(None) => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
/// One recorded call and how it ended.
pub struct Entry {
    /// What the script asked for.
    pub call: Recorded,
    /// How that call ended.
    pub outcome: Outcome,
}

/// Where the next call stands relative to the tape.
pub enum Begin {
    /// Already recorded: answer from the tape.
    Replay(Outcome),
    /// Past the end of the tape: perform it live, then [`Tape::finish`] it at this index.
    Live(usize),
}

/// The tape's bounds (smaller in tests).
#[derive(Debug, Clone, Copy)]
struct Limits {
    entries: usize,
    bytes: usize,
    readings: usize,
    output: usize,
}

#[derive(Debug)]
/// The ordered record of host calls, clock reads and random values for a run.
pub struct Tape {
    entries: Vec<Entry>,
    cursor: usize,
    clock: Vec<i64>,
    clock_cursor: usize,
    random: Vec<f64>,
    random_cursor: usize,
    limits: Limits,
    bytes: usize,
    /// Bytes of output lines recorded so far.
    output: usize,
    /// Bytes of window updates recorded so far.
    windows: usize,
    /// Why the run must stop, once a bound was crossed. Every later call fails with it.
    overflow: Option<String>,
}

impl Default for Tape {
    fn default() -> Self {
        Self::with_limits(MAX_ENTRIES, MAX_BYTES, MAX_READINGS)
    }
}

impl Tape {
    fn with_limits(entries: usize, bytes: usize, readings: usize) -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            clock: Vec::new(),
            clock_cursor: 0,
            random: Vec::new(),
            random_cursor: 0,
            limits: Limits { entries, bytes, readings, output: MAX_OUTPUT_BYTES },
            bytes: 0,
            output: 0,
            windows: 0,
            overflow: None,
        }
    }

    /// Rewind for a new attempt; recorded entries stay.
    pub fn rewind(&mut self) {
        self.cursor = 0;
        self.clock_cursor = 0;
        self.random_cursor = 0;
    }

    /// Recorded calls in order, including those the current attempt has not reached.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Recorded entries the current attempt has not reached.
    pub fn unconsumed(&self) -> usize {
        self.entries.len() - self.cursor
    }

    /// The named error for a crossed bound, once one was crossed.
    pub fn overflow(&self) -> Option<&str> {
        self.overflow.as_deref()
    }

    /// Charge `bytes` more recorded data. `false` (and a remembered stop) past the bound.
    fn charge(&mut self, bytes: usize) -> bool {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > self.limits.bytes && self.overflow.is_none() {
            self.overflow = Some(format!("The script was stopped: its recorded host calls took more than {} MiB.", self.limits.bytes >> 20));
        }
        self.overflow.is_none()
    }

    fn reading_allowed(&mut self) -> bool {
        if self.clock.len() + self.random.len() >= self.limits.readings && self.overflow.is_none() {
            self.overflow = Some(format!("The script was stopped: it read the clock or Math.random() more than {} times.", self.limits.readings));
        }
        self.overflow.is_none() && self.charge(8)
    }

    /// Whether `calls` more calls, recording about `bytes` with their results, still fit: for
    /// calls that must not be split, so the first is not made when the second would be refused.
    /// Calls already recorded (a replay) fit.
    ///
    /// # Errors
    /// The named error of the bound they would cross.
    pub fn room_for(&self, calls: usize, bytes: usize) -> Result<(), String> {
        if let Some(m) = &self.overflow {
            return Err(m.clone());
        }
        let live = (self.cursor + calls).saturating_sub(self.entries.len());
        if self.entries.len() + live > self.limits.entries {
            return Err(format!("The script was stopped: it made more than {} host calls (output lines, dialogs and commands).", self.limits.entries));
        }
        if live > 0 && self.bytes.saturating_add(live * ENTRY_BYTES).saturating_add(bytes) > self.limits.bytes {
            return Err(format!("The script was stopped: its recorded host calls took more than {} MiB.", self.limits.bytes >> 20));
        }
        Ok(())
    }

    /// Start the next call.
    ///
    /// # Errors
    /// A divergence message when the tape holds a different call at this position, or the
    /// named error of a crossed bound.
    pub fn begin(&mut self, call: Call) -> Result<Begin, String> {
        if let Some(m) = &self.overflow {
            return Err(m.clone());
        }
        let call = Recorded::from(call);
        let at = self.cursor;
        match self.entries.get(at) {
            Some(e) if e.call == call => {
                self.cursor += 1;
                Ok(Begin::Replay(e.outcome.clone()))
            }
            Some(e) => {
                Err(crate::value::cut(&format!("replay divergence at call {at}: recorded {:?}, script asked for {:?}", e.call, call), MAX_DIVERGENCE_CHARS))
            }
            None if self.entries.len() >= self.limits.entries => {
                let m = format!("The script was stopped: it made more than {} host calls (output lines, dialogs and commands).", self.limits.entries);
                self.overflow = Some(m.clone());
                Err(m)
            }
            None => {
                if let Recorded::Output { len, .. } = &call {
                    self.output = self.output.saturating_add(*len);
                    if self.output > self.limits.output {
                        let m = format!("The script was stopped: it wrote more than {} MiB of output.", self.limits.output >> 20);
                        self.overflow = Some(m.clone());
                        return Err(m);
                    }
                }
                if let Recorded::Window { len, .. } = &call {
                    self.windows = self.windows.saturating_add(*len);
                    if self.windows > MAX_WINDOW_BYTES {
                        let m = format!("The script was stopped: it sent more than {} MiB of window updates.", MAX_WINDOW_BYTES >> 20);
                        self.overflow = Some(m.clone());
                        return Err(m);
                    }
                }
                if !self.charge(ENTRY_BYTES + call.bytes()) {
                    return Err(self.overflow.clone().unwrap_or_default());
                }
                self.cursor += 1;
                // Placeholder outcome until the live call finishes.
                self.entries.push(Entry { call, outcome: Outcome::Modal(None) });
                Ok(Begin::Live(at))
            }
        }
    }

    /// Record how a live call ended. Past the byte bound the run is stopped at its next call.
    pub fn finish(&mut self, at: usize, outcome: Outcome) {
        self.charge(outcome.bytes());
        if let Some(e) = self.entries.get_mut(at) {
            e.outcome = outcome;
        }
    }

    /// The open modal waiting for an answer, if any (always the last entry).
    pub fn pending_modal(&self) -> Option<(usize, &Entry)> {
        let at = self.entries.len().checked_sub(1)?;
        let e = self.entries.get(at)?;
        matches!(e.outcome, Outcome::Modal(None)).then_some((at, e))
    }

    /// Answer the pending modal. Past the byte bound the run is stopped when it resumes.
    ///
    /// # Errors
    /// When no modal is waiting.
    pub fn answer(&mut self, value: Value) -> Result<(), String> {
        let at = self.pending_modal().map(|(i, _)| i).ok_or("no dialog is waiting for an answer")?;
        self.charge(json_bytes(&value));
        self.entries.get_mut(at).ok_or("no dialog is waiting for an answer")?.outcome = Outcome::Modal(Some(value));
        Ok(())
    }

    /// The wall-clock reading for `Date`: recorded, or `live()` recorded now. Past the reading
    /// bound the live value is returned unrecorded and [`Tape::overflow`] is set.
    pub fn clock(&mut self, live: impl FnOnce() -> i64) -> i64 {
        let v = match self.clock.get(self.clock_cursor) {
            Some(&v) => v,
            None => {
                let v = live();
                if self.reading_allowed() {
                    self.clock.push(v);
                }
                v
            }
        };
        self.clock_cursor += 1;
        v
    }

    /// The next recorded `Math.random()` value, if this attempt has not used them all.
    pub fn take_random(&mut self) -> Option<f64> {
        let v = self.random.get(self.random_cursor).copied()?;
        self.random_cursor += 1;
        Some(v)
    }

    /// Record a live `Math.random()` value. Past the reading bound it is not recorded and
    /// [`Tape::overflow`] is set.
    pub fn push_random(&mut self, v: f64) {
        if self.reading_allowed() {
            self.random.push(v);
        }
        self.random_cursor += 1;
    }
}

/// The first `max` characters of `s`, with a marker when it was longer.
#[cfg(test)]
mod tests {
    use super::{Begin, Call, MAX_BYTES, MAX_ENTRIES, MAX_READINGS, Outcome, Tape};
    use serde_json::json;

    fn cmd(id: &str) -> Call {
        Call::Command { id: id.into(), params: json!({}) }
    }

    #[test]
    fn total_output_is_bounded() {
        let mut t = Tape::default();
        t.limits.output = 10;
        let Ok(Begin::Live(at)) = t.begin(Call::Output("12345".into())) else { panic!("live") };
        t.finish(at, Outcome::Return(json!(null)));
        let Ok(Begin::Live(at)) = t.begin(Call::Output("12345".into())) else { panic!("live") };
        t.finish(at, Outcome::Return(json!(null)));
        let m = "The script was stopped: it wrote more than 0 MiB of output.";
        assert_eq!(t.begin(Call::Output("1".into())).err().as_deref(), Some(m));
        // Every later call fails with it too.
        assert_eq!(t.begin(cmd("a")).err().as_deref(), Some(m));
        // A replayed line is not charged again.
        let mut t = Tape::default();
        t.limits.output = 5;
        let Ok(Begin::Live(at)) = t.begin(Call::Output("12345".into())) else { panic!("live") };
        t.finish(at, Outcome::Return(json!(null)));
        t.rewind();
        assert!(matches!(t.begin(Call::Output("12345".into())), Ok(Begin::Replay(_))));
    }

    #[test]
    fn replays_recorded_calls_and_detects_divergence() {
        let mut t = Tape::default();
        let Ok(Begin::Live(0)) = t.begin(cmd("a")) else { panic!("first call is live") };
        t.finish(0, Outcome::Return(json!(1)));
        t.rewind();
        let Ok(Begin::Replay(Outcome::Return(v))) = t.begin(cmd("a")) else { panic!("replayed") };
        assert_eq!(v, json!(1));
        t.rewind();
        assert!(t.begin(cmd("b")).is_err(), "a different call at a recorded position diverges");
    }

    #[test]
    fn output_is_kept_as_a_hash_and_still_replays() {
        let mut t = Tape::default();
        let line = "x".repeat(1024);
        let Ok(Begin::Live(0)) = t.begin(Call::Output(line.clone())) else { panic!("live") };
        t.finish(0, Outcome::Return(json!(null)));
        assert!(format!("{:?}", t.entries()).len() < 200);
        t.rewind();
        assert!(matches!(t.begin(Call::Output(line)), Ok(Begin::Replay(_))));
        t.rewind();
        let err = t.begin(Call::Output("y".repeat(100_000))).err().unwrap_or_default();
        assert!(err.starts_with("replay divergence at call 0") && err.len() < 600, "{err}");
    }

    #[test]
    fn too_many_entries_stop_the_run() {
        let mut t = Tape::with_limits(3, MAX_BYTES, MAX_READINGS);
        for i in 0..3 {
            assert!(matches!(t.begin(cmd("a")), Ok(Begin::Live(at)) if at == i));
        }
        let stop = "The script was stopped: it made more than 3 host calls (output lines, dialogs and commands).";
        assert_eq!(t.begin(cmd("a")).err().as_deref(), Some(stop));
        t.rewind();
        assert_eq!(t.begin(cmd("a")).err().as_deref(), Some(stop), "every later call fails too");
        assert_eq!(t.overflow(), Some(stop));
    }

    #[test]
    fn too_many_bytes_stop_the_run() {
        let stop = Some("The script was stopped: its recorded host calls took more than 1 MiB.");
        let mut t = Tape::with_limits(MAX_ENTRIES, 1 << 20, MAX_READINGS);
        let big = Call::Command { id: "a".into(), params: json!("x".repeat(1 << 20)) };
        assert_eq!(t.begin(big).err().as_deref(), stop);
        // A large outcome stops the run at its next call.
        let mut t = Tape::with_limits(MAX_ENTRIES, 1 << 20, MAX_READINGS);
        let Ok(Begin::Live(0)) = t.begin(cmd("a")) else { panic!("live") };
        t.finish(0, Outcome::Return(json!("y".repeat(1 << 20))));
        assert_eq!(t.overflow(), stop);
        assert_eq!(t.begin(cmd("b")).err().as_deref(), stop);
        // So does a large dialog answer.
        let mut t = Tape::with_limits(MAX_ENTRIES, 1 << 20, MAX_READINGS);
        let Ok(Begin::Live(0)) = t.begin(Call::Modal { kind: crate::ModalKind::Prompt, args: json!([]) }) else { panic!("live") };
        assert!(t.answer(json!("z".repeat(1 << 20))).is_ok());
        assert_eq!(t.overflow(), stop);
    }

    /// Room is for the calls still to be recorded: two calls fit while two entries are left, not
    /// when one is, and calls a replay already holds always fit.
    #[test]
    fn room_counts_only_the_calls_still_to_record() {
        let mut t = Tape::with_limits(3, MAX_BYTES, MAX_READINGS);
        let Ok(Begin::Live(0)) = t.begin(cmd("a")) else { panic!("live") };
        t.finish(0, Outcome::Return(json!(null)));
        assert!(t.room_for(2, 0).is_ok());
        let Ok(Begin::Live(1)) = t.begin(cmd("b")) else { panic!("live") };
        t.finish(1, Outcome::Return(json!(null)));
        assert!(t.room_for(2, 0).is_err(), "one entry left");
        assert!(t.room_for(1, 0).is_ok());
        t.rewind();
        assert!(t.room_for(2, 0).is_ok(), "both are recorded");
        assert!(Tape::with_limits(MAX_ENTRIES, 1 << 20, MAX_READINGS).room_for(2, 1 << 20).is_err(), "past the bytes");
    }

    #[test]
    fn too_many_readings_stop_recording() {
        let mut t = Tape::with_limits(MAX_ENTRIES, MAX_BYTES, 3);
        t.clock(|| 1);
        t.push_random(0.5);
        t.clock(|| 2);
        assert_eq!(t.overflow(), None);
        assert_eq!(t.clock(|| 3), 3, "the live value is still returned");
        t.push_random(0.25);
        assert_eq!((t.clock.len(), t.random.len()), (2, 1), "nothing more is recorded");
        let stop = "The script was stopped: it read the clock or Math.random() more than 3 times.";
        assert_eq!(t.overflow(), Some(stop));
        assert_eq!(t.begin(cmd("a")).err().as_deref(), Some(stop));
    }

    #[test]
    fn clock_and_random_are_stable_across_attempts() {
        let mut t = Tape::default();
        t.push_random(0.25);
        let a = (t.clock(|| 5), 0.25);
        t.rewind();
        let b = (t.clock(|| 99), t.take_random().unwrap());
        assert_eq!(a, b);
        assert_eq!(t.clock(|| 7), 7, "past the recorded readings the live value is used");
    }
}
