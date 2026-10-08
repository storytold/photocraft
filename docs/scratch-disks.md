# Scratch disks

On native builds, Edit > Preferences > Scratch Disks chooses a directory for cold pixel tiles.
The first enabled, non-empty directory is used. `(system temp)` selects the operating system's
temporary directory. Disable every entry to keep pixels in RAM. Web builds keep pixels in RAM.

Half of Preferences > Performance > Memory Usage is the resident tile budget. The remaining
half leaves room for decoding, compositor caches and application state; this is not a hard cap
on total process memory or GPU memory. `PHOTOCRAFT_TILE_BUDGET_MB` overrides the tile budget for
diagnostics. Tiles held by readers or writers cannot be evicted, so temporary overshoot is possible.

Current documents, masks, channels and undo snapshots share the same tile storage. Cold tiles
are compressed with LZ4, CRC-checked and written to a temporary scratch file. Reading restores
them on demand; editing keeps copy-on-write semantics. With scratch enabled, History States
still limits undo depth but the byte limit no longer discards older states merely for their pixels.

A scratch write failure retains the resident pixels and stops eviction. A failed read retains
the original extent and blocks edit commits and saves until those pixels are readable again;
display placeholders are never stored as replacement tiles. Errors appear in the status bar.
Failed reads also prevent derived pixel caches and native-format hashes from being committed;
retrying after storage recovery uses the original pixels.
`ui.inspect` exposes counters under `scratch`: resident and scratch bytes, evictions, reloads,
the directory and the last error.

This does not provide streaming PSD/PSB import, virtual GPU canvas textures, or lazy `.pcraft`
packs. Very large documents can still exhaust decoding or GPU resources. Scratch files are
temporary storage, not durable crash recovery. Autosave remains a separate service.

Adapted from robertocantore126/photocraft commit
`8c0d36bc1b6bfe79437202c36018885f21aa691e` (MIT OR Apache-2.0), with the current upstream
PSD importer retained, guarded read failures, and bounded color conversion batches.
