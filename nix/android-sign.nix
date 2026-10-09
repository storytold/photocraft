# `nix run .#photocraft-android-sign -- [--install] [OUT.apk]`
#
# Signs the unsigned APK from `photocraft-android` with the Android debug key and, with
# --install, installs it on the device adb sees. The key is ~/.android/debug.keystore, the one
# Android Studio and Gradle use; it is created on first use. Installation still depends on the
# device ABI, Android version and policy. Keep the same key for updates. Uninstalling an app
# signed with another key deletes its app-private documents; back them up first.
{
  lib,
  writeShellApplication,
  jdk_headless,
  android-tools,
  coreutils,
  photocraft-android,
}:

let
  inherit (photocraft-android) version buildTools;
  apk = "${photocraft-android}/photocraft-${version}-android-unsigned.apk";
in
writeShellApplication {
  name = "photocraft-android-sign";
  runtimeInputs = [
    jdk_headless
    android-tools
    coreutils
  ];
  text = ''
    umask 077
    install=0
    out_set=0
    out="photocraft-${version}-android.apk"
    for arg in "$@"; do
      case "$arg" in
        --install) install=1 ;;
        -h|--help)
          echo "usage: photocraft-android-sign [--install] [OUT.apk]  (default OUT: $out)"
          exit 0 ;;
        -*) echo "unknown option: $arg" >&2; exit 2 ;;
        *)
          if [ "$out_set" = 1 ]; then
            echo "expected one output APK path" >&2
            exit 2
          fi
          out="$arg"
          out_set=1 ;;

      esac
    done

    if [ -e "$out" ]; then
      echo "output already exists: $out; choose a new path" >&2
      exit 2
    fi

    keystore="''${ANDROID_DEBUG_KEYSTORE:-$HOME/.android/debug.keystore}"
    if [ ! -f "$keystore" ]; then
      mkdir -p "$(dirname "$keystore")"
      keytool -genkeypair -keystore "$keystore" -storepass android -keypass android \
        -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 \
        -dname "CN=Android Debug,O=Android,C=US" >/dev/null
      echo "created debug key $keystore" >&2
    fi

    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    cp ${apk} "$tmp/unsigned.apk"
    chmod u+w "$tmp/unsigned.apk"
    ${buildTools}/apksigner sign --ks "$keystore" --ks-pass pass:android \
      --ks-key-alias androiddebugkey --key-pass pass:android \
      --out "$out" "$tmp/unsigned.apk"
    ${buildTools}/apksigner verify "$out"
    echo "signed: $out"

    if [ "$install" = 1 ]; then
      adb install -r "$out"
      adb shell am start -n ai.storyteller.photocraft/android.app.NativeActivity
    fi
  '';
  meta.description = "Sign the PhotoCraft Android APK with the debug key and optionally install it";
  meta.platforms = lib.platforms.linux;
}
