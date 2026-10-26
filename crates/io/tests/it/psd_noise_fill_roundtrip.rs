//! #6272: a Photoshop noise `GdFl` gradient that is not provably constant-RGB still opens as
//! a fill layer — the placeholder ramp stands in where the model has no stops, Photoshop's
//! own pixels ride along as the fill cache — and its raw `GdFl` block survives a re-save byte
//! for byte. Before the fix, `fill_from_desc` returned `None` for those gradients: the layer
//! imported as pixels and the export dropped the `GdFl` (`RASTER_STALE_KEYS`), so opening and
//! re-saving a file silently turned the fill layer into a plain pixel layer.

use photocraft_doc::{Fill, LayerContent};
use photocraft_io::*;
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value, VersionedDescriptor};
use photocraft_psd::testgen;
use photocraft_psd::{Compression, LayerRecord, PsdFile, TaggedBlock, Version};

/// Photoshop's default Noise gradient (RGB, min 0/0/0, max 100/100/100) as `GdFl` block data.
fn default_noise_gdfl() -> Vec<u8> {
    let values = |v: [i32; 4]| Value::List(v.into_iter().map(Value::Integer).collect());
    let gradient = Descriptor::new("Grdn")
        .with("GrdF", Value::Enumerated { type_id: Id::new("GrdF"), value: Id::new("ClNs") })
        .with("ClrS", Value::Enumerated { type_id: Id::new("ClrS"), value: Id::new("RGBC") })
        .with("ShTr", Value::Boolean(false))
        .with("VctC", Value::Boolean(true))
        .with("RndS", Value::Integer(9052369))
        .with("Smth", Value::Integer(2048))
        .with("Mnm ", values([0, 0, 0, 0]))
        .with("Mxm ", values([100, 100, 100, 100]));
    let d = Descriptor::new("null")
        .with("Grad", Value::Descriptor(gradient))
        .with("Angl", Value::UnitFloat { unit: *b"#Ang", value: 0.0 })
        .with("Type", Value::Enumerated { type_id: Id::new("Type"), value: Id::new("CstS") });
    VersionedDescriptor::new(d).to_bytes()
}

#[test]
fn variable_noise_gradient_stays_a_fill_layer_and_keeps_its_gdfl() {
    let _ = UnicodeString::new_nul("x");
    let mut f = testgen::small(Version::Psd, Compression::Raw);
    let gdfl = default_noise_gdfl();
    f.layers_mut().push(LayerRecord {
        name: b"noise fill".to_vec(),
        blocks: vec![TaggedBlock::unicode_name("noise fill"), TaggedBlock::new(*b"GdFl", gdfl.clone())],
        ..Default::default()
    });
    for b in f.layers().last().unwrap().blocks.iter() {
        b.check_structure().unwrap();
    }
    let bytes = f.to_bytes().unwrap();
    let imp = import("in.psd", &bytes).unwrap();
    let l = imp.document.layers.iter().find(|l| l.name == "noise fill").unwrap();
    assert!(matches!(l.content, LayerContent::Fill(Fill::Gradient { .. })), "a variable noise GdFl opens as a gradient fill layer, not pixels");
    let out = export(&imp.document, "out.psd", &ExportOptions::default()).unwrap().bytes;
    let back = PsdFile::from_bytes(&out).unwrap();
    let rec = back.layers().iter().find(|l| l.block(b"GdFl").is_some()).expect("the re-saved layer carries its GdFl");
    assert_eq!(rec.block(b"GdFl").unwrap().data, gdfl, "the raw GdFl survives the round trip byte for byte");
}

/// psd-tools `gradients/noise-gradient-{rgb,lab}.psd`: each file's `hsb-noise` / `lab-noise`
/// layer is constant noise (min == max), and PhotoCraft re-renders Photoshop's stored pixels
/// within one level. The same fills in the other documents' colour spaces are not asserted:
/// their stored pixels depend on Photoshop's working and CMYK profiles (the untagged RGB file
/// stores Lab(60, 25, 25) as 178/126/102, within a level of Adobe RGB; PhotoCraft's sRGB gives
/// 195/127/102).
#[cfg(feature = "corpus")]
#[test]
fn constant_noise_fills_match_photoshop_stored_pixels() {
    use std::path::PathBuf;
    for (file, layer_name) in [("noise-gradient-rgb.psd", "hsb-noise"), ("noise-gradient-lab.psd", "lab-noise")] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/psd-tools/gradients").join(file);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}: run `cargo xtask corpus --all`", path.display()));
        let doc = import(file, &bytes).expect("opens").document;
        let layer = doc.layers.iter().find(|l| l.name == layer_name).unwrap_or_else(|| panic!("{file}: layer {layer_name}"));
        let LayerContent::Fill(fill @ Fill::Gradient { stops, .. }) = &layer.content else { panic!("{file}: {layer_name} is not a gradient fill") };
        assert!(stops.len() >= 2 && stops.iter().all(|(_, c)| *c == stops[0].1), "{file}: {layer_name} stops are constant");
        let cache = layer.fill_cache.as_ref().expect("Photoshop's stored pixels");
        let buf = photocraft_compose::render_fill_content(layer, fill, doc.bounds(), &[]);
        let bounds = cache.surface.content_bounds();
        assert!(!bounds.is_empty(), "{file}: {layer_name} has stored pixels");
        let (mut max, mut sum, mut n) = (0.0f32, 0.0f32, 0u32);
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                let ours = photocraft_raster::from_rgba(&cache.surface.format(), buf.get(x, y));
                let stored = cache.surface.pixel(x, y);
                for d in ours.iter().zip(&stored).take(3).map(|(a, b)| (a * 255.0).round() - (b * 255.0).round()) {
                    max = max.max(d.abs());
                    sum += d.abs();
                    n += 1;
                }
            }
        }
        eprintln!("{file} {layer_name}: max {max} mean {:.3}", sum / n as f32);
        assert!(max <= 1.0, "{file}: {layer_name} max {max} > 1 level");
    }
}
