# Gates: PR 9645 integration

Scope: Rebase web autosave and clarify total-storage errors without deleting snapshots.

- [x] G1: Upstream crash hooks and autosave recovery coexist after rebase.
  EVIDENCE: web.rs retains keep_panic_message, listen_for_trap, and save_when_hidden; Cargo.toml retains ErrorEvent, PromiseRejectionEvent, VisibilityState. Parent independently reviewed.
- [x] G2: Old-session quota regression passes.
  CHECK: cargo test -p photocraft-web total_limit_guidance_reaches_the_user_when_old_sessions_are_not_claimed
  EXPECT: test result: ok
  EVIDENCE: Independent parent regression: 1 passed; 0 failed; 35 filtered.
- [x] G3: Required crate tests pass.
  CHECK: cargo test -p photocraft-web
  EXPECT: test result: ok
  EVIDENCE: cargo test -p photocraft-web: 36 passed; 0 failed; 0 ignored.
- [x] G4: Clippy passes.
  CHECK: cargo clippy -p photocraft-web --all-targets -- -D warnings
  EXPECT: Finished
  EVIDENCE: cargo clippy -p photocraft-web --all-targets -- -D warnings: exit 0, Finished dev profile.
- [x] G5: Layer check passes.
  CHECK: cargo xtask layers
  EXPECT: layers
  EVIDENCE: cargo xtask layers: exit 0, OK: 31 crates, no layering violations.
- [x] G6: Wasm check passes.
  CHECK: cargo xtask wasm
  EXPECT: Finished
  EVIDENCE: cargo xtask wasm: exit 0, 24 crates and photocraft-codecs --features heif passed; existing dead-code warnings.
- [x] G7: Wiki index and devlog describe change and verification status.
  EVIDENCE: wiki/INDEX.md, wiki/pr-9645-web-recovery.md, log/devlog.md.

- [x] G8: Final source formatting matches CI.
  CHECK: cargo fmt --all -- --check
  EXPECT: /^$/
  EVIDENCE: Full workspace formatting check exits 0 after formatting TOTAL_LIMIT_MSG; CI job 128081392382 identified the mismatch.
