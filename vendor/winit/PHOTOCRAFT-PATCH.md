# Local macOS Korean first-key repair

Base: crates.io winit 0.30.13, unmodified source except the files listed below.
License: Apache-2.0 (see LICENSE). This is a third-party dependency, not a member of
PhotoCraft's Rust workspace. No unsafe code is added to PhotoCraft's own crates.

Changed files: src/platform_impl/macos/{mod.rs,view.rs,ime_startup.rs}.
The existing native AppKit backend reinterprets the same retained key event once
when the initial Apple Korean IME response consists solely of insertText with the
same single compatibility jamo as the event. That callback has not yet emitted
an application event. A successful retry starts ordinary preedit; an unsuccessful
retry falls through to the original keyboard dispatch once. It never recombines
text, waits, schedules a timer, or queues keys. Recording is disarmed before the
retry; source/focus/IME changes, nested key events, marked text, other callbacks,
repeats and Command/Control/Option combinations exclude retries.

References: https://github.com/rust-windowing/winit/issues/3095
and https://github.com/rust-windowing/winit/pull/4693 (open proposal; not copied).
The immediate retry was independently reproduced on macOS 27.0.1 using a minimal
eframe 0.36.2 TextEdit and PhotoCraft's real Type tool, including a fresh application
identifier for each A/B/A trial. A delay/daemon-startup cause is not assumed.

Tests: cargo test --manifest-path vendor/winit/Cargo.toml --lib
Native checks require macOS and the Apple Korean 2-set input source: launch a fresh
bundle, type g k s r m f and Space, and inspect the resulting text: 한글 plus one
space. A stock build produced ㅎㅏㄴ; the repaired build composed 한 on first input.
Also check single-jamo+Space, double consonants, Backspace, Cmd+A, paste of literal
jamo, and Korean→English→Korean. Keep this patch independent of UI translation
and PhotoCraft's event-order fixes. Replace this vendor override when a released,
verified upstream fix is available; do not remove it merely on a version bump.
