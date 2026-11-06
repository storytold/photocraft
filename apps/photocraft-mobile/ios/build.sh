#!/bin/sh
# PhotoCraft.app for the iPad simulator (default) or a device: ios/build.sh [sim|device]
# CraftKit lives in craft-mobile, checked out beside the Craft repos.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
kit=${CRAFT_MOBILE_DIR:-$here/../../../../../craft-mobile}
exec "$kit/swift/build-ios.sh" photocraft-mobile "$here" PhotoCraft "${1:-sim}"
