# Scripting: what runs where, and how it is tested

PhotoCraft runs `.js` / `.jsx` automation scripts (`crates/script-js`) from **File › Scripts ›
Browse…** on the desktop. This page says how a script runs, what it can do so far, and what is
tested, platform by platform, so nobody mistakes a compile check for a test. Update it when a
platform gains a test run or a feature.

## What a script can do so far

- Write output with `$.writeln`. It shows in the **Script Output** window, with the message of a
  run that failed.
- Ask the user with `alert`, `confirm` and `prompt`. They open as the app's own dialogs, so the
  keyboard, screen readers and automation (`ui.inspect`, `ui.dialog.set`, `ui.dialog.confirm`,
  `ui.dialog.cancel`) handle them as any other. Their title always starts "Script: <name>", so a
  script's own title can't make one pass for the app's.
- Read `app.name`, `app.version`, `$.os`, `app.documents` and `app.activeDocument`. A
  `Document` has `name`, `resolution` and `saved`, and setting `app.activeDocument` switches to
  it.

A busy run offers **Stop** after half a second, and each of a script's dialogs has **Stop
Script**, so a script that keeps asking can still be ended. While a script's dialog is open, the
app runs no other command, as under any of its own dialogs; the automation channel included.
While the app is in another dialog (or another modal state, such as a pending crop), the script's
next command waits, as a click would; the app notices a run that ended meanwhile (its host died)
once it is out of that state.

`file.scripts.run` (`{"path"}` or `{"name", "source"}`) starts a run from the menus, a key or
the automation channel. An agent under a workspace policy can't start one yet: the policy refuses
`file.*` commands it doesn't list, and it judges answering a script's dialog as running a
script, so such an agent can't answer one either. `file.scripts.browse` of a `.js` or `.jsx` file
runs it the same way. The headless engine (`photocraft-cli`) has no script host, so
there the command fails and says the run needs the app.

## How a script runs

| Platform | How a script runs | Its limits |
|---|---|---|
| Linux | In a script host process of its own (`photocraft-script-host`, next to the app), started per run | Time (the app's watchdog); memory (the host's heap cap, `RLIMIT_DATA`, and the app polls resident size); process group stopped with the run |
| macOS | As on Linux | Time; memory by the heap cap and resident-size polling (the kernel enforces neither `RLIMIT_DATA` nor `RLIMIT_AS`) |
| Windows | As on Linux | Time; memory by the heap cap and resident-size polling (a job-object memory limit would need unsafe code); a job object ends the host with the app |
| FreeBSD | As on Linux | Time; memory by the heap cap and resident-size polling; process group stopped with the run |
| 32-bit systems | As above, with a 64 MiB script stack (256 MiB on 64-bit) and a 1 GiB memory limit (2 GiB on 64-bit), to fit 2 GiB of address space | As above |
| Web (wasm) | Scripts don't run in the browser yet | — |

**Isolation, not containment.** The host starts with an empty environment and the temporary
folder as its working directory. It runs with the user's rights: the separate process keeps a
crash, a runaway loop or a runaway allocation away from the app, but it is not a sandbox, and
a memory-safety bug in the script engine could reach what the user can.

**The memory limit.** Only the script host counts its allocations: once a run starts, an
allocation that would take its heap past the run's limit (less the script stack) fails at once.
Where the engine allocates fallibly (an `ArrayBuffer`, some strings) the script gets a
`RangeError`; anywhere else the host ends with a named memory failure. The app polls the host's
resident size as a second line. The app's own allocator is the system's, unchanged.

**Other limits.** The run's clock stops while the app answers a command or the user a dialog,
but a command answer may take at most 30 minutes and an open dialog at most 12 hours before the
run ends. A run may compute for 30 minutes in all. Commands, dialogs and output lines together
are capped in number per run, and output at 32 MiB. The app gives one run's commands at most 30
minutes of its own time.

A script reaches only the commands in `script-js/src/commands.rs`, each parameter checked by the
host and again by the app, and its commands go through the app's own command path
(`ui-egui/src/scripts.rs`): a run started through automation keeps automation's gates (the
app's and the engine's) for every command, and every run's commands are programmatic, as `engine.execute`'s are (they answer at once
and open no dialog).

**Nightly builds** carry only the app, so a script run there says the script host is missing.

## Building it yourself

The script host is a binary of the app's package (`apps/photocraft/src/bin`):
`cargo build -p photocraft` builds both, and the app finds the host next to itself. A
`cargo run -p photocraft` builds only the app, so run `cargo build -p photocraft` once first.

## What is tested

| Platform | Unit and integration tests (`photocraft-script-js`) | Script host process tests (`tests/it/isolate.rs`) | App tests (`ui-egui` scripts; the desktop service and the shipped host, `photocraft` `tests/it/script_host.rs`) | By hand |
|---|---|---|---|---|
| Linux x86_64 | CI (gate) and locally | CI (gate) and locally | CI (gate) and locally | Not yet |
| Windows x64 | CI (Windows unit and integration jobs) | CI (Windows integration job; the Unix-only ones are skipped) | CI (Windows unit and integration jobs) | Not yet |
| macOS | Not run on pull requests | Not run | Not run | Not yet |
| Windows ARM64 and x86 | Compile-checked only | Not run | Not run | Not yet |
| FreeBSD | Not run on pull requests | Not run | Not run | Not yet |

The tests run scripts against a real session (`tests/it/run.rs`), against hostile script hosts
that forge or flood messages (`tests/it/isolate.rs`), through the app with a fake script service
(`ui-egui/src/scripts/tests.rs`), and end to end through the host PhotoCraft ships.

## Scripts that check the app's name

`app.name` is "PhotoCraft" and `app.version` is "15.0.0", the scripting interface's
compatibility level, so version checks pass.

## The Boa engine

Scripts run on Boa (`boa_engine`, MIT OR Unlicense), pinned exactly (`=0.22.0`, with `boa_gc`):
stopping a run relies on 0.22 treating every engine error as uncatchable
(`script-js/src/abort.rs`, whose tests check it). On a Boa security advisory: bump both crates
together, run `cargo test -p photocraft-script-js` (the abort and isolate suites are what a Boa
change breaks), and ship it as its own pull request. A script that breaks the engine still only
breaks its own host process.

## Still to do

- Run the tests on macOS and FreeBSD, and by hand on every platform.
- The document model, ScriptUI windows, the Scripts Manager, script steps in actions, and scripts
  in the browser, each in its own pull request.
