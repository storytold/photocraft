# Guide editing drafts and interaction checks

Edit Selected Guides previews orientation, position and colour on the canvas without editing
the document. OK sends one `view.editGuide` command; Cancel and Escape discard the preview.
An unchanged OK leaves Undo/Redo intact. Invalid text keeps the last valid preview and cannot
commit. A document revision, target or ruler-unit change invalidates the draft; reopen the
dialog to start again. Undoing a concurrent edit does not revive an old draft.
If the final command is refused (for example by the automation policy), the dialog reopens
with the typed fields, error and last valid preview. Retrying uses the original target and
revision; switching documents or editing the target before retry still invalidates the draft.

The UI uses [edit-session](https://github.com/PerchAnton/edit-session) at
`28620b8264660ea84a182110cf5894da90281c42` for the draft lifecycle and no-op/stale checks.
PhotoCraft retains guide selection, parsing, rendering and history ownership. Validation runs
the existing command on a temporary COW document only when parameters change. The live document
is changed only by the ordinary command after successful confirmation. Rounded ruler text that
was left untouched preserves the exact original position.

This external dependency is an explicit exception in CI's Git-source allowlist, restricted to
the package name, version, repository and full commit. Both external projects are dual licensed
MIT OR Apache-2.0. The integration is proposed for review; upstream adoption requires maintainer
approval, including this source-policy exception.

## Running the scenarios

The [interaction-check](https://github.com/PerchAnton/interaction-check) runner is development-only;
Python is not a PhotoCraft runtime dependency. CI checks it out at the immutable revision below.
The adapter belongs to PhotoCraft and calls the shipped dialog handlers, control field setter,
egui canvas/dialog frames, engine commands and history. Expectations are never passed to the
adapter. These are semantic integration checks, not a substitute for screenshot or pointer tests.

```sh
git clone https://github.com/PerchAnton/interaction-check ../interaction-check
git -C ../interaction-check checkout --detach f8d4282d61576ffe78d18553c642d215887e2d9f
cargo build --locked -p photocraft-ui-egui --example guide_edit_scenarios
python3 ../interaction-check/interaction_check.py --timeout 30 \
  --report guide-edit-report.json tests/interactions/guide-edit/*.json -- \
  target/debug/examples/guide_edit_scenarios
```

Each scenario gets a fresh application with a 400 × 200 document at 100 ppi and vertical guides
at 10 and 20 px. Reported state reads committed and preview guides, history depth, Redo, dialog
validity and the real session command journal. The scenarios cover a single committed edit,
Undo/Redo, cancellation, invalid input and recovery, no-ops, fractional positions, orientation
and colour, document switching, deleted index reuse and an edit followed by Undo.
