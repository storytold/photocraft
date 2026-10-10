# Autosave and crash recovery

PhotoCraft's desktop app checkpoints every open document in the background at
Preferences → File Handling → Autosave minutes (10 minutes by default). Autosave
and recovery on launch are enabled by default. A normal Save keeps the recovery
checkpoint while the tab is open, preserving undo even for a saved document.
Closing the tab or confirming a normal quit explicitly retires its checkpoint;
cancelling quit or disabling autosave does not consume existing checkpoints.

Each checkpoint contains the complete native document, undo and redo stacks,
labels and cursor, original path, active/selected layers, tab order and active tab,
tool, zoom and pan. Recovery reopens documents as unsaved and resets unfinished
gesture coalescing. It does not resume an interrupted brush gesture, modal dialog,
background job, OS window arrangement or clipboard. It restores the last
**completed** checkpoint, not edits made after that checkpoint. The web build
still has no filesystem recovery service.

On macOS storage is `~/Library/Application Support/Photocraft/Recovery`; on
Windows it is `%APPDATA%\Photocraft\Recovery`; on Linux it is
`$XDG_CONFIG_HOME/photocraft/Recovery` or `~/.config/photocraft/Recovery`.
`PHOTOCRAFT_CONFIG_DIR` and portable mode relocate it with the other app data.

## Durability and failure behavior

- Capturing document/history handles uses copy-on-write references. The live RAM
  document continues changing independently; completion never replaces it.
- Each document's mailbox owns at most one active and one newest pending request.
  Cold undo states are hydrated one at a time on a worker. Publication and cleanup
  are serialized; UI frames queue requests and poll results. Initial ownership
  locks are acquired at startup or the first queue request.
- Current, undo and redo use the existing native-format manifests and compressed,
  content-addressed tiles/blobs. Unchanged objects are shared within the recovery
  bundle; unchanged file identities avoid redundant validation/compression.
- Objects are written with the existing atomic-write/fsync primitive, then one
  sidecar atomically publishes all checkpoint manifests and navigation context.
  A failed write keeps the previous published checkpoint. Queue acceptance is
  never reported as durable completion. The filesystem's durability guarantees
  still apply; directory fsync is best effort in the shared atomic writer.
- Errors appear persistently in the status area and unchanged failed revisions
  retry after a 30-second backoff. Newer overdue edits catch up after a slow save.
- Startup reports corrupt/missing recovery data and retains it for inspection;
  successful recovery also leaves the original intact for a second crash.
  Legacy document-only recovery files remain readable with empty history.
- Native OS file locks prevent two running apps from claiming the same checkpoint;
  the OS releases these locks on process termination. Fresh documents use unique
  session keys. Unsafe keys and symlinked recovery descriptors/objects are refused.
- Recovered undo states load lazily, share unchanged live tile/blob allocations
  through the undo cache's native decoder cache, and pin their object root. Later checkpoints,
  history-cache eviction and tab retirement cannot delete objects they still need.
  Once the final lease is released, retired roots are cleaned on a worker.

## Storage boundaries

The undo cache's shared 4 GiB RAM / 8 GiB scratch defaults remain independent.
Recovery is durable and cannot use temporary undo-cache leases as its authority.
Both systems reuse native-format serialization. Pending recovery snapshots extend
COW allocation lifetimes; the memory preference is a managed-data target, not a
hard cap on process memory.

Recovery has an 8 GiB admission ceiling **per document bundle**, not an aggregate
quota or preallocation. The descriptor is capped at 256 MiB, navigation context
at 64 KiB, and history at 10,000 retained states; native decoder limits also apply.
A full bundle fails visibly and preserves the last checkpoint. It never silently
shortens history to make a checkpoint fit. Roots with live recovered-history
leases conservatively defer object garbage collection until those leases expire.
Many open documents can use more than 8 GiB in total. Crashes can leave orphan
object roots that are not automatically reclaimed. A configurable shared
recovery quota and finer per-object pinning remain follow-ups.

## Validation

Regression tests cover asynchronous failures/retry, bounded request retention,
corrupt object repair, previous-checkpoint preservation, key/symlink rejection,
legacy files, lazy history lifetime, undo/redo order and document-ID collisions.
A subprocess test exits after a completed save without running Rust destructors,
then verifies exact current, undo and redo documents at 8-bit, 16-bit and float
sample depths, including a second recovery attempt.

Run the synthetic 24 MP workload with:

```sh
cargo run --release -p photocraft-format --example recovery_workload
```

It compares document-only and complete-history checkpoint capture, enqueue and
completion cost for shared solid-colour tiles. This is deliberately compressible
synthetic data, not a general storage or latency guarantee.

Recorded on macOS on 2026-10-06: the 24 MP shared solid-colour scenario retained
three undo states and one redo state. Capture took 2–9 µs, enqueue took 2–26 µs,
and warm full-history completion took 31–43 ms (document-only warm completion
was 30–159 ms under variable machine load). Initial document-only/full-history
completion was 260/482 ms. Five/eight distinct tiles were written initially;
warm checkpoints wrote zero new tiles. These are three observations per scenario,
not statistically controlled speedup claims.
