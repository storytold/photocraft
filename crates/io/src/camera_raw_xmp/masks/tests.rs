#![allow(clippy::unwrap_used)]

use photocraft_algo::camera_raw::CameraRaw;

use super::super::{load, save};
use super::*;

fn comp(shape: Shape) -> Component {
    Component { op: Op::Add, invert: false, shape }
}

#[test]
fn masks_round_trip_exactly_through_photocraft_attribute() {
    let p = CameraRaw {
        masks: vec![
            Mask {
                name: "Sky & \"sea\" <1>".into(),
                components: vec![comp(Shape::Sky), Component { op: Op::Subtract, invert: true, shape: Shape::Luminance { range: [10.0, 40.0], smoothness: 20.0 } }],
                adjust: Local { exposure: -0.5, ..Local::default() },
                ..Mask::default()
            },
            Mask { components: vec![comp(Shape::Linear { zero: [0.5, 0.6], full: [0.5, 0.1] })], adjust: Local { dehaze: 30.0, ..Local::default() }, amount: 150.0, ..Mask::default() },
        ],
        ..CameraRaw::default()
    };
    let text = save(&p).unwrap();
    assert!(text.contains("crs:GradientBasedCorrections"), "the linear mask is also written for Photoshop");
    let mut back = CameraRaw::default();
    load(&mut back, text.as_bytes()).unwrap();
    assert_eq!(back.masks, p.masks);
}

const PHOTOSHOP: &str = r#"<crs:GradientBasedCorrections><rdf:Seq><rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="1" crs:CorrectionActive="true" crs:LocalExposure2012="0.25" crs:LocalContrast2012="0.5">
<crs:CorrectionMasks><rdf:Seq><rdf:li crs:What="Mask/Gradient" crs:MaskValue="1" crs:ZeroX="0.5" crs:ZeroY="0.2" crs:FullX="0.5" crs:FullY="0.8"/></rdf:Seq></crs:CorrectionMasks></rdf:Description></rdf:li></rdf:Seq></crs:GradientBasedCorrections>
<crs:CircularGradientBasedCorrections><rdf:Seq><rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="0.5" crs:LocalSaturation="-1">
<crs:CorrectionMasks><rdf:Seq><rdf:li crs:What="Mask/CircularGradient" crs:Top="0.25" crs:Left="0.25" crs:Bottom="0.75" crs:Right="0.75" crs:Angle="0" crs:Feather="40" crs:Flipped="false"/></rdf:Seq></crs:CorrectionMasks></rdf:Description></rdf:li></rdf:Seq></crs:CircularGradientBasedCorrections>
<crs:PaintBasedCorrections><rdf:Seq><rdf:li><rdf:Description crs:What="Correction" crs:LocalShadows2012="0.4"><crs:CorrectionMasks><rdf:Seq><rdf:li><rdf:Description crs:What="Mask/Paint" crs:MaskValue="1" crs:Radius="0.05" crs:Flow="1" crs:CenterWeight="0.5"><crs:Dabs><rdf:Seq><rdf:li>d 0.3 0.4</rdf:li><rdf:li>d 0.31 0.4</rdf:li><rdf:li>junk</rdf:li></rdf:Seq></crs:Dabs></rdf:Description></rdf:li></rdf:Seq></crs:CorrectionMasks></rdf:Description></rdf:li></rdf:Seq></crs:PaintBasedCorrections>"#;

fn wrap(inner: &str, attrs: &str) -> String {
    format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"{RDF_NS}\"><rdf:Description xmlns:crs=\"{CRS_NS}\" xmlns:pcraft=\"{PCRAFT_NS}\" crs:Exposure2012=\"+0.30\"{attrs}>{inner}</rdf:Description></rdf:RDF></x:xmpmeta>"
    )
}

#[test]
fn photoshop_corrections_are_read_and_written_back() {
    let mut p = CameraRaw::default();
    load(&mut p, wrap(PHOTOSHOP, "").as_bytes()).unwrap();
    assert_eq!(p.masks.len(), 3);
    assert_eq!(p.masks[0].adjust.exposure, 1.0);
    assert_eq!(p.masks[0].adjust.contrast, 50.0);
    assert_eq!(p.masks[0].components[0].shape, Shape::Linear { zero: [0.5, 0.2], full: [0.5, 0.8] });
    assert_eq!(p.masks[1].amount, 50.0);
    assert!(p.masks[1].components[0].invert, "unflipped: outside the ellipse");
    assert!(matches!(&p.masks[2].components[0].shape, Shape::Brush { dabs } if dabs.len() == 2 && dabs[0].feather == 50.0));
    assert!(p.validate().is_ok());
    // Written in Photoshop's layout only (no PhotoCraft attribute needed to read them back
    // the same), and in PhotoCraft's.
    let again = save(&p).unwrap();
    let ours = again.find("pcraft:CameraRawMasks").map(|i| {
        let rest = &again[i..];
        let end = rest.find("\"\n").unwrap();
        again.replacen(&rest[..end + 1], "", 1)
    });
    let mut q = CameraRaw::default();
    load(&mut q, ours.unwrap().as_bytes()).unwrap();
    assert_eq!(q.masks.len(), 3);
    for (a, b) in q.masks.iter().zip(&p.masks) {
        assert_eq!(a.components, b.components);
        assert_eq!(a.adjust, b.adjust);
        assert_eq!(a.amount, b.amount);
    }
}

#[test]
fn hostile_mask_xml_is_survived() {
    let paint = r#"<crs:PaintBasedCorrections><rdf:Seq><rdf:li><rdf:Description crs:What="Correction" crs:LocalExposure2012="NaN"><crs:CorrectionMasks><rdf:Seq><rdf:li><rdf:Description crs:What="Mask/Paint" crs:Radius="1e30"><crs:Dabs><rdf:Seq><rdf:li>d inf 0.4</rdf:li></rdf:Seq></crs:Dabs></rdf:Description></rdf:li></rdf:Seq></crs:CorrectionMasks></rdf:Description></rdf:li></rdf:Seq></crs:PaintBasedCorrections>"#;
    let own = " pcraft:CameraRawMasks=\"[{&quot;components&quot;:[{&quot;kind&quot;:&quot;depth&quot;}]}]\"";
    let mut p = CameraRaw::default();
    let r = load(&mut p, wrap(paint, own).as_bytes()).unwrap();
    assert!(r.ignored.contains(&"CameraRawMasks".to_string()), "a Depth mask is refused");
    assert!(p.masks.is_empty(), "no finite dabs: nothing to make a mask from");
    assert!(p.validate().is_ok());
    let garbage = " pcraft:CameraRawMasks=\"not json\"";
    assert!(load(&mut CameraRaw::default(), wrap("", garbage).as_bytes()).is_ok());
}

/// `text` without the `pcraft:CameraRawMasks` attribute: what Photoshop's layout alone holds.
fn classic_only(text: &str) -> String {
    let i = text.find("pcraft:CameraRawMasks").unwrap();
    let end = text[i..].find("\"\n").unwrap();
    text.replacen(&text[i..i + end + 1], "", 1)
}

fn dab(x: f32, size: f32) -> Dab {
    Dab { x, y: 0.5, size, feather: 0.0, flow: 100.0, density: 100.0, erase: false, auto: false }
}

fn one(shape: Shape) -> Mask {
    Mask { components: vec![comp(shape)], adjust: Local { exposure: 1.0, ..Local::default() }, ..Mask::default() }
}

#[test]
fn auto_mask_brushes_stay_out_of_camera_raws_layout() {
    let m = one(Shape::Brush { dabs: vec![Dab { auto: true, ..dab(0.5, 0.05) }] });
    assert!(write(std::slice::from_ref(&m)).is_empty(), "Camera Raw's dabs cannot say Auto Mask");
    let mut back = CameraRaw::default();
    load(&mut back, save(&CameraRaw { masks: vec![m.clone()], ..CameraRaw::default() }).unwrap().as_bytes()).unwrap();
    assert_eq!(back.masks, vec![m]);
}

#[test]
fn classic_brushes_keep_every_stroke() {
    // Alternating sizes: 40 Mask/Paint runs of one dab each, all of one brush.
    let dabs: Vec<Dab> = (0..40).map(|i| dab(0.1 + i as f32 * 0.02, if i % 2 == 0 { 0.02 } else { 0.03 })).collect();
    let p = CameraRaw { masks: vec![one(Shape::Brush { dabs })], ..CameraRaw::default() };
    let mut q = CameraRaw::default();
    load(&mut q, classic_only(&save(&p).unwrap()).as_bytes()).unwrap();
    assert!(matches!(&q.masks[0].components[..], [Component { shape: Shape::Brush { dabs }, .. }] if dabs.len() == 40), "{:?}", q.masks);
}

#[test]
fn a_nan_dab_does_not_stall_the_writer() {
    let m = one(Shape::Brush { dabs: vec![dab(0.5, f32::NAN); 3] });
    assert_eq!(write(&[m]).matches("Mask/Paint").count(), 3);
}

#[test]
fn range_masked_corrections_are_reported_not_applied_unrestricted() {
    let xml = r#"<crs:GradientBasedCorrections><rdf:Seq><rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="1" crs:LocalExposure2012="0.25"><crs:CorrectionMasks><rdf:Seq><rdf:li crs:What="Mask/Gradient" crs:MaskValue="1" crs:ZeroX="0.5" crs:ZeroY="0.2" crs:FullX="0.5" crs:FullY="0.8"/></rdf:Seq></crs:CorrectionMasks><crs:CorrectionRangeMask><rdf:Description crs:Version="2" crs:Type="1" crs:ColorAmount="0.5"/></crs:CorrectionRangeMask></rdf:Description></rdf:li></rdf:Seq></crs:GradientBasedCorrections>"#;
    let mut p = CameraRaw::default();
    let r = load(&mut p, wrap(xml, "").as_bytes()).unwrap();
    assert!(p.masks.is_empty(), "{:?}", p.masks);
    assert!(r.ignored.contains(&"CorrectionRangeMask".to_string()), "{:?}", r.ignored);
}

#[test]
fn overflowing_radial_bounds_are_dropped() {
    let xml = r#"<crs:CircularGradientBasedCorrections><rdf:Seq><rdf:li><rdf:Description crs:What="Correction" crs:LocalExposure2012="0.25"><crs:CorrectionMasks><rdf:Seq><rdf:li crs:What="Mask/CircularGradient" crs:Top="-3e38" crs:Left="0.25" crs:Bottom="3e38" crs:Right="0.75"/></rdf:Seq></crs:CorrectionMasks></rdf:Description></rdf:li></rdf:Seq></crs:CircularGradientBasedCorrections>"#;
    let mut p = CameraRaw::default();
    load(&mut p, wrap(xml, "").as_bytes()).unwrap();
    assert!(p.masks.is_empty());
    assert!(p.validate().is_ok());
}

#[test]
fn photocraft_masks_count_as_applied() {
    let json = escape(&serde_json::to_string(&[one(Shape::Sky)]).unwrap());
    let xml = format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"{RDF_NS}\"><rdf:Description xmlns:crs=\"{CRS_NS}\" xmlns:pcraft=\"{PCRAFT_NS}\" crs:HasSettings=\"True\" pcraft:CameraRawMasks=\"{json}\"/></rdf:RDF></x:xmpmeta>"
    );
    let r = load(&mut CameraRaw::default(), xml.as_bytes()).unwrap();
    assert_eq!(r.applied, 1);
}

#[test]
fn saved_masks_reload_without_ignored_settings() {
    let p = CameraRaw { masks: vec![one(Shape::Linear { zero: [0.5, 0.6], full: [0.5, 0.1] })], ..CameraRaw::default() };
    let r = load(&mut CameraRaw::default(), save(&p).unwrap().as_bytes()).unwrap();
    assert!(r.ignored.is_empty(), "{:?}", r.ignored);
}

#[test]
fn an_empty_corrections_list_clears_the_masks() {
    let mut p = CameraRaw { masks: vec![one(Shape::Sky)], ..CameraRaw::default() };
    load(&mut p, wrap("<crs:PaintBasedCorrections><rdf:Seq/></crs:PaintBasedCorrections>", "").as_bytes()).unwrap();
    assert!(p.masks.is_empty());
}

#[test]
fn settings_without_masks_clear_them_on_load() {
    let mut p = CameraRaw { masks: vec![one(Shape::Sky)], ..CameraRaw::default() };
    load(&mut p, save(&CameraRaw::default()).unwrap().as_bytes()).unwrap();
    assert!(p.masks.is_empty());
}

#[test]
fn save_refuses_a_file_load_would_refuse() {
    // Valid masks: 8 brushes of the most dabs, each dab its own run (sizes vary).
    let dabs: Vec<Dab> = (0..masks::MAX_DABS).map(|i| Dab { y: (i / 100) as f32 / 40.0, ..dab((i % 100) as f32 / 100.0, 0.01 + (i % 7) as f32 * 0.001) }).collect();
    let p = CameraRaw { masks: vec![one(Shape::Brush { dabs }); 8], ..CameraRaw::default() };
    assert!(p.validate().is_ok());
    let e = save(&p).unwrap_err();
    assert!(e.contains("too large"), "{e}");
}
