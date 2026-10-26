//! Windows Explorer thumbnails for PSD, PSB and PhotoCraft files.
//!
//! Explorer asks a registered `IThumbnailProvider` COM server for the preview of each `.psd`. This
//! crate is that server: it reads the embedded thumbnail resource (1036, written by PhotoCraft's
//! "Image Previews" setting) straight out of the file and hands Explorer a bitmap, with no image
//! crates involved.
//!
//! On macOS, Linux and the web this crate is empty: everything below sits behind
//! `cfg(target_os = "windows")`.

// The DLL's COM entry points (`DllGetClassObject` and friends) are plain `#[no_mangle]` exports;
// MSVC's linker notes (LNK4104) that they could also be marked PRIVATE in a .def file, which is
// not needed.
#![allow(linker_messages)]

#[cfg(any(target_os = "windows", test))]
mod pcraft;
#[cfg(any(target_os = "windows", test))]
mod psd;
#[cfg(target_os = "windows")]
mod registration;
#[cfg(target_os = "windows")]
mod windows_thumbnail;

#[cfg(target_os = "windows")]
pub use registration::{enable_explorer_thumbnails, ensure_explorer_thumbnails};
