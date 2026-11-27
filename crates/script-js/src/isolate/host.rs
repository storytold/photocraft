//! The script host: the process that runs one script for the app.
//!
//! It reads [`ToHost`] frames on standard input and writes [`FromHost`] frames on standard
//! output. A reader thread forwards input to the run and ends the process when the input ends,
//! so a host never outlives its app. The run itself executes on a thread with a large explicit
//! stack ([`STACK`]).

use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};

use super::link::Link;
use super::protocol::{FromHost, MAX_FROM_HOST, MAX_TO_HOST, ToHost, read_frame, write_frame};

/// Stack of the script thread: the parser and some builtins recurse on nested input. Smaller on
/// a 32-bit system, whose 2 GiB of address space must also hold the heap.
pub const STACK: usize = if cfg!(target_pointer_width = "64") { 256 << 20 } else { 64 << 20 };

/// Exit status when the input was not a valid start of a run.
const BAD_INPUT: i32 = 2;

fn send(msg: &FromHost) -> Result<(), String> {
    let mut out = io::stdout().lock();
    match write_frame(&mut out, msg, MAX_FROM_HOST) {
        Ok(()) => Ok(()),
        // The app is gone; the reader thread ends the process.
        Err(e) => Err(e.to_string()),
    }
}

/// The channel to the app: frames on standard output, answers from the input reader thread.
struct StdioLink {
    inbox: Receiver<ToHost>,
}

impl Link for StdioLink {
    fn send(&mut self, msg: &FromHost) -> Result<(), String> {
        send(msg)
    }

    fn recv(&mut self) -> Result<ToHost, String> {
        self.inbox.recv().map_err(|_| "the app closed the run".to_owned())
    }
}

fn drive(source: &str, app_name: Option<String>, script_name: Option<String>, inbox: Receiver<ToHost>) -> Result<(), String> {
    super::link::run(Rc::new(RefCell::new(StdioLink { inbox })), source, app_name, script_name)
}

/// Serve one run on standard input and output; returns the process exit status. `heap` is the
/// executable's global allocator ([`super::host_allocator`]), which the run's memory limit
/// caps.
pub fn serve(heap: &'static super::HostAllocator) -> i32 {
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::Builder::new().name("script-host-input".into()).spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            match read_frame::<_, ToHost>(&mut input, MAX_TO_HOST) {
                Ok(Some(msg)) => {
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
                // The app closed the pipe or died: nothing can use this run any more.
                Ok(None) => std::process::exit(0),
                Err(e) => {
                    let _ = writeln!(io::stderr(), "script host: bad input: {e}");
                    std::process::exit(BAD_INPUT);
                }
            }
        }
    });
    if reader.is_err() {
        return BAD_INPUT;
    }
    let Ok(ToHost::Start { source, app_name, script_name, memory_limit }) = rx.recv() else {
        let _ = writeln!(io::stderr(), "script host: the first message was not a start");
        return BAD_INPUT;
    };
    let result = match super::limits::limit_memory(memory_limit, heap) {
        Err(e) => Err(format!("Couldn't limit the script's memory: {e}")),
        Ok(()) => match std::thread::Builder::new().name("script".into()).stack_size(STACK).spawn(move || drive(&source, app_name, script_name, rx)) {
            Err(e) => Err(format!("Couldn't start the script: {e}")),
            Ok(t) => t.join().unwrap_or_else(|_| Err("The script stopped unexpectedly: internal error in the script host.".into())),
        },
    };
    let _ = send(&FromHost::Finished { error: result.err() });
    // Wait for the app to close the pipe (the reader thread exits the process then).
    loop {
        std::thread::park();
    }
}
