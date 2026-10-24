use crate::icons;
use crate::panels::adjustment_icon;
use photocraft_doc::Adjustment;

/// Verifies that every `Adjustment` variant maps to an existing registered icon in `icon_data`.
#[test]
fn every_adjustment_kind_has_a_registered_icon() {
    let adjustments = [
        Adjustment::BrightnessContrast { brightness: 0.0, contrast: 0.0, legacy: false },
        Adjustment::Levels { master: Default::default(), per_channel: Default::default(), space: Default::default(), black: Default::default() },
        Adjustment::Curves { master: Vec::new(), per_channel: Default::default(), space: Default::default(), black: Vec::new(), pencil: false },
        Adjustment::Exposure { exposure: 0.0, offset: 0.0, gamma: 1.0 },
        Adjustment::Vibrance { vibrance: 0.0, saturation: 0.0 },
        Adjustment::default_hue_saturation(),
        Adjustment::ColorBalance { shadows: [0.0; 3], midtones: [0.0; 3], highlights: [0.0; 3], preserve_luminosity: true },
        Adjustment::BlackWhite { weights: [0.0; 6], tint: None },
        Adjustment::PhotoFilter { color: [0.0; 3], density: 0.25, preserve_luminosity: true },
        Adjustment::ChannelMixer { matrix: [[0.0; 4]; 3], monochrome: false },
        Adjustment::ColorLookup { name: "test".into(), lut: None, size: 0, tetrahedral: false, dither: false },
        Adjustment::Invert,
        Adjustment::Posterize { levels: 4 },
        Adjustment::Threshold { level: 128.0 },
        Adjustment::GradientMap { stops: Vec::new(), reverse: false, dither: false },
        Adjustment::SelectiveColor { relative: false, adjustments: [[0.0; 4]; 9] },
        Adjustment::Unsupported { psd_key: "test".into(), raw: std::sync::Arc::new([]) },
    ];

    for adj in &adjustments {
        let icon_name = adjustment_icon(adj);
        assert!(icons::exists(icon_name), "Icon '{icon_name}' for adjustment '{}' must be registered in icons map", adj.label());
    }
}
