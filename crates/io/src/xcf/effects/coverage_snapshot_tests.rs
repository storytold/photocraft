//! Recorded channel samples are coverage, independent of channel-overlay UI.
//! No invented trailing channel/effect-mask filter list is part of these fixtures.
use super::coverage::{self, Admission, Failure};
use crate::native_xcf_tones::fixtures::masked_exposure;
use craft_xcf::{
    Xcf,
    effects::{Budget, Reader},
    records,
};
use photocraft_raster::Interrupt;

#[test]
fn recorded_auxiliary_samples_ignore_channel_display_color_opacity_and_indicates() {
    for display_opacity in [0f32, 1.] {
        let mut fixture = masked_exposure(Default::default());
        let properties = {
            let xcf = Xcf::parse(&fixture.bytes).unwrap();
            let mut effects = Budget::default();
            let inventory = xcf.layer_effects(0, &mut effects, &|| true).unwrap();
            let effect = Reader::new(&fixture.bytes, 22).read(inventory.owner, inventory.offsets[0], &mut effects, &|| true).unwrap();
            let header = xcf.effect_mask_header(&effect.record, &mut records::Budget::default(), &|| true).unwrap().unwrap();
            header
                .properties()
                .iter()
                .filter(|property| matches!(property.kind, 14 | 33 | 38))
                .map(|property| (property.kind, property.offset + 8, property.payload.len()))
                .collect::<Vec<_>>()
        };
        assert_eq!(properties.iter().map(|(kind, _, _)| *kind).collect::<Vec<_>>(), [33, 14, 38]);
        for (kind, at, length) in properties {
            let replacement = match kind {
                14 => 0u32.to_be_bytes().to_vec(),
                33 => display_opacity.to_be_bytes().to_vec(),
                38 => [1f32, 0.25, 0.5].into_iter().flat_map(f32::to_be_bytes).collect(),
                _ => panic!("selected display property"),
            };
            assert_eq!(replacement.len(), length);
            fixture.bytes[at..at + length].copy_from_slice(&replacement);
        }
        let xcf = Xcf::parse(&fixture.bytes).unwrap();
        let mut effects = Budget::default();
        let inventory = xcf.layer_effects(0, &mut effects, &|| true).unwrap();
        let effect = Reader::new(&fixture.bytes, 22).read(inventory.owner, inventory.offsets[0], &mut effects, &|| true).unwrap();
        let mut retained = 16 << 20;
        let mut source = 8;
        let mut work = 32 << 20;
        let mut global = retained;
        let mut metadata = records::Budget::default();
        let mut admission = Admission { retained: &mut global, source: &mut source, metadata: &mut metadata, options: Default::default() };
        let decoded = coverage::decode(&xcf, &effect.record, &mut admission, &mut retained, &mut work, &Interrupt::NONE).unwrap();
        assert_eq!(source, 0, "the exact recorded sample allowance is used once");
        let expected: Vec<_> = [0u8, 64, 192, 255, 255, 128, 32, 0].map(|sample| f32::from(sample) / 255.).into();
        assert_eq!(decoded.native.mask.surface.read_region(photocraft_geom::Rect::from_xywh(0, 0, 4, 2)), expected);
        assert_eq!(decoded.native.mask.density, 1.);
        assert!(decoded.native.mask.enabled);
    }
}

#[test]
fn auxiliary_metadata_limit_or_cancel_precedes_native_and_sample_work() {
    let fixture = masked_exposure(Default::default());
    let xcf = Xcf::parse(&fixture.bytes).unwrap();
    let mut effects = Budget::default();
    let inventory = xcf.layer_effects(0, &mut effects, &|| true).unwrap();
    let effect = Reader::new(&fixture.bytes, 22).read(inventory.owner, inventory.offsets[0], &mut effects, &|| true).unwrap();
    for cancelled in [false, true] {
        let mut retained = 16 << 20;
        let mut source = 8;
        let mut work = 32 << 20;
        let mut global = retained;
        let mut metadata = records::Budget::new(0, 1 << 20);
        let mut admission = Admission { retained: &mut global, source: &mut source, metadata: &mut metadata, options: Default::default() };
        let cancel = || cancelled;
        let result = coverage::decode(&xcf, &effect.record, &mut admission, &mut retained, &mut work, &Interrupt::cancel_only(&cancel));
        if cancelled {
            assert!(matches!(result, Err(Failure::Cancelled)));
        } else {
            assert!(matches!(result, Err(Failure::Limit)));
        }
        assert_eq!(retained, 16 << 20);
        assert_eq!(source, 8);
        assert_eq!(work, 32 << 20);
        assert_eq!(global, retained);
    }
}
