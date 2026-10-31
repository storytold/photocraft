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
with a matching name remains independent. External bindings never become local filesystem paths.

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
device-download result also cannot close a document edited after its capture.

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

## Trust boundary and limits

The API is a capability for trusted scripts executing in the editor's own origin. It has no
cross-window `postMessage` listener, arbitrary engine-command endpoint, URL fetcher, storage
credentials or filesystem access. Serve only host integrations that should be able to read
the current document and acknowledge their own saves. Backend ownership checks, authentication,
conflict detection, retention and quotas remain the provider's responsibility. Serialize writes
and acknowledgements targeting the same mutable resource, or use immutable version IDs; the
bridge does not implement remote compare-and-swap or cross-instance concurrency control.

Document IDs and revisions are decimal strings to preserve 64-bit values in JavaScript.
Resource metadata, request counts and queued bytes are bounded. Request IDs include a fresh
cryptographic page nonce. Hosts must tolerate normal exceptions rather than treating rejected
inputs as successful operations. Resources not explicitly returned by a provider are never
inferred from local paths or filenames.

## Relationship to embedding and validation

This bridge is independent of the proposed embedding API in [PR #2263](https://github.com/storytold/photocraft/pull/2263).
It does not install or replace `window.photocraft`. Taking an embedding/export snapshot and
marking it clean is not a substitute for a durable provider acknowledgement.

The source and regression fixtures are a candidate implementation. Compilation, native and
WASM checks, and visual review have not yet been run for this branch. No upstream acceptance
or server interoperability is claimed. Before submitting, run the affected-crate tests,
Clippy, the layer/WASM checks required by `AGENTS.md`, and review both default-local and
registered-provider flows. Browser editing checks and backend integration need their own
results. This document describes the intended contract, not a validation record.
