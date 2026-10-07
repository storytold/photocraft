# Type-tool IME editing boundaries

An IME owns the temporary marked range while composing a syllable or word.
PhotoCraft replaces that range for each preedit update and makes it ordinary
text on commit. This patch changes that event boundary; it does not implement
a Korean input method.

## Behaviors covered

| Event sequence | Expected result |
|---|---|
| Empty preedit with no active composition and text selected | Selection and text are preserved |
| Preedit ㅎ → 하 → 한, then commit | Exactly 한 remains |
| Navigation before preedit in one frame | Navigation runs before the new marked range is created |
| Navigation after commit in one frame | Navigation runs on committed text |
| Backspace after preedit clears | Ordinary deletion resumes |
| Text/edit shortcuts during active preedit | The IME retains ownership of the marked range |

The guard reads current editing state for every event, rather than deciding
ownership once for the whole frame. A host's empty preedit notification must not
replace a selection when there is no marked range to clear.

## Verification

Run `cargo test -p photocraft-ui-egui --locked` and
`cargo clippy -p photocraft-ui-egui --all-targets --locked -- -D warnings`.
Regression cases are in `type_tool.rs`. The full-shell characterization test in
`type_tool_tests.rs` verifies that creating text enables IME immediately and that
synthetic preedit updates do not request a composition reset.

These synthetic tests begin with already-correct IME events. They cannot prove
that macOS delivered the correct first event. The separate native first-key
problem was reproduced in a plain egui TextEdit without PhotoCraft's Type tool;
fixing it requires the native input boundary. Do not claim this change alone
fixes all Korean input or PhotoCraft issue #456.

For native review, use a synthetic document and test a selection, composition,
commit, arrow keys and Backspace with Apple Korean 2-set. Also test another IME
and normal English shortcuts before generalizing across platforms. No UI
translation, font lookup change or dependency override belongs in this patch.
