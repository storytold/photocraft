#!/usr/bin/env bash
# Build PhotoCraft's native SteamOS tarball in Valve's Steam Linux Runtime 3.0 (Sniper) SDK.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUST_VERSION=1.95.0
RUSTUP_VERSION=1.29.3
JOBS="${PHOTOCRAFT_STEAMOS_JOBS:-1}"

usage() {
  cat <<'EOF'
usage: packaging/steamos/build.sh <x86_64|aarch64>

Builds the existing Linux tar package against Valve's Sniper SDK:
  x86_64   Steam Deck
  aarch64  Steam Frame
EOF
}

[[ $# = 1 ]] || { usage >&2; exit 2; }
[[ "$JOBS" =~ ^[1-9][0-9]*$ ]] || { echo "error: PHOTOCRAFT_STEAMOS_JOBS must be a positive integer" >&2; exit 2; }
ARCH="$1"
case "$ARCH" in
  x86_64)
    PLATFORM=linux/amd64
    # registry.gitlab.steamos.cloud/steamrt/sniper/sdk:latest, resolved 2026-11-05.
    STEAM_RUNTIME_IMAGE=registry.gitlab.steamos.cloud/steamrt/sniper/sdk@sha256:bb30053d3cd19d35c76ae69b2790dbaa1430448006f8240cb6d7e723f2d1940e
    RUSTUP_SHA256=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55
    ;;
  aarch64)
    PLATFORM=linux/arm64
    # registry.gitlab.steamos.cloud/steamrt/sniper/sdk/arm64:latest, resolved 2026-11-05.
    STEAM_RUNTIME_IMAGE=registry.gitlab.steamos.cloud/steamrt/sniper/sdk/arm64@sha256:966ad3ef70751b98253f031d22c5b75798ceb9c49612b0a3489fc5f8f58858fc
    RUSTUP_SHA256=9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282
    ;;
  -h | --help)
    usage
    exit 0
    ;;
  *)
    echo "error: unsupported SteamOS architecture: $ARCH" >&2
    usage >&2
    exit 2
    ;;
esac

IMAGE="photocraft-steamos-sniper:$ARCH"
docker build \
  --platform "$PLATFORM" \
  --build-arg "STEAM_RUNTIME_IMAGE=$STEAM_RUNTIME_IMAGE" \
  --build-arg "RUSTUP_VERSION=$RUSTUP_VERSION" \
  --build-arg "RUSTUP_SHA256=$RUSTUP_SHA256" \
  --build-arg "RUST_VERSION=$RUST_VERSION" \
  --tag "$IMAGE" \
  "$ROOT/packaging/steamos"

mkdir -p "$ROOT/dist/release"
docker run --rm \
  --platform "$PLATFORM" \
  --user "$(id -u):$(id -g)" \
  --env CARGO_HOME=/cargo-cache \
  --env "CARGO_BUILD_JOBS=$JOBS" \
  --env CARGO_TARGET_DIR=/target \
  --env DIST=/source/dist/release \
  --env HOME=/tmp/photocraft-home \
  --volume "photocraft-steamos-cargo-$ARCH:/cargo-cache" \
  --volume "photocraft-steamos-target-$ARCH:/target" \
  --volume "$ROOT:/source" \
  --workdir /source \
  "$IMAGE" \
  packaging/linux/package.sh --formats tar
