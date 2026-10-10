#!/usr/bin/env bash
# Fetch the same local BiRefNet General Lite model used by CanvaSmith.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
model="$root/assets/models/birefnet-general-lite.onnx"
checksum=5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333
verify() {
  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s  %s\n' "$checksum" "$1" | sha256sum -c -
  else
    printf '%s  %s\n' "$checksum" "$1" | shasum -a 256 -c -
  fi
}
mkdir -p "$(dirname "$model")"
if [ -f "$model" ] && verify "$model"; then exit 0; fi
partial="$(mktemp "${model}.XXXXXX")"
trap 'rm -f "$partial"' EXIT
curl --fail --location --retry 3 --user-agent Photocraft-dev \
  https://github.com/danielgatis/rembg/releases/download/v0.0.0/BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx \
  --output "$partial"
verify "$partial"
mv "$partial" "$model"
echo 'Local AI background removal model is ready.'
