# PR 9645 web recovery rebase

Rebased onto upstream main 2ff8f419c. Retained the web runner panic and trap hooks and browser event features alongside autosave. Clarified that previous sessions can occupy recovery storage and how to review their data; no snapshot deletion was added. Native regression covers old-session records and reported guidance. Verification:

- `cargo test -p photocraft-web`: 36 passed, 0 failed, 0 ignored.
- `cargo test -p photocraft-web total_limit_guidance_reaches_the_user_when_old_sessions_are_not_claimed`: 1 passed, 0 failed, 35 filtered (independent parent run).
- `cargo clippy -p photocraft-web --all-targets -- -D warnings`: passed.
- `cargo xtask layers`: 31 crates, no violations.
- `cargo xtask wasm`: 24 crates and `photocraft-codecs --features heif` passed; dead-code warnings emitted.
- Live browser verification was not run.

## CI formatting follow-up

CI job 128081392382 stopped at cargo fmt --all -- --check: the final message edit had left the constant split across two lines. Applied rustfmt; no behavior changed. The full formatting check and git diff --check pass.
