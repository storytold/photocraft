# Scripting: what runs where, and how it is tested

PhotoCraft runs `.js` / `.jsx` automation scripts (`crates/script-js`). This page says, platform
by platform, how a script runs and what is tested, so nobody mistakes a compile check for a
test. Update it when a platform gains a test run or a feature.

## How a script runs

| Platform | How a script runs | Its limits |
|---|---|---|
| Linux | In a script host process of its own (`photocraft --script-host`), started per run | Time (the app's watchdog), memory (`RLIMIT_DATA` in the host, and the app polls resident size), process group stopped with the run |
| macOS | As on Linux | Time; memory by polling resident size (the kernel does not enforce `RLIMIT_DATA`) |
| Windows | As on Linux | Time; memory by polling resident size; a job object ends the host with the app. No job-object memory limit yet |
| FreeBSD | As on Linux | As on Linux |
| Web (wasm) | Not at all: scripts are off until they can run in a Worker the page can stop | — |

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
| Web (wasm) | `script-js` is compile-checked for wasm32 by the web build job; the web app no longer links it | — | — | — |

The tests run scripts against a real session (`tests/dom.rs`, `tests/run.rs`), against hostile
script hosts that forge or flood messages (`tests/isolate.rs`), and through the app with a fake
script service (`ui-egui/src/scripts/tests.rs`). No real-world community script has been run
end to end yet.

## Still to do

- Run the tests on macOS and FreeBSD, and by hand on every platform.
- Run real-world community scripts end to end and keep them as fixtures.
- A web Worker that holds the engine and that the page can stop, then scripts on the web.
