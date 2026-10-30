//! New Document icon vocabulary. Keys are the catalog's canonical (untranslated) category and
//! preset names, never displayed translations, dimensions, or user-supplied snapshot names.

use crate::{icons, theme::Tokens};

/// Unknown categories keep a neutral document symbol until explicit metadata is added.
pub(super) fn category_icon(category: &str) -> &'static str {
    match category {
        "Recent" => "clock",
        "Saved" => "save",
        "Photo" => "image",
        "Print" => "file",
        "Art & Illustration" => "brush",
        "Web" => "app-window",
        "Mobile" => "smartphone",
        "Film & Video" => "clapperboard",
        _ => "file",
    }
}

/// Only explicit built-in device identities override the category symbol. Keep this table
/// independent of orientation and preferred resolution, which change a preset's dimensions.
pub(super) fn builtin_icon(category: &str, preset: &str) -> &'static str {
    match (category, preset) {
        ("Web", "MacBook Pro 16\"") => "laptop",
        ("Web", "iMac 24\"") => "monitor",
        ("Mobile", "iPad Pro 13\"") => "tablet",
        ("Mobile", "Apple Watch 45mm") => "watch",
        _ => category_icon(category),
    }
}

/// Icon and text share one accessible button and hit target. Wrapping belongs to the parent row.
pub(super) fn category_tab(ui: &mut egui::Ui, category: &str, selected: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let tint = if selected { t.text } else { t.text_dim };
    let label = tl!(category);
    let response = ui
        .add(
            egui::Button::new((icons::image(category_icon(category), 16.0, tint), egui::RichText::new(label).size(13.0).color(tint)))
                .gap(4.0)
                .wrap_mode(egui::TextWrapMode::Extend)
                .frame(false),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, ui.is_enabled(), selected, label));
    if selected {
        ui.painter().line_segment([response.rect.left_bottom(), response.rect.right_bottom()], egui::Stroke::new(2.0, t.text));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::new_doc_ui::{CATEGORIES, CLIPBOARD, Card, at_pref_resolutions, oriented};
    use photocraft_engine::prefs::UnitsAndRulers;
    use serde_json::{Map, json};

    #[test]
    fn categories_and_explicit_devices_use_embedded_distinct_icons() {
        let mut categories = std::collections::HashSet::new();
        for category in CATEGORIES.iter().map(|c| c.0).chain(["Saved"]) {
            let icon = category_icon(category);
            assert!(icons::exists(icon), "{category}: {icon}");
            assert!(categories.insert(icon), "{category} must be visually distinct");
        }
        for (category, preset, expected) in [
            ("Photo", "Portrait, 4 x 6", "image"),
            ("Print", "A4", "file"),
            ("Art & Illustration", "Poster", "brush"),
            ("Web", "Web Large", "app-window"),
            ("Web", "MacBook Pro 16\"", "laptop"),
            ("Web", "iMac 24\"", "monitor"),
            ("Mobile", "iPhone 16", "smartphone"),
            ("Mobile", "Android 1080p", "smartphone"),
            ("Mobile", "iPad Pro 13\"", "tablet"),
            ("Mobile", "Apple Watch 45mm", "watch"),
            ("Film & Video", "UHD 8K", "clapperboard"),
        ] {
            assert!(CATEGORIES.iter().any(|c| c.0 == category && c.1.iter().any(|p| p.0 == preset)));
            assert_eq!(builtin_icon(category, preset), expected);
            assert!(icons::exists(expected));
        }
    }

    #[test]
    fn snapshot_and_unknown_sources_never_guess_category_from_title_or_dimensions() {
        use photocraft_engine::document_preset_cmds::DocumentSettings;
        let settings: DocumentSettings = serde_json::from_value(json!({
            "width":2064,"height":2752,"resolution":72,"mode":"rgb","depth":8,"background":"white","unit":"px","resolutionUnit":"in"
        }))
        .unwrap();
        assert_eq!(Card::Recent(settings).icon("Mobile"), "clock", "even a matching tablet size has no category provenance");
        for name in ["iPad Pro 13\"", "UHD 8K", "Custom", "Clipboard"] {
            assert_eq!(builtin_icon("Saved", name), "save");
            assert_eq!(builtin_icon("Recent", name), "clock");
            assert_eq!(builtin_icon("future category", name), "file");
        }
        assert_eq!(Card::Builtin((CLIPBOARD, 300000, 1, 72.0)).icon("Recent"), "clipboard");
        assert!(icons::exists("clipboard"));
    }

    #[test]
    fn icon_identity_survives_orientation_and_resolution() {
        let units = UnitsAndRulers { print_resolution: 200.0, screen_resolution: 96.0, ..Default::default() };
        for (category, presets) in CATEGORIES {
            for preset in *presets {
                let expected = builtin_icon(category, preset.0);
                for orientation in ["portrait", "landscape"] {
                    let mut fields = Map::new();
                    fields.insert("__orientation".into(), json!(orientation));
                    let displayed = oriented(preset, &fields);
                    let scaled = at_pref_resolutions(&displayed, &units);
                    assert_eq!(Card::Builtin(displayed).icon(category), expected);
                    assert_eq!(Card::Builtin(scaled).icon(category), expected);
                }
            }
        }
    }
    /// Exercise the actual icon + label controls, including wrap and their shared hit rectangles.
    #[test]
    fn category_icons_and_translated_labels_fit_across_themes_scales_and_widths() {
        for theme in crate::theme::ThemeKind::ALL {
            let ctx = egui::Context::default();
            crate::PhotocraftApp::setup_context(&ctx, theme);
            for scale in [1.0, 2.0] {
                ctx.set_pixels_per_point(scale);
                for width in [520.0, 800.0] {
                    for lang in crate::i18n::Lang::all() {
                        crate::i18n::with_language(lang, || {
                            let mut output = ctx.run_ui(
                                egui::RawInput {
                                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width + 16.0, 300.0))),
                                    ..Default::default()
                                },
                                |ui| {
                                    ui.set_width(width);
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
                                        let mut available = ui.max_rect();
                                        // The wrapped row starts one line high and grows during layout.
                                        available.max.y = ui.clip_rect().bottom();
                                        let mut previous: Option<egui::Rect> = None;
                                        for category in CATEGORIES.iter().map(|c| c.0).chain(["Saved"]) {
                                            let response = category_tab(ui, category, category == "Mobile");
                                            assert!(
                                                available.contains_rect(response.rect),
                                                "{theme:?} {} {scale} {width} {category}: {:?} in {available:?}",
                                                lang.code(),
                                                response.rect
                                            );
                                            assert!(response.rect.height() >= 16.0 && response.rect.width() >= 16.0);
                                            if let Some(previous) = previous {
                                                assert!(!previous.intersects(response.rect), "overlapping category targets");
                                            }
                                            previous = Some(response.rect);
                                        }
                                    });
                                },
                            );
                            // This headless geometry test intentionally has no renderer.
                            output.textures_delta.clear();
                        });
                    }
                }
            }
        }
    }
}
