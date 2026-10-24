//! Layer-effect blend modes Photoshop 2026 stores as the layer-record key (`Md  ` = `BlnM`
//! `vLit`, not `vividLight`) open as that mode, not Normal (#8567).

use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::*;
use photocraft_geom::{Rect, Size};
use photocraft_io::{ExportOptions, export, import};
use photocraft_psd::PsdFile;
use photocraft_psd::descriptor::{Id, Value, VersionedDescriptor};

/// A white layer with a 10 % orange Color Overlay in `mode`, saved as a PSD whose overlay `Md  `
/// is then rewritten to `key`.
fn overlay_psd(mode: BlendMode, key: &str) -> Vec<u8> {
    let mut d = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut l = Layer::raster("white", PixelFormat::RGBA8);
    l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 8, 8), &[1.0, 1.0, 1.0, 1.0]);
    l.effects.items = vec![Effect::ColorOverlay { common: FxCommon::new(mode, 0.1), color: Color::rgb(239.0 / 255.0, 97.0 / 255.0, 18.0 / 255.0) }];
    d.layers.push(l);
    let mut f = PsdFile::from_bytes(&export(&d, "x.psd", &ExportOptions::default()).unwrap().bytes).unwrap();
    let block = f.layers_mut().iter_mut().find_map(|r| r.block_mut(b"lfx2")).expect("lfx2");
    let (mut vd, _) = VersionedDescriptor::parse_prefix(&block.data[4..]).unwrap();
    let Some((_, Value::Descriptor(sofi))) = vd.descriptor.items.iter_mut().find(|(k, _)| k.is("SoFi")) else { panic!("no SoFi") };
    let (_, md) = sofi.items.iter_mut().find(|(k, _)| k.is("Md  ")).expect("Md");
    *md = Value::Enumerated { type_id: Id::new("BlnM"), value: Id::new(key) };
    block.data.truncate(4);
    block.data.extend(vd.to_bytes());
    f.to_bytes().unwrap()
}

#[test]
fn effect_blend_stored_as_layer_key_opens_as_that_mode() {
    for (mode, key) in [
        (BlendMode::VividLight, "vLit"),
        (BlendMode::LinearDodge, "lddg"),
        (BlendMode::LinearBurn, "lbrn"),
        (BlendMode::LighterColor, "lgCl"),
        (BlendMode::LinearLight, "lLit"),
    ] {
        let doc = import("x.psd", &overlay_psd(mode, key)).unwrap().document;
        let Some(Effect::ColorOverlay { common, .. }) = doc.layers[1].effects.items.first() else { panic!("{key}: no Color Overlay") };
        assert_eq!(common.blend, mode, "{key}");
    }
}

#[test]
fn vivid_light_overlay_stored_as_vlit_leaves_white_white() {
    // Vivid Light over white is white; the Normal it used to open as tints it to about
    // (0.99, 0.94, 0.91), the pink wash reported in #8567.
    let px = |bytes: &[u8]| photocraft_compose::render(&import("x.psd", bytes).unwrap().document, Rect::from_xywh(4, 4, 1, 1)).px[0];
    let vlit = px(&overlay_psd(BlendMode::VividLight, "vLit"));
    assert!(vlit[..3].iter().all(|&c| c > 0.995), "{vlit:?}");
    let normal = px(&overlay_psd(BlendMode::Normal, "Nrml"));
    assert!(normal[2] < 0.95, "the Normal control tints white: {normal:?}");
}
