#!/bin/sh
# PhotoCraft for iPad: ios/build.sh [sim|device] builds PhotoCraft.app for the simulator (default)
# or a device; ios/build.sh xcode generates PhotoCraft.xcodeproj (XcodeGen) for signing and
# debugging on a device.
# CraftKit comes from craft-libs (crates/mobile), checked out beside the Craft repos at the commit
# the workspace Cargo.toml pins for craft-mobile; CRAFT_MOBILE_DIR points elsewhere.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
kit=$(cd "${CRAFT_MOBILE_DIR:-$here/../../../../craft-libs/crates/mobile}" && pwd)
if [ "${1:-}" = xcode ]; then
  CRAFT_MOBILE_DIR=$kit exec xcodegen --spec "$here/project.yml"
fi
exec "$kit/swift/build-ios.sh" photocraft-mobile "$here" PhotoCraft "${1:-sim}"
