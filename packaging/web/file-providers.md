# Optional external file providers

This bridge lets a trusted host of the web editor supply file storage without adding a server,
an account system or a particular storage service to PhotoCraft. The editor still imports and
exports files through its existing codecs. A provider handles its own chooser, authentication,
transport, durable writes and list of resources.

Without an explicit registration, browser Open, Save, Export and local preferences retain their
existing behaviour. Desktop and CLI do not install the browser API. There is no cloud switch
saved in user preferences and no dependency on a particular launcher, hostname or backend.

This initial implementation supports **one registered provider per editor instance**. A host
can implement its own storage service or select among services behind that provider. The bridge
does not claim concurrent mounts or a built-in WebDAV, S3 or Google Drive client.

## Registration and lifecycle

The web build publishes the frozen object `window.photocraftFileProviders`, with
`protocolVersion: 1`, `app: "photocraft"` and a capability list. It dispatches the window event
`photocraft-file-provider`, whose `detail.type` is `ready`. Install the event listener first and
also check whether the object already exists: the host may load after the editor.

Register with `registerProvider(id, label, namespace)`. The label identifies the provider in the
editor. The namespace identifies the authenticated storage account, not a password or bearer
token. Registration is the explicit opt-in. Re-registering the same identity is idempotent.
Changing identity requires unregistering first. `unregisterProvider()` rejects unsettled work,
so another account cannot acquire an outstanding save or read.

Registration returns an opaque `connectionId`. Keep that value with each asynchronous list
request and pass it to `setResources(connectionId, resources)`. A result from an earlier
connection is rejected even if the same provider and account are registered again. Looking
up the new connection ID to label an older response would defeat this correlation.

Unregistering clears the external resource list and bindings. It does not close, discard or
erase documents already open in the editor. Hosts on shared computers remain responsible for
their account-switch and logout policy.

`setEnabled(false)` stops new provider operations and restores the local route for subsequent
saves. It preserves the registered identity, bindings and outstanding confirmations. It is a
host integration control, not an end-user preference. Use `unregisterProvider()` to detach an
idle provider. Explicitly cancel failed reads or saves that the host cannot complete.

## Methods

Methods report input errors with ordinary JavaScript exceptions. Success enqueues work or
changes bridge state; it is not proof that a file opened or was durably saved. Commands enter
the editor on an egui frame rather than re-entering a document from a callback.

| Method | Contract |
| --- | --- |
| `registerProvider(id, label, namespace)` | Register the identity and enable its integration. Three strings; returns its opaque `connectionId`. |
| `unregisterProvider()` | Detach an idle provider and clear its resource state; returns `true`. |
| `setEnabled(boolean)` | Enable or suspend new provider operations; returns `true`. |
| `setResources(connectionId, resources)` | Replace that connection's recent resource list with at most 100 entries. |
| `requestOpen(resourceId)` | Start an opening request and return its opaque `requestId`; dispatches `openRequest`. |
| `openResource(requestId, filename, bytes)` | Supply a known read request's file as a `Uint8Array`, up to 80 MiB per file and across all queued opens. |
| `cancelOpen(requestId, error)` | Finish a failed read. An empty error cancels it; a nonempty error reports failure. |
| `resolveSaveDialog(requestId, destination, filename)` | Answer a pending Save destination request with `provider`, `device` or `cancel`. |
| `saveResult(requestId, status, resourceId)` | Report `saved`, `failed`, `download` or `cancel` for a captured save. Only a valid `saved` acknowledges persistence. |
| `snapshot()` | Inspect provider identity, active document, captured revision and outstanding save state. |

A byte-size or queue-capacity rejection does not retire a known pending read request. Retry
with acceptable bytes after queued work is consumed, or call `cancelOpen` to cancel or report
failure for that request.

A resource list entry has this shape:

```json
{"id":"opaque-resource-id","name":"Drawing.psd","updatedAt":"2026-10-30 18:00"}
```

Only these fields are read; extra properties are ignored without serializing their values.
The provider owns sorting and refreshing the list. `updatedAt` is bounded display text, not a
timestamp the editor interprets. Provider IDs, account namespaces and resource IDs must each
contain 1–160 ASCII letters, digits, hyphens (`-`) or underscores (`_`). Hosts must map storage
identifiers that do not satisfy this syntax before passing them to the bridge. Resource IDs are
opaque identifiers, never filesystem paths. External Recents are separate from the editor's
existing local paths and preferences.

## Save and durable acknowledgement

An interactive document Save or Save As first emits `saveDialog`, including its `requestId`,
`documentId`, `intent` and suggested `filename`. The provider presents its destination/name
chooser and calls `resolveSaveDialog`. Cancelling must not export or mark a document clean.
The chosen name determines the codec before encoding. Existing format-option prompts still
apply. Save a Copy and Export retain their existing local writer.

A Save of a clean document to its existing provider resource and filename may instead finish
with `saveUnchanged` after the host resolves `saveDialog`. Its `requestId` is the resolved chooser
request, so the host can close that operation. The event also carries `documentId`, the already
acknowledged `revision` (both strings), `resourceId` and `filename`, plus the usual `connectionId`,
`providerId` and `namespace` identity fields. It contains no `detail.bytes`, emits no `save` for
that operation and requires **no `saveResult` call**: that revision is already stored by the
provider. Treat it as a completed chooser operation rather than waiting for another upload.

After choosing provider storage and encoding, the `save` event contains the captured bytes:

```json
{
  "type":"save",
  "requestId":"opaque-page-and-request-id",
  "connectionId":"opaque-registration-id",
  "providerId":"my-provider",
  "namespace":"opaque-account-id",
  "documentId":"42",
  "revision":"9",
  "intent":"save",
  "destination":"provider",
  "resourceId":null,
  "filename":"Drawing.psd",
  "copy":false
}
```

`detail.bytes` is also present as a `Uint8Array`, not JSON. Copy the bytes and metadata
**synchronously before awaiting anything**. An event consumer can mutate a typed array.
The host is responsible for retaining an immutable capture and retrying that same operation.
For example, a server-backed provider can keep an IndexedDB outbox before uploading, check the
server's committed operation identity, byte count and integrity, and acknowledge only after
the transaction is durable. An in-memory provider must not claim durable persistence.

Only after completing and verifying the write may it call
`saveResult(requestId, "saved", resourceId)` with the resulting nonempty resource identifier.
The editor retains the request's own document and revision; callers cannot supply a newer
revision to make a later edit appear saved. A successful acknowledgement marks only the
captured revision. Work edited since capture stays dirty. It does not switch tabs or alter
other documents with the same filename. If a write replaces a resource also open in another tab,
that other tab loses its stale provider binding and must be saved explicitly; a local file
with a matching name remains independent. Pending captures of an invalidated alias keep the
operation busy until it settles, but an older acknowledgement cannot restore that resource's
checkpoint, rebind the tab or release its close prompt. A completed write to the same resource
still invalidates its other aliases: the backend may have committed uploads in either order.
An independent Save As to another resource remains valid. A separately saved local file keeps its durable checkpoint.
External bindings never become local filesystem paths.

`copy: true` never makes the editable document clean or binds it to the copy. `failed` retains
the native pending save for an explicit provider retry. `cancel` retires it without cleaning
the document. The provider must retain retry bytes itself. The editor does not silently
re-upload them. Unknown, retired and earlier-page request IDs are rejected.

A provider choosing `device` requests a normal browser copy and reports `download` after
requesting that download. Browser download initiation has no durable acknowledgement and
must not be reported as `saved`. A redirect to login, HTTP error, unavailable quota or a copy
only in browser memory also cannot be reported as `saved`.

Only one save may be outstanding for a document. Editing can continue, but a second Save waits
for the capture's outcome. Pending provider work participates in the editor's close/leave
guard. An acknowledgement that leaves later edits dirty cannot release a close prompt. A delayed
device-download result also cannot close a document edited after its capture. Provider-driven
close/quit continuations wait until the editor has completed a UI pass, then recheck the same
document, captured revision, provider connection and still-valid resource checkpoint. Input
received with the acknowledgement gets processed first; active gestures, text-field drafts and tool previews keep the document
open until they settle. A later model revision cancels automatic closing and keeps the prompt.

`download` is accepted only for a capture whose chosen destination was `device`; it cannot
complete a `provider` upload or release its unsaved-changes prompt. A failed upload must be
retried or explicitly cancelled, rather than relabeled as a local download.

Provider-mediated Save and Save As operations (including a host's device choice and format
options) are omitted from the Actions recording journal. Their provider/account capability
cannot be serialized as an ordinary local `file.save` or `file.saveAs` path. Existing local
save recordings, Save a Copy and exports keep their normal recording behavior. Provider-aware
action playback is not implemented by this bridge.

## Read requests and decoder completion

Clicking an external Recent entry or calling `requestOpen(resourceId)` creates an `openRequest`
with both its unique `requestId` and `resourceId`. Fetch and verify that resource under the
event's provider identity, then call `openResource(requestId, filename, bytes)`. A cancelled or
unknown request cannot open late bytes in another provider's session. Programmatic opens use
the same request flow; there is no uncorrelated byte-injection shortcut.

Supplying bytes is not opening success. The native importer can be delayed by a background
decode or a colour/profile prompt. `opened` is emitted only when the editor admits the decoded
document, including `requestId`, `resourceId` and the actual `documentId`. It is then safe for
the host to consider a deep link consumed. Failed decoding emits `openFailed`; cancellation
emits `openCancelled`. Failed fetches use `cancelOpen` so an absent file cannot leave the
provider permanently busy.

PSD/TIFF imports that require additional memory approval fail the synchronous provider
request with `openFailed`; they do not leave a local background open running after that
failure. This bridge does not implement a provider memory-approval continuation. Ordinary
local opens retain the editor's existing approval flow, including while a provider is
registered. Memory limits and admission checks are not bypassed.

Pending reads remain busy for account/disconnection and leave guards, but are not active
saves: they do not hide an unrelated unsaved-changes prompt or prevent its answer.

A read that arrives while a dialog, focused field draft, live tool preview, held gesture or
unprocessed input could be retargeted fails with its correlated `openFailed` before decoding
or admitting a document. The current document, dialog and draft remain in place. Settle or
cancel that interaction, then start a new `requestOpen` to retry; a failed delivery is terminal
and does not keep the provider busy. An idle document may remain dirty while another file opens.
Pointer motion or leaving the editor alone does not block an otherwise idle read; held buttons
and existing drafts or tool interactions remain protected.

## Event delivery recovery

The browser publisher copies save bytes through a catch-enabled `Uint8Array.from` import. Recoverable JavaScript exceptions in the copy or dispatch path fail only a still-owned matching request and allow the bounded current batch to continue. This does not claim recovery from fatal browser/WASM OOM, process termination, or ordinary DOM listener exceptions that the browser reports without propagating from dispatchEvent.

A save delivery failure retains its exact request/document/revision/account/destination/resource identity until the actual application-thread consumer removes that capture and reports `saveFailed`. It never acknowledges persistence, advances a saved checkpoint, invalidates another resource alias, records a local action/save destination, or completes a close continuation. A late reply for that retired request is rejected. The user can start a fresh save capturing current work. This is distinct from an ordinary accepted `saveResult(..., "failed", ...)`: the existing provider-upload retry keeps that same immutable capture pending and is unchanged.

An undelivered openRequest immediately loses its read-response authority, including under full outbound backpressure. A bounded overflow retains its correlated openFailed terminal until publication can attempt it; later openResource/cancelOpen callbacks cannot admit that retired read. An undelivered chooser releases only its unanswered dialog, sends an unavailable result to the real application dialog receiver, clears that chooser's prepared intent, and permits a fresh chooser after the application consumes the error. A synchronous accepted choice/open/save callback always wins over a subsequent dispatch error.

Queued terminals and the entire currently taken publisher batch retain their connection. A listener cannot disconnect/reconnect midway and silently discard or relabel a later terminal. Nested publication does not drain the outer batch. Reads and retained read terminals remain distinct from save-specific pending state and do not lock unrelated discard answers. Normal local/default-off, provider destination recording, cancel/download/Save a Copy and exact-revision alias/ACK rules remain on their existing paths.

## Trust boundary and limits

The API is a page-local capability for trusted scripts executing in the editor's own origin. The
object is a set of direct function calls on the editor's window; its CustomEvent notifications are
also dispatched on that window. It has no incoming MessageEvent or cross-window postMessage
listener. An iframe on another origin cannot read the parent window's API under the browser's
same-origin policy. There is consequently no incoming message origin to validate and no wildcard
or configurable origin allowlist in this bridge. Adding a cross-window transport would require a
separate reviewed protocol and origin/source validation; this API does not silently provide one.

All scripts executing in this page share that capability, including analytics or other third-party
scripts loaded into the page: the origin of a script's download does not sandbox its execution.
Registration is explicit opt-in, not authentication or isolation from other page scripts. Only load
integrations that may read the current document and acknowledge their own saves. Keep untrusted
scripts out of the editor page. The provider backend must authenticate and authorize every list,
read and write for the appropriate account; provider namespaces and request correlation do not
replace those checks. The bridge has no arbitrary engine-command endpoint, URL fetcher, storage
credentials or filesystem access. Conflict detection, retention and quotas remain the provider's
responsibility. Serialize writes and acknowledgements targeting the same mutable resource, or use
immutable version IDs; the bridge does not implement remote compare-and-swap or cross-instance
concurrency control.

Document IDs and revisions are decimal strings to preserve 64-bit values in JavaScript.
Resource metadata, request counts and queued bytes are bounded. Request IDs include a fresh
cryptographic page nonce. Hosts must tolerate normal exceptions rather than treating rejected
inputs as successful operations. Resources not explicitly returned by a provider are never
inferred from local paths or filenames.

## Relationship to embedding and validation

This bridge is independent of the proposed embedding API in [PR #2263](https://github.com/storytold/photocraft/pull/2263).
It does not install or replace `window.photocraft`. Taking an embedding/export snapshot and
marking it clean is not a substitute for a durable provider acknowledgement.

Related upstream proposals were open and unmerged when checked on 2026-11-01:

| Proposal | Scope and relationship |
| --- | --- |
| [craft-libs #413](https://github.com/storytold/craft-libs/pull/413) | A common window command/request API over the control channel. Its current scope leaves file delivery and dirty-state messages with each app; the embedding package mentioned in its title was removed from that proposal. |
| [craft-libs #420](https://github.com/storytold/craft-libs/pull/420) | A proposed shared host-message contract and bridge for page or same-origin-parent embedding, including open, save/saved, dirty and handoff messages. App adoption is proposed separately. |
| [DeckCraft #983](https://github.com/storytold/deckcraft/pull/983), [#1143](https://github.com/storytold/deckcraft/pull/1143) and [SoundCraft #866](https://github.com/storytold/soundcraft/pull/866) | App-specific postMessage embedding proposals for file delivery and host notifications. |

Those proposals are relevant to upstream contract alignment. This candidate does not claim to
implement their message protocol, replace a landed shared contract or decide which API maintainers
should adopt. Its page-local surface and account/revision acknowledgements can be evaluated against
that work without adding an unused postMessage listener.

This is a candidate integration for upstream review, not a bundled storage service. Protocol
and native UI regressions live in `crates/ui-egui/src/file_providers/tests.rs` and
`crates/ui-egui/src/file_provider_tests.rs`. They cover captured revisions, account changes,
read correlation, local saves and cancellation without depending on a storage server.

Changes to this bridge follow the affected-crate tests, Clippy, layer/WASM checks and visual
review required by `AGENTS.md`. Review both default-local and registered-provider flows.
Browser editing and a real storage backend need their own end-to-end results: a passing codec
or protocol fixture does not establish server durability, authentication or interoperability.

For an offscreen visual check without a browser or a storage service, the existing snapshot
example accepts `--file-provider-preview`. It uses three synthetic resources and does not
fetch files or persist anything:

```sh
cargo run -p photocraft-ui-egui --example snapshot -- --out target/provider-home.png --size 1440x900 --scale 1 --safe-gpu --file-provider-preview
```

Omit the last flag for the default-local Home. This preview checks layout only; it is not a
provider persistence or browser interoperability test.

## Browser-boundary regression (test only)

The web crate has a real-browser wasm-bindgen-test fixture named
page_local_provider_boundary_checks_default_off_inputs_limits_and_opaque_frames. It installs the
actual page API with a fresh Host and egui Context, without starting the editor or a GPU renderer.
One fixture owns and removes its installed object, so its asynchronous cases do not depend on test
ordering. It checks:

- exposure without registration keeps storage disabled, and actual callbacks reject malformed
  registration/enable arguments;
- JS resource types, bounded fields, duplicate IDs and a 101-entry list are rejected without
  mutating accepted metadata; valid registration and a 100-entry list also succeed;
- a real 80 MiB + 1 Uint8Array is rejected by the JS/WASM adapter's size check while the valid
  request remains cancellable; ordinary arrays, empty bytes and invalid filenames are rejected;
- an allow-scripts sandboxed iframe without allow-same-origin has an opaque origin and cannot read
  the existing parent API. It sends a forged registration message to the exact parent origin,
  never a wildcard; that MessageEvent does not change the Host.

The opaque child uses Rust code from the same selected generated test module. Before installing
any API, the child role genuinely attempts the parent API lookup/register call and posts the
forged request and a nonce-correlated report to the exact parent origin. The parent checks the
actual iframe WindowProxy, MessageEvent origin `null`, sandbox flags, nonce and exactly one real
forged request, then retains all unchanged Host/default-off/cancellation/input-limit assertions.
It does not infer the result solely from the parent's access to the child.

The explicit `browser-test-provider-origin` xtask starts a temporary Rust artifact relay on
127.0.0.1 and an ephemeral port, then runs only the named browser fixture. A separate owned
ephemeral listener chooses a nonzero test HTTP address, passed unchanged through the pinned
runner's `WASM_BINDGEN_TEST_ADDRESS` setting for both server binding and browser navigation.
Port zero must not be passed: runner 0.2.129 would navigate headless Chrome to port zero rather
than the server's assigned port. The temporary HTTP reservation is released immediately before
the nested Cargo spawn; if another process claims that port, the runner's bind fails without a
retry or port scan. The ChromeDriver announcement is not an HTTP-server origin. Any HTTP-origin
announcement must occur at most once and match the configured address. The successful real-child
proof must name that exact parent origin. Only the retained generated directory is discovered
from stdout. The helper proxies the unchanged generated index from its configured test HTTP
origin and streams only the generated run/module/snippets/WASM allowlist from that retained
directory. It adds `Access-Control-Allow-Origin: null` (no wildcard) and
`Cross-Origin-Resource-Policy: cross-origin`, while preserving COOP same-origin and COEP
require-corp. CORP explicitly permits cross-origin embedding of only these public generated test
artifacts under the parent's COEP policy. It grants no access to the parent window's provider API:
the child remains sandboxed with only `allow-scripts`, without `allow-same-origin`. This test-only
header does not change the application API or backend contract. The relay never exposes arbitrary
workspace files, authentication, storage or provider backends. No application startup or ordinary
CI command starts this support service. There is no handwritten JS/TS, srcdoc script or Function
constructor in the fixture.

The helper requires a Unix managed validation scope, an existing task-owned temporary
directory and a fresh evidence filename. Bounds are 96 connections, 16 KiB request headers,
128 KiB index body, 512 MiB per generated artifact, 2 GiB total served bytes, 64 KiB streaming
buffers, 32 MiB stdout and a configurable 600–3600 second lifetime (1800 by default), with at
most a 10-second socket-I/O stop grace. The child has a 600-second report deadline and the runner
a 900-second test timeout. Choose a resource budget that accommodates the actual generated artifact sizes and measured browser time. Every served
artifact receives a SHA256/length/header receipt and a fresh durable snapshot beside the evidence
JSON (at most the same 2 GiB total). No snapshot or result is overwritten. The relay is stopped and joined on success or
failure, and its Cargo subprocess is supervised. Served artifact snapshots and partial logs
are preserved outside temporary storage for review. The enclosing managed unit must still finalize and prove all
compiler/driver/Chrome descendants absent.

Use the already pinned wasm-bindgen-test-runner 0.2.129, real Chrome/WebDriver configuration and
existing Cargo target/cache. For example:

```sh
CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=1 RAYON_NUM_THREADS=1 \
CHROMEDRIVER=/absolute/path/to/pinned/chromedriver \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=/absolute/path/to/pinned/wasm-bindgen-test-runner \
cargo run --locked --offline -p xtask -- browser-test-provider-origin \
  --tmp-root /absolute/task-owned/tmp \
  --evidence /absolute/task-owned/evidence/origin.json \
  --timeout-seconds 1800
```

Run this command under process, memory and disk supervision suitable for the test artifacts.
The helper does not install tools or manage unrelated processes. Keep the temporary root, browser
profile/cache and evidence paths task-owned, and verify the actual Chrome/WebDriver and matching
wasm-bindgen versions. Enclosing validation tooling remains responsible for collecting launch
evidence and confirming that every owned compiler, driver and browser process has terminated. Compilation embeds a fresh test-only relay URL, so this selected
test target is rebuilt for each invocation in the existing target directory.

Success requires the original literal fixture marker exactly once, one passed/zero failed/zero
ignored, an actual `PHOTOCRAFT_OPAQUE_CHILD_PROOF` from the parent and durable matching index/
module/WASM receipts. The child has its own generated test context; its output does not replace
the top-level test result. Missing/unbound support fails rather than silently skipping the
foreign-origin scenario. Nine native support regressions cover the nonzero loopback HTTP
binding, separation from ChromeDriver's URL, exact proof origin, route escapes, chunk framing,
streaming SHA256 against the existing implementation, bare evidence filenames and accepted-socket
blocking setup. The real bounded loopback HTTP responses retain exactly one CORP, COOP, COEP and
opaque-origin CORS header at their required values for HTML, JavaScript and WASM artifacts.

The existing malformed-open Rust fixture injects an importer failure. It does not demonstrate
decoding of a real corrupt file. Neither native nor browser boundary tests establish remote
storage durability, backend authorization or cross-client interoperability. Adding this source is
not a syntax, compile or real-browser pass. Actual opaque-origin startup/CORS compatibility,
resource adequacy and runtime termination remain to be verified on the final published source.
