# photocraft-s3

`.pcraft` bundles on S3-compatible object storage (AWS S3, Cloudflare R2, MinIO), as a
`photocraft_format::BundleStore` (#3163). Native only: on wasm32 the crate is empty.

```rust
let mut store = S3Store::new(config, "pool/", "docs/poster/")?;
let doc = photocraft_format::load_bytes_source(&store, &LoadOptions::default())?;
writer.save_to(&doc, &mut store, &SaveOptions::default())?;   // one PcraftWriter per open document
```

`S3Config` holds the endpoint (`https://s3.<region>.amazonaws.com`,
`https://<account>.r2.cloudflarestorage.com`, `http://127.0.0.1:9000`), region (`auto` on R2),
bucket, credentials and path-style or virtual-hosted addressing. Requests are signed with AWS
Signature Version 4 over `ureq` (the client the app's updater already ships).

## Layout

```text
<pool>tiles/<blake3>.zst         content-addressed objects, shared by every version and every
<pool>blobs/<blake3>.zst         document that uses the pool
<document>manifest.json          the document's head: the version you open
<document>thumb.png              previews of that version
<document>composite/preview.png
```

## Saving

1. Objects the store hasn't read or written yet go up in parallel (16 at a time by default) with
   `If-None-Match: *`, so an object some other save already stored is left alone and counted as
   reused. A document saved again uploads only its new tiles.
2. The manifest is written with `If-Match: <the ETag this store last read or wrote>`, or
   `If-None-Match: *` for a new document. If someone else saved in between, the save fails with
   `HeadChanged` (`is_conflict(&err)`) and nothing is overwritten: reopen, or save under another
   name.
3. Previews are written after the manifest, best effort (a failure is logged; the save stands).

Nothing is deleted from the pool, so every object an older manifest references stays readable.
Removing unreferenced objects (and keeping version history) needs a collector that knows every
document sharing the pool; that's later work.

## Tests

`cargo test -p photocraft-s3` runs against an in-process fake S3 (`tests/it/fake.rs`) and checks
the signer against AWS's published SigV4 examples. The same lifecycle test runs against a real
service when it is configured (use a throwaway bucket; nothing is deleted):

```sh
PHOTOCRAFT_S3_TEST_ENDPOINT=http://127.0.0.1:9000 PHOTOCRAFT_S3_TEST_BUCKET=pcraft-test \
PHOTOCRAFT_S3_TEST_ACCESS_KEY=… PHOTOCRAFT_S3_TEST_SECRET_KEY=… \
cargo test -p photocraft-s3 --test it minio
```

## The web build (not built here)

A browser must not hold storage credentials, so the web app would not use this crate. Its host
server, which knows the user and which documents they may open, hands out short-lived presigned
URLs for the same layout:

- GET URLs for the document's manifest and previews, and for pool objects;
- PUT URLs for the pool objects a save needs, each bound to its key and signed with
  `If-None-Match: *`;
- one PUT URL for the manifest, signed with the `If-Match` ETag the browser opened, so the
  browser can't skip the version check.

The bucket's CORS rules must allow those methods and expose the `ETag` header. Browser `fetch` is
asynchronous, so that store would live in the web app, beside the platform services.
