# GPU stack prefix reuse

Design for #209, particularly the opacity, fill, blend-mode and visibility scenarios P2–P5.
The current compositor caches input pages and effect maps, but executes the complete pass list
over every damaged chunk. This proposal adds reusable lower-stack composites without changing
the blend equations, colour conversion or damage policy.

## Safe boundaries

The planner records a checkpoint after a complete root-level stack unit: one base layer and all
layers clipped to it. Nested groups, pass-through groups and adjustments finish through the
existing planner before this boundary. Only the accumulated backdrop is live there; restoring
it into the checkpoint's slot is sufficient to execute the remaining passes. No checkpoint is
placed inside a group, clipping unit or adjustment program.

Blend If runs on the GPU (#4395) within this rule: like channel restrictions, it reads only the
backdrop from before its own unit (retained across the unit, released at its end), so nothing
else is live at the boundary. Advanced Blending knockout (shallow and deep, also of clipped
layers), Blend Clipped Layers as Group off, Transparency Shapes Layer off, Layer / Vector Mask
Hides Effects and Blend Interior Effects as Group run on the GPU within the same rule (the last
three change only how one layer's own passes blend, shape, mask and apply Fill to its effects:
Blend Interior Effects as Group builds the layer at full fill and scales its alpha by Fill after
the interior effects, before the inner shadow; Hides Effects plans the layer's content and effects
without the hiding masks, with the effect maps built from that layer, and mixes the result back
towards the backdrop retained across that one layer by those masks, so neither adds a live slot
across a unit): a shallow
knockout's floor (the starting backdrop of a non-root stack) lives only inside that stack, the
floor of individually clipped layers (the backdrop beneath the base) lives only inside that
clipping group's planning, and a deep knockout builds its target (a cleared slot with the
Background composited onto it, even when occlusion left the Background out of the plan) fresh at
each use, so only the root stack's accumulated backdrop is live between units. The remaining
cases still return `Unsupported` from the GPU planner or compositor and use the CPU path:
Transparency Shapes Layer off with effects or a special-eight blend mode on a layer drawn atop
its clipping base (Blend Clipped Layers as Group on; individually blended clipped layers run on
the GPU), and Hard Mix with Transparency Shapes Layer off in a 16/32-bit document, or without
effects on an adapter with half-float accumulation. So does Blend Clipped Layers as Group off in
a 32-bit float document when the base takes more than one pass (a group, or a pattern fill
without cached pixels): its coverage would go through an accumulator slot, which may be half
float. A later GPU implementation of any of them preserves the boundary invariant.

## Invalidation

Retain a COW snapshot of the checkpoint's lower layers and the document's non-layer state.
Compare the complete layer structures, including masks, nested children, effects, derived caches
and pixel surfaces. Compare all non-layer document fields conservatively, including colour mode,
depth, profile, global light and patterns. This deliberately accepts unnecessary invalidations
(for example selection or metadata changes) in exchange for avoiding an incomplete render key.
Shared `Arc<Tile>` data stays pinned and equality short-circuits for unchanged shared tiles;
there is no fingerprint collision or dependence on command IDs or declared damage being complete.

If a lower layer changes, choose an earlier root checkpoint before that unit, discard cached
chunks and render once to populate it. Subsequent edits above that checkpoint reuse it. Start
with the boundary before the last root unit. Validate the planner's starting layer, pass offset
and root slot too: an opaque layer above the checkpoint can change occlusion pruning.

Admit only whole grid chunks — rectangles aligned to the compositor's chunk grid — keyed by
their grid position, in the compositor's accumulation format. Partial damage is served, not
missed: a damage rectangle maps to the whole chunks it intersects, and a partial rectangle is
copied as a sub-rect of the cached whole chunk, so brush strokes (a new partial rect every
frame) populate whole chunks instead of filling the budget with entries keyed to exact damage
rects that never hit again. A lower-prefix change invalidates every chunk, including offscreen
ones.
Switching documents, closing the cached document, resizing, changing the texture limit or losing
the device releases or invalidates the cache. Undo/redo is covered by snapshot comparison.

## Memory and submission

Keep one prefix per compositor, with a bounded set of chunk textures. Limit it to 512 MiB and
reserve its allowance from the existing effect/cache part of `set_memory_budget`; do not reduce
the existing layer-page allowance or silently exceed the configured total. Stop admitting new
chunks when full so a repeated full-canvas refresh cannot evict its own next hits. Budget changes
trim the cache immediately. Tiny budgets and documents without useful boundaries render normally.

Copy the stored chunk into its accumulator slot, execute the suffix, and send the final root to
the existing display sink. Capture the checkpoint when its last prefix pass finishes. No readback
or lower-precision storage is introduced. Expose reused-chunk and skipped-pass counters and the
held byte count for tests and measurement.

Only `render` owns submission and can establish reuse. `encode` takes a caller-owned encoder,
which may be dropped without submission: it must neither read nor populate prefix checkpoints.
On a failed render discard prefix state; device-health checks still gate all GPU work.

## Validation and delivery

Before implementing, publish this design separately for review. The implementation is a focused
follow-up and references this document. Keep all existing public rendering results and CPU fallback
semantics intact. Compare warm cached output with both the uncached GPU path and CPU oracle, at
8/16/32-bit depths and simulated small texture limits. Cover top/middle/bottom edits, property
changes, clipping, pass-through and isolated groups, adjustments, masks, effects, global inputs,
occlusion, structural changes, undo-like snapshots, disjoint partial damage, budget shrink/zero,
document replacement and discarded encoders. Run existing GPU parity and real-file corpora.

Measure release rendering on dense 24–36 MP stacks, cold population and repeated property edits,
with reuse enabled and disabled in the same binary. Also run the existing P2–P5 scenarios before
and after; their styled fixture edits a low layer, so only the layers below that unit can be reused.
Report timings, actual skipped work and cache bytes. This cannot promise the 10–20 ms budgets:
all layers above the change still blend, and full-canvas output encoding/mips still run. Retain
published baselines until a representative full measurement justifies changing them.
