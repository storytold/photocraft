# PhotoCraft for Android: apps/photocraft-android as a NativeActivity APK, built without Gradle.
#
# cargo cross-compiles the app to a shared library with the NDK's clang, aapt2 links
# packaging/android/AndroidManifest.xml and the launcher icons into an APK, the library goes in
# as lib/<abi>/libphotocraft_android.so, and zipalign aligns it. The APK is not signed: a signing
# key does not belong in the Nix store. `nix run .#photocraft-android-sign` signs it with the
# Android debug key (~/.android/debug.keystore) and can install it with adb.
#
# Needs a nixpkgs instance with `config.android_sdk.accept_license = true` and the rust-overlay
# overlay (`rust-bin`), which provides the Android targets' standard library.
{
  lib,
  callPackage,
  stdenv,
  stdenvNoCC,
  rustPlatform,
  rust-bin,
  rustc,
  androidenv,
  zip,

  craftFonts ? null,
  buildSha ? null,
  buildDate ? null,
  # Android ABIs to include: "arm64-v8a" for phones and tablets, "x86_64" for the emulator.
  abis ? [ "arm64-v8a" ],
}:

let
  appId = "ai.storyteller.photocraft";
  # Prototype baseline. Runtime compatibility still needs a device or emulator test.
  minSdk = 28;
  targetSdk = 36;
  ndkVersion = "29.0.14206865";
  buildToolsVersion = "36.1.0";

  version = (lib.importTOML ../Cargo.toml).workspace.package.version;
  # 0.3.0 -> 3000; 1.2.3 -> 1002003. Keep codes monotonic for upgrades.
  versionCode =
    let
      parts = map lib.toInt (lib.splitString "." (lib.head (lib.splitString "-" version)));
    in
    lib.foldl' (acc: p: acc * 1000 + p) 0 parts;

  triples = {
    "arm64-v8a" = "aarch64-linux-android";
    "x86_64" = "x86_64-linux-android";
  };
  tripleOf =
    abi:
    triples.${abi}
      or (throw "photocraft-android: unknown ABI ${abi}; use one of ${toString (lib.attrNames triples)}");

  # The same Rust release as the desktop package, plus the Android standard libraries.
  rustToolchain = rust-bin.stable.${rustc.version}.minimal.override {
    targets = map tripleOf abis;
    extensions = [ "clippy" ];
  };

  sdk =
    (androidenv.composeAndroidPackages {
      platformVersions = [ (toString targetSdk) ];
      buildToolsVersions = [ buildToolsVersion ];
      includeNDK = true;
      ndkVersions = [ ndkVersion ];
      includeEmulator = false;
      includeSystemImages = false;
      includeSources = false;
    }).androidsdk;
  sdkRoot = "${sdk}/libexec/android-sdk";
  ndkBin = "${sdkRoot}/ndk/${ndkVersion}/toolchains/llvm/prebuilt/linux-x86_64/bin";
  buildTools = "${sdkRoot}/build-tools/${buildToolsVersion}";

  # cargo and the cc crate (blake3, android-activity) both need the NDK compiler for each target.
  targetEnv = lib.concatMapStrings (
    abi:
    let
      triple = tripleOf abi;
      envTriple = lib.replaceStrings [ "-" ] [ "_" ] triple;
      clang = "${ndkBin}/${triple}${toString minSdk}-clang";
    in
    ''
      export CARGO_TARGET_${lib.toUpper envTriple}_LINKER=${clang}
      export CC_${envTriple}=${clang}
      export AR_${envTriple}=${ndkBin}/llvm-ar
    ''
  ) abis;

  # Launcher icons: the hicolor PNGs at Android's density buckets (48 dp).
  densities = {
    mdpi = "48x48";
    hdpi = "64x64"; # 72 px wanted; the nearest size the repository ships
    xhdpi = "128x128"; # 96 px wanted
    xxhdpi = "256x256"; # 144 px wanted
    xxxhdpi = "256x256"; # 192 px wanted
  };
in
stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "photocraft-android";
  inherit version;

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

  cargoDeps = callPackage ./android-input.nix {
    cargoDeps = rustPlatform.importCargoLock { lockFile = ../Cargo.lock; };
  };

  # The library is stripped with the target NDK below, not the host ELF tools.
  dontStrip = true;
  dontPatchELF = true;

  nativeBuildInputs = [
    rustPlatform.cargoSetupHook
    rustToolchain
    stdenv.cc # Host linker for the policy and patched-egui regression tests.
    zip
  ];

  env =
    lib.optionalAttrs (buildSha != null) { PHOTOCRAFT_BUILD_SHA = buildSha; }
    // lib.optionalAttrs (buildDate != null) { PHOTOCRAFT_BUILD_DATE = buildDate; }
    // lib.optionalAttrs (craftFonts != null) {
      CRAFT_FONTS_DIR = "${craftFonts}";
      CRAFT_FONTS_REQUIRED = "1";
    };

  buildPhase = ''
    runHook preBuild
    ${targetEnv}
  ''
  + lib.concatMapStrings (abi: ''
    cargo build --release --frozen --package photocraft-android --target ${tripleOf abi} -j $NIX_BUILD_CORES
    install -Dm755 target/${tripleOf abi}/release/libphotocraft_android.so apk/lib/${abi}/libphotocraft_android.so
    ${ndkBin}/llvm-strip --strip-unneeded apk/lib/${abi}/libphotocraft_android.so
  '') abis
  + ''
    mkdir -p res
    ${lib.concatStrings (
      lib.mapAttrsToList (density: size: ''
        install -Dm644 assets/app-icon/hicolor/${size}/apps/${appId}.png res/mipmap-${density}/ic_launcher.png
      '') densities
    )}
    ${buildTools}/aapt2 compile --dir res -o res.zip
    substitute packaging/android/AndroidManifest.xml AndroidManifest.xml \
      --subst-var-by VERSION ${version} \
      --subst-var-by VERSION_CODE ${toString versionCode}
    ${buildTools}/aapt2 link -o unaligned.apk \
      -I ${sdkRoot}/platforms/android-${toString targetSdk}/android.jar \
      --manifest AndroidManifest.xml \
      --min-sdk-version ${toString minSdk} --target-sdk-version ${toString targetSdk} \
      res.zip
    install -Dm644 LICENSE-MIT LICENSE-APACHE -t apk/assets/licenses
    ${lib.optionalString (craftFonts != null) ''
      for lic in ${craftFonts}/fonts/*/OFL.txt; do
        install -Dm644 "$lic" "apk/assets/licenses/OFL-$(basename "$(dirname "$lic")").txt"
      done
    ''}
    (cd apk && zip -q -r ../unaligned.apk lib assets)
    ${buildTools}/zipalign -P 16 -f 4 unaligned.apk photocraft.apk
    runHook postBuild
  '';

  # Type-check and lint the Android-only shell. Host tests cannot reach its cfg-gated code.
  doCheck = true;
  checkPhase = ''
    runHook preCheck
    ${targetEnv}
    cargo test --release --frozen --package photocraft-android --lib --target ${stdenv.hostPlatform.rust.rustcTarget} -j $NIX_BUILD_CORES
    cp -r ${./tests/pointer-cancel} pointer-cancel-check
    chmod -R u+w pointer-cancel-check
    cargo test --offline --release --manifest-path pointer-cancel-check/Cargo.toml --target ${stdenv.hostPlatform.rust.rustcTarget} -j $NIX_BUILD_CORES
    ${lib.concatMapStrings (abi: ''
      cargo clippy --release --frozen --package photocraft-android --target ${tripleOf abi} -j $NIX_BUILD_CORES -- -D warnings
    '') abis}
    runHook postCheck
  '';

  installPhase = ''
    runHook preInstall
    install -Dm644 photocraft.apk $out/photocraft-${version}-android-unsigned.apk
    cp -r apk/lib $out/lib
    install -Dm644 LICENSE-MIT LICENSE-APACHE README.md -t $out/share/doc/photocraft
    ${lib.optionalString (craftFonts != null) ''
      for lic in ${craftFonts}/fonts/*/OFL.txt; do
        install -Dm644 "$lic" "$out/share/doc/photocraft/OFL-$(basename "$(dirname "$lic")").txt"
      done
    ''}
    runHook postInstall
  '';

  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    ${buildTools}/aapt2 dump badging $out/photocraft-${version}-android-unsigned.apk | tee badging.txt
    grep -q "package: name='${appId}' versionCode='${toString versionCode}'" badging.txt
    ${lib.concatMapStrings (abi: ''
      grep -q "native-code:.*'${abi}'" badging.txt
      ${ndkBin}/llvm-nm --dynamic --defined-only $out/lib/${abi}/libphotocraft_android.so > symbols.txt
      grep -q ' T android_main$' symbols.txt
      grep -q ' T ANativeActivity_onCreate$' symbols.txt
      ${ndkBin}/llvm-readelf --program-headers --wide $out/lib/${abi}/libphotocraft_android.so > segments.txt
      # Android devices can use 16 KiB pages. Every load segment must support them.
      awk '$1 == "LOAD" { if ($NF != "0x4000" && $NF != "0x10000") exit 1; found=1 } END { if (!found) exit 1 }' segments.txt
    '') abis}
    ${buildTools}/zipalign -c -P 16 4 $out/photocraft-${version}-android-unsigned.apk
    runHook postInstallCheck
  '';

  passthru = {
    inherit
      sdk
      sdkRoot
      buildTools
      rustToolchain
      targetEnv
      ;
  };

  meta = {
    description = "PhotoCraft image editor for Android (unsigned APK)";
    homepage = "https://getartcraft.com/apps/photocraft";
    license = [
      lib.licenses.mit
      lib.licenses.asl20
    ]
    ++ lib.optional (craftFonts != null) lib.licenses.ofl;
    platforms = [ "x86_64-linux" ];
  };
})
