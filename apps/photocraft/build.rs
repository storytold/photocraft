//! Records the checkout's git commit for `--version` (`git_checkout.rs`, every target), and on
//! Windows only embeds the app icon and version info (VERSIONINFO) into `photocraft.exe`.
//!
//! On every other target the Windows part does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `PHOTOCRAFT_REQUIRE_WINRES=1` (set by the
//! release workflow) turns it into an error.

mod git_checkout;

#[path = "src/build_channel.rs"]
mod build_channel;

/// Embeds the Windows resources and delay-loads the print-only system DLLs (#5884, #8272).
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PHOTOCRAFT_BUILD_CHANNEL");
    println!("cargo:rustc-check-cfg=cfg(photocraft_nightly_v1)");
    let nightly = match std::env::var("PHOTOCRAFT_BUILD_CHANNEL").as_deref() {
        Ok("nightly-v1") => {
            println!("cargo:rustc-cfg=photocraft_nightly_v1");
            true
        }
        // An empty value (an unset CI input) means the default stable build.
        Ok("stable" | "") | Err(std::env::VarError::NotPresent) => false,
        _ => {
            eprintln!("unsupported PHOTOCRAFT_BUILD_CHANNEL (expected stable or nightly-v1)");
            std::process::exit(1);
        }
    };
    println!("cargo:rerun-if-changed=../../assets/app-icon/photocraft.ico");
    println!("cargo:rerun-if-changed=../../assets/app-icon/photocraft-nightly.ico");
    println!("cargo:rerun-if-env-changed=PHOTOCRAFT_REQUIRE_WINRES");
    println!("cargo:rerun-if-changed=git_checkout.rs");
    git_checkout::emit();
    println!("cargo:rerun-if-env-changed=PHOTOCRAFT_VERSION");
    let built_version = if nightly {
        match std::env::var("PHOTOCRAFT_VERSION") {
            Ok(version) if build_channel::is_nightly_version(&version) => version,
            _ => {
                eprintln!("nightly-v1 requires an explicit PHOTOCRAFT_VERSION feed identity");
                std::process::exit(1);
            }
        }
    } else {
        std::env::var("CARGO_PKG_VERSION").unwrap_or_default()
    };
    println!("cargo:rustc-env=PHOTOCRAFT_BUILT_VERSION={built_version}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Printing to PDF (winprint's `PdfCreateRenderer`) imports Windows.Data.Pdf.dll, and its
    // printer options (the Print Ticket API) import prntvpt.dll; stripped Windows images (NTLite,
    // some N/LTSC builds) lack them. A load-time import stops the exe from starting at all
    // ("windows.data.pdf.dll was not found", #5884; "prntvpt.dll", #8272), so load them on first
    // use; the print commands check for them first and report an error instead.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:windows.data.pdf.dll");
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:prntvpt.dll");
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon(build_channel::windows_icon_path(nightly))
        .set("ProductName", if nightly { "PhotoCraft Nightly" } else { "PhotoCraft" })
        .set("FileDescription", if nightly { "PhotoCraft Nightly image editor" } else { "PhotoCraft image editor" })
        .set("CompanyName", "Learning Machines LLC")
        .set("LegalCopyright", "Copyright (c) the PhotoCraft authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "photocraft.exe")
        .set("InternalName", "photocraft");
    if nightly {
        // Launcher verifies the PE ProductVersion against the exact feed identity, not FileVersion.
        res.set("ProductVersion", &built_version);
    }
    if let Err(e) = res.compile() {
        if std::env::var_os("PHOTOCRAFT_REQUIRE_WINRES").is_some() {
            panic!("embedding Windows resources failed: {e}");
        }
        println!("cargo:warning=photocraft.exe built without icon/version resources: {e}");
    }
}
