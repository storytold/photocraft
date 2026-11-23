use super::*;
use craft_xcf::effects::{Argument, Budget, Reader, SourceProperty};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_compose::{
    Buffer,
    adjust::{self, Transfer},
};
use photocraft_doc::adjust::Adjustment;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::json;

pub(super) fn source<'a>(operation: &'a str, arguments: Vec<Argument<'a>>) -> OwnedEffect<'a> {
    // Build an actual shared-reader record, including its private provenance.
    // Pure mapping tests may change public typed fields; mask admission tests
    // instead use complete unmodified source records from their own fixtures.
    static BASE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let bytes = BASE.get_or_init(|| {
        let mut bytes = vec![0; 8];
        for text in ["Original effect", "", "gegl:exposure", "1:0"] {
            bytes.extend_from_slice(&(text.len() as u32 + 1).to_be_bytes());
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
        }
        for _ in 0..4 {
            bytes.extend_from_slice(&0u32.to_be_bytes());
        }
        bytes
    });
    let mut source: OwnedEffect<'a> = Reader::new(bytes, 26).read(Owner { kind: OwnerKind::Layer, offset: 77 }, 8, &mut Budget::default(), &|| true).unwrap();
    source.record.operation = operation;
    source.record.version = Some(if operation == "gegl:exposure" { "1:0" } else { "0:0" });
    source.record.arguments = arguments;
    source
}
pub(super) fn scalar<'a>(name: &'a str, value: f32) -> Argument<'a> {
    Argument { name, value: ArgumentValue::Float(value) }
}
pub(super) fn prepared(source: &OwnedEffect<'_>) -> Mapping {
    map(source, &Interrupt::NONE).unwrap().unwrap()
}

/// The exact native parameter domains from engine-core::adjust_params. Building
/// the ordinary typed adjustment also exercises the actual native compositor;
/// this does not import engine-core upward into IO or claim engine dispatch ran.
fn adjustment(filter: &SmartFilter) -> Adjustment {
    assert_eq!(filter.blend, BlendMode::Normal);
    assert_eq!(filter.opacity, 1.);
    assert!(filter.visible);
    let fields = filter.params.as_object().unwrap();
    let number = |name: &str| filter.params.get(name).unwrap().as_f64().unwrap();
    match filter.command.as_str() {
        "image.adjustments.invert" => {
            assert!(fields.is_empty());
            Adjustment::Invert
        }
        "image.adjustments.brightnessContrast" => {
            assert_eq!(fields.len(), 3);
            let (brightness, contrast) = (number("brightness"), number("contrast"));
            assert!((-150.0..=150.).contains(&brightness));
            assert!((-100.0..=100.).contains(&contrast));
            assert_eq!(filter.params["legacy"], true);
            Adjustment::BrightnessContrast { brightness: brightness as f32, contrast: contrast as f32, legacy: true }
        }
        "image.adjustments.exposure" => {
            assert_eq!(fields.len(), 3);
            let (exposure, offset, gamma) = (number("exposure"), number("offset"), number("gamma"));
            assert!((-20.0..=20.).contains(&exposure));
            assert!((-0.5..=0.5).contains(&offset));
            assert_eq!(gamma, 1.);
            Adjustment::Exposure { exposure: exposure as f32, offset: offset as f32, gamma: gamma as f32 }
        }
        "image.adjustments.levels" => {
            use photocraft_doc::adjust::{LevelsChannel, ToneSpace};
            assert_eq!(fields.len(), 5);
            assert_eq!(number("gamma"), 1.);
            Adjustment::Levels {
                master: LevelsChannel {
                    in_black: number("inBlack") as f32 / 255.,
                    in_white: number("inWhite") as f32 / 255.,
                    gamma: 1.,
                    out_black: number("outBlack") as f32 / 255.,
                    out_white: number("outWhite") as f32 / 255.,
                },
                per_channel: Default::default(),
                space: ToneSpace::Rgb,
                black: LevelsChannel::default(),
            }
        }
        command => panic!("unexpected native command {command}"),
    }
}
fn render(adjustments: &[Adjustment], sample: SampleType, values: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let format = PixelFormat { mode: ColorMode::Rgb, sample, alpha: true };
    let rect = Rect::new(0, 0, values.len() as i32, 1);
    let quantize = |pixels: &[[f32; 4]]| {
        let mut surface = Surface::new(format);
        for (x, pixel) in pixels.iter().enumerate() {
            surface.write_pixel(x as i32, 0, pixel);
        }
        (0..pixels.len()).map(|x| surface.rgba(x as i32, 0)).collect::<Vec<_>>()
    };
    let mut buffer = Buffer { rect, px: quantize(values) };
    for adjustment in adjustments {
        adjust::apply_depth(adjustment, &mut buffer, Transfer::for_document(ColorMode::Rgb, sample), Some(sample));
        buffer.px = quantize(&buffer.px);
    }
    std::mem::take(&mut buffer.px)
}

#[test]
fn tone_preparation_matches_independent_native_controls_at_every_depth() {
    let brightness = source("gegl:brightness-contrast", vec![scalar("brightness", 0.5), scalar("contrast", -2.)]);
    let exposure = source("gegl:exposure", vec![scalar("black-level", 0.1), scalar("exposure", 4.)]);
    let cases = [
        (
            &brightness,
            vec![
                Adjustment::Invert,
                Adjustment::BrightnessContrast { brightness: 0., contrast: 50., legacy: true },
                Adjustment::BrightnessContrast { brightness: 127.5, contrast: 0., legacy: true },
            ],
        ),
        (&exposure, vec![Adjustment::Exposure { exposure: 0., offset: -0.1, gamma: 1. }, Adjustment::Exposure { exposure: 4., offset: 0., gamma: 1. }]),
    ];
    let pixels = [[0.1, 0.25, 0.8, 0.5], [0.9, 0.6, 0.2, 1.]];
    for (source, reference) in cases {
        let mapped = prepared(source);
        let native = mapped.filters.iter().map(adjustment).collect::<Vec<_>>();
        for sample in SampleType::ALL {
            assert_eq!(render(&native, sample, &pixels), render(&reference, sample, &pixels), "{sample:?}");
        }
        assert!(!mapped.notes.is_empty());
    }
}

#[test]
fn float_tone_controls_preserve_signed_hdr_and_alpha_with_independent_math() {
    let pixels = [[-0.25, 0.25, 2., 0.375], [0.5, 1.5, -2., 0.75]];
    let brightness = source("gegl:brightness-contrast", vec![scalar("brightness", 0.5), scalar("contrast", -2.)]);
    let mapped = prepared(&brightness);
    let native = mapped.filters.iter().map(adjustment).collect::<Vec<_>>();
    let output = render(&native, SampleType::F32, &pixels);
    let pivot = 127. / 255.; // Independent current native F32 legacy pivot.
    for (input, output) in pixels.iter().zip(output) {
        for channel in 0..3 {
            assert!((output[channel] - ((1. - input[channel] - pivot) * 2. + pivot + 0.5)).abs() < 0.00001);
        }
        assert_eq!(output[3], input[3]);
    }
    for exposure in [1., 4.] {
        let source = source("gegl:exposure", vec![scalar("black-level", 0.1), scalar("exposure", exposure)]);
        let mapped = prepared(&source);
        let native = mapped.filters.iter().map(adjustment).collect::<Vec<_>>();
        let output = render(&native, SampleType::F32, &pixels);
        for (input, output) in pixels.iter().zip(output) {
            for channel in 0..3 {
                assert!((output[channel] - (input[channel] - 0.1) * 2f32.powf(exposure)).abs() < 0.00001);
            }
            assert_eq!(output[3], input[3]);
        }
    }
}

#[test]
fn source_ranges_and_stored_float_endpoints_fit_bounded_native_schemas() {
    for brightness in [-3., 0., 3.] {
        for gain in [-5., 0., 0.5, 1., 5.] {
            let mapped = prepared(&source("gegl:brightness-contrast", vec![scalar("brightness", brightness), scalar("contrast", gain)]));
            assert!((1..=8).contains(&mapped.filters.len()));
            for filter in &mapped.filters {
                adjustment(filter);
            }
        }
    }
    for exposure in [-20., 0., 0.75, 20.] {
        for black in [-0.1, 0., 0.1] {
            let mapped = prepared(&source("gegl:exposure", vec![scalar("black-level", black), scalar("exposure", exposure)]));
            assert!((1..=2).contains(&mapped.filters.len()));
            for filter in &mapped.filters {
                adjustment(filter);
            }
        }
    }
    for (operation, name, endpoint) in [
        ("gegl:brightness-contrast", "brightness", 3f32),
        ("gegl:brightness-contrast", "contrast", 5.),
        ("gegl:exposure", "black-level", 0.1),
        ("gegl:exposure", "exposure", 20.),
    ] {
        let outside = f32::from_bits(endpoint.to_bits() + 1);
        assert!(matches!(map(&source(operation, vec![scalar(name, outside)]), &Interrupt::NONE), Err(Failure::Range(_))));
    }
}

#[test]
fn unknown_duplicate_mistyped_or_invalid_source_controls_never_get_defaults() {
    for value in [ArgumentValue::String("0.5"), ArgumentValue::Bool(true), ArgumentValue::Enum(1), ArgumentValue::Float(f32::NAN)] {
        let source = source("gegl:brightness-contrast", vec![Argument { name: "brightness", value }]);
        assert!(matches!(map(&source, &Interrupt::NONE), Err(Failure::Invalid(_))));
    }
    for value in [ArgumentValue::Int(1), ArgumentValue::UInt(1)] {
        let typed = source("gegl:exposure", vec![Argument { name: "exposure", value }]);
        assert_eq!(prepared(&typed).filters[0].params, json!({"exposure":1.,"offset":-0.0,"gamma":1.}));
    }
    let duplicate = source("gegl:exposure", vec![scalar("exposure", 1.), scalar("exposure", 2.)]);
    assert!(matches!(map(&duplicate, &Interrupt::NONE), Err(Failure::Invalid(_))));
    let unknown = source("gegl:exposure", vec![scalar("future-control", 1.)]);
    assert!(matches!(map(&unknown, &Interrupt::NONE), Err(Failure::Unsupported(_))));
    let mut version = source("gegl:exposure", vec![]);
    version.record.version = Some("future");
    assert!(matches!(map(&version, &Interrupt::NONE), Err(Failure::Unsupported(_))));
    version.record.version = None;
    assert!(map(&version, &Interrupt::NONE).unwrap().is_some());
    let mut unknown_property = source("gegl:exposure", vec![]);
    unknown_property.record.unknown_properties.push(SourceProperty { offset: 0, kind: 999, bytes: &[1] });
    assert!(matches!(map(&unknown_property, &Interrupt::NONE), Err(Failure::Unsupported(_))));
    assert!(map(&source("example:unmapped", vec![]), &Interrupt::NONE).unwrap().is_none());
}

#[test]
fn compound_envelope_keeps_owner_disabled_state_opacity_and_required_mask_once() {
    let mut source = source("gegl:exposure", vec![scalar("exposure", 4.), scalar("black-level", 0.1)]);
    source.record.visible = false;
    source.record.opacity = 0.375;
    source.record.mask_offset = 123;
    let mapped = prepared(&source);
    assert_eq!(mapped.owner, source.owner);
    assert_eq!(mapped.filters.len(), 2);
    assert_eq!(mapped.envelope, Envelope { visible: false, opacity: 0.375, blend: BlendMode::Normal, mask: MaskRequirement::Required(123) });
    assert!(mapped.filters.iter().all(|filter| filter.visible && filter.opacity == 1. && filter.blend == BlendMode::Normal));
    assert_eq!(source.record.mask_offset, 123, "preparation never resolves or discards source mask data");
    source.record.mask_offset = 0;
    assert_eq!(prepared(&source).envelope.mask, MaskRequirement::None);
    for kind in [OwnerKind::Channel, OwnerKind::Selection, OwnerKind::LayerMask, OwnerKind::EffectMask] {
        source.owner.kind = kind;
        assert!(matches!(map(&source, &Interrupt::NONE), Err(Failure::Unsupported(_))));
    }
}

#[test]
fn nonneutral_envelopes_and_cancellation_refuse_before_any_installation() {
    let mut source = source("gegl:exposure", vec![]);
    for kind in 0..6 {
        source.record.blend_space = 0;
        source.record.composite_space = 0;
        source.record.composite_mode = 0;
        source.record.clip = false;
        source.record.region = 0;
        source.record.mode = 62;
        match kind {
            0 => source.record.blend_space = 1,
            1 => source.record.composite_space = 1,
            2 => source.record.composite_mode = 1,
            3 => source.record.clip = true,
            4 => source.record.region = 1,
            _ => source.record.mode = 30,
        }
        assert!(matches!(map(&source, &Interrupt::NONE), Err(Failure::Unsupported(_))));
    }
    for opacity in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        source.record.opacity = opacity;
        assert!(matches!(map(&source, &Interrupt::NONE), Err(Failure::Invalid(_))));
    }
    source.record.opacity = 1.;
    assert!(matches!(map(&source, &Interrupt::cancel_only(&|| true)), Err(Failure::Cancelled)));
    source.record.errors.push("invalid field".into());
    assert!(matches!(map(&source, &Interrupt::NONE), Err(Failure::Invalid(_))));
}

#[test]
fn shared_record_decoder_feeds_native_preparation_without_an_app_wire_parser() {
    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn text(bytes: &mut Vec<u8>, value: &str) {
        word(bytes, value.len() as u32 + 1);
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    fn property(bytes: &mut Vec<u8>, kind: u32, payload: &[u8]) {
        word(bytes, kind);
        word(bytes, payload.len() as u32);
        bytes.extend_from_slice(payload);
    }
    let mut bytes = vec![0; 8];
    for value in ["Original effect", "", "gegl:exposure", "1:0"] {
        text(&mut bytes, value);
    }
    let mut argument = Vec::new();
    text(&mut argument, "exposure");
    word(&mut argument, 3);
    word(&mut argument, 0.75f32.to_bits());
    property(&mut bytes, 45, &argument);
    property(&mut bytes, 8, &0u32.to_be_bytes());
    property(&mut bytes, 33, &0.375f32.to_be_bytes());
    property(&mut bytes, 7, &62u32.to_be_bytes());
    word(&mut bytes, 0);
    word(&mut bytes, 0);
    bytes.extend_from_slice(&8u64.to_be_bytes());
    let original = bytes.clone();
    let owner = Owner { kind: OwnerKind::Layer, offset: 77 };
    let source = Reader::new(&bytes, 26).read(owner, 8, &mut Budget::default(), &|| true).unwrap();
    let mapped = prepared(&source);
    assert_eq!(mapped.owner, owner);
    assert_eq!(mapped.envelope.mask, MaskRequirement::Required(8));
    assert!(!mapped.envelope.visible);
    assert_eq!(mapped.envelope.opacity, 0.375);
    assert_eq!(mapped.filters[0].params, json!({"exposure":0.75,"offset":-0.0,"gamma":1.}));
    assert_eq!(bytes, original);
}

#[test]
fn levels_maps_only_ascending_native_endpoint_controls_and_matches_native_pixels() {
    use photocraft_doc::adjust::{LevelsChannel, ToneSpace};
    let effect = source("gegl:levels", vec![scalar("in-low", 0.125), scalar("in-high", 0.875), scalar("out-low", 0.25), scalar("out-high", 0.75)]);
    let mapping = map(&effect, &Interrupt::NONE).unwrap().unwrap();
    assert_eq!(mapping.filters.len(), 1);
    let native = &mapping.filters[0];
    assert_eq!(native.command, "image.adjustments.levels");
    assert_eq!(native.params, json!({"inBlack":31.875,"inWhite":223.125,"gamma":1.,"outBlack":63.75,"outWhite":191.25}));
    let expected = Adjustment::Levels {
        master: LevelsChannel { in_black: 0.125, in_white: 0.875, gamma: 1., out_black: 0.25, out_white: 0.75 },
        per_channel: Default::default(),
        space: ToneSpace::Rgb,
        black: LevelsChannel::default(),
    };
    let actual = adjustment(native);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let input = [[-0.25, 0.25, 2., 0.5], [0.75, 1.5, -0.5, 1.]];
        assert_eq!(render(std::slice::from_ref(&actual), depth, &input), render(std::slice::from_ref(&expected), depth, &input));
    }
    assert!(mapping.notes[0].contains("native approximation"));
}

#[test]
fn levels_invalid_unknown_collapsed_hdr_and_tiny_spans_are_not_silently_repaired() {
    for args in [
        vec![scalar("in-low", f32::NAN)],
        vec![scalar("out-high", f32::INFINITY)],
        vec![scalar("in-low", -0.1)],
        vec![scalar("out-high", 1.25)],
        vec![scalar("in-low", 0.75), scalar("in-high", 0.25)],
        vec![scalar("in-low", 0.5), scalar("in-high", 0.5)],
        vec![scalar("in-high", 1. / 255.)],
        vec![scalar("in-low", 0.5), scalar("in-high", 0.5 + 1. / 255.)],
        vec![scalar("out-low", 0.75), scalar("out-high", 0.25)],
        vec![scalar("out-low", 0.5), scalar("out-high", 0.5)],
        vec![scalar("gamma", 2.)],
        vec![scalar("in-low", 0.1), scalar("in-low", 0.2)],
    ] {
        assert!(map(&source("gegl:levels", args), &Interrupt::NONE).is_err());
    }
    let mut effect = source("gegl:levels", vec![]);
    effect.record.version = Some("99:0");
    assert!(map(&effect, &Interrupt::NONE).is_err());
    assert!(matches!(map(&source("gegl:levels", vec![]), &Interrupt::cancel_only(&|| true)), Err(Failure::Cancelled)));
}
