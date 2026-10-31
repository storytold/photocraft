#!/bin/sh
set -eu

REPO_ROOT="${SRCROOT}/../.."
CARGO_BIN="${CARGO:-${HOME}/.cargo/bin/cargo}"

case "${PLATFORM_NAME:-}" in
    iphoneos)
        RUST_TARGET="aarch64-apple-ios"
        ;;
    iphonesimulator)
        case "${CURRENT_ARCH:-arm64}" in
            arm64)
                RUST_TARGET="aarch64-apple-ios-sim"
                ;;
            *)
                echo "PhotoCraft: unsupported iPad simulator architecture: ${CURRENT_ARCH:-unknown}" >&2
                exit 1
                ;;
        esac
        ;;
    *)
        echo "PhotoCraft: unsupported Xcode platform: ${PLATFORM_NAME:-unknown}" >&2
        exit 1
        ;;
esac

if [ ! -x "${CARGO_BIN}" ]; then
    echo "PhotoCraft: cargo was not found at ${CARGO_BIN}; install Rust 1.95 with rustup" >&2
    exit 1
fi

PROFILE="debug"
if [ "${CONFIGURATION:-Debug}" = "Release" ]; then
    PROFILE="release"
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target/ios-xcode}"
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-17.0}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
if [ "${PROFILE}" = "debug" ]; then
    export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
fi

set -- build \
    --manifest-path "${REPO_ROOT}/Cargo.toml" \
    --package photocraft-ios \
    --target "${RUST_TARGET}" \
    --locked
if [ "${PROFILE}" = "release" ]; then
    set -- "$@" --release
fi
"${CARGO_BIN}" "$@"

RUST_BINARY="${CARGO_TARGET_DIR}/${RUST_TARGET}/${PROFILE}/photocraft-ios"
APP_BINARY="${TARGET_BUILD_DIR}/${EXECUTABLE_PATH}"
mkdir -p "$(dirname "${APP_BINARY}")"
cp "${RUST_BINARY}" "${APP_BINARY}"
