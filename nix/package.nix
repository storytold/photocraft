{ lib
, stdenv
, rustPlatform
, makeWrapper
, pkg-config
, libxkbcommon
, wayland
, xorg ? { }
, libX11 ? xorg.libX11
, libXcursor ? xorg.libXcursor
, libXi ? xorg.libXi
, libxcb ? xorg.libxcb
, vulkan-loader
, libGL
, darwin ? { }
}:

let
  runtimeLibs = lib.optionals stdenv.hostPlatform.isLinux [
    libxkbcommon
    wayland
    libX11
    libXcursor
    libXi
    libxcb
    vulkan-loader
    libGL
  ];
in
rustPlatform.buildRustPackage {
  pname = "photocraft";
  version = "0.5.0";

  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: type:
      let
        baseName = baseNameOf path;
      in
        ! (type == "directory" && (
          baseName == "target" ||
          baseName == "dist" ||
          baseName == "corpus" ||
          baseName == "log" ||
          baseName == "plan" ||
          baseName == ".git"
        ));
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs = lib.optionals stdenv.hostPlatform.isLinux runtimeLibs
    ++ lib.optionals stdenv.hostPlatform.isDarwin (with darwin.apple_sdk.frameworks; [
      AppKit
      Metal
      CoreGraphics
      Security
      Foundation
    ]);

  buildFeatures = [ "heif" ];
  cargoBuildFlags = [
    "-p" "photocraft"
    "-p" "photocraft-cli"
  ];

  postInstall = ''
    install -Dm644 packaging/linux/ai.storyteller.photocraft.desktop $out/share/applications/ai.storyteller.photocraft.desktop
    install -Dm644 packaging/linux/ai.storyteller.photocraft.mime.xml $out/share/mime/packages/ai.storyteller.photocraft.xml

    mkdir -p $out/share/metainfo
    sed -e "s/@VERSION@/0.5.0/g" -e "s/@DATE@/2026-10-09/g" \
      packaging/linux/ai.storyteller.photocraft.metainfo.xml.in > $out/share/metainfo/ai.storyteller.photocraft.metainfo.xml

    mkdir -p $out/share/icons
    cp -R assets/app-icon/hicolor $out/share/icons/
  '';

  postFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
    wrapProgram $out/bin/photocraft \
      --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibs}"
    wrapProgram $out/bin/photocraft-cli \
      --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibs}"
  '';

  meta = with lib; {
    description = "Open-source, native image editor that works the way Photoshop users expect";
    homepage = "https://getartcraft.com/apps/photocraft";
    license = with licenses; [ mit asl20 ];
    mainProgram = "photocraft";
    platforms = platforms.linux ++ platforms.darwin;
  };
}
