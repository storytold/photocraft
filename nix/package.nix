{
  lib,
  stdenv,
  rustPlatform,
  pkg-config,
  makeWrapper,
  wayland,
  wayland-scanner,
  libxkbcommon,
  libx11,
  libxcb,
  libxcursor,
  libxi,
  libxrandr,
  vulkan-loader,
  libglvnd,
  dbus,
  fontconfig,
  zenity,
  xdg-utils,
  desktop-file-utils,
  appstream,
  autoAddDriverRunpath,
  craft-fonts,
  buildDate,
  systems,
}:
let
  root = ../.;
  version = (builtins.fromTOML (builtins.readFile (root + "/Cargo.toml"))).workspace.package.version;
  isLinux = stdenv.hostPlatform.isLinux;
  appId = "ai.storyteller.photocraft";
  # winit, wgpu and rfd load libraries dynamically; ELF NEEDED entries alone are insufficient.
  runtimeLibs = lib.optionals isLinux [
    wayland
    libxkbcommon
    libx11
    libxcb
    libxcursor
    libxi
    libxrandr
    vulkan-loader
    libglvnd
    dbus
    fontconfig
  ];
  runtimeTools = lib.optionals isLinux [
    fontconfig
    xdg-utils
    zenity
  ];
in
rustPlatform.buildRustPackage {
  pname = "photocraft";
  inherit version;
  src = lib.cleanSourceWith {
    src = lib.cleanSource root;
    filter =
      path: _:
      !(builtins.elem (lib.removePrefix "${toString root}/" (toString path)) [
        "target"
        "log"
        "plan"
        "corpus"
        "dist"
        "flake.nix"
        "flake.lock"
        "nix"
      ]);
  };
  cargoLock = {
    lockFile = root + "/Cargo.lock";
    allowBuiltinFetchGit = true;
  };
  cargoBuildFlags = [
    "-p"
    "photocraft"
    "-p"
    "photocraft-cli"
  ];
  buildFeatures = [
    "photocraft/heif"
    "photocraft-cli/heif"
  ];
  checkFeatures = [ "heif" ];
  # Native AppKit integration tests require a GUI session unavailable in the build sandbox.
  cargoTestFlags = [
    "-p"
    "photocraft-cli"
    "--lib"
    "--bins"
  ];
  strictDeps = true;
  nativeBuildInputs = [
    pkg-config
  ]
  ++ lib.optionals isLinux [
    makeWrapper
    wayland-scanner
    desktop-file-utils
    appstream
    autoAddDriverRunpath
  ];
  buildInputs = runtimeLibs;
  env = {
    CRAFT_FONTS_DIR = toString craft-fonts;
    CRAFT_FONTS_REQUIRED = "1";
    PHOTOCRAFT_BUILD_DATE = buildDate;
  };
  postInstall = ''
    doc=$out/share/doc/photocraft
    mkdir -p "$doc"
    cp README.md LICENSE-MIT LICENSE-APACHE NOTICE ATTRIBUTION.md "$doc/"
    cp assets/fonts/OFL-*.txt assets/icons/LICENSE-*.txt assets/dict/LICENSE-SCOWL.txt "$doc/"
    cp crates/ui-egui/src/LICENSE-solarized.txt crates/ui-egui/src/i18n/LICENSE-translations.txt \
      crates/algo/data/LICENSE-libfacedetection.txt "$doc/"
    cp assets/app-icon/LICENSE.txt "$doc/LICENSE-app-icon.txt"
    for lic in ${craft-fonts}/fonts/*/OFL.txt; do
      cp "$lic" "$doc/OFL-$(basename "$(dirname "$lic")").txt"
    done
  ''
  + lib.optionalString isLinux ''
    install -Dm644 packaging/linux/${appId}.desktop $out/share/applications/${appId}.desktop
    install -Dm644 packaging/linux/${appId}.mime.xml $out/share/mime/packages/${appId}.xml
    mkdir -p $out/share/icons $out/share/metainfo
    cp -R assets/app-icon/hicolor $out/share/icons/
    substitute packaging/linux/${appId}.metainfo.xml.in $out/share/metainfo/${appId}.metainfo.xml \
      --subst-var-by VERSION ${version} --subst-var-by DATE ${buildDate}
    desktop-file-validate $out/share/applications/${appId}.desktop
    XDG_CACHE_HOME=$TMPDIR/.cache appstreamcli validate --no-net $out/share/metainfo/${appId}.metainfo.xml
    for bin in photocraft photocraft-cli; do
      wrapProgram "$out/bin/$bin" \
        --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibs}" \
        --prefix PATH : "${lib.makeBinPath runtimeTools}"
    done
  ''
  + lib.optionalString stdenv.hostPlatform.isDarwin ''
    app=$out/Applications/PhotoCraft.app/Contents
    mkdir -p "$app/MacOS" "$app/Resources"
    mv $out/bin/photocraft "$app/MacOS/PhotoCraft"
    ln -s ../Applications/PhotoCraft.app/Contents/MacOS/PhotoCraft $out/bin/photocraft
    cp assets/app-icon/photocraft.icns "$app/Resources/PhotoCraft.icns"
    substitute packaging/macos/Info.plist.in "$app/Info.plist" \
      --subst-var-by VERSION ${version} --subst-var-by SHORT_VERSION ${lib.head (lib.splitString "-" version)} \
      --subst-var-by BUILD_SHA unknown \
      --replace-fail '<string>11.0</string>' '<string>${stdenv.hostPlatform.darwinMinVersion}</string>'
    printf 'APPL????' > "$app/PkgInfo"
  '';
  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    $out/bin/photocraft --version
    $out/bin/photocraft-cli --version
    test -s $out/share/doc/photocraft/NOTICE
    runHook postInstallCheck
  '';
  passthru = { inherit runtimeLibs runtimeTools; };
  meta = {
    description = "Native image editor with a headless CLI and MCP server";
    homepage = "https://github.com/storytold/photocraft";
    license = with lib.licenses; [
      mit
      asl20
    ];
    platforms = systems;
    mainProgram = "photocraft";
  };
}
