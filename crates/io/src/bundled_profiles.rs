//! Licensed camera calibration data for the native app; independent of other installations.
use super::{CameraProfile, CameraSettings, Loaded, Sensor};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Mutex, OnceLock},
};

struct Asset {
    model: &'static str,
    name: &'static str,
    file: &'static str,
    license: &'static str,
    sha256: &'static str,
    gzip: &'static [u8],
}
#[path = "bundled_profiles_data.rs"]
mod data;
const REVISION: &str = "5f486d3678b34c74ba0c63571c17babe20935019";

pub(super) fn available() -> Vec<Value> {
    data::ASSETS
        .iter()
        .map(|a| {
            json!({"source":"bundled-dcp","cameraModel":a.model,
        "profile":a.name,"file":a.file,"license":a.license,"sha256":a.sha256,"embedPolicy":3})
        })
        .collect()
}

fn parse(asset: &Asset) -> Result<CameraProfile, String> {
    const LIMIT: u64 = 32 * 1024 * 1024;
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(asset.gzip).take(LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("bundled camera profile exceeds 32 MiB".into());
    }
    let profile = CameraProfile::from_dcp(&bytes).map_err(|e| format!("{}: {e}", asset.file))?;
    let licensed = match asset.license {
        "CC0-1.0" => profile.copyright.contains("RawTherapee CC0"),
        "Public-Domain" => profile.copyright.eq_ignore_ascii_case("public domain"),
        _ => false,
    };
    if profile.embed_policy != 3 || !licensed || profile.camera_model != asset.model || profile.name != asset.name {
        return Err(format!("{}: bundled profile metadata differs from the audited manifest", asset.file));
    }
    Ok(profile)
}

pub(super) fn load(sensor: &Sensor, settings: Option<&CameraSettings>) -> Result<Option<Loaded>, String> {
    let Some(model) = sensor.model.as_deref() else { return Ok(None) };
    let Some(asset) = data::ASSETS.iter().find(|a| photocraft_raw::camera_model_matches(a.model, sensor.make.as_deref().unwrap_or(""), model)) else {
        return Ok(None);
    };
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Arc<CameraProfile>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let profile = if let Some(p) = cache.get(asset.file) {
        p.clone()
    } else {
        let p = Arc::new(parse(asset)?);
        cache.insert(asset.file, p.clone());
        p
    };
    drop(cache);
    profile.validate(sensor).map_err(|e| e.to_string())?;
    let mut notes = vec!["Bundled camera calibration does not reproduce the camera's JPEG look or Picture Control adjustments".to_string()];
    if profile.default_black_render == 0 {
        notes.push("profile requests automatic black rendering; only the sensor black level is subtracted".into());
    }
    let pc = settings.and_then(|s| s.picture_control.as_ref()).map(|p| {
        json!({"name":p.name,"base":p.base,"version":p.version,
        "adjustment":p.adjustment,"contrastCode":p.contrast_code,"brightnessCode":p.brightness_code,
        "saturationCode":p.saturation_code,"hueCode":p.hue_code,"filterEffectCode":p.filter_effect_code,"toningEffectCode":p.toning_effect_code})
    });
    let report = json!({"source":"bundled-dcp","profile":profile.name,"cameraModel":profile.camera_model,
        "file":asset.file,"license":asset.license,"sha256":asset.sha256,"upstreamRevision":REVISION,
        "embedPolicy":profile.embed_policy,"copyright":profile.copyright,"pictureControl":pc,
        "calibrationIlluminants":profile.calibrations.iter().map(|c|c.temperature).collect::<Vec<_>>(),
        "exposureOffset":profile.exposure_offset,"toneCurvePoints":profile.tone_curve.len(),
        "hueSatMaps":profile.hue_sat_maps.len(),"lookTable":profile.look_table.as_ref().map(|t|t.dims),"limitations":notes});
    Ok(Some(Loaded { profile, report }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raw::{
        Limits, RawFormat, decode,
        testgen::{DngSpec, RafSpec, XTRANS, mosaic_cfa, scene},
    };

    #[test]
    fn entire_bundle_is_parseable_model_bound_and_explicitly_licensed() {
        assert_eq!(data::ASSETS.len(), 55);
        let manifest: Value = serde_json::from_str(include_str!("../../../assets/camera-profiles/manifest.json")).unwrap();
        let entries = manifest["profiles"].as_array().unwrap();
        assert_eq!(entries.len(), data::ASSETS.len());
        for asset in data::ASSETS {
            let p = parse(asset).unwrap();
            let entry = entries.iter().find(|e| e["file"] == asset.file).unwrap();
            assert_eq!(entry["sha256"], asset.sha256);
            assert_eq!(entry["license"], asset.license);
            assert_eq!(entry["copyright"], p.copyright);
            let mut spec = DngSpec::cfa(8, 8, vec![512; 64]);
            spec.model = asset.model.into();
            let mut sensor = decode(&spec.build(), &Limits::default()).unwrap();
            sensor.format = RawFormat::Nef;
            p.validate(&sensor).unwrap();
        }
    }

    #[test]
    fn zf_loads_without_external_files_and_unknown_models_are_not_substituted() {
        let mut spec = DngSpec::cfa(8, 8, vec![512; 64]);
        spec.model = "NIKON Z f".into();
        let mut sensor = decode(&spec.build(), &Limits::default()).unwrap();
        sensor.format = RawFormat::Nef;
        let loaded = load(&sensor, None).unwrap().unwrap();
        assert_eq!(loaded.report["source"], "bundled-dcp");
        assert_eq!(loaded.report["embedPolicy"], 3);
        assert!(!loaded.report["limitations"].as_array().unwrap().is_empty());
        let rendered = photocraft_raw::develop_sensor_profile(&sensor, &Default::default(), Some(&loaded.profile)).unwrap();
        assert_eq!(rendered.rgb.len(), 8 * 8 * 3);
        sensor.model = Some("NIKON Z f fictional successor".into());
        assert!(load(&sensor, None).unwrap().is_none());
    }

    #[test]
    fn uncompressed_xtrans_raf_uses_its_matching_bundled_profile() {
        let (w, h) = (36, 24);
        let mut spec = RafSpec::new(w, h, mosaic_cfa(&scene(w, h), w, &XTRANS, 6, 256, 16383), XTRANS.to_vec());
        spec.black = vec![256; 36];
        let mut sensor = decode(&spec.build(), &Limits::default()).unwrap();
        assert_eq!(sensor.format, RawFormat::Raf);
        assert!(!sensor.cfa.as_ref().unwrap().is_bayer());
        sensor.model = Some("FUJIFILM X-T3".into());
        let loaded = load(&sensor, None).unwrap().unwrap();
        assert_eq!(loaded.report["source"], "bundled-dcp");
        assert_eq!(loaded.report["cameraModel"], "FUJIFILM X-T3");
        let developed = photocraft_raw::develop_sensor_profile(&sensor, &Default::default(), Some(&loaded.profile)).unwrap();
        assert_eq!((developed.width, developed.height), (36, 24));
        assert_eq!(developed.rgb.len(), w * h * 3);
    }
}
