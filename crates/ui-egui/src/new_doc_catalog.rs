//! New Document formats, aspect ratios and localized search.

/// A blank-document preset: (name, width px, height px, ppi).
pub type Preset = (&'static str, u32, u32, f32);

/// Blank-document formats; the legacy Recent group remains addressable by automation.
pub const CATEGORIES: &[(&str, &[Preset])] = &[
    (
        "Popular",
        &[
            ("Stories / Reels / Shorts", 1080, 1920, 72.0),
            ("Instagram post, 3:4", 1080, 1440, 72.0),
            ("Instagram post, 4:5", 1080, 1350, 72.0),
            ("Square post", 1080, 1080, 72.0),
            ("YouTube thumbnail, 4K", 3840, 2160, 72.0),
            ("HDTV 1080p", 1920, 1080, 72.0),
        ],
    ),
    (
        "Social Media",
        &[
            ("Stories / Reels / Shorts", 1080, 1920, 72.0),
            ("Instagram post, 3:4", 1080, 1440, 72.0),
            ("Instagram post, 4:5", 1080, 1350, 72.0),
            ("Square post", 1080, 1080, 72.0),
            ("Landscape post", 1200, 630, 72.0),
            ("YouTube thumbnail, 4K", 3840, 2160, 72.0),
            ("Shorts thumbnail, 4K", 2160, 3840, 72.0),
            ("Video thumbnail, HD", 1280, 720, 72.0),
        ],
    ),
    ("Recent", &[("Default PhotoCraft Size", 2100, 1500, 300.0), ("HDTV 1080p", 1920, 1080, 72.0)]),
    (
        "Photo",
        &[
            ("Landscape, 6 x 4", 1800, 1200, 300.0),
            ("Landscape, 7 x 5", 2100, 1500, 300.0),
            ("Landscape, 10 x 8", 3000, 2400, 300.0),
            ("Portrait, 4 x 6", 1200, 1800, 300.0),
            ("Portrait, 5 x 7", 1500, 2100, 300.0),
            ("Square, 5 x 5", 1500, 1500, 300.0),
        ],
    ),
    (
        "Print",
        &[
            ("Letter", 2550, 3300, 300.0),
            ("Legal", 2550, 4200, 300.0),
            ("Tabloid", 3300, 5100, 300.0),
            ("A4", 2480, 3508, 300.0),
            ("A3", 3508, 4961, 300.0),
            ("A5", 1748, 2480, 300.0),
        ],
    ),
    (
        "Art & Illustration",
        &[("Poster", 5400, 7200, 300.0), ("Postcard", 1800, 1200, 300.0), ("Comic Book", 1988, 3075, 300.0), ("Square, 12 x 12", 3600, 3600, 300.0)],
    ),
    (
        "Web",
        &[
            ("Web Most Common", 1366, 768, 72.0),
            ("Web Minimum", 1024, 768, 72.0),
            ("Web Large", 1920, 1080, 72.0),
            ("MacBook Pro 16\"", 3456, 2234, 72.0),
            ("iMac 24\"", 4480, 2520, 72.0),
        ],
    ),
    (
        "Mobile",
        &[
            ("iPhone 16", 1179, 2556, 72.0),
            ("iPhone 16 Pro Max", 1320, 2868, 72.0),
            ("iPad Pro 13\"", 2064, 2752, 72.0),
            ("Android 1080p", 1080, 1920, 72.0),
            ("Apple Watch 45mm", 396, 484, 72.0),
        ],
    ),
    (
        "Film & Video",
        &[
            ("HDTV 1080p", 1920, 1080, 72.0),
            ("HDTV 720p", 1280, 720, 72.0),
            ("UHD 4K", 3840, 2160, 72.0),
            ("DCI 4K", 4096, 2160, 72.0),
            ("UHD 8K", 7680, 4320, 72.0),
        ],
    ),
];

/// Reduced integer ratio, also used by search (never a rounded decimal approximation).
pub fn aspect_ratio(w: u32, h: u32) -> String {
    let (mut a, mut b) = (w.max(1), h.max(1));
    while b != 0 {
        (a, b) = (b, a % b);
    }
    format!("{}:{}", w.max(1) / a, h.max(1) / a)
}

fn aliases(name: &str) -> &'static str {
    match name {
        "Stories / Reels / Shorts" => "instagram tiktok youtube vertical",
        "Instagram post, 3:4" | "Instagram post, 4:5" | "Square post" | "Landscape post" => "instagram facebook social",
        "YouTube thumbnail, 4K" | "Shorts thumbnail, 4K" => "youtube cover",
        "Video thumbnail, HD" => "youtube cover hd",
        _ => "",
    }
}

pub fn matches(p: &Preset, query: &str, lang: crate::i18n::Lang) -> bool {
    let haystack = format!("{} {} {} {}x{} {} {}", p.0, crate::i18n::tr(lang, p.0), aliases(p.0), p.1, p.2, aspect_ratio(p.1, p.2), p.3).to_lowercase();
    let query = query.to_lowercase().replace('×', "x");
    query.split_whitespace().all(|term| haystack.contains(term))
}

/// Search spans categories; duplicates in Popular and their home category appear once.
pub fn formats(category: &str, query: &str, lang: crate::i18n::Lang) -> Vec<Preset> {
    let mut names = std::collections::BTreeSet::new();
    CATEGORIES
        .iter()
        .filter(|(name, _)| category == "All" || !query.trim().is_empty() || *name == category)
        .flat_map(|(_, presets)| presets.iter().copied())
        .filter(|p| matches(p, query, lang) && names.insert(p.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_platform_dimensions_ratio_and_translated_names_without_duplicates() {
        let en = crate::i18n::Lang::EN;
        let vertical = formats("Print", "tiktok", en);
        assert_eq!(vertical, vec![("Stories / Reels / Shorts", 1080, 1920, 72.0)]);
        assert_eq!(formats("Popular", "1080 × 1350", en), vec![("Instagram post, 4:5", 1080, 1350, 72.0)]);
        let portraits = formats("All", "3:4", en);
        assert!(portraits.iter().any(|p| p.0 == "Instagram post, 3:4"));
        assert!(formats("All", "unknown-format", en).is_empty());
        for lang in crate::i18n::Lang::all() {
            for p in CATEGORIES.iter().flat_map(|c| c.1) {
                assert!(formats("All", crate::i18n::tr(lang, p.0), lang).contains(p));
                if lang.complete_menus() && CATEGORIES.iter().find(|c| c.0 == "Social Media").is_some_and(|c| c.1.contains(p)) {
                    assert!(crate::i18n::has(lang, p.0), "{}: {}", lang.code(), p.0);
                }
            }
        }
        let all = formats("All", "", en);
        assert_eq!(all.iter().filter(|p| p.0 == "HDTV 1080p").count(), 1);
    }

    #[test]
    fn common_ratios_are_exact_and_degenerate_inputs_are_safe() {
        assert_eq!(aspect_ratio(1080, 1920), "9:16");
        assert_eq!(aspect_ratio(1080, 1440), "3:4");
        assert_eq!(aspect_ratio(3840, 2160), "16:9");
        assert_eq!(aspect_ratio(300000, 1), "300000:1");
        assert_eq!(aspect_ratio(0, 0), "1:1");
    }
}
