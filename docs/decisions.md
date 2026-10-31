# Implementation decisions

## 2026-10-10: Smart Object contents identity (#2163)

Smart Object instances carry a document-scoped contents ID separately from their layer ID. Normal duplicates preserve it; New Smart Object via Copy creates a new one. Contents edits update the matching instances together, retaining their individual placement and filters. Copies into another document remap contents IDs as a batch, preserving sharing within the copied layers without joining existing destination objects.

Native files persist this ID. Legacy embedded objects without an ID remain independent: identical file bytes do not prove that two objects were intended to share edits. Legacy nonempty linked paths and PSD source identifiers retain their explicit sharing relationship. PSD export keys embedded sources by contents identity as well as bytes, so independent equal-content objects stay independent.

## 2026-10-17: keeping a replaced file's ACL and named streams (#7073, #7094, #7096)

Saving replaces a file by renaming a new one over it, which drops everything the old file
carried. Carrying the security-relevant parts across needs platform calls that have no safe API
in `std`, and the workspace forbids `unsafe`, so each platform took a dependency rather than an
exception.

**macOS ACLs: `exacl`, not `fclonefileat`.** The two routes that avoid `unsafe` are a macOS-only
`exacl` dependency (a safe wrapper over `acl_get_file`/`acl_set_file`) and `fclonefileat` with
the undocumented-in-`rustix` `CLONE_ACL` bit. The clone keeps the ACL, but only on APFS, and it
means writing every save as clone-then-truncate-then-write with a fallback for every other file
system — restructuring the one write path the whole app's durability rests on, to preserve an
ACL. `exacl` is a best-effort step beside the existing permission copy, it works wherever ACLs
do, and a failure cannot affect the bytes.

**Windows: `ReplaceFileW` via `winsafe`, with a backup name.** Copying the `Zone.Identifier`
stream by hand with `std` would cover the Mark of the Web but not the DACL, and enumerating every
stream needs `FindFirstStreamW`. `ReplaceFileW` is the call Microsoft documents for the
save-through-a-temporary-file pattern and preserves the DACL, named streams, creation time and
object identifier in one step; `winsafe`'s `kernel` feature wraps it safely and pulls in nothing.

The backup-file argument is a correctness requirement, not an option: `ReplaceFileW` without one
documents `ERROR_UNABLE_TO_MOVE_REPLACEMENT` as leaving the replaced file deleted and the
replacement under its temporary name — a failed save that loses the user's document. With a
backup name the same failure leaves both files alone, and the one failure that does move things
(`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`) is unwound in `atomic/meta.rs`.

## 2026-10-31: fractional Minimum / Maximum radius (#10474)

Filter › Other › Minimum and Maximum take a radius from 0.2 to 500 px, but the kernels only had
whole structuring elements: Squareness rounded the radius, and Roundness changed only where r²
crossed a sum of two squares, so 0.2 to 0.9 px did nothing. A fractional radius now blends from
the exact result at the radius toward the result with the next larger element, by how far the
radius has moved between the two. For Squareness that is the square of half-width floor(r) and
the one of floor(r) + 1, with weight r − floor(r). For Roundness it is the hard disc
dx² + dy² ≤ n_lo the radius selects and the disc of the next sum of two squares n_hi, with weight
(r − √n_lo) / (√n_hi − √n_lo). This is our model, not a measurement of Photoshop's output at
fractional radii, which we have no oracle for yet.

The ladder keeps every integer radius bit-identical, so recorded actions and smart filters at
those radii render as before. It is continuous and monotone in the radius and the same for Minimum
and Maximum. The next square is the current one grown by a 3×3 square, so Squareness costs about
5–20% more at a fractional radius. The next disc adds a ring of a few offsets, one shifted-row pass
each, so Roundness costs up to about twice as much at small radii (where the disc itself is only a
few rows) and about 20% more from r 5.
We rejected per-offset coverage weights (an anti-aliased kernel): they change integer radii too,
need an anchor value that min/max morphology does not define, and lose the separable O(1) square.

Two rules make the ladder sound. The disc's row half-width is the largest h with h² + dy² ≤ r²,
tested exactly in f64 (an f32 radius squared is exact there). The old `+ 1e-4` epsilon on the
half-width let points of one ring enter at slightly different radii, which made the ladder step
backwards in narrow windows below radii like 5 and √50; at integer radii the exact test gives the
same half-widths. And with alpha, colour blends premultiplied: a straight blend would pull the
colour of transparent pixels into the half-transparent fringe (a grey edge around a white shape on
an empty layer). Infinite HDR samples snap to the nearer end rather than producing NaN.

Fractional radii now render differently, which is the fix. Squareness radii in [k, k + 0.5) are
stronger than before (1.4 was the 3×3 square and is now 40% of the way to 5×5), and radii in
[k + 0.5, k + 1) are weaker (1.6 was the full 5×5 and is now 60% of the way there). A GPU path
must use the same rule: `other::square_step`, `other::disc_step` and `other::disc_half_widths`
give the elements and the weight, and `other::blend` the per-sample arithmetic (with alpha, colour
uses the premultiplied weight).
