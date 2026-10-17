//! Headless PhotoCraft command line. `run` is the whole program, so it can be
//! tested in-process as well as through the binary.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::Write;

mod actions;
mod args;
mod automation;
mod batch;
mod document;
mod output;

pub use actions::{Steps, load_actions, load_actions_file, parse_actions};
pub use args::USAGE;
use args::{SUBCOMMANDS, parse};

pub(crate) type R = Result<(), String>;

/// [`run`] on the raw process arguments. An argument that is not valid Unicode is a usage error
/// naming its position and a lossy rendering (exit 2), never a panic: `std::env::args()` aborts
/// the process on one (issue #1108).
pub fn run_os(args: &[std::ffi::OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let mut strings = Vec::with_capacity(args.len());
    for (i, a) in args.iter().enumerate() {
        match a.clone().into_string() {
            Ok(s) => strings.push(s),
            Err(raw) => {
                let _ = writeln!(err, "error: argument {} is not valid Unicode: `{}`\n\n{USAGE}", i + 1, raw.to_string_lossy());
                return 2;
            }
        }
    }
    run(&strings, out, err)
}

/// Run the CLI. Returns the process exit code (0 ok, 1 failure, 2 usage).
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let Some(cmd) = args.first() else {
        let _ = write!(err, "{USAGE}");
        return 2;
    };
    match cmd.as_str() {
        "-h" | "--help" | "help" => {
            let _ = write!(out, "{USAGE}");
            return 0;
        }
        "--version" | "version" => {
            let _ = writeln!(out, "photocraft-cli {}", photocraft_engine::build_info::long_version());
            return 0;
        }
        _ => {}
    }
    let Some(sub) = SUBCOMMANDS.iter().find(|s| s.name == cmd) else {
        let _ = writeln!(err, "error: unknown command `{cmd}`\n\n{USAGE}");
        return 2;
    };
    let parsed = match parse(sub, &args[1..]) {
        Ok(Some(p)) => p,
        // `<subcommand> --help`: usage, without reading inputs or starting a server.
        Ok(None) => {
            let _ = write!(out, "{USAGE}");
            return 0;
        }
        Err(e) => {
            let _ = writeln!(err, "error: {e}\n\n{USAGE}");
            return 2;
        }
    };
    match (sub.run)(&parsed, out, err) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "error: {e}");
            1
        }
    }
}
