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
    let text = save(&p);
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
    let again = save(&p);
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
