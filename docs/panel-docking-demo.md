# Panel docking repair demonstrations

A full editor keeps its normal height when joined to a short compact owner and when
subsequently detached. A collapsed top/bottom group stays attached as a tab bar;
expanding and collapsing it no longer turns it into a floating miniature. Closed
bars offer insertion slots before, between and after their visible tabs.

| Transition | Before | After |
|---|---|---|
| Full editor joined to a compact owner | ![Incoming editor squeezed to the compact owner's height](images/panel-repair/full-height-before.png) | ![Incoming editor retains its full height](images/panel-repair/full-height-after.png) |
| Collapse a top-attached group | ![Group incorrectly turns into a floating miniature](images/panel-repair/top-collapse-before.png) | ![Group remains an attached tab bar](images/panel-repair/top-collapse-after.png) |

Before images are contributor captures; after images are deterministic offscreen
pointer replay. They show equivalent transitions rather than identical theme or
localization settings. The capture scenes contain no document artwork or personal
photos. Image provenance and licenses are next to the images.

To reproduce the after states in PowerShell, from the repository root:

```powershell
$env:PHOTOCRAFT_FINAL_REVIEW_CAPTURES = "$PWD/plan/panel-final-captures"
cargo test -p photocraft-ui-egui --lib capture_final_review_transitions -- --nocapture
```

The capture test covers three Pro themes and 100/125/150/200 percent scale, with
fitted rectangles saved beside the PNGs. Functional regressions live in
`crates/ui-egui/src/panels/panel_docking/final_transition_tests.rs`. The sanitized
problem-workspace matrix and physical-pixel seam checks live in the other panel
regression modules. See [the docking contract](panel-docking-contract.md) for the
host/shared-framework boundary and dependency update checks. Optional upstream
shared-crate test inputs are documented in `vendor/craft-ui/PATCH.md`.
