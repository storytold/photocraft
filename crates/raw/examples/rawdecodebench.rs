//! Decode-only, interleaved benchmark. No disk I/O or development is timed.
//! cargo run --release -p photocraft-raw --example rawdecodebench -- FILE...
//! Emits one JSON object per file, including failures; two warmups, nine measured runs.
use std::{panic::catch_unwind, time::Instant};

use photocraft_raw::{DecodeBackend, Limits, Sensor, decode_with_backend_traced};
use serde_json::{Value, json};

const MODES: [&str; 4] = ["builtin", "flashraw", "auto", "flashraw-direct"];
const WARMUPS: usize = 2;
const RUNS: usize = 9;

enum Decoded {
    Sensor(Box<Sensor>, DecodeBackend),
    #[cfg(feature = "flashraw")]
    Direct(Box<flashraw::RawFrame>, flashraw::RawFormat),
}

impl Decoded {
    fn actual_backend(&self) -> String {
        match self {
            Self::Sensor(_, backend) => format!("{backend:?}"),
            #[cfg(feature = "flashraw")]
            Self::Direct(_, _) => "FlashRaw-library".into(),
        }
    }
    fn format(&self) -> String {
        match self {
            Self::Sensor(s, _) => s.format.name().into(),
            #[cfg(feature = "flashraw")]
            Self::Direct(_, format) => format.name().into(),
        }
    }
    fn geometry(&self) -> (usize, usize, usize) {
        match self {
            Self::Sensor(s, _) => (s.width, s.height, s.samples),
            #[cfg(feature = "flashraw")]
            Self::Direct(f, _) => (f.width as usize, f.height as usize, f.cpp as usize),
        }
    }

    fn samples(&self) -> Option<&[u16]> {
        match self {
            Self::Sensor(s, _) => Some(&s.data),
            #[cfg(feature = "flashraw")]
            Self::Direct(f, _) => f.data.as_u16(),
        }
    }
}

fn run(bytes: &[u8], mode: &str) -> Result<Decoded, String> {
    let backend = match mode {
        "builtin" => DecodeBackend::Builtin,
        "flashraw" => DecodeBackend::FlashRaw,
        "auto" => DecodeBackend::Auto,
        "flashraw-direct" => return direct(bytes),
        _ => return Err(format!("unknown benchmark backend: {mode}")),
    };
    decode_with_backend_traced(bytes, &Limits::default(), backend).map(|(s, backend)| Decoded::Sensor(Box::new(s), backend)).map_err(|e| e.to_string())
}

#[cfg(feature = "flashraw")]
fn direct(bytes: &[u8]) -> Result<Decoded, String> {
    use flashraw::{CorruptionPolicy, DecodeOptions, FlashRaw, OutputFormat};
    let limits = Limits::default();
    let fl = flashraw::Limits::default()
        .with_max_dimension(limits.max_width.max(limits.max_height))
        .with_max_samples(limits.max_pixels.saturating_mul(3).min(limits.max_alloc / 4));
    let raw = FlashRaw::open_with(bytes, fl).map_err(|e| e.to_string())?;
    let format = raw.format();
    let info = raw.frame_info();
    if info.width > limits.max_width || info.height > limits.max_height || u64::from(info.width) * u64::from(info.height) > limits.max_pixels {
        return Err("frame exceeds PhotoCraft benchmark limits".into());
    }
    let frame = raw
        .decode(DecodeOptions {
            output: OutputFormat::RawU16,
            crop: false,
            apply_opcodes: false,
            corruption_policy: CorruptionPolicy::Strict,
            preview_fallback: false,
            limits: fl,
            threads: cfg!(target_arch = "wasm32").then_some(1),
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    if frame.is_incomplete() {
        return Err("incomplete frame".into());
    }
    Ok(Decoded::Direct(Box::new(frame), format))
}

#[cfg(not(feature = "flashraw"))]
fn direct(_: &[u8]) -> Result<Decoded, String> {
    Err("FlashRaw feature disabled".into())
}

// The diagnostic boundary also records panics in a dependency without losing later files.
fn guarded(bytes: &[u8], mode: &str) -> Result<Decoded, String> {
    catch_unwind(|| run(bytes, mode)).unwrap_or_else(|payload| {
        let message = payload.downcast_ref::<String>().map(String::as_str).or_else(|| payload.downcast_ref::<&str>().copied()).unwrap_or("non-string panic");
        Err(format!("PANIC: {message}"))
    })
}

fn metadata_equal(a: &Sensor, b: &Sensor) -> bool {
    a.format == b.format
        && a.make == b.make
        && a.model == b.model
        && a.cfa == b.cfa
        && a.linearization == b.linearization
        && a.black == b.black
        && a.white == b.white
        && a.active == b.active
        && a.crop == b.crop
        && a.color == b.color
        && a.camera_wb == b.camera_wb
        && a.orientation == b.orientation
        && a.baseline_exposure == b.baseline_exposure
        && a.gain_maps == b.gain_maps
        && a.tone_curve == b.tone_curve
        && a.black_point == b.black_point
        && a.warnings == b.warnings
}

fn comparison(a: &Decoded, b: &Decoded) -> Value {
    let same_geometry = a.geometry() == b.geometry();
    let samples = a.samples().zip(b.samples()).map(|(a, b)| a == b);
    let metadata = match (a, b) {
        (Decoded::Sensor(a, _), Decoded::Sensor(b, _)) => Some(metadata_equal(a, b)),
        #[cfg(feature = "flashraw")]
        _ => None,
    };
    json!({"geometry_equal":same_geometry, "samples_equal":samples, "metadata_equal":metadata})
}

fn summary(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().map(|v| {
        if sorted.len().is_multiple_of(2) { (v + sorted.get(sorted.len().saturating_div(2).saturating_sub(1)).copied().unwrap_or(v)) / 2.0 } else { v }
    });
    json!({"median_ms":median,"min_ms":sorted.first(),"max_ms":sorted.last(),"runs_ms":values})
}

fn order(round: usize) -> impl Iterator<Item = usize> {
    (0..MODES.len()).map(move |index| (index + round) % MODES.len())
}

fn benchmark(file: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let size = std::fs::metadata(file)?.len();
    if size > 2 << 30 {
        return Err("benchmark input exceeds 2 GiB".into());
    }
    let bytes = std::fs::read(file)?;
    // Untimed exact sample/metadata checks. Prefer Builtin as the reference, then Auto.
    let mut initial: Vec<_> = MODES.iter().map(|mode| guarded(&bytes, mode)).collect();
    let reference_index = [0, 2, 1, 3].into_iter().find(|&index| initial.get(index).is_some_and(Result::is_ok));
    let mut rows = Vec::new();
    for (mode, result) in MODES.iter().zip(&initial) {
        rows.push(match result {
            Ok(decoded) => {
                let (w, h, cpp) = decoded.geometry();
                let check = reference_index.and_then(|i| initial.get(i)).and_then(|r| r.as_ref().ok()).map(|reference| comparison(reference, decoded));
                json!({"backend":mode,"actual_backend":decoded.actual_backend(),"format":decoded.format(),"status":"ok","width":w,"height":h,"samples_per_pixel":cpp,"comparison":check})
            }
            Err(error) => json!({"backend":mode,"status":"error","error":error}),
        });
    }
    // Drop all decoded planes before timing so validation doesn't inflate working memory.
    initial.clear();
    let mut times: [Vec<f64>; 4] = std::array::from_fn(|_| Vec::new());
    for round in 0..WARMUPS + RUNS {
        for index in order(round) {
            let Some(row) = rows.get_mut(index) else { continue };
            if row.get("status").and_then(Value::as_str) != Some("ok") {
                continue;
            }
            let Some(mode) = MODES.get(index) else { continue };
            let start = Instant::now();
            let result = guarded(&bytes, mode);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            match result {
                Ok(decoded) => {
                    std::hint::black_box(&decoded);
                    if round >= WARMUPS
                        && let Some(values) = times.get_mut(index)
                    {
                        values.push(elapsed);
                    }
                }
                Err(error) => {
                    *row = json!({"backend":mode,"status":"error","error":error,"failed_round":round});
                }
            }
        }
    }
    for (row, values) in rows.iter_mut().zip(&times) {
        row["timing"] = summary(values);
    }
    let reference = reference_index.and_then(|i| MODES.get(i)).copied();
    Ok(json!({"file":file,"input_bytes":bytes.len(),"reference_backend":reference,"warmups":WARMUPS,"measured_runs":RUNS,"results":rows}))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files: Vec<_> = std::env::args().skip(1).collect();
    if files.is_empty() {
        return Err("usage: rawdecodebench FILE...".into());
    }
    for file in files {
        let value = match benchmark(&file) {
            Ok(value) => value,
            Err(error) => json!({"file":file,"status":"input-error","error":error.to_string()}),
        };
        println!("{}", serde_json::to_string(&value)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_reports_odd_even_and_empty_measurements_without_inventing_zero_time() {
        assert_eq!(summary(&[9.0, 1.0, 5.0])["median_ms"], 5.0);
        assert_eq!(summary(&[9.0, 1.0, 3.0, 5.0])["median_ms"], 4.0);
        assert!(summary(&[])["median_ms"].is_null());
    }

    #[test]
    fn rotating_order_gives_each_backend_each_position() {
        let rounds: Vec<Vec<_>> = (0..MODES.len()).map(|round| order(round).collect()).collect();
        for position in 0..MODES.len() {
            let mut modes: Vec<_> = rounds.iter().filter_map(|round| round.get(position)).copied().collect();
            modes.sort_unstable();
            assert_eq!(modes, vec![0, 1, 2, 3]);
        }
    }

    #[test]
    fn validation_detects_last_sample_and_metadata_changes_separately() {
        use photocraft_raw::testgen::tiff_ep;
        let bytes = tiff_ep("NIKON CORPORATION", 16, 16, &vec![1000; 256], [0, 1, 1, 2], 16, vec![]);
        let (sensor, _) = decode_with_backend_traced(&bytes, &Limits::default(), DecodeBackend::Builtin).unwrap();
        let mut changed = sensor.clone();
        *changed.data.last_mut().unwrap() += 1;
        let reference = Decoded::Sensor(Box::new(sensor), DecodeBackend::Builtin);
        let mut altered = Decoded::Sensor(Box::new(changed), DecodeBackend::Builtin);
        assert_eq!(comparison(&reference, &altered)["samples_equal"], false);
        assert_eq!(comparison(&reference, &altered)["metadata_equal"], true);
        match &mut altered {
            Decoded::Sensor(s, _) => s.orientation = 6,
            #[cfg(feature = "flashraw")]
            Decoded::Direct(_, _) => panic!("test expects a PhotoCraft sensor"),
        }
        assert_eq!(comparison(&reference, &altered)["metadata_equal"], false);
    }
}
