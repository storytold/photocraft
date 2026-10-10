//! PhotoCraft platform services for Android's application-private filesystem.
//!
//! This shell never treats a content:// URI as a path: access to photos in other
//! apps requires Android's Storage Access Framework, which is not implemented
//! here. Only files under AndroidApp::internal_data_path()/Documents are used.

use std::path::{Path, PathBuf};

use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_ui_egui::{ExportSettings, Services};

use crate::PendingDialogs;

fn checked_document_path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    // Desktop Save can pass a full path. Android saves only the leaf filename
    // under our private Documents directory. Do not accept path traversal.
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or_default();
    if leaf.is_empty() || leaf == "." || leaf == ".." || leaf.contains('\0') {
        return Err("invalid document file name".into());
    }
    Ok(dir.join(leaf))
}

fn export_options(settings: &ExportSettings) -> photocraft_io::ExportOptions {
    let mut opts = photocraft_io::ExportOptions::default();
    if let Some(quality) = settings.jpeg_quality {
        opts.encode.jpeg_quality = quality;
    }
    opts.encode.webp_lossless = settings.webp_lossless;
    if let Some(quality) = settings.webp_quality {
        opts.encode.webp_quality = quality;
    }
    opts.tiff_layers = settings.tiff_layers;
    opts.xmp = if settings.xmp_all {
        photocraft_io::XmpEmbed::All
    } else {
        photocraft_io::XmpEmbed::None
    };
    opts
}

/// Native import/export implementations are shared with desktop and web.
/// Persistent files remain inside the app's private Documents directory.
pub fn android_services(data_dir: Option<PathBuf>, dialogs: PendingDialogs) -> Services {
    let documents_dir = data_dir.as_ref().map(|dir| dir.join("Documents"));
    if let Some(dir) = &documents_dir {
        if let Err(err) = std::fs::create_dir_all(dir) {
            log::error!("Cannot create Android Documents directory: {err}");
        }
    }

    let prefs = data_dir.map(|dir| dir.join("preferences.json"));
    let prefs_read = prefs.clone();
    let prefs_write = prefs;

    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], max_svg_group_depth: usize| {
            photocraft_io::import_with_svg_group_depth(name, bytes, max_svg_group_depth)
                .map(|r| (r.document, r.warnings))
                .map_err(|err| err.to_string())
        })),
        export: Some(Box::new(|doc: &Document, name: &str, settings: &ExportSettings| {
            photocraft_io::export(doc, name, &export_options(settings))
                .map(|r| (r.bytes, r.warnings))
                .map_err(|err| err.to_string())
        })),
        encode_png: Some(Box::new(|width, height, rgba| {
            let img = Image::from_u8(width, height, ChannelLayout::Rgba, rgba.to_vec())
                .map_err(|err| err.to_string())?;
            photocraft_codecs::encode(
                &img,
                photocraft_codecs::Format::Png,
                &EncodeOptions::default(),
            )
            .map_err(|err| err.to_string())
        })),
        file_dialog: Some(Box::new(move |request, _parent, reply| {
            dialogs.borrow_mut().push_back((request, reply));
        })),
        write: Some(Box::new(move |name: &str, bytes: &[u8]| {
            let dir = documents_dir.as_ref().ok_or("Android application data directory unavailable")?;
            let path = checked_document_path(dir, name)?;
            photocraft_format::atomic_write(&path, bytes).map_err(|err| err.to_string())
        })),
        load_prefs: Some(Box::new(move || {
            std::fs::read_to_string(prefs_read.as_ref()?).ok()
        })),
        save_prefs: Some(Box::new(move |text: &str| {
            let path = prefs_write.as_ref().ok_or("Android application data directory unavailable")?;
            photocraft_format::atomic_write(path, text.as_bytes()).map_err(|err| err.to_string())
        })),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::checked_document_path;
    use std::path::Path;

    #[test]
    fn documents_never_escape_private_directory() {
        let dir = Path::new("/private/Documents");
        assert_eq!(checked_document_path(dir, "picture.png").unwrap(), dir.join("picture.png"));
        assert_eq!(checked_document_path(dir, "../escape.png").unwrap(), dir.join("escape.png"));
        assert_eq!(checked_document_path(dir, "folder\\escape.png").unwrap(), dir.join("escape.png"));
        assert!(checked_document_path(dir, "..").is_err());
        assert!(checked_document_path(dir, "").is_err());
    }
}
