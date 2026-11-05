# SteamOS builds

PhotoCraft uses its native Linux application on SteamOS. Steam Deck is x86_64; Steam Frame is
ARM64. Both builds use [Valve's Steam Linux Runtime 3.0 (Sniper) SDK][steam-runtime] rather than
compiling against the host distribution, so their glibc and shared-library baseline matches the
runtime selected by Steam.

Docker Desktop (macOS/Windows) or Docker Engine (Linux) must be running. Build each architecture
separately:

```sh
packaging/steamos/build.sh x86_64
packaging/steamos/build.sh aarch64
```

The wrapper produces the existing Linux tar artifacts in `dist/release/`:

```text
photocraft-<version>-linux-x86_64.tar.gz
photocraft-<version>-linux-aarch64.tar.gz
```

Cargo defaults to one build job because release LTO otherwise exceeds Docker
Desktop's common 4 GB memory allocation. A larger build host can override this with
`PHOTOCRAFT_STEAMOS_JOBS=<n>`.

Building x86_64 through Docker Desktop on an ARM Mac requires at least 6 GB of Docker memory;
Rosetta's x86_64 LTO process exceeds the default 4 GB allocation. Native x86_64 hosts and the
native ARM64 build do not incur that emulation overhead.

They deliberately reuse `packaging/linux/package.sh`; SteamOS is not a separate PhotoCraft shell
or package format. The SDK images, rustup installers and Rust toolchain are pinned in the build
files. Cargo registries and build outputs are kept in architecture-specific Docker volumes.

For Steam Frame testing, extract the ARM64 tarball and upload its top-level directory with the
[SteamOS Devkit Client][frame-upload]. Set the start command to `bin/photocraft` and select
**Steam Linux Runtime 3.0 ARM64 (Sniper)**. For Steam Deck, add the extracted x86_64 archive as a
non-Steam application with `bin/photocraft` as its executable and select **Steam Linux Runtime
3.0 (Sniper)** as the compatibility tool.

## CI and release integration

The tarballs deliberately retain PhotoCraft's existing Linux asset names. Release CI must make
these Sniper-built files the authoritative x86_64 and ARM64 Linux tar artifacts rather than upload
another pair with SteamOS-specific names.

This repository's workflows delegate release builds to `storytold/craft-common-actions` and its
contribution policy forbids adding a feature-specific local job. Landing this change therefore
requires a coordinated `craft-common-actions` change that invokes this architecture matrix and
uses its tarballs in the shared release workflow. The PhotoCraft feature PR itself should not edit
`.github/workflows/`.

Hardware acceptance still needs to cover Vulkan/wgpu startup, Wayland and X11 windowing, file
dialogs, clipboard, controller navigation, suspend/resume and the Steam Frame non-VR presentation.

[frame-upload]: https://partner.steamgames.com/doc/steamhardware/steamframe/loadgames
[steam-runtime]: https://github.com/ValveSoftware/steam-runtime#building-in-the-runtime
