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
up to 30 seconds at a time. Commands that render the whole canvas refuse one too large to hold, with
that reason, rather than try to hold more than the app's memory: Flatten, the stamps, Copy
Merged, Background From Layer, Match Color's source and printing past the budget a command's
working buffers may take (what Image Size resamples within: 8 GiB, 2 GiB on 32-bit systems
and in the browser); the merges past that budget or when a band they render in (the merged
area's width) would pass 2^30 pixels; Save a Copy, Indexed
Color, Trim and Duplicate Merged past 2^30 pixels (a 300000 px square document is far past
both).

**In the browser.** Each run gets its own Worker. The Worker posts its requests to the page and
waits for each answer in shared memory (`Atomics.wait`), so a run continues where it was after a
command or a dialog, as on the desktop. The page checks every message as it arrives: the command
and its parameters, dialogs, windows, the per-run bounds, and a Worker that sends while it should
wait. The page counts the run's time from the messages' arrival, with the clock stopped while
the Worker waits, so a hidden tab drawing no frames does not eat a run's time. It checks the
limit each time it polls the run, every frame while one runs, and terminates the Worker on the
first poll past it; while the page draws no frames the Worker can compute on past the limit
until the next one. A poll checks at most 64 of the Worker's messages, for at most 2 ms, so a large
batch is spread over frames; while some wait, each is checked against the limit by when it arrived, and the run
clock is read again once they are through. A crash in the Worker (a failed allocation, a stack overflow, an engine
panic) ends that run only. The Worker's stack in its memory is 64 MiB, as the script host's; the browser's own call stack still limits how deeply a script nests there, a little less deeply than on the desktop. A command's answer to
a script holds at most 32 MiB there (the shared memory the Worker reads it from).

On every platform a script reaches only the commands in `script-js/src/commands.rs`, each
parameter checked, writes only to files and folders the user chose in that run
(`script-js/src/grants.rs`), and its commands go through the app's own command path with a
per-run budget (`ui-egui/src/scripts.rs`).

## What is tested

| Platform | Unit and integration tests (`photocraft-script-js`) | Script host process tests (`tests/it/isolate.rs`) | App tests (`ui-egui` scripts, actions) | By hand |
|---|---|---|---|---|
| Linux x86_64 | CI (gate) and locally | CI (gate) and locally | CI (gate) and locally | Not yet |
| Windows x64 | CI (Windows unit and integration jobs) | CI (Windows integration job; the Unix-only ones are skipped) | CI (Windows unit job) | Not yet |
| macOS | Not run on pull requests | Not run | Not run | Not yet |
| Windows ARM64 and x86 | Compile-checked only | Not run | Not run | Not yet |
| FreeBSD | Not run on pull requests | Not run | Not run | Not yet |
| Web (wasm) | The page's side (`isolate/web_job.rs`) on every platform, with a fake Worker; the Worker itself is compile-checked for wasm32 by the web build job | — | The web app's script service with a fake Worker (`photocraft-web/src/scripts.rs`) | Not yet |

The tests run scripts against a real session (`tests/it/dom.rs`, `tests/it/run.rs`), against hostile
script hosts that forge or flood messages (`tests/it/isolate.rs`), and through the app with a fake
script service (`ui-egui/src/scripts/tests.rs`). Published community scripts run unchanged
in `tests/it/community.rs`:

- **Export Layer Coordinates** runs to its end: it asks for a folder, walks the layers into
  groups, and reports. It also writes a text file, and scripts can't do that yet. The script
  swallows that error itself, so it reports success with no file written.
- **Export Layers To Files (Fast)** gets through its setup and stops where it reads its dialog's
  resource file next to itself. Scripts can't read files yet either.

Running them first turned up five missing pieces, now added: `app.bringToFront()`, `$.os`, `app`'s
properties named bare (`activeDocument`, `preferences`), `app.stringIDToTypeID` and
`doc.suspendHistory`. Trying more published scripts by hand added the layer lock properties
(`allLocked`, `pixelsLocked`, `positionLocked`, `transparentPixelsLocked`) and a list's `selection`
as an item (`list.selection = 0` selects the first item; reading it gives the item, with `.text`).
Most of the others tried stop at the Action Manager (`ActionReference`, `executeActionGet`), which
scripts don't have yet, or at `doc.save()`.

## Saves and the file system

A script saves only to a file or folder the user chose in that run. The check reads the path;
it also refuses a target that is a link (or, on Windows, a junction or other reparse point) and,
on Windows, a name with an alternate data stream (`photo.png:hidden`). A race remains: another
program that swaps a granted file for a link between the check and the write redirects the write.
A script has no other way to touch the file system, so it can't arrange that by itself.

In the browser a script can't save at all: the browser's file and folder pickers give back a bare
name (`out.png`), not a path, and a bare name grants nothing, so a save after the picker fails.

## Scripts that check the app's name

`app.name` is "PhotoCraft" and `app.version` is "15.0.0", the scripting interface's
compatibility level, so version checks pass. Many published scripts also refuse to run unless
`app.name` matches the application they were written for. For those, give the script a tag in the
Scripts Manager: a listed script's first tag is what it sees as `app.name` when run from the
list, so the check passes and nothing else about the run changes.

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
- Text file reads and writes for scripts, within the files the user chose in the run. The Action
  Manager (`ActionReference`, `executeAction`, `executeActionGet`) for the descriptor-based
  scripts, then more of them as fixtures, and ScriptUI resource strings.
- Try scripts by hand in a browser. In the browser a run's commands go about one per frame, since
  the page reads the Worker's messages between frames; a shared-memory channel both ways would
  let a frame take several.
- A CI run that starts the script host on 32-bit Windows, and the `script-js/fuzz` target in the
  fuzz job's list (both are CI changes, for `craft-common-actions`).
