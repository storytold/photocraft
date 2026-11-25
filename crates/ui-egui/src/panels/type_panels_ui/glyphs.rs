//! The Glyphs panel body: the framework's `GlyphsPanel` over PhotoCraft's text engine.

use std::hash::{Hash, Hasher};

use craft::font_manager::glyphs::{CATEGORIES, GlyphCell, GlyphSource, GlyphsPanel};
use craft::ui::contract::Bind;
use serde_json::json;

use super::run;
use crate::PhotocraftApp;
use crate::type_tool;

/// The glyph cells of the shared text engine. It holds no lock between calls.
pub(crate) struct PhotoGlyphs;

/// The style of `family`'s named face `style`, as the engine's face lookup takes it.
fn char_style_for(family: &str, style: &str) -> photocraft_doc::text::CharStyle {
    let mut st = photocraft_doc::text::CharStyle { font_family: family.into(), ..Default::default() };
    photocraft_engine::type_cmds::apply_char_props(&mut st, &json!({ "fontStyle": style }));
    st
}

/// Whether `c` belongs to the panel's `category` (a [`CATEGORIES`] id).
fn in_category(c: char, category: &str) -> bool {
    let u = c as u32;
    match category {
        "Latin" => ["Basic Latin", "Latin 1", "Latin Extended-A", "Latin Extended-B"].iter().any(|name| photocraft_text::glyphs::in_category(c, name)),
        "Letterlike Symbols" => (0x2100..=0x214F).contains(&u),
        "Superscripts and Subscripts" => (0x2070..=0x209F).contains(&u) || matches!(u, 0xB9 | 0xB2 | 0xB3),
        // Alternates belong to a text selection's glyph, which the engine does not list per face.
        "Alternates for Selection" => false,
        other => photocraft_text::glyphs::in_category(c, other),
    }
}

impl GlyphSource for PhotoGlyphs {
    /// The face's mapped characters in the category; the engine lists no unencoded glyphs.
    fn cells(&self, family: &str, style: &str, category: &str) -> Vec<GlyphCell> {
        let st = char_style_for(family, style);
        let charmap = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()).fonts.charmap_for_style(&st);
        charmap.into_iter().filter(|c| category == CATEGORIES[0] || in_category(*c, category)).map(GlyphCell::Char).collect()
    }

    fn glyph(&self, family: &str, style: &str, cell: GlyphCell, px: u32) -> Option<egui::ColorImage> {
        let GlyphCell::Char(c) = cell else { return None };
        let st = char_style_for(family, style);
        let (n, alpha) = {
            let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
            photocraft_text::glyphs::preview(&mut eng, &st, c, px)
        };
        if n == 0 {
            return None;
        }
        let pixels: Vec<u8> = alpha.iter().flat_map(|a| [255, 255, 255, *a]).collect();
        Some(egui::ColorImage::from_rgba_unmultiplied([n as usize, n as usize], &pixels))
    }

    fn ready(&self) -> bool {
        photocraft_text::try_shared().is_some()
    }

    fn revision(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (photocraft_text::fonts::shared_generation(), photocraft_text::served::generation()).hash(&mut hasher);
        hasher.finish()
    }
}

/// Font family and style the panel shows: its own choice, else the type selection's font.
fn face(app: &PhotocraftApp) -> (String, String) {
    // PLAN: assumed `TypePanelsUi.glyph_face: Option<(String, String)>` (panels-w42), the panel's own pick.
    if let Some((family, style)) = &app.ui.type_panels.glyph_face {
        return (family.clone(), if style.is_empty() { tl!("Regular").into() } else { style.clone() });
    }
    let layer = app.ui.text_edit.as_ref().map(|e| photocraft_doc::LayerId(e.layer)).or_else(|| app.session.active().and_then(|s| s.active_layer));
    if let Some(st) = app.session.active()
        && let Some(id) = layer
        && let Some(photocraft_doc::LayerContent::Text(tl)) = st.doc.layer(id).map(|l| &l.content)
        && let Some(r) = tl.char_runs().first()
    {
        let family = if r.style.font_family.is_empty() { photocraft_text::fonts::DEFAULT_FAMILY.to_string() } else { r.style.font_family.clone() };
        return (family, type_tool::selected_style(&r.style));
    }
    (photocraft_text::fonts::DEFAULT_FAMILY.into(), tl!("Regular").into())
}

/// Whether a glyph has somewhere to go: a text edit, or an active type layer.
fn can_insert(app: &PhotocraftApp) -> bool {
    app.ui.text_edit.is_some()
        || app
            .session
            .active()
            .and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id)))
            .is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Text(_)))
}

/// The `glyphs` registry body.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (mut family, mut style) = face(app);
    let can = can_insert(app);
    let families = type_tool::families();
    let styles = type_tool::styles(&family);
    let out =
        GlyphsPanel::new(tl!("Glyphs"), &PhotoGlyphs, Bind::bound(&mut family), Bind::bound(&mut style), Bind::bound(&mut app.ui.type_panels.glyph_settings))
            .families(&families)
            .styles(&styles)
            .can_insert(can)
            .show(ui);
    if out.font_changed {
        let mut fonts = type_tool::font_state(app);
        fonts.settings.note_recent(&family);
        type_tool::finish_font_state(app, fonts);
        app.ui.type_panels.glyph_face = Some((family, style));
    }
    if let Some(GlyphCell::Char(c)) = out.activated
        && !insert(app, &c.to_string())
    {
        app.ui.status = tl!("Select a type layer or click into text to insert glyphs").into();
    }
}

/// Inserts `g` into the type being edited (replacing the selection), else at the end of the active type layer.
/// Returns whether it was inserted.
pub(crate) fn insert(app: &mut PhotocraftApp, g: &str) -> bool {
    let p = if let Some(ed) = app.ui.text_edit.clone() {
        let (a, b) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor));
        json!({ "layer": ed.layer, "text": g, "range": [a, b], "coalesce": ed.session })
    } else {
        json!({ "text": g })
    };
    let Some(r) = run(app, "type.insertText", p) else { return false };
    if let (Some(e), Some(c)) = (app.ui.text_edit.as_mut(), r["caret"].as_u64()) {
        e.caret = c as usize;
        e.anchor = e.caret;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of_active(app: &PhotocraftApp) -> String {
        let st = app.session.active().unwrap();
        let Some(photocraft_doc::LayerContent::Text(t)) = st.active_layer.and_then(|id| st.doc.layer(id)).map(|l| &l.content) else { panic!("not text") };
        t.text.clone()
    }

    #[test]
    fn glyph_insertion_and_recent() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 300, "height": 120, "background": "white"})).unwrap();
        app.run("type.create", json!({"x": 10, "y": 60, "text": "ab"})).unwrap();
        app.sync_views();
        let layer = app.session.active().unwrap().active_layer.unwrap().0;
        app.ui.text_edit = Some(crate::state::TextEdit { layer, document: None, caret: 2, anchor: 2, session: "glyphs".into(), created: false, mask: false });
        assert!(insert(&mut app, "→"));
        assert!(insert(&mut app, "€"));
        assert_eq!(text_of_active(&app), "ab→€");
        assert_eq!(app.ui.text_edit.as_ref().map(|e| (e.caret, e.anchor)), Some((4, 4)));
        app.ui.text_edit = None;
        assert!(insert(&mut app, "!"));
        assert_eq!(text_of_active(&app), "ab→€!");
    }
}
