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
use args::{Command, parse};
use automation::{commands, mcp, serve};
use batch::{batch, droplet};
use document::{convert, info, run_cmds};

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
                let usage = <args::Cli as clap::CommandFactory>::command().render_usage();
                let _ = writeln!(err, "error: argument {} is not valid Unicode: `{}`\n\n{usage}", i + 1, raw.to_string_lossy());
                return 2;
            }
        }
    }
    run(&strings, out, err)
}

/// Run the CLI. Returns the process exit code (0 ok, 1 failure, 2 usage).
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if matches!(args.first().map(String::as_str), Some("--version" | "version")) {
        let _ = writeln!(out, "photocraft-cli {}", photocraft_engine::build_info::long_version());
        return 0;
    }
    // Help (`help`, `-h`, `<subcommand> --help`) comes back as an error too, for `out`, exit 0: the
    // subcommand never runs.
    let cli = match parse(args) {
        Ok(cli) => cli,
        Err(e) => {
            let to: &mut dyn Write = if e.use_stderr() { err } else { out };
            let _ = write!(to, "{}", e.render());
            return e.exit_code();
        }
    };
    let result = match &cli.command {
        Command::Convert(a) => convert(a, out, err),
        Command::Info(a) => info(a, out),
        Command::Run(a) => run_cmds(a, out, err),
        Command::Batch(a) => batch(a, out, err),
        Command::Droplet(a) => droplet(a, out, err),
        Command::Commands(a) => commands(a, out),
        Command::Mcp(a) => mcp(a),
        Command::Serve(a) => serve(a, err),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "error: {e}");
            1
        }
    }
}
