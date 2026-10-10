# Hosting PhotoCraft for the web

`photocraft-web-<version>.zip` (from the GitHub release, or `packaging/web/package.sh`) holds a
static site in `photocraft-web-<version>/`:

| File | What it is |
|---|---|
| `index.html` | The page. It loads everything through relative URLs. |
| `photocraft-web-<hash>.js` | wasm-bindgen glue (generated, ES module) |
| `photocraft-web-<hash>_bg.wasm` | The app: about 19 MiB raw, 8 MiB with gzip, 5.6 MiB with Brotli (see [Sizes](#sizes)) |
| `_headers`, `.htaccess` | Sample header rules for Netlify/Cloudflare Pages and Apache |
| `fonts/` (optional) | Fonts loaded on demand, listed in `fonts/manifest.txt` (see [Fonts](#fonts)) |

There is no server-side code. Upload the folder's contents anywhere that serves static files.

## Docker

> The Docker recipe (`Dockerfile`, `.dockerignore`, `packaging/web/nginx.conf`) is
> **community-maintained**: it is not built in CI or used for releases, so it can lag behind the
> official web build above. Fixes are welcome.

From the repository root (Docker is the only build prerequisite):

```sh
docker build --load -t photocraft-web:local .
docker run -d --name photocraft-web --restart unless-stopped \
  -p 8080:8080 photocraft-web:local
```

Open **http://localhost:8080/**. The multi-stage `Dockerfile` builds the existing Rust/Wasm
app with Trunk, including HEIF support, then copies only the static site into NGINX. The
runtime runs as the `nginx` user on port **8080**, with a `/healthz` endpoint and Docker
health check. It serves precompressed gzip assets, the Wasm MIME type, immutable hashed
assets, and an HTML page that revalidates after deployments. No Rust installation, Node.js,
database, GPU passthrough, or document volume is needed on the host.

The first build downloads Rust dependencies and Trunk's Wasm tools and can take several
minutes. Subsequent builds reuse Cargo caches. The builder supports Linux amd64 and arm64;
the default Rust version is the stable release CI used when the `Dockerfile` was last updated
(CI and releases always use the latest stable), and Trunk is pinned to the release workflow's
`TRUNK_VERSION`. `RUST_VERSION`, `TRUNK_VERSION`, and `NGINX_VERSION` can be overridden with
`--build-arg`. To stamp the About dialog with the commit and date instead of "dev build", pass
`--build-arg PHOTOCRAFT_BUILD_SHA=$(git rev-parse HEAD) --build-arg PHOTOCRAFT_BUILD_DATE=$(date -u +%F)`.
The web build does not embed the optional `craft-fonts` checkout.

The Docker build uses thin LTO and one Cargo build job to reduce peak memory, while keeping
the web profile's size optimizations and `wasm-opt -Oz`. Its Wasm size may differ from the
release zip figures below; NGINX has no 25 MiB file limit. To use the release zip's fat LTO on
a builder with more memory, pass `--build-arg CARGO_PROFILE_WASM_RELEASE_LTO=fat`.
`--build-arg CARGO_BUILD_JOBS=4` enables more concurrent compilation on larger builders. A compiler killed with
`SIGKILL`/`cannot allocate memory` means the Docker/Podman VM needs more memory or swap.

For a public deployment, put an HTTPS reverse proxy or your hosting platform's TLS endpoint
in front of container port 8080. With a reverse proxy on the same host, bind only loopback
(`-p 127.0.0.1:8080:8080`); with a proxy container, connect both through a Docker network.
HTTPS enables WebGPU and browser clipboard APIs. If hosting under `/photocraft/`, redirect
`/photocraft` to `/photocraft/` and strip that prefix when proxying to the container. Relative
asset URLs then work without rebuilding. For example, in an existing HTTPS NGINX server:

```nginx
location = /photocraft { return 301 /photocraft/; }
location /photocraft/ {
    proxy_pass http://127.0.0.1:8080/;
}
```

The editor and image processing run on the visitor's device. Open uses the browser file
picker; Save and Export download files. Preferences use browser localStorage. There is no
server-side document storage, desktop TCP control server, or web autosave/crash recovery;
save/download work before closing or reloading the page. A browser with WebGPU or WebGL2 is
required; `?webgl` forces the fallback when troubleshooting.

To verify the browser deployment, open the page, open an image and save/export it.

## Sizes

Measured on the 0.2.x build (`packaging/web/package.sh`; gzip `-9`, Brotli quality 11):

| File | Raw | gzip | Brotli |
|---|---|---|---|
| `photocraft-web-<hash>_bg.wasm` | 19,680,726 bytes (18.8 MiB) | 8,177,260 (7.8 MiB) | 5,868,731 (5.6 MiB) |
| `photocraft-web-<hash>.js` | about 160 KB | about 23 KB | about 20 KB |

Per-file upload limits: Cloudflare Pages and Workers static assets reject any file over
**25 MiB** (26,214,400 bytes). The wasm fits, and `packaging/web/package.sh` fails the build if
it ever grows past 24 MiB, so a release can't ship a file a common host refuses. (Releases up to
0.2.0 shipped a 25.8 MiB wasm, which Cloudflare rejected; issue #198.) The size comes from the
`wasm-release` Cargo profile (fat LTO, size-optimized code with the pixel crates kept at full
speed) plus `wasm-opt -Oz`.

## Any path works

All URLs in `index.html` are relative (`public_url = "./"` in `apps/photocraft-web/Trunk.toml`),
so the site works at a domain root (`https://example.com/`), under a prefix
(`https://example.com/tools/photocraft/`) and from a CDN bucket. The asset names carry a content
hash, so they can be cached forever. Only `index.html` needs revalidation.

## Required server settings

- **MIME type:** serve `.wasm` as `application/wasm`. Browsers refuse to stream-compile it under
  any other type, and the app then loads slowly or not at all. Serve `.js` as `text/javascript`.
  Most hosts already do both. For nginx, check that `mime.types` has `application/wasm wasm;`.
- **Compression:** turn on gzip or Brotli for `.wasm`, `.js` and `.html`. That takes the
  download from about 19 MiB to about 8 MiB (gzip) or 5.6 MiB (Brotli). You can also
  precompress (`brotli -k *.wasm`) and let the server send `Content-Encoding: br`.
- **Caching:** `Cache-Control: public, max-age=31536000, immutable` on the hashed `.wasm` and
  `.js` files, and `no-cache` on `index.html`.
- **HTTPS:** WebGPU (and the clipboard) only work in a secure context, which means `https://`
  or `http://localhost`. Over plain HTTP elsewhere, the app falls back to WebGL2.
- **No special isolation headers:** PhotoCraft doesn't use `SharedArrayBuffer`, so it doesn't
  need `Cross-Origin-Opener-Policy` or `Cross-Origin-Embedder-Policy`. If your site already sends
  COEP `require-corp`, also send `Cross-Origin-Resource-Policy: same-origin` (or `cross-origin`
  when the files live on a CDN) on the app's files.

nginx example:

```nginx
location /photocraft/ {
    types { application/wasm wasm; text/javascript js; text/html html; }
    gzip on;
    gzip_types application/wasm text/javascript text/html;
    location ~* \.(wasm|js)$ { add_header Cache-Control "public, max-age=31536000, immutable"; }
    location ~* index\.html$ { add_header Cache-Control "no-cache"; }
}
```

Local test: `python3 -m http.server 8765` inside the folder, then open http://localhost:8765/.

## Fonts

The app has two fonts of its own, Inter and JetBrains Mono: a browser offers no system fonts, and
embedding more would push the wasm over the size gate. A host can serve more fonts next to the
app. They show in the font menus straight away, and a family's files are downloaded the first time
it is picked or a document's text needs it, so starting the app costs one small text file.

1. Put the font files (`.ttf`, `.otf`, `.ttc`) anywhere under `fonts/` next to `index.html`.
2. List them in `fonts/manifest.txt`, one file per line, in the format of
   [craft-fonts](https://github.com/storytold/craft-fonts)' own `fonts/manifest.txt`:

   ```text
   # family | style | file (relative to the site root) | scripts | licence | licence file | sha256 | source
   Open Sans | Regular | fonts/open-sans/OpenSans[wdth,wght].ttf
   Open Sans | Italic | fonts/open-sans/OpenSans-Italic[wdth,wght].ttf
   Lobster | Regular | fonts/lobster/Lobster-Regular.ttf | Latn,Cyrl | OFL-1.1 | fonts/lobster/OFL.txt
   ```

   The app reads the first three fields; the others are optional here. `family` must be the font's
   own family name (the typographic family in its `name` table, else the family), which is what the
   font menu shows once the file is in. The path is relative to the site root and can't leave it
   (no `..`, no `/` at the start, no other host). A variable font is one line: the style menu offers
   the weights of its `wght` axis.
3. Put each font's licence next to it; the SIL Open Font License, for one, requires that.

A craft-fonts checkout is already laid out this way, and so is any folder with a
`fonts/manifest.txt` and the files it lists: `PHOTOCRAFT_WEB_FONTS_DIR=<that folder>
packaging/web/package.sh` copies the manifest, the fonts and their licences into the zip, keeping
their paths. Without `fonts/manifest.txt` (a 404) nothing changes.

Serving: let `fonts/manifest.txt` revalidate (`Cache-Control: no-cache`) so edits show up; cache
the font files for long only if their names change with their contents. Serve `.ttf` as `font/ttf`
and `.otf` as `font/otf`, and compress them (Brotli roughly halves a TrueType file). The app
fetches them with `fetch()`, so a Content Security Policy needs them allowed by `connect-src`.

## Embedding in a page (iframe)

```html
<iframe
  src="https://example.com/photocraft/"
  title="PhotoCraft image editor"
  style="width: 100%; height: 720px; border: 0;"
  allow="fullscreen; clipboard-read; clipboard-write"
  allowfullscreen>
</iframe>
```

- The app fills the iframe and follows its size, so size the iframe and not the app.
- Keyboard shortcuts go to the iframe after the user clicks into it, as with any embedded app.
- **Cross-origin embeds** work. Preferences are kept in the iframe's `localStorage`. Browsers
  that partition or block third-party storage may forget them between visits, and the app
  then starts with defaults.
- **Sandboxed iframes** need at least
  `sandbox="allow-scripts allow-same-origin allow-downloads allow-popups"`. Without
  `allow-same-origin` there's no storage. Without `allow-downloads`, Save and Export (browser
  downloads) are blocked.
- Don't send `X-Frame-Options: DENY` or a `frame-ancestors` CSP that excludes the embedding page.

### Driving the embedded app (#1614)

A page can hand the app a document and take the edited one back.

**Same origin:** `window.photocraft` in the app's page.

```js
photocraft.info()                       // {app: "0.6.0", format: 1}: app and .pcraft format versions
photocraft.isDirty()                    // unsaved changes in an open document?
await photocraft.open(bytes, "a.psd")   // ArrayBuffer, typed array or Blob → {warnings} once open
await photocraft.save("psd")            // the active document as a Uint8Array (default "pcraft")
await photocraft.request("ui.inspect")  // a control-channel method (docs/control-protocol.md)
window.addEventListener("photocraft-dirty", e => e.detail)   // true / false as it changes
```

**Cross-origin iframe:** add the embedding page's origin to the app's URL,
`…/photocraft/?embed=https://admin.example.com`. The app then takes the same calls over
`postMessage` from that origin only, and posts only to it:

```js
frame.contentWindow.postMessage({photocraft: 1, method: "open", name: "a.psd", data: buffer}, appOrigin, [buffer]);
frame.contentWindow.postMessage({photocraft: 2, method: "save", format: "psd"}, appOrigin);
// replies: {photocraft: 2, ok: true, result: ArrayBuffer (transferred)} or {photocraft: 2, ok: false, error}
// isDirty, info, and request ({request: "ui.inspect", params}) work the same way
```

It posts `{photocraft: "ready", info}` once it is up, and `{photocraft: "dirty", dirty}` when
that changes. `open` replies once the document is open (`{warnings}`), or with the error File ›
Open would show. A save the page takes counts as a save: a PSD, PSB or `.pcraft` leaves the
document clean, as File › Save does. `request` isn't offered over `postMessage`.

`?embed=` decides who may talk to the app and hear from it, not who may embed it: any site can
iframe the app and name itself there. To keep other sites from embedding your copy at all, send a
`frame-ancestors` CSP from the app's host that lists the pages allowed to embed it.

**Photopea-compatible mode:** a page written for Photopea's iframe
(https://www.photopea.com/api/) can point it at PhotoCraft instead. A JSON config in the URL
hash, `…/photocraft/#{"files":["https://…/a.psd"],"server":{"url":"https://…/save","formats":["psd"]}}`,
opens `files` at start (fetched without cookies, as Photopea does), and File › Save of a document
in one of `formats` POSTs it to `server.url` in Photopea's body (a 2000-byte JSON header
`{"source","versions":[{"format","start","size"}]}` padded with spaces, then the file) instead of
downloading it. The parent window gets the same strings as from Photopea: `"done"` once the files
are open, then for a save `"saving"` and the text of an `app.echoToOE("…")` in the server's JSON
`script` reply, else `"saved"` or `"error"`. PhotoCraft adds `"loading:<name>"` and
`"open-error:<name>"`. These go to any origin, as Photopea's do, unless `?embed=` names one. The
app counts a document saved once it hands it over; if the upload then fails it says so in the app
and the page asks before it is left, until a save gets through.

## Renderer selection and fallback flags

PhotoCraft renders with wgpu. It uses **WebGPU** when the browser has it and falls back to
**WebGL2** on its own. URL query flags override this, and they work on the iframe `src` too:

| Flag | Effect |
|---|---|
| *(none)* | WebGPU if available, otherwise WebGL2 |
| `?webgl` | Force the WebGL2 backend (useful when a WebGPU driver misbehaves) |
| `?cpu` | Force the CPU canvas path (slowest, most compatible) |

For example: `<iframe src="https://example.com/photocraft/?webgl" ...>`.

A browser with neither WebGPU nor WebGL2 gets a message in place of the app.
