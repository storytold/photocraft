//! Windows only: embed the app icon and version info (VERSIONINFO) into `photocraft.exe`.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `PHOTOCRAFT_REQUIRE_WINRES=1` (set by the
//! release workflow) turns it into an error.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Fork: the "Sp" tile (`assets/app-icon/sp/`) instead of upstream's kitsune.
    println!("cargo:rerun-if-changed=../../assets/app-icon/sp/sp.ico");
    println!("cargo:rerun-if-env-changed=PHOTOCRAFT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Printing to PDF (winprint's `PdfCreateRenderer`) imports Windows.Data.Pdf.dll, which
    // stripped Windows images (NTLite, some N/LTSC builds) lack. A load-time import stops the exe
    // from starting at all ("windows.data.pdf.dll was not found", #5884), so load it on first use;
    // the print command checks for it first and reports an error instead.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:windows.data.pdf.dll");
        // Fork: the same for the CUDA libraries candle's cudarc imports when the native engine
        // is built in (`native-diffusion`): only the `--engine` worker calls them, with the
        // runtime folder on its PATH, so the editor starts on a PC that has none
        // (plan-native-engine §10). cuBLAS carries the toolkit's major version
        // (`CUDARC_CUDA_VERSION` 13030 → `cublas64_13.dll`); cuRAND stays `curand64_10.dll`.
        println!("cargo:rerun-if-env-changed=CUDARC_CUDA_VERSION");
        if std::env::var_os("CARGO_FEATURE_NATIVE_DIFFUSION").is_some() {
            let major = std::env::var("CUDARC_CUDA_VERSION").ok().and_then(|v| v.trim().parse::<u32>().ok()).map_or(13, |v| v / 1000);
            println!("cargo:rustc-link-arg-bins=/DELAYLOAD:cublas64_{major}.dll");
            println!("cargo:rustc-link-arg-bins=/DELAYLOAD:cublasLt64_{major}.dll");
            println!("cargo:rustc-link-arg-bins=/DELAYLOAD:curand64_10.dll");
        }
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/sp/sp.ico")
        .set("ProductName", "ShotoPhopEX")
        .set("FileDescription", "ShotoPhopEX image editor")
        .set("CompanyName", "Learning Machines LLC")
        .set("LegalCopyright", "Copyright (c) the PhotoCraft authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "photocraft.exe")
        .set("InternalName", "photocraft");
    if let Err(e) = res.compile() {
        if std::env::var_os("PHOTOCRAFT_REQUIRE_WINRES").is_some() {
            panic!("embedding Windows resources failed: {e}");
        }
        println!("cargo:warning=photocraft.exe built without icon/version resources: {e}");
    }
}
