Source: https://github.com/storytold/craft-libs/tree/c5a1cbfb5f9049f9270955caffb5bee5997aebf7/crates/ui

Reconciled with PhotoCraft main on 2026-10-31. The revision update changes
`combo.rs`, `combo/tests.rs`, `docking/command_tests.rs` and `text.rs`; these four
files are copied unchanged from the new source. The six local docking patch files
do not intersect that update. All other 76 vendored source files match the new
revision, with the same optional test-input exclusions. Dependency pins and the
build-time revision contract advance together; egui's revision is unchanged.

Local review patch: exclusive iconic-column host hook, fitted column reporting, host-owned resize chrome, a separate optional side-dock gap, keyboard-only empty-header focus, and dock seam handles registered after their sibling canvas. Other split/row handles retain their geometry and input. Modified source files: docking/surface.rs, docking/chrome.rs and docking/render/{dock,floating,group,mod}.rs. FloatingFrame also offers an opt-out for its separate top resize gutter; edge handles remain available. No Cargo checkout modifications. Remaining craft-libs crates keep the immutable revision. Font binaries are excluded. Replace this snapshot with an immutable upstream revision after API acceptance.

Shared-crate unit tests also read upstream test inputs. Restore `tests/fonts/` and `tests/fixtures/` from the pinned source before running its standalone lib tests (`cargo test --manifest-path vendor/craft-ui/Cargo.toml --features serde,native-windows,screen-testing --lib docking::` with `--config vendor/craft-ui/tests-config.toml` supplying the app’s pinned dependency overrides); both directories are ignored local test inputs. No font files are committed. PhotoCraft panel regressions live in its own UI crate.

The optional `custom_group` hook also lets a host exclusively draw a compact part of
a floating split tree. Returning false preserves the complete stock group behavior;
true skips its stock tabs/body/targets, with limits still supplied through the existing
host limits path. PhotoCraft owns ordinary drop feedback (including empty results)
and uses the stock resolver for expanded Ctrl/Cmd force-floating. Compact host
sources keep host sizing even when force-floating.

The optional `group_collapsed` hook suppresses an active-editor visual and skips
floating-size recording for a host's closed header. Its default is false. The
optional `split_separator` hook paints a divider in its owning layer; its default
does nothing. `owns_floating_width` lets a host replace horizontal outer frame handles.
`owns_floating_split_width` separately delegates horizontal internal split input;
PhotoCraft supplies independent part handles for both. Both default to false;
`FloatingFrame::resize_axes` defaults to both axes. PhotoCraft uses these hooks to
resize one column of a compound floating window without changing its neighbours'
widths. No additional persistent layout tree is stored.

Disabled resize axes are projected out of corner directions, so the other axis
remains available there. The default two-axis behavior is unchanged. PhotoCraft
reserves the outer bottom corners for this vertical input rather than registering
a competing horizontal part handle over them.


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

Review follow-up (2026-11-01): shared API extraction is under review at
https://github.com/storytold/craft-libs/pull/470 (commit 235fffb). Geometry is
recorded before exclusive column/group delegation; presentation collapse also
hides stripless bodies. Split handles use the host resize style. Frame-width
ownership alone no longer suppresses internal split handles; the explicit
internal-width hook is forwarded through Overrides and enabled by PhotoCraft
only where its own independent part input replaces those handles. The upstream
PR is not merged; this remains the explicit local patch at the source pin above.

The subsequent shared-only docked-custom-group fix in craft-libs#470 at
`4f263ed` is outside this consumer snapshot. PhotoCraft returns exclusive
`custom_group` ownership only for floating groups, so the affected docked path
is unused by this adapter. This snapshot retains the tested `235fffbd` patch;
replace the complete vendor bridge with the accepted upstream revision rather
than treating later shared-only results as consumer verification.
