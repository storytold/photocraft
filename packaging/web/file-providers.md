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
| `openResource(requestId, filename, bytes)` | Supply a known read request's file as a `Uint8Array`, up to 80 MiB. |
| `cancelOpen(requestId, error)` | Finish a failed read. An empty error cancels it; a nonempty error reports failure. |
| `resolveSaveDialog(requestId, destination, filename)` | Answer a pending Save destination request with `provider`, `device` or `cancel`. |
| `saveResult(requestId, status, resourceId)` | Report `saved`, `failed`, `download` or `cancel` for a captured save. Only a valid `saved` acknowledges persistence. |
| `snapshot()` | Inspect provider identity, active document, captured revision and outstanding save state. |

A resource list entry has this shape:

```json
{"id":"opaque-resource-id","name":"Drawing.psd","updatedAt":"2026-10-30 18:00"}
```

Only these fields are read; extra properties are ignored without serializing their values.
The provider owns sorting and refreshing the list. `updatedAt` is bounded display text, not a
timestamp the editor interprets. Resource IDs are opaque identifiers, never filesystem paths.
External Recents are separate from the editor's existing local paths and preferences.

## Save and durable acknowledgement

An interactive document Save or Save As first emits `saveDialog`, including its `requestId`,
`documentId`, `intent` and suggested `filename`. The provider presents its destination/name
chooser and calls `resolveSaveDialog`. Cancelling must not export or mark a document clean.
The chosen name determines the codec before encoding. Existing format-option prompts still
apply. Save a Copy and Export retain their existing local writer.

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

Pending reads remain busy for account/disconnection and leave guards, but are not active
saves: they do not hide an unrelated unsaved-changes prompt or prevent its answer.

A read that arrives while a dialog, focused field draft, live tool preview, held gesture or
unprocessed input could be retargeted fails with its correlated `openFailed` before decoding
or admitting a document. The current document, dialog and draft remain in place. Settle or
cancel that interaction, then start a new `requestOpen` to retry; a failed delivery is terminal
and does not keep the provider busy. An idle document may remain dirty while another file opens.
Pointer motion or leaving the editor alone does not block an otherwise idle read; held buttons
and existing drafts or tool interactions remain protected.

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

The small inline iframe helper is restricted to this test module. It is not a product transport,
UI or generated hosting script. The existing malformed-open Rust fixture injects an importer
failure; it does not demonstrate decoding of a real corrupt file. Neither native nor browser
boundary tests establish remote storage durability, backend authorization or cross-client
interoperability.

Use a browser WebDriver and wasm-bindgen-test-runner matching the locked wasm-bindgen version
(0.2.129 for this change). With those tools already available, an example targeted command is:

```sh
CHROMEDRIVER=/absolute/path/to/chromedriver \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
cargo test --locked -p photocraft-web --bin photocraft-web --target wasm32-unknown-unknown \
  page_local_provider_boundary_checks_default_off_inputs_limits_and_opaque_frames
```

The fixture forces browser execution. Resolving the test-only dependency and running this command
are separate validation steps; adding these sources does not claim a passing browser result.
