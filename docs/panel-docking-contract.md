# Panel docking compatibility

PhotoCraft owns one serialized `craft_ui::docking::Workspace`. Compact appearance is
view metadata in `PanelPresentation`; it is not another arrangement tree. Attached
widths belong to the shared column, floating compact widths to the presentation.
Transfer widths only when ownership changes. A receiving column keeps its own width.
Horizontal floating parts remain nodes in the same shared split tree. Each part keeps
its own compact membership and width; toggling one adjusts the window around its
neighbour rather than toggling or covering it. Normal group sizes reuse the existing
presentation size cache. No second placement tree or persistent snapping graph is added.

## Shared UI boundary

`panels/panel_docking/contract.rs` maps PhotoCraft tokens and chrome metrics onto
`DockStyle`. Header controls use its common header/padding constants. Do not derive
panel geometry independently from the shared layout. The fitted `column_rect` hook
reports the rectangles used for content, clipping, canvas allocation and drop targets.
`owns_iconic_column` gives the host exclusive compact painting and input; the shared
renderer must not also register its controls. `custom_group` similarly replaces a
compact part inside a horizontal floating tree; the normal shared split constraints
fit its width and the host supplies its size limits. The default hook returns false.
The host owns the final resize seam and
outline through `resize_handle_style`. Dock gaps are separate from actual row/split
gaps. Keep upstream's default behavior for other consumers of these optional hooks.

The local `vendor/craft-ui` snapshot is explicit and pinned in `PATCH.md`. Do not edit
Cargo's checkout. `build/panel_contract.rs` compares the workspace's requested craft-ui
and egui revisions with the snapshot's reviewed revisions. A revision bump with the
old override produces an actionable build error. Reconcile the adapter and rerun the
checks before changing its metadata. Removing the override for a reviewed upstream
implementation is supported. This check catches stale dependency pins; it cannot
prove behavioral compatibility with arbitrary future UI changes.

## Mouse targets and header surfaces

The host resolver handles every ordinary panel drag, including a deliberate empty
result. `None` from the shared hook is reserved for expanded Ctrl/Cmd force-floating;
compact sources keep host sizing without docking targets. The stock resolver cannot
add alternative targets behind the host result. Tab insertion
uses one vertical marker per slot before/between/after visible tabs; the final slot
extends through unused header space before the controls. The middle of a tab or body
does not provide another merge target. Separate sections use distinct outer edges,
including the top edge. The canvas edge marker uses the actual fitted canvas width.
A group header moves that group; the compact palette grip still moves its palette.

Expanded flyouts and normal shared headers use the same active-tab/strip colors,
font and padding treatment. Flyout resize keeps the open view and uses both committed
size axes on the next frame. Adjacent compact parts prefer outward peeks/expansion,
and a left expansion retains the neighbour's position and returns to its anchor.
Click controls use the pointing-hand cursor; grips and resize edges keep their own
interaction cursors.

## Editor and compound-window transitions

An open editor is a distinct drag source from its compact palette. Tearing off its
header creates an expanded floating editor at the visible editor size. Dragging an
icon or grip retains the compact presentation. Neither operation changes siblings'
presentation. A child editor uses its saved normal size, never the compound owner's
aggregate or collapsed rectangle. Resizing it stores both axes independently of the
compound window; the visible size fits the available space on its opening side.
Opening side and top anchor remain stable during resize. An attached right column
opens its editor strictly to the left of its icon strip, including rightOuter.

A move within the same floating window must not add the incoming pane width again.
When a horizontal part departs, the host observes the remaining fitted group
intervals and supplies their packed extent to the shared workspace action. Tabs and
vertically overlapping groups share an interval. This prevents vacated space from
accumulating after detach/rejoin while keeping one shared layout tree. Raw drop
commands without observed geometry retain the shared model's frame semantics.

## Compact-chain height and physical seams

Joining already compact parts must use the compact tree's natural height, just as
collapsing an expanded chain does. The rendered layout also fits a restored compact
chain with an old expanded frame height; saved normal editor sizes remain intact.
Mixed chains retain their expanded height. The nearest horizontal sibling in the
shared tree determines collapse/expand arrows; header-control coordinates are not
an orientation signal.

Embedded outside strokes need a clip that includes the one physical pixel split
gap. They share the outer floating frame and must not add an inside outline. An
editor and its iconic owner overlap only in their shared one physical pixel border
band. Two touching inside outlines otherwise create a double seam, especially at
fractional DPI. Check raster pixels on both sides, not only rectangle distance or
stroke width. Regression and opt-in WGPU checks: `seam_followup_tests.rs`.

## Full-size transfers and horizontal tab bars

A full source joining a compact floating owner contributes both its visible width
and height; the owner's short compact rectangle must not shrink the incoming editor.
Retain normal sizes before structural moves so detach and horizontal-dock restoration
do not inherit a squeezed or canvas-wide frame.

Root tab groups remain attached when collapsed. Their existing app-owned collapse
flags project a header-only bar, including top/bottom edges, without moving it into
a floating palette. A closed bar has no body callback; the fitted group observer
still supplies its real header rectangle for before/between/after insertion slots.
New tabs inherit a closed receiving bar's state. Header buttons toggle the current
mode, and save/restore preserves it. No additional layout tree is introduced.

A closed horizontal bar retains its selected member for restoration, but paints no
active editor tab. Its detached preview and resulting palette use the compact
size, including force-floating with Ctrl/Cmd. An expanded editor detaches at its
normal editor size. The lower edge of a header-only receiving group remains a
split-below target; an explicit group target takes precedence over canvas edges.

Independent parts of a floating horizontal split own their width gestures. A drag
changes the selected column's width and the outer frame's width, preserving its
neighbours' widths. The preview uses a transient gesture snapshot; release commits
fixed split sizes to the same shared workspace. Escape, loss of focus, modal state,
workspace locking and generation changes cancel it. Stock vertical resizing stays
available. Shared dividers and the Tools boundary each paint one physical seam in
the owning layer, clipped to that owner's rectangle.

## Restore and fallback

A drop immediately below the document tab row creates one protected
`__photocraft_document_tabs__` node in the same shared workspace. This separates
the existing document-tab view from the canvas body; it does not add an arrangement
tree. The existing canvas tab renderer supplies its widgets, overflow, selection,
reordering and close behavior. A per-viewport, per-pass record prevents drawing
the row twice. Old layouts remain unchanged until that placement is used. Repeated
drops reuse the node; Home and hidden-tab modes give it no header height. The node
cannot float, close, or share a tab group with user panels. Placements above the
document row remain distinct from placements below it.

Expanding a wholly compact floating stack restores its remembered normal group
sizes into shared split ratios, then fits the outer frame to its viewport. A UI
collapse supplies fitted per-group `normalSizes` and the physical-pixel `gap` to
the existing group command; these inputs are validated before the atomic commit.
Control-only collapse uses shared `split_rect` geometry when no fitted sizes are
supplied. The size-only `stacked_sizes` cache remembers displayed group dimensions;
`floating_sizes` keeps the independent editor size used for detaching and flyouts.
Neither cache stores membership, order or positions. This preserves a previously resized stack rather than dividing a short
compact frame among full editors. Bottom corners retain vertical resize input
when horizontal sizing belongs to the host's independent part handles.

An unsupported non-null workspace payload is retained verbatim together with its
return locations, collapse/presentation metadata and Tools position, even if its
version tag was not advanced. Rendering a fallback must not overwrite that payload.
The status explains the fallback; an explicit layout edit/reset replaces the saved
unsupported arrangement. Legacy layouts without a shared payload still migrate.

Presentation fields deserialize independently when whole-object decoding fails, so
an invalid Tools size does not discard valid compact widths or membership. Values
are then validated against known panel identities and finite size bounds. The metrics
revision migrates only the previous default width once; later explicit sizes survive.
Transient drag/resize/animation state is never persistence authority.

## Updating the host UI

1. Review changes to shared docking, egui, modal-workspace handling and the host's
   frame allocation. Keep modal gating behind the existing central workspace query.
2. Run panel dependency-contract, panel_docking, dock, workspace, toolbar, visibility
   and overflow tests. Theme, DPI, viewport and surrounding host chrome changes must
   preserve canonical panel identity, preferences and save/restore results.
3. Render the saved audit layout at 100/125/150/200 percent in all three Pro themes.
   Review compact/full/floating states, hover corners, adjacent seams, all docking
   directions, resize previews and 0/50/150/300 ms landing frames. Confirm no document
   history edit occurs during panel gestures. Opt-in capture tests live in the panel
   regression modules; local evidence stays under `plan/panel-repair/`.
4. Run the UI Clippy, layers and wasm gates required by AGENTS.md and the shared
   docking tests when its implementation changes. Keep CI workflows out of this fix.

Offscreen checks do not replace native window/monitor/IME and physical mixed-DPI
acceptance. Record these separately when testing the user's own application build.


When running these gates with a shared Cargo target directory, build the `xtask`
helper in a checkout-specific target first. It embeds `CARGO_MANIFEST_DIR`; a
shared top-level `debug/xtask` alias from another checkout can otherwise validate
the wrong tree despite the caller's working directory. Check the UI crate's source
path in the wasm log before accepting the result.
