# Implementation decisions

## 2026-10-11: Responsive artboard program and compatibility

Responsive rules belong to the native document and are stored in the versioned `.pcraft` manifest (v2); v1 migrates to an empty program. PSD stores only PhotoCraft's versioned semantic-role annotation in a private `shmd` item. It does not store variants or solver rules because Photoshop's public PSD specification has no interoperable layout-program schema. Unknown role keys/versions/values are ignored on import, and unknown PSD blocks remain losslessly preserved. No sidecar is created implicitly.

The solver works in document pixels and uses transformed layer-content bounds, excluding layer effects whose visual bounds are not reliable across all layer types. A document's variants share its PPI. Export scaling is explicit. Anchors are physical canvas coordinates; text alignment is logical start/center/end for RTL. A graph solver is out of scope: groups act as units and stack rules provide only local linear spacing. Minimum/maximum constraints clamp dimensions; dimensions remain at least one pixel even when edge-preserving stretch would otherwise collapse or invert them. Text reflow preserves paragraph alignment, and shrink is bounded by an explicit minimum font size.

The master is a document-level artboard ID. Variants reference that master and carry only explicit overrides. Duplicating a layer copies its role/rule; duplicating a variant copies its overrides. Master propagation runs as a cancellable background job on native, commits all refreshed variants atomically, keeps stable mapped IDs and preserves variant-only subtrees; the command reports preserved content as conflicts for review instead of deleting it silently. Position-locked layers remain fixed relative to their artboard during reflow. Deleting or renaming an artboard updates references by stable layer ID; dangling IDs are pruned when loading. Per-variant backgrounds are allowed. The hard limit is 64 variants per document. The overview retains one active-document revision cache, including diagnostics, and renders thumbnails incrementally. It replaces that cache on document/revision changes rather than retaining multiple documents in an LRU, so old previews cannot accumulate across open documents. Role-source provenance is stored alongside explicit semantic roles in native metadata; PSD import marks existing role data as file-provided, and subsequent user changes as manual. Automatic suggestions are not persisted as explicit roles. Bulk application is restricted to the current artboard and matching content kind plus inferred role, and is capped at 10,000 layers.

Safe-area presets use generic ratios and names, with multiple named custom inset zones stored per document as fractions from 0 to 0.5. Their edges and centers join the Guides snap family; malformed persisted insets do not generate snap targets. Portable versioned JSON templates are imported and exported only by explicit user action; they use artboard-relative child paths and bounded validation. There is no cloud sync or telemetry. Affinity role mapping is unsupported absent a documented equivalent. PDF/SVG behavior remains the existing single-artboard/document export; multi-variant export is limited to formats the current encoder can faithfully represent. No Photoshop/Photopea/GIMP/Affinity roundtrip claim is made until verified against those applications; `psd-tools` structural parsing is the automated interoperability check.

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
