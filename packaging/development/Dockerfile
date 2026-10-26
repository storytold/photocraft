FROM ubuntu:22.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential ca-certificates curl pkg-config \
    gcc-mingw-w64-x86-64-posix g++-mingw-w64-x86-64-posix nsis \
    libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev \
    desktop-file-utils appstream zsync zip unzip python3 file git \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL -o /tmp/nfpm.deb https://github.com/goreleaser/nfpm/releases/download/v2.47.0/nfpm_2.47.0_amd64.deb \
    && dpkg -i /tmp/nfpm.deb && rm /tmp/nfpm.deb
ENV PATH=/rust/bin:$PATH CARGO_HOME=/cargo CARGO_BUILD_JOBS=2
WORKDIR /src
