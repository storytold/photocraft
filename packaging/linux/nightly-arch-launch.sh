#!/bin/sh
set -eu

real_bin="$(dirname "$0")/.photocraft-nightly/photocraft"
if [ "${1:-}" = "--version" ] || [ "${1:-}" = "-V" ] || [ "${1:-}" = "--launch-profile" ]; then
  exec "$real_bin" "$@"
fi
profile_base="${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}"
exec "$real_bin" --launch-profile nightly-v1 --profile-root "$profile_base/photocraft-nightly" -- "$@"
