ARG STEAM_RUNTIME_IMAGE=registry.gitlab.steamos.cloud/steamrt/sniper/sdk@sha256:bb30053d3cd19d35c76ae69b2790dbaa1430448006f8240cb6d7e723f2d1940e
FROM ${STEAM_RUNTIME_IMAGE}

ARG TARGETARCH
ARG RUSTUP_VERSION
ARG RUSTUP_SHA256
ARG RUST_VERSION

SHELL ["/bin/sh", "-eux", "-c"]

# Valve's SDK contains the native compiler and desktop development libraries but not Rust. Pin
# both the rustup installer and the toolchain: a mutable installer would make release artifacts
# impossible to reproduce or audit.
RUN case "$TARGETARCH" in \
        amd64) rust_arch=x86_64 ;; \
        arm64) rust_arch=aarch64 ;; \
        *) echo "unsupported Docker architecture: $TARGETARCH" >&2; exit 2 ;; \
    esac; \
    curl --proto '=https' --tlsv1.2 -fsSLo /tmp/rustup-init \
        "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/$rust_arch-unknown-linux-gnu/rustup-init"; \
    echo "$RUSTUP_SHA256  /tmp/rustup-init" | sha256sum --check --strict; \
    chmod +x /tmp/rustup-init; \
    /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain "$RUST_VERSION"; \
    rm /tmp/rustup-init; \
    mkdir -p /cargo-cache /target; \
    chmod 0777 /cargo-cache /target

ENV RUSTUP_HOME=/root/.rustup
ENV PATH=/root/.cargo/bin:${PATH}

WORKDIR /source
