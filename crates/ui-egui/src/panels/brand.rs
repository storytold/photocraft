//! The PhotoCraft brand mark at the left of the title bar: the app icon (the kitsune,
//! `assets/app-icon/`, see its README), where Photoshop shows its "Ps" tile. The PNG carries the
//! icon's rounded corners; the framework `Picture` stores the decoded image once and draws it at the
//! title bar's icon size.

use craft::ui::display::Picture;
use egui::Color32;
use photocraft_codecs::{ChannelLayout, SampleType};

/// 128 px: sharp at 20 pt on a 3× display.
const ICON_PNG: &[u8] = include_bytes!("../../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.photocraft.png");

/// The key the picture store keeps the decoded icon under.
const KEY: u64 = 0x7068_6f74_6f63_7266;

/// The icon's pixels (one transparent pixel if it couldn't be decoded, which a test rules out).
fn decode() -> egui::ColorImage {
    let rgba = photocraft_codecs::decode(ICON_PNG).ok().map(|img| img.convert(ChannelLayout::Rgba, SampleType::U8));
    match rgba {
        Some(img) if img.data().len() == img.width() as usize * img.height() as usize * 4 => {
            egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.data())
        }
        _ => egui::ColorImage::filled([1, 1], Color32::TRANSPARENT),
    }
}

/// The brand mark as a decorative picture: the empty label keeps a screen reader from announcing it.
pub(crate) fn mark() -> Picture<'static> {
    Picture::new("", KEY, &decode)
}

/// Paints the brand mark fitted into `rect`, reusing the cached texture.
pub(crate) fn paint_mark(ui: &egui::Ui, rect: egui::Rect) {
    use craft::ui::primitives::{Revision, TextureStore, TextureStoreOptions, fit_rect};
    let store = TextureStore::<u64>::in_context(ui.ctx(), "brand", TextureStoreOptions::thumbnails());
    let texture = store.get_or_put(ui.ctx(), KEY, Revision::default(), decode);
    let target = fit_rect(rect, texture.size_vec2());
    let whole = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0));
    ui.painter().image(texture.id, target, whole, egui::Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_decodes_to_a_rounded_128_px_tile() {
        let img = decode();
        assert_eq!(img.size, [128, 128]);
        // Rounded corners: the corner pixel is transparent, the centre opaque.
        assert_eq!(img.pixels[0].a(), 0);
        assert_eq!(img.pixels[64 * 128 + 64].a(), 255);
    }
}
