//! UI fallbacks from the host's runtime manifest. Unlike a build-generated `ServedFace`, that
//! manifest has no cmap coverage or static metadata. Keep the verified, downloaded bytes owned
//! by the context instead of inventing a served-face table or starting another fetch path.
use std::sync::Arc;

use craft::font_catalog::sfnt::{FaceNames, read_faces_bytes};
use craft::ui::fonts::{FontRef, SEMIBOLD, UiFonts, set_weight};
use egui::epaint::text::Blob;

// UI fallbacks are an optional cache, not an unbounded second document-font database. These
// limits also bound metadata work for hostile collections and repeated host deliveries.
const MAX_BYTES: usize = 128 << 20;
const MAX_FILE_BYTES: usize = 64 << 20;
const MAX_FACES: usize = 256;
const MAX_FILES: usize = 64;

#[derive(Clone)]
struct File {
    bytes: Blob,
    faces: Vec<FaceNames>,
}

#[derive(Clone, Default)]
struct Loaded {
    files: Vec<File>,
    bytes: usize,
    faces: usize,
}

impl Loaded {
    fn add(&mut self, bytes: &[u8], families: &[String]) -> bool {
        if bytes.is_empty()
            || bytes.len() > MAX_FILE_BYTES
            || bytes.len() > MAX_BYTES.saturating_sub(self.bytes)
            || self.files.len() >= MAX_FILES
            || self.files.iter().any(|file| file.bytes.as_ref().as_ref() == bytes)
        {
            return false;
        }
        let faces: Vec<_> = read_faces_bytes(bytes).into_iter().filter(|face| !face.italic && families.contains(&face.family)).collect();
        if faces.is_empty() || faces.len() > MAX_FACES.saturating_sub(self.faces) {
            return false;
        }
        self.bytes += bytes.len();
        self.faces += faces.len();
        // One owned copy for the UI, shared by every face/weight and later stack rebuild. The
        // document engine consumes the original Vec independently.
        self.files.push(File { bytes: Arc::new(bytes.to_vec()), faces });
        true
    }

    fn selected(&self, families: &[String]) -> Vec<(String, egui::FontData)> {
        families
            .iter()
            .filter_map(|family| {
                let (file_index, file, face) = self
                    .files
                    .iter()
                    .enumerate()
                    .flat_map(|(i, file)| file.faces.iter().map(move |face| (i, file, face)))
                    .filter(|(_, _, face)| &face.family == family)
                    .min_by_key(|(_, _, face)| face.weight.abs_diff(400))?;
                let mut data = egui::FontData::from_blob(Arc::clone(&file.bytes), face.index);
                data.tweak.set_baseline_fallback(true);
                Some((format!("photocraft-host-font-{file_index}-{}", face.index), data))
            })
            .collect()
    }
}

fn id() -> egui::Id {
    egui::Id::unique("photocraft-loaded-host-fonts")
}

/// Called only on successful host delivery, after its path and optional SRI were checked.
/// Document-only families do not become UI fallbacks merely because a document uses them.
pub(super) fn cache(ctx: &egui::Context, bytes: &[u8]) -> bool {
    let families = photocraft_text::served::script_fallback_families();
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Loaded>(id()).add(bytes, &families))
}

/// Append to the app's primary faces, retaining language order on every theme/size rebuild.
pub(crate) fn append<'a>(ctx: &egui::Context, mut fonts: UiFonts<'a>, language: &str) -> UiFonts<'a> {
    let mut families = photocraft_text::served::script_fallback_families();
    if let Some(preferred) = photocraft_text::served::fallback_for_language(language) {
        families.retain(|family| family != &preferred);
        families.insert(0, preferred);
    }
    let faces = ctx.data(|d| d.get_temp::<Loaded>(id()).map(|loaded| loaded.selected(&families))).unwrap_or_default();
    for (name, data) in faces {
        for (family, weight) in [(FontRef::proportional(), 400), (FontRef::monospace(), 400), (FontRef::named("medium"), 500), (FontRef::named(SEMIBOLD), 600)]
        {
            let mut data = data.clone();
            set_weight(&mut data, weight);
            fonts = fonts.app_face(&format!("{name}@{weight}"), data, &[family]);
        }
    }
    fonts
}

#[cfg(test)]
mod tests;
