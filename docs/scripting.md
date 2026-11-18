# Scripting: what runs where, and how it is tested

PhotoCraft runs `.js` / `.jsx` automation scripts (`crates/script-js`). This page says, platform
by platform, how a script runs and what is tested, so nobody mistakes a compile check for a
test. Update it when a platform gains a test run or a feature.

## How a script runs

| Platform | How a script runs | Its limits |
|---|---|---|
| Linux | In a script host process of its own (`photocraft --script-host`), started per run | Time (the app's watchdog); memory (the host's heap cap, `RLIMIT_DATA`, and the app polls resident size); process group stopped with the run |
| macOS | As on Linux | Time; memory by the heap cap and resident-size polling (the kernel enforces neither `RLIMIT_DATA` nor `RLIMIT_AS`) |
| Windows | As on Linux | Time; memory by the heap cap and resident-size polling (a job-object memory limit would need unsafe code); a job object ends the host with the app |
| FreeBSD, OpenBSD | As on Linux | Time; memory by the heap cap and resident-size polling; process group stopped with the run |
| 32-bit systems | As above, with a 64 MiB script stack and a 1 GiB default limit, to fit 2 GiB of address space | As above |
| Web (wasm) | In a Worker of its own (`photocraft-script-worker`), started per run; the page checks every request the Worker makes as the desktop app checks a script host's (`isolate/web_job.rs`). Needs a cross-origin-isolated page (`packaging/web/README.md`); otherwise scripts stay off and say why | Time (the page counts it and terminates the Worker); memory (the Worker's heap cap, 1 GiB) |

**The memory limit.** The host's heap is capped on every platform: the executable installs a
counting allocator, and once a run starts, an allocation that would take the heap past the run's
limit (less the script stack) fails at once: where the engine allocates fallibly (an
`ArrayBuffer`, some strings) the script gets a `RangeError`; anywhere else the host ends with a
named memory failure. Memory reserved but not yet touched counts too, so a fast allocation can't
get past it between polls.
The app also polls the host's resident size as a second line. The cap costs the app one atomic
counter update per allocation; the app itself sets no limit.

**Other limits.** The run's clock stops while the app answers a command or the user a dialog,
but a command answer may take at most 30 minutes (what the app gives all of a run's commands)
and an open dialog at most 12 hours before the run ends. Commands, dialogs, output lines and
window updates together are capped in number per run; output lines at 32 MiB, window updates at
256 MiB. Waiting for a busy interface to take the run's output does not count against its time,
up to 30 seconds at a time.

**In the browser.** Each run gets its own Worker. The Worker posts its requests to the page and
waits for each answer in shared memory (`Atomics.wait`), so a run continues where it was after a
command or a dialog, as on the desktop. The page checks every message as it arrives: the command
and its parameters, dialogs, windows, the per-run bounds, and a Worker that sends while it should
wait. The page counts the run's time from the messages' arrival, with the clock stopped while
the Worker waits, so a hidden tab drawing no frames does not eat a run's time, and terminates the
Worker at the limit. A crash in the Worker (a failed allocation, a stack overflow, an engine
panic) ends that run only.

On every platform a script reaches only the commands in `script-js/src/commands.rs`, each
parameter checked, writes only to files and folders the user chose in that run
(`script-js/src/grants.rs`), and its commands go through the app's own command path with a
per-run budget (`ui-egui/src/scripts.rs`).

## What is tested

| Platform | Unit and integration tests (`photocraft-script-js`) | Script host process tests (`tests/isolate.rs`) | App tests (`ui-egui` scripts, actions) | By hand |
|---|---|---|---|---|
| Linux x86_64 | CI (gate) and locally | CI (gate) and locally | CI (gate) and locally | Not yet |
| Windows x64 | CI (Windows unit and integration jobs) | CI (Windows integration job; the Unix-only ones are skipped) | CI (Windows unit job) | Not yet |
| macOS | Not run on pull requests | Not run | Not run | Not yet |
| Windows ARM64 and x86 | Compile-checked only | Not run | Not run | Not yet |
| FreeBSD | Not run on pull requests | Not run | Not run | Not yet |
| Web (wasm) | The page's side (`isolate/web_job.rs`) on every platform, with a fake Worker; the Worker itself is compile-checked for wasm32 by the web build job | — | The web app's script service with a fake Worker (`photocraft-web/src/scripts.rs`) | Not yet |

The tests run scripts against a real session (`tests/dom.rs`, `tests/run.rs`), against hostile
script hosts that forge or flood messages (`tests/isolate.rs`), and through the app with a fake
script service (`ui-egui/src/scripts/tests.rs`). No real-world community script has been run
end to end yet.

## Saves and the file system

A script saves only to a file or folder the user chose in that run. The check reads the path;
it also refuses a target that is a link (or, on Windows, a junction or other reparse point) and,
on Windows, a name with an alternate data stream (`photo.png:hidden`). A race remains: another
program that swaps a granted file for a link between the check and the write redirects the write.
A script has no other way to touch the file system, so it can't arrange that by itself.

## Actions and replays

Recording an action records a script step as the script's file path or, for a script kept in the
Scripts Manager without a file, its text (at most 256 KiB there); never its tags or its place in
the list. The
commands the script runs are not recorded: playing the action runs the script again. The replay
tape that lets a run continue after a dialog lives only in memory for that run and is never
saved: output lines on it are kept as hashes from Rust's `DefaultHasher`, which is not stable
between Rust versions.

## The Boa engine

Scripts run on Boa (`boa_engine`, MIT OR Unlicense), pinned exactly (`=0.22.0`, with `boa_gc`)
because the dialog replay depends on its behaviour (`script-js/src/abort.rs`). On a Boa
security advisory: bump both crates together, run `cargo test -p photocraft-script-js` (the
replay, abort and isolate suites are what a Boa change breaks), and ship it as its own pull
request. A script that breaks the engine still only breaks its own host process.

## Still to do

- Run the tests on macOS and FreeBSD, and by hand on every platform.
- Run real-world community scripts end to end and keep them as fixtures.
- Try scripts by hand in a browser. In the browser a run's commands go about one per frame, since
  the page reads the Worker's messages between frames; a shared-memory channel both ways would
  let a frame take several.
- A CI run that starts the script host on 32-bit Windows, and the `script-js/fuzz` target in the
  fuzz job's list (both are CI changes, for `craft-common-actions`).
