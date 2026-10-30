Source: https://github.com/storytold/craft-libs/tree/92d392cff132eaf53e13afff77f38b5c11b73aa0/crates/ui

Local review patch: exclusive iconic-column host hook, fitted column reporting, host-owned resize chrome, a separate optional side-dock gap, keyboard-only empty-header focus, and dock seam handles registered after their sibling canvas. Other split/row handles retain their geometry and input. Modified source files: docking/surface.rs, docking/chrome.rs and docking/render/{dock,floating,group,mod}.rs. FloatingFrame also offers an opt-out for its separate top resize gutter; edge handles remain available. No Cargo checkout modifications. Remaining craft-libs crates keep the immutable revision. Font binaries are excluded. Replace this snapshot with an immutable upstream revision after API acceptance.

Shared-crate unit tests also read upstream test inputs. Restore `tests/fonts/` and `tests/fixtures/` from the pinned source before running its standalone lib tests (`cargo test --manifest-path vendor/craft-ui/Cargo.toml --features serde,native-windows,screen-testing --lib docking::` with `--config vendor/craft-ui/tests-config.toml` supplying the app’s pinned dependency overrides); both directories are ignored local test inputs. No font files are committed. PhotoCraft panel regressions live in its own UI crate.

The optional `custom_group` hook also lets a host exclusively draw a compact part of
a floating split tree. Returning false preserves the complete stock group behavior;
true skips its stock tabs/body/targets, with limits still supplied through the existing
host limits path. PhotoCraft owns ordinary drop feedback (including empty results)
and uses the stock resolver for Ctrl/Cmd force-floating only.


To restore the optional test inputs without adding fonts to PhotoCraft, use a
separate checkout of the source revision above and copy only its
`crates/ui/tests/fonts/` and `crates/ui/tests/fixtures/` into this crate's `tests/`.
A sparse checkout is sufficient. Then, from the PhotoCraft root:

```sh
cargo test --manifest-path vendor/craft-ui/Cargo.toml --config vendor/craft-ui/tests-config.toml --features serde,native-windows,screen-testing --lib docking::
```

Keep the test configuration's egui/winit revisions aligned with the workspace when
reviewing a dependency update. Normal PhotoCraft builds do not require these optional
upstream unit-test inputs. The ignored test inputs and fonts are never part of the PR.
