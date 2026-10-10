# macOS first-key Korean IME recovery

On a fresh macOS app launch with Apple Korean 2-set, the first key may arrive as raw `insertText("ㅎ")` instead of `setMarkedText("ㅎ")`. winit 0.30.13 then forwards ordinary text; typing g/k/s produces ㅎㅏㄴ. The same retained event, interpreted one more time, starts normal preedit and produces 한.

### Native boundary repair

Detect only the initial Apple Korean batch containing exactly one insertText callback with the same single compatibility jamo as the original event. Reinterpret that retained event synchronously once, before any ordinary keyboard event is emitted. Existing marked text, other callbacks, modifiers/repeats, nested keys, or changed source/focus/IME state exclude replay. The batch is removed and the attempt is consumed before re-entering AppKit; no borrow crosses native calls. A failed retry falls through to the existing keyboard dispatch once.

No timer, delayed key queue, user-text recomposition or input logging is added. `docs/macos-korean-ime.md` documents reproduction, the decision boundary and limitations.

### Dependency integration decision — draft for review

This draft temporarily vendors the crates.io winit 0.30.13 source under Apache-2.0 and changes only three macOS source files. The dependency is excluded from PhotoCraft's workspace. The large source diff mostly consists of the unchanged dependency; the actual repair is in `ime_startup.rs`, `view.rs` and the module declaration. The manifest override affects all targets, but behavioral changes are macOS-only.

Related upstream work: rust-windowing/winit#3095 and open PR rust-windowing/winit#4693 at `4a2da8232e0d6211a1322ad9c33d45729bc02bfb`. That proposal uses a delayed retry. This independently tested alternative retries immediately and does not reuse its timer/queue implementation. Please review whether PhotoCraft should carry a temporary vendor override or wait for a released upstream fix. Remove the override only after verifying an upstream replacement.

### Evidence and limits

On macOS 27.0.1 / eframe 0.36.2 / Apple Korean 2-set, with a fresh bundle identifier for each trial:

| Trial | First g/k/s input |
|---|---|
| Stock minimal egui TextEdit | ㅎㅏㄴ |
| Immediate-retry minimal TextEdit | 한 |
| Second stock minimal TextEdit | ㅎㅏㄴ |

The earlier combined PhotoCraft build and final Korean bundle both accepted `한글 ` on their first native input with Inter 48 pt, without deletion/retyping. Double consonants, composing Backspace, Cmd+A, literal jamo paste and Korean→English→Korean were checked. These native observations predate this branch's rebase to current 0.3.0 `main`; they are not a claim of a new native run on this rebased branch. Seven policy regression tests are included.

The exact OS-internal reason for the initial fallback is not established; the fix does not depend on asserting a daemon timeout. Other macOS versions/input sources remain unverified. This does not fix PSD font matching or claim to resolve all of PhotoCraft #456.

### Branch verification

- The seven startup-policy tests pass when compiled from this branch's source.
- Earlier combined build: 3,225 workspace tests and 14 winit library tests passed; these counts are historical evidence, not a fresh full-suite result for this standalone 0.3.0 branch.
- Reproduce dependency tests with `cargo test --manifest-path vendor/winit/Cargo.toml --lib --locked`. Build the minimal native probe with `cargo build -p photocraft-ui-egui --example ime_minimal --locked`.
- A fresh native run on the rebased branch and maintainer approval of the dependency integration remain review gates.
