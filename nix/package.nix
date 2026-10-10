# PhotoCraft desktop app and headless CLI, built from this checkout.
#
# Written in nixpkgs style (`callPackage`, `finalAttrs`, `lib.fileset`) so it can be upstreamed
# to pkgs/by-name with only `src` and `cargoHash` changed. The flake (../flake.nix) calls it with
# the pinned craft-fonts checkout and the commit's provenance, which makes it the same build as
# a release: packaging/linux/package.sh installs the same files.
{
  lib,
  stdenv,
  rustPlatform,
  makeBinaryWrapper,
  desktop-file-utils,
  appstream,
  versionCheckHook,

  # Loaded at runtime with dlopen (winit, wgpu, rfd), so they are not ELF NEEDED entries: the
  # same list as packaging/linux/nfpm.yaml and apps/photocraft/src/linux_libs.rs.
  dbus,
  libGL,
  libx11,
  libxcb,
  libxcursor,
  libxi,
  libxkbcommon,
  vulkan-loader,
  wayland,

  # A storytold/craft-fonts checkout to embed (crates/text/build.rs), or null for none.
  # Releases embed it at the commit pinned in .github/workflows/release.yml.
  craftFonts ? null,
  # Build provenance shown by `--version` and Help › About (crates/engine/src/build_info.rs).
  # Without them the binaries report a dev build.
  buildSha ? null,
  # YYYY-MM-DD; also the date of the AppStream release entry.
  buildDate ? null,
  # "release" for the shipped build; "debug" for the dev profile (debug assertions, full debug
  # info, unstripped) used by the photocraft-debug flake output.
  buildType ? "release",
}:

let
  appId = "ai.storyteller.photocraft";
  isDebug = buildType == "debug";

  runtimeLibraries = lib.optionals stdenv.hostPlatform.isLinux [
    dbus # libdbus-1.so.3: rfd's file dialogs talk to the xdg-desktop-portal over D-Bus
    libGL # libEGL.so.1 (libglvnd)
    libx11 # libX11.so.6, libX11-xcb.so.1
    libxcb
    libxcursor
    libxi
    libxkbcommon # libxkbcommon.so.0, libxkbcommon-x11.so.0
    vulkan-loader
    wayland # libwayland-client.so.0
  ];
in
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "photocraft" + lib.optionalString isDebug "-debug";
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  # Only what the build reads, so edits to docs, CI or this directory don't rebuild it. Every
  # workspace member must be present for cargo to load the workspace.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../.cargo
      ../Cargo.toml
      ../Cargo.lock
      ../apps
      ../crates
      ../xtask
      ../assets
      ../packaging
      ../README.md
      ../LICENSE-MIT
      ../LICENSE-APACHE
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  inherit buildType;

  __structuredAttrs = true;
  strictDeps = true;

  nativeBuildInputs = lib.optionals stdenv.hostPlatform.isLinux [ makeBinaryWrapper ];

  env =
    lib.optionalAttrs (buildSha != null) { PHOTOCRAFT_BUILD_SHA = buildSha; }
    // lib.optionalAttrs (buildDate != null) { PHOTOCRAFT_BUILD_DATE = buildDate; }
    // lib.optionalAttrs (craftFonts != null) {
      CRAFT_FONTS_DIR = "${craftFonts}";
      # A bad checkout fails the build instead of silently shipping without the fonts.
      CRAFT_FONTS_REQUIRED = "1";
    };

  cargoBuildFlags = [
    "--package=photocraft"
    "--package=photocraft-cli"
  ];
  cargoTestFlags = finalAttrs.cargoBuildFlags;

  # The whole workspace's tests run in CI; here, the two shipped packages. The debug variant
  # is a debugging aid built from the same source, so it skips them.
  doCheck = !isDebug;
  dontStrip = isDebug;

  postInstall = ''
    install -Dm644 README.md LICENSE-MIT LICENSE-APACHE -t $out/share/doc/photocraft
  ''
  # Builds that embed craft-fonts ship each font's licence (packaging/env.sh copy_font_licences).
  + lib.optionalString (craftFonts != null) ''
    for lic in ${craftFonts}/fonts/*/OFL.txt; do
      install -Dm644 "$lic" "$out/share/doc/photocraft/OFL-$(basename "$(dirname "$lic")").txt"
    done
  ''
  + lib.optionalString stdenv.hostPlatform.isLinux ''
    install -Dm644 packaging/linux/${appId}.desktop -t $out/share/applications
    install -Dm644 packaging/linux/${appId}.mime.xml $out/share/mime/packages/${appId}.xml
    mkdir -p $out/share/metainfo $out/share/icons
    substitute packaging/linux/${appId}.metainfo.xml.in $out/share/metainfo/${appId}.metainfo.xml \
      --subst-var-by VERSION ${finalAttrs.version} \
      --subst-var-by DATE ${
        if buildDate != null then buildDate else ''"$(date -u -d "@$SOURCE_DATE_EPOCH" +%F)"''
      }
    cp -R assets/app-icon/hicolor $out/share/icons/
  ''
  # The same bundle as packaging/macos/package.sh, unsigned; bin/photocraft points into it.
  + lib.optionalString stdenv.hostPlatform.isDarwin ''
    app=$out/Applications/PhotoCraft.app/Contents
    mkdir -p $app/MacOS $app/Resources
    mv $out/bin/photocraft $app/MacOS/PhotoCraft
    ln -s $app/MacOS/PhotoCraft $out/bin/photocraft
    cp assets/app-icon/photocraft.icns $app/Resources/PhotoCraft.icns
    version=${finalAttrs.version}
    substitute packaging/macos/Info.plist.in $app/Info.plist \
      --subst-var-by VERSION "$version" \
      --subst-var-by SHORT_VERSION "''${version%%-*}" \
      --subst-var-by BUILD_SHA ${if buildSha != null then buildSha else "unknown"}
  '';

  # After fixupPhase's `patchelf --shrink-rpath`, which would drop these again: none of them is
  # an ELF NEEDED entry. The start-up check (linux_libs.rs) looks for them in the host's linker
  # cache and FHS directories, which says nothing about this closure (on another distro it would
  # refuse to start over a library the RUNPATH provides), so it is switched off by default.
  postFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
    patchelf --add-rpath ${lib.makeLibraryPath runtimeLibraries} $out/bin/photocraft
    wrapProgram $out/bin/photocraft --set-default PHOTOCRAFT_SKIP_LIB_CHECK 1
  '';

  nativeInstallCheckInputs = [
    versionCheckHook
  ]
  ++ lib.optionals stdenv.hostPlatform.isLinux [
    desktop-file-utils
    appstream
  ];
  doInstallCheck = true;
  versionCheckProgramArg = "--version";
  # What packaging/linux/package.sh validates before it builds a .deb, .rpm or AppImage.
  installCheckPhase = ''
    runHook preInstallCheck
  ''
  + lib.optionalString stdenv.hostPlatform.isLinux ''
    desktop-file-validate $out/share/applications/${appId}.desktop
    appstreamcli validate --no-net --explain $out/share/metainfo/${appId}.metainfo.xml
  ''
  + ''
    $out/bin/photocraft-cli --version
    runHook postInstallCheck
  '';

  passthru = {
    # For dev shells: `cargo run` needs the same libraries on LD_LIBRARY_PATH.
    inherit runtimeLibraries;
  };

  meta = {
    description = "Open-source, native image editor that works the way Photoshop users expect";
    longDescription = ''
      PhotoCraft is a native image editor with layers, masks, adjustment layers, layer styles,
      type and brushes. It opens and saves layered PSD/PSB files, edits 8, 16 and 32-bit
      documents and composites on the GPU (wgpu). It includes photocraft-cli, a headless
      converter, command runner and MCP server.
    '';
    homepage = "https://getartcraft.com/apps/photocraft";
    license = [
      lib.licenses.mit
      lib.licenses.asl20
    ]
    # The embedded craft-fonts are OFL-1.1.
    ++ lib.optional (craftFonts != null) lib.licenses.ofl;
    mainProgram = "photocraft";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
})
