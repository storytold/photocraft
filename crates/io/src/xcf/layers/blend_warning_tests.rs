use crate::xcf::tests::{TestImage, TestLayer};

#[test]
fn blend_approximation_warning_describes_native_behavior_without_editor_brands() {
    for (mode, expected) in [(29, "Normal"), (19, "SoftLight"), (37, "Hue")] {
        let mut image = TestImage::rgb8(1, 1);
        let mut layer = TestLayer::rgba("Blend policy", 1, 1, vec![17, 31, 57, 255]);
        layer.mode = mode;
        image.layers.push(layer);
        let imported = crate::import("blend-policy.xcf", &image.write()).unwrap();
        let warning = imported.warnings.iter().find(|warning| warning.contains("reference editor")).unwrap();
        assert!(warning.contains(&format!("native {expected}")), "{warning}");
        assert!(!warning.contains("GIMP") && !warning.contains("Photoshop"), "{warning}");
    }
}
