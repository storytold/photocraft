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

## 2026-11-02: Generative Expand through a local helper, behind a provider interface (#41)

The first generative feature is Image › Generative Expand, and it ships no model. A
`GenerativeProvider` trait in `crates/ml` takes an RGBA image and a mask and returns pixels;
the engine command, the dialog and the preferences only know that trait. Every provider goes
through a `Transport` trait, so the engine and the web build compile without an HTTP client,
tests use a fake, and the one real transport (plain HTTP over `ureq`, loopback) lives behind a
cargo feature only the desktop app turns on.

**Why a helper program and not an in-process runtime.** Running a model in Rust means picking
an inference runtime (ort, burn, candle), bundling weights and a licence review per model, and
a GPU story per platform. A local [IOPaint](https://github.com/Sanster/IOPaint) process already
serves LaMa and a dozen other models behind one small HTTP API, is Apache-2.0, runs on a CPU,
and keeps the pixels on the user's machine. The provider interface is the part that lasts; the
helper is the cheapest first backend, and a remote or in-process one is one more `impl`.

**Why LaMa first and no prompt.** It is small (about 200 MB), free, runs in seconds on a CPU,
and continues textures and simple structures well enough for the common "more sky, more
floor" use. Diffusion models with prompts are slower than most laptops tolerate and raise
licence questions; the trait carries `prompt` and `accepts_prompt()` so they slot in later,
and the dialog shows a Prompt field only when the provider says it uses one.

**Why the feature is absent, not greyed out, until configured.** Issue #41's rules: off by
default, a privacy notice before the first request, no placeholder menus. The session's
provider is `Option`; `None` means no menu item and "not configured" from the commands. The
three preferences (on/off, address, privacy accepted) live on the existing AI Integrations
page rather than a new section.
