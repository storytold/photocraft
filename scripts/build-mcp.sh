#!/usr/bin/env bash
# Build the PhotoCraft MCP server (photocraft-cli) in release mode and run its test suites.
# Linux and macOS. Windows: scripts/build-mcp.ps1.
#
#   scripts/build-mcp.sh                  # build + test
#   scripts/build-mcp.sh --install        # ... then `cargo install` photocraft-cli onto PATH
#   scripts/build-mcp.sh --check-targets  # ... also type-check the MCP crates for the other
#                                         #     desktop targets whose std is installed (rustup)
#   scripts/build-mcp.sh --no-test        # build only
#
# Honours CARGO_TARGET_DIR. Pass extra cargo flags in CARGO_FLAGS (for example
# CARGO_FLAGS="--features heif").
set -euo pipefail

cd "$(dirname "$0")/.."

install=0
check_targets=0
run_tests=1
for arg in "$@"; do
    case "$arg" in
        --install) install=1 ;;
        --check-targets) check_targets=1 ;;
        --no-test) run_tests=0 ;;
        -h | --help)
            sed -n '2,13p' "$0"
            exit 0
            ;;
        *)
            echo "build-mcp.sh: unknown option $arg (see --help)" >&2
            exit 2
            ;;
    esac
done

# shellcheck disable=SC2206 # CARGO_FLAGS is split on purpose
flags=(${CARGO_FLAGS:-})

echo "==> building photocraft-cli (release)"
# Take the binary's path from Cargo's own report: with --target or CARGO_BUILD_TARGET it lives
# under target/<triple>/release, not target/release. Diagnostics still go to stderr.
bin="$(cargo build --release --locked -p photocraft-cli ${flags[@]+"${flags[@]}"} --message-format=json-render-diagnostics |
    grep '"name":"photocraft-cli"' | sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' | tail -n 1)"
if [ -z "$bin" ] || [ ! -x "$bin" ]; then
    echo "build-mcp.sh: cargo did not report a photocraft-cli executable" >&2
    exit 1
fi

if [ "$run_tests" = 1 ]; then
    echo "==> testing photocraft-automation and photocraft-cli (unit, MCP protocol, image, file-permission tests)"
    cargo test --locked -p photocraft-automation -p photocraft-cli --lib --tests ${flags[@]+"${flags[@]}"}
fi

if [ "$check_targets" = 1 ]; then
    host="$(rustc -vV | sed -n 's/^host: //p')"
    installed="$(rustup target list --installed 2>/dev/null || true)"
    for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-msvc aarch64-pc-windows-msvc x86_64-pc-windows-gnu aarch64-apple-darwin x86_64-apple-darwin; do
        [ "$target" = "$host" ] && continue
        if printf '%s\n' "$installed" | grep -qx "$target"; then
            echo "==> cargo check --target $target"
            cargo check --locked -p photocraft-automation -p photocraft-cli --target "$target" ${flags[@]+"${flags[@]}"}
        else
            echo "--- skipping $target (rustup target add $target to check it)"
        fi
    done
fi

echo "==> built $bin"
"$bin" --version

if [ "$install" = 1 ]; then
    echo "==> installing photocraft-cli with cargo install"
    cargo install --path apps/photocraft-cli --locked ${flags[@]+"${flags[@]}"}
    echo "Installed into ${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin; register it with your MCP client (docs/mcp.md)."
fi
