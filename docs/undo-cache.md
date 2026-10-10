# Undo cache

PhotoCraft keeps current documents and adjacent undo/redo states in shared,
copy-on-write RAM. Older history spills to a session-private scratch directory
only under memory pressure. Defaults are **4096 MiB RAM** and **8192 MiB disk**
shared by the open documents in one Session. Neither limit reserves memory or
preallocates disk files. History States remains 50 per document by default.

Preferences → Performance sets the memory budget. Preferences → Scratch Disks
sets the disk budget and preferred paths; zero disk budget disables new spills.
The first enabled path is used, or the system temporary directory. Path changes
apply to the next cache session so existing history stays reachable. Budget
changes apply live; reducing disk capacity may retire oldest history states.
The web build remains RAM-only.

The memory budget measures unique managed document/history pixel payloads and
preserved shared blobs, not process RSS. Current documents, immediately adjacent
history, external snapshots, GPU textures, font data, metadata, filter working
buffers, and allocator overhead can exceed it. A current document is never
silently deleted to satisfy a cache setting. Crossing the RAM target starts
spilling toward a 90% low-water mark to avoid oscillating at the threshold.

## Storage and latency

- RAM snapshots retain existing COW tile sharing. Changes that share every
  payload with current/adjacent states do not trigger redundant scratch writes.
- One bounded worker writes cold snapshots; compression and file cleanup stay
  off the UI thread. Pending snapshots remain accounted as resident data.
- The existing native format writes complete metadata manifests and
  content-addressed, fast lossless zstd tiles/blobs. Unchanged objects are stored
  once across snapshots. Weak allocation caches avoid rehashing live shared
  tiles and reuse them on restoration, without retaining their pixels in RAM.
- Actual compressed objects and manifests count against the disk budget. A
  failed write never publishes a history snapshot. Retired snapshot leases queue
  garbage collection; shared files survive until their final reference expires.
- Current and immediate undo/redo states stay hot when resident. Loading an older
  cold state is synchronous and explicitly fallible. Immutable published files
  can be read without waiting on the writer lock. On a read/decode/hash error,
  the document and both history cursors remain unchanged and the command fails.
- Disk quota, slow-write backpressure or unavailable storage can shorten oldest chronological history;
  the current document and adjacent undo remain. Failures appear in the status
  area and through `Session::take_history_cache_notice()`.

Scratch storage is ephemeral, separate from autosave/recovery. Normal session
shutdown cleans the private directory. A process crash can leave its temporary
directory behind; automatic crash-leftover cleanup is a follow-up, not permission
to delete another running application's files. Permissions/deletion failures
remain accounted and are reported.

Automation uses existing `prefs.set` / `prefs.get`:

```json
{"values":{"performance.memoryUsageMb":4096,"scratchDisks.budgetMb":8192}}
```

`session.inspect` reports actual resident payload bytes, compressed disk bytes,
configured budgets and worker activity. Existing undo, redo, Fade, History Brush,
Fill from History and Reselect preserve their command interfaces; cold restore
errors propagate rather than substituting current pixels.

## Research and implementation choice

[GEGL tile-cache controls](https://developer.gimp.org/api/gegl/property.Config.tile-cache-size.html)
and [swap compression](https://developer.gimp.org/api/gegl/property.Config.swap-compression.html)
support independently bounded RAM and compressed disk tiers.
[Linux zswap](https://www.kernel.org/doc/html/latest/admin-guide/mm/zswap.html)
provides a useful model for demand-grown storage and threshold hysteresis.
[SQLite atomic commit](https://www.sqlite.org/atomiccommit.html) illustrates why
incomplete writes must not become authoritative; no SQLite dependency is needed
for ephemeral immutable scratch objects.

A new LZ4 codec and per-operation replay were considered. Reusing PhotoCraft's
existing pure-Rust native format avoids a second document serializer and preserves
all masks, selections, patterns, video frames and embedded content. It also avoids
a layering violation: ops L2 depends only on an archive interface; engine L5
connects it to format L3. This is an incremental, correctness-first architecture,
not a claim of optimal latency. Files per object and synchronous cold restoration
remain opportunities for packed segments and bounded adjacent-state prefetch.
Benchmark codec and layout changes before selecting them.

## Validation and measurement

`cargo run --release -p photocraft-engine --example history_cache_bench -- --steps 12`
uses a deterministic 24 MP image, a deliberately constrained 128 MiB RAM target,
and compares disk-disabled history with the 8 GiB disk tier. It reports edit,
enqueue, worker-settle and hot/cold undo percentiles, retained steps and actual
RAM/disk payload bytes. Disk-disabled history retains fewer states; it is not an
equivalent-retention performance baseline or proof of a speedup.

Regression tests cover lazy allocation, cross-document sharing, quota rollback,
8/16/32f exact restoration, malformed scratch data, failed writes, no writer-lock
contention during cold reads or snapshot drops, count/budget reduction, redo
invalidation, close/purge cleanup and atomic command failure. The CI workload
publishes a reproducible report independently of the regular native/wasm tests.

Release-mode measurement on Ubuntu CI / Rust 1.99, 2026-10-06
([run](https://github.com/TheSeanLavery/photocraft/actions/runs/37552870516)):

| Metric | Disk disabled | 8 GiB disk tier |
|---|---:|---:|
| Undo steps retained | 1 | 12 (11 archived) |
| Resident managed payloads | 192 MiB | 192 MiB |
| Actual disk payloads + manifests | 0 | 7.36 MiB |
| Edit/enqueue overhead p50 / p95 | 0.265 / 8.585 ms | 0.207 / 0.231 ms |
| Background settle p50 / p95 | 0 / 0 ms | 246.374 / 295.409 ms |
| Cold undo p50 / p95 | unavailable | 37.759 / 47.645 ms |

The current document plus immediately adjacent undo state occupy 192 MiB, so the
128 MiB stress target is exceeded by pinned data even after eviction. Each edit
mutates the full image; the benchmark settles after each edit. The synthetic
repeated texture compresses unusually well. Actual photos, rapid consecutive
edits, larger embedded assets and slower disks need separate measurements; these
numbers do not certify UI latency or a hard process-memory ceiling.

Preferences verified offscreen on Apple M1 Max / Metal:

![Shared memory budget](images/undo-memory-preferences.png)

![Demand-grown scratch disk budget](images/undo-scratch-preferences.png)

Linux SSH validation at `3e215e63` with Rust 1.99 passed 1,097 engine, 42 format,
17 history/ops and 1,483 UI unit tests, strict all-target Clippy, adversarial
commands, layering, L0–L6 WebAssembly and the changed corpus (2,486 tests across
107 suites). Both preference dialogs were rendered offscreen and visually
inspected. Exact-head cross-platform CI is linked from the PR; these local
results do not certify a later integration head.

## Small moves and detailed strokes

Simple integer raster-layer moves retain a shared immutable source and a small
absolute-offset record for each step. A thousand or ten thousand nudges remain
independently undoable: this does not group separate keypresses into one step.
Undo and redo materialize the selected position from the source. Pixel storage
therefore depends on the source and current drawing rather than the number of
nudges. Descriptor storage still grows with the number of steps; `session.inspect`
reports it separately as `historyCache.metadataBytes`. The history-count preference
allows up to 10,000 states; its default remains 50.

Masks, vector content, effect reference positions, and moves near coordinate
saturation retain ordinary exact snapshots. The replay source is resident and
cannot be reclaimed while dependent moves survive. Source handles for subsequent
moves are weak so closing a document or purging history releases them.

Painting keeps exact changed COW tiles. One intricate stroke is one history step
but may retain many tiles; step count is not a byte estimate. The
`undo_edit_costs` example compares 1,000 and 10,000 separate nudges with one long
zigzag brush stroke on a synthetic 6000×4000 canvas, checks stroke undo/redo, and
reports elapsed time, payload bytes, descriptor bytes and scratch bytes. Its initial
art is sparse and its repeated moves alternate direction; it is not a dense-photo
move-performance claim. Regression tests additionally cover monotonic nudges,
negative tile boundaries, interrupted edits, coalesced gestures, source lifetimes
and saturation fallback at U8/U16/F32.

Release measurement on the Linux build box at `3e215e63`, with the history
limit explicitly set to 10,000:

| Operation | Retained steps | Managed pixel payload | History descriptors | Scratch bytes | Elapsed |
|---|---:|---:|---:|---:|---:|
| 1,000 nudges | 1,000 | 111,411,200 B | 148,060 B | 0 | 14.86 s |
| 10,000 nudges | 10,000 | 111,411,200 B | 1,480,060 B | 0 | 153.44 s |
| One 64 px, 16-row zigzag stroke after the nudges | 10,000 | 207,355,904 B | 1,479,970 B | 0 | 349.47 ms |

The nudges kept the same pixel payload at both checkpoints. The stroke added
91.5 MiB of pixel payload. Exact document equality passed after stroke undo and
redo; undo took 4.94 ms. The initial sparse canvas used 10,747,904 bytes: the
first shifts materialize additional current-image tiles. These are managed
payload and descriptor measurements, not process RSS or a hard memory ceiling.
The default history limit is still 50.
