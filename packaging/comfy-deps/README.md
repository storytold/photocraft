# ComfyUI model dependency service

A separate Rust service installs only the five fixed model files in
[`dependency-models.json`](../../crates/ai/src/dependency-models.json).
Run it on the ComfyUI host with its existing `models` directory mounted at
`/models`. It does not install Python packages/custom-node code, control Docker,
restart ComfyUI, or modify queued image jobs.

## Build and run

Build `cargo build --release -p photocraft-comfy-deps` on Ubuntu 22.04 or a
compatible build container. Stage that executable plus `LICENSE-MIT` and
`LICENSE-APACHE` in a temporary build directory, then build this Dockerfile:

```sh
docker build -f packaging/comfy-deps/Dockerfile -t photocraft-comfy-deps:0.8.2-ai /path/to/staging
```

Create a private environment file outside Git with these settings:

```dotenv
PHOTOCRAFT_DEPS_TOKEN=replace-with-a-random-token-of-at-least-24-characters
PHOTOCRAFT_MODELS_DIR=/path/to/ComfyUI/models
PHOTOCRAFT_MODELS_UID=1000
PHOTOCRAFT_MODELS_GID=1000
PHOTOCRAFT_DEPS_HOST_IP=127.0.0.1
```

Restrict it to its owner (`chmod 600`). Set the UID/GID to the model directory's
owner and choose a reachable host binding when clients run elsewhere. Start with
`docker compose --env-file /path/to/private.env -f packaging/comfy-deps/compose.yml up -d`.
Use TLS through a reverse proxy when the network is not trusted.

In **Edit › Preferences › AI Integrations…**, enter this service's base URL and
token under **ComfyUI · Dependency service**, then **Apply**. Choose
**Manage ComfyUI dependencies…**, check the status, select missing models, review
the license links and start the download. Progress/cancellation use PhotoCraft's
normal background-job controls. A disconnected client abandons its download
within the three-minute lease plus a pending network read timeout.

## Integrity and limits

- Requests accept catalog IDs only. URLs, paths, sizes and SHA-256 digests are fixed.
- New files are downloaded to unique temporary files and verified before an
  atomic no-overwrite hard link installs them. Partial files are removed on error.
- Existing files are checked by size and explicitly reported as hash-unverified.
  They are never replaced, including files with a size mismatch.
- One download job runs at a time; disk space must cover the selected missing models.
- Model licenses differ (including Qwen Research). Review them before downloading.
  Weights are not bundled in PhotoCraft or committed to this repository.
- Required core nodes are checked against ComfyUI's API. Missing nodes require
  a compatible ComfyUI update administered separately.
- API: authenticated `GET /models`, `POST /jobs`, `GET /jobs/{id}` and
  `DELETE /jobs/{id}`; unauthenticated `GET /healthz` exposes readiness only.
