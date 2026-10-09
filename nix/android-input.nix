# Patch only the APK's Cargo vendor tree. Desktop and web use the original locked crates.
# Keep the rejection policy in the app's source tree so host tests exercise the exact file
# compiled into winit. Upgrading any of these dependencies must rebase and retest this patch.
{
  runCommand,
  patch,
  python3,
  cargoDeps,
}:
runCommand "cargo-vendor-dir"
  {
    nativeBuildInputs = [
      patch
      python3
    ];
  }
  ''
    mkdir -p "$out"
    cp -r --no-dereference ${cargoDeps}/. "$out/"
    chmod u+w "$out"
    for dep in winit-0.30.13 egui-0.36.2 egui-winit-0.36.2; do
      source="$(readlink -f "$out/$dep")"
      test -L "$out/$dep"
      rm "$out/$dep"
      cp -r "$source" "$out/$dep"
      chmod -R u+w "$out/$dep"
    done
    patch --batch --fuzz=0 -d "$out" -p1 < ${./patches/android-palm-rejection.patch}
    cp ${../apps/photocraft-android/src/palm_rejection.rs} "$out/winit-0.30.13/src/platform_impl/android/palm_rejection.rs"
    python3 ${./refresh-cargo-checksums.py} "$out" winit-0.30.13 egui-0.36.2 egui-winit-0.36.2
  ''
