//! Bundled calibration and explicitly installed user camera profiles.
use photocraft_raw::{CameraProfile, CameraSettings, Sensor};
use serde_json::{Value, json};
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
#[path = "bundled_profiles.rs"]
mod bundled;
#[path = "raw_profile_catalog.rs"]
mod catalog;

pub(super) fn available(refresh: bool) -> Vec<Value> {
    let external = catalog::entries(refresh)
        .into_iter()
        .map(|p| json!({"source":"external-dcp","cameraModel":p.model,"profile":p.name,"file":p.path.file_name().unwrap_or_default().to_string_lossy()}));
    #[cfg(not(target_arch = "wasm32"))]
    {
        bundled::available().into_iter().chain(external).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        external.collect()
    }
}

pub(super) struct Loaded {
    pub profile: Arc<CameraProfile>,
    pub report: Value,
}

#[cfg(not(target_arch = "wasm32"))]
fn safe_component(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && !matches!(s, "." | "..") && !s.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn requested(settings: Option<&CameraSettings>) -> (String, Vec<String>) {
    let mut notes = Vec::new();
    let control = settings.and_then(|s| s.picture_control.as_ref());
    let name = control.map_or("Standard", |p| p.name.as_str());
    let known = |s: &str| {
        ["Standard", "Neutral", "Vivid", "Monochrome", "Portrait", "Landscape", "Flat", "Rich Tone Portrait", "Flat Monochrome", "Deep Tone Monochrome"]
            .into_iter()
            .find(|n| n.eq_ignore_ascii_case(s))
    };
    let chosen = if name.eq_ignore_ascii_case("Auto") {
        notes.push("Picture Control Auto uses its Standard base; Nikon's adaptive scene adjustments are not reproduced".into());
        "Standard"
    } else if let Some(name) = known(name) {
        name
    } else {
        name
    };
    if let Some(p) = control
        && [p.contrast_code, p.brightness_code, p.saturation_code, p.hue_code].iter().any(|v| !matches!(*v, 0x80 | 0xff))
    {
        notes.push("Picture Control manual tone/color offsets are recorded but not approximated; only the actual profile is applied".into());
    }
    let mut profile = format!("Camera {chosen}");
    if let Some(p) = control {
        if chosen.eq_ignore_ascii_case("Monochrome") {
            let filter = match p.filter_effect_code {
                0x81 => Some("Yellow"),
                0x82 => Some("Orange"),
                0x83 => Some("Red"),
                0x84 => Some("Green"),
                _ => None,
            };
            if let Some(filter) = filter {
                profile = format!("Camera Monochrome ({filter} Filter)");
            }
        }
        if !matches!(p.toning_effect_code, 0x80 | 0xff) {
            notes.push("Picture Control toning is recorded but is not approximated by an unrelated profile".into());
        }
    }
    (profile, notes)
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn load(sensor: &Sensor, settings: Option<&CameraSettings>) -> Result<Option<Loaded>, String> {
    use std::{
        io::Read,
        path::{Path, PathBuf},
    };
    let Some(model) = sensor.model.as_deref().filter(|m| safe_component(m)) else { return Ok(None) };
    let (requested, mut notes) = requested(settings);
    let path = if let Some(path) = std::env::var_os("PHOTOCRAFT_CAMERA_PROFILE") {
        Some(PathBuf::from(path))
    } else {
        let entries = catalog::entries(false);
        let cameras: Vec<_> = entries.iter().filter(|p| photocraft_raw::camera_model_matches(&p.model, sensor.make.as_deref().unwrap_or(""), model)).collect();
        let mut selected = None;
        let base = settings.and_then(|s| s.picture_control.as_ref()).map(|p| format!("Camera {}", p.base)).unwrap_or_else(|| "Camera Standard".into());
        for name in [&requested, &base, "Camera Standard", "Adobe Standard"] {
            if let Some(p) = cameras.iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
                if !p.name.eq_ignore_ascii_case(&requested) {
                    notes.push(format!("{requested} is unavailable; using the installed {} profile", p.name));
                }
                selected = Some(p.path.clone());
                break;
            }
        }
        selected.or_else(|| {
            cameras.first().map(|p| {
                notes.push(format!("{requested} is unavailable; using the installed {} profile", p.name));
                p.path.clone()
            })
        })
    };
    let Some(path) = path else { return bundled::load(sensor, settings) };
    // Cache only successful reads, by file identity. New installations are found on the next open.
    type Cache = std::collections::HashMap<PathBuf, (u64, Option<std::time::SystemTime>, Arc<CameraProfile>, String)>;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Cache>> = std::sync::OnceLock::new();
    let metadata = std::fs::metadata(&path).map_err(|e| format!("camera profile: {e}"))?;
    if metadata.len() > 32 * 1024 * 1024 {
        return Err("camera profile exceeds 32 MiB".into());
    }
    let modified = metadata.modified().ok();
    let cached = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&path)
        .filter(|(len, time, _, _)| *len == metadata.len() && *time == modified)
        .map(|(_, _, p, d)| (p.clone(), d.clone()));
    let (profile, digest) = if let Some(cached) = cached {
        cached
    } else {
        let mut bytes = Vec::new();
        std::fs::File::open(&path).map_err(|e| e.to_string())?.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let profile = Arc::new(CameraProfile::from_dcp(&bytes).map_err(|e| format!("{}: {e}", path.file_name().unwrap_or_default().to_string_lossy()))?);
        profile.validate(sensor).map_err(|e| e.to_string())?;
        let digest = blake3::hash(&bytes).to_hex().to_string();
        let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(path.clone(), (metadata.len(), modified, profile.clone(), digest.clone()));
        (profile, digest)
    };
    profile.validate(sensor).map_err(|e| e.to_string())?;
    if profile.default_black_render == 0 {
        notes.push("profile requests automatic black rendering; only the sensor black level is subtracted".into());
    }
    let pc = settings.and_then(|s| s.picture_control.as_ref()).map(|p| {
        json!({"name":p.name,"base":p.base,"version":p.version,"adjustment":p.adjustment,
        "contrastCode":p.contrast_code,"brightnessCode":p.brightness_code,"saturationCode":p.saturation_code,"hueCode":p.hue_code,"filterEffectCode":p.filter_effect_code,"toningEffectCode":p.toning_effect_code})
    });
    let report = json!({"source":"external-dcp","profile":profile.name,"cameraModel":profile.camera_model,
        "file":Path::new(&path).file_name().unwrap_or_default().to_string_lossy(),"digest":digest,"pictureControl":pc,
        "calibrationIlluminants":profile.calibrations.iter().map(|c|c.temperature).collect::<Vec<_>>(),"exposureOffset":profile.exposure_offset,
        "toneCurvePoints":profile.tone_curve.len(),"hueSatMaps":profile.hue_sat_maps.len(),"lookTable":profile.look_table.as_ref().map(|t|t.dims),
        "embedPolicy":profile.embed_policy,"copyright":profile.copyright,"limitations":notes});
    Ok(Some(Loaded { profile, report }))
}

#[cfg(target_arch = "wasm32")]
pub(super) fn load(_: &Sensor, _: Option<&CameraSettings>) -> Result<Option<Loaded>, String> {
    Ok(None)
}

pub(super) fn record(doc: &mut photocraft_doc::Document, loaded: &Loaded) {
    let json = loaded.report.to_string().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    doc.metadata.xmp = Some(format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:pc="https://photocraft.app/ns/raw/1.0/"><pc:CameraProfile>{json}</pc:CameraProfile></rdf:Description></rdf:RDF></x:xmpmeta>"#
    ));
}
pub(super) fn report(doc: &photocraft_doc::Document) -> Option<Value> {
    let text = doc.metadata.xmp.as_deref()?;
    let (_, tail) = text.split_once("<pc:CameraProfile>")?;
    let (body, _) = tail.split_once("</pc:CameraProfile>")?;
    if body.len() > 16_384 {
        return None;
    }
    serde_json::from_str(&body.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Document, Size};

    #[test]
    fn profile_provenance_survives_native_save_without_embedding_vendor_assets() {
        let mut doc = Document::new("profile provenance", Size::new(2, 2), ColorMode::Rgb, SampleType::U16);
        let profile = CameraProfile {
            name: "A & B <look>".into(),
            camera_model: "Test Camera".into(),
            calibrations: Vec::new(),
            exposure_offset: 0.0,
            tone_curve: Vec::new(),
            hue_sat_maps: Vec::new(),
            look_table: None,
            default_black_render: 1,
            embed_policy: 3,
            copyright: "Original synthetic test data".into(),
        };
        let expected = json!({"source":"external-dcp","profile":profile.name,"cameraModel":profile.camera_model,"digest":"test digest"});
        record(&mut doc, &Loaded { profile: Arc::new(profile), report: expected.clone() });
        assert_eq!(report(&doc), Some(expected));
        let xmp = doc.metadata.xmp.as_deref().unwrap();
        assert!(xmp.contains("&amp;"));
        assert!(!xmp.contains("calibrations"));
        let encoded = crate::export(&doc, "profile.pcraft", &Default::default()).unwrap();
        let restored = crate::import("profile.pcraft", &encoded.bytes).unwrap();
        assert_eq!(report(&restored.document), report(&doc));
        doc.metadata.xmp = Some("<pc:CameraProfile>not json</pc:CameraProfile>".into());
        assert!(report(&doc).is_none());
    }

    #[test]
    fn picture_control_selects_profiles_without_fabricating_auto_offsets() {
        let mut settings = CameraSettings {
            make: "NIKON CORPORATION".into(),
            model: "NIKON Z f".into(),
            color_space: Some(1),
            picture_control: Some(photocraft_raw::PictureControl {
                version: "0310".into(),
                name: "AUTO".into(),
                base: "AUTO".into(),
                adjustment: 1,
                contrast_code: 255,
                brightness_code: 255,
                saturation_code: 255,
                hue_code: 255,
                filter_effect_code: 255,
                toning_effect_code: 255,
            }),
        };
        let (name, notes) = requested(Some(&settings));
        assert_eq!(name, "Camera Standard");
        assert!(notes.iter().any(|n| n.contains("adaptive")));
        settings.picture_control.as_mut().unwrap().name = "VIVID".into();
        let (name, _) = requested(Some(&settings));
        assert_eq!(name, "Camera Vivid");
    }
}
