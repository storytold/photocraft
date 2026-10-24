//! Reproducible original synthetic XCF stress, not stock GIMP producer evidence.
//! Generate once, then measure one case in a fresh process for case-scoped peak RSS.
use serde_json::{Value, json};
use std::{
    error::Error,
    ffi::OsString,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const MAX_INPUT_BYTES: usize = 512 << 20;
const GRAPH: &str = "id=in over aux=[ref=in invert-linear]";

struct Wire(Vec<u8>);
impl Wire {
    fn new(capacity: usize) -> Result<Self> {
        if capacity > MAX_INPUT_BYTES {
            return Err("synthetic source exceeds benchmark input limit".into());
        }
        let mut data = Vec::new();
        data.try_reserve_exact(capacity)?;
        Ok(Self(data))
    }
    fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let size = self.0.len().checked_add(bytes.len()).ok_or("wire length overflow")?;
        if size > MAX_INPUT_BYTES {
            return Err("synthetic wire exceeds benchmark input limit".into());
        }
        self.0.try_reserve(bytes.len())?;
        self.0.extend_from_slice(bytes);
        Ok(())
    }
    fn word(&mut self, value: u32) -> Result<()> {
        self.bytes(&value.to_be_bytes())
    }
    fn pointer(&mut self, value: usize) -> Result<()> {
        self.bytes(&u64::try_from(value)?.to_be_bytes())
    }
    fn slot(&mut self) -> Result<usize> {
        let at = self.0.len();
        self.pointer(0)?;
        Ok(at)
    }
    fn patch(&mut self, at: usize) -> Result<()> {
        let value = u64::try_from(self.0.len())?.to_be_bytes();
        let end = at.checked_add(value.len()).ok_or("wire patch overflow")?;
        self.0.get_mut(at..end).ok_or("wire patch outside source")?.copy_from_slice(&value);
        Ok(())
    }
    fn text(&mut self, value: &str) -> Result<()> {
        self.word(u32::try_from(value.len())?.checked_add(1).ok_or("wire text overflow")?)?;
        self.bytes(value.as_bytes())?;
        self.bytes(&[0])
    }
    fn property(&mut self, kind: u32, value: &[u8]) -> Result<()> {
        self.word(kind)?;
        self.word(u32::try_from(value.len())?)?;
        self.bytes(value)
    }
    fn end(&mut self) -> Result<()> {
        self.word(0)?;
        self.word(0)
    }
}
#[derive(Clone, Copy)]
struct Case {
    width: u32,
    height: u32,
    float: bool,
    graph: bool,
}
fn case(id: &str) -> Result<Case> {
    Ok(match id {
        "smoke-graph-f32" => Case { width: 96, height: 65, float: true, graph: true },
        "u16-36mp" => Case { width: 7200, height: 5000, float: false, graph: false },
        "f32-hdr-24mp" => Case { width: 6000, height: 4000, float: true, graph: false },
        "graph-u16-24mp" => Case { width: 6000, height: 4000, float: false, graph: true },
        "graph-f32-hdr-24mp" => Case { width: 6000, height: 4000, float: true, graph: true },
        _ => return Err("unknown case: use smoke-graph-f32, u16-36mp, f32-hdr-24mp, graph-u16-24mp or graph-f32-hdr-24mp".into()),
    })
}
/// Keep the measured wire layout, pixel arithmetic and fixed graph unchanged.
fn source(c: Case) -> Result<Vec<u8>> {
    if c.width == 0 || c.height == 0 || c.width > 7200 || c.height > 5000 {
        return Err("invalid fixed-case dimensions".into());
    }
    let across = c.width.div_ceil(64);
    let down = c.height.div_ceil(64);
    let count = usize::try_from(across.checked_mul(down).ok_or("tile count overflow")?)?;
    let payload = usize::try_from(c.width)?
        .checked_mul(usize::try_from(c.height)?)
        .and_then(|n| n.checked_mul(if c.float { 16 } else { 8 }))
        .ok_or("pixel byte count overflow")?;
    let capacity = count.checked_mul(8).and_then(|n| n.checked_add(payload)).and_then(|n| n.checked_add(1024)).ok_or("source size overflow")?;
    let mut w = Wire::new(capacity)?;
    w.bytes(b"gimp xcf v026\0")?;
    for n in [c.width, c.height, 0, if c.float { 600 } else { 250 }] {
        w.word(n)?;
    }
    w.property(17, &[0])?;
    w.end()?;
    let layer = w.slot()?;
    w.pointer(0)?;
    w.pointer(0)?;
    w.pointer(0)?;
    w.patch(layer)?;
    for n in [c.width, c.height, 1] {
        w.word(n)?;
    }
    w.text("Original deterministic high-depth stress pixels")?;
    w.end()?;
    let hierarchy = w.slot()?;
    w.pointer(0)?;
    let effect = if c.graph { Some(w.slot()?) } else { None };
    w.pointer(0)?;
    w.patch(hierarchy)?;
    for n in [c.width, c.height, if c.float { 16 } else { 8 }] {
        w.word(n)?;
    }
    let level = w.slot()?;
    w.pointer(0)?;
    w.patch(level)?;
    w.word(c.width)?;
    w.word(c.height)?;
    let mut slots = Vec::new();
    slots.try_reserve_exact(count)?;
    for _ in 0..count {
        slots.push(w.slot()?);
    }
    w.pointer(0)?;
    for (index, slot) in slots.into_iter().enumerate() {
        w.patch(slot)?;
        let index = u32::try_from(index)?;
        let x = (index % across) * 64;
        let y = (index / across) * 64;
        for dy in 0..(c.height - y).min(64) {
            for dx in 0..(c.width - x).min(64) {
                let (x, y) = (x + dx, y + dy);
                // Fixed dimensions above bound every integer expression; preserve the
                // original float order so generator output remains byte-identical.
                let values = [
                    ((x * 17 + y * 31) % 1024) as f32 / 1023.0,
                    ((x * 7 + y * 11) % 1024) as f32 / 1023.0,
                    ((x * 23 + y * 5) % 1024) as f32 / 1023.0,
                    0.25 + ((x + y) % 4) as f32 * 0.25,
                ];
                for (channel, value) in values.into_iter().enumerate() {
                    if c.float {
                        let value = if channel < 3 { value * 2.0 - 0.25 } else { value };
                        w.bytes(&value.to_be_bytes())?;
                    } else {
                        w.bytes(&((value * 65535.0).round() as u16).to_be_bytes())?;
                    }
                }
            }
        }
    }
    if let Some(slot) = effect {
        w.patch(slot)?;
        w.text("Original supported native fork/join stress")?;
        w.text("")?;
        w.text("gegl:gegl")?;
        w.text("0:0")?;
        w.property(33, &1.0f32.to_be_bytes())?;
        w.property(8, &1u32.to_be_bytes())?;
        let mut argument = Wire::new(128)?;
        argument.text("string")?;
        argument.word(4)?;
        argument.text(GRAPH)?;
        w.property(45, &argument.0)?;
        w.end()?;
        w.pointer(0)?;
    }
    Ok(w.0)
}
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let size = usize::try_from(file.metadata()?.len())?;
    if size > limit {
        return Err("input exceeds benchmark read limit".into());
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size)?;
    let mut chunk = [0u8; 64 << 10];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            return Ok(bytes);
        }
        let size = bytes.len().checked_add(read).ok_or("input length overflow")?;
        if size > limit {
            return Err("input grew beyond benchmark read limit".into());
        }
        bytes.try_reserve(read)?;
        bytes.extend_from_slice(chunk.get(..read).ok_or("invalid read size")?);
    }
}
struct Cli {
    mode: String,
    id: String,
    case: Case,
    path: PathBuf,
    reps: usize,
}
fn arguments(mut args: impl Iterator<Item = OsString>) -> Result<Cli> {
    let _program = args.next();
    let mode = args.next().ok_or("usage: xcf_import_bench generate|measure CASE FILE [REPS]")?.into_string().map_err(|_| "mode must be UTF-8")?;
    if mode != "generate" && mode != "measure" {
        return Err("expected generate or measure".into());
    }
    let id = args.next().ok_or("missing CASE")?.into_string().map_err(|_| "case must be UTF-8")?;
    let case = case(&id)?;
    let path = PathBuf::from(args.next().ok_or("missing FILE")?);
    let reps = match args.next() {
        Some(reps) if mode == "measure" => reps.to_str().ok_or("REPS must be UTF-8")?.parse::<usize>()?,
        Some(_) => return Err("generate does not accept REPS".into()),
        None => 5,
    };
    if reps == 0 || reps > 5 {
        return Err("REPS must be 1..5 (plus one warmup)".into());
    }
    if args.next().is_some() {
        return Err("unexpected trailing argument".into());
    }
    Ok(Cli { mode, id, case, path, reps })
}
fn emit(value: &Value) -> Result<()> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}
fn graph_applied(doc: &photocraft_doc::Document) -> bool {
    use photocraft_doc::import_conversion::{ConversionDisposition as D, ImportConversion};
    matches!(&doc.metadata.import_conversion, Some(ImportConversion::Report(report)) if report.entries.iter().any(|entry|
        entry.feature_code == "xcf.layer_effect" && matches!(entry.disposition,D::NativeEquivalent|D::NativeApproximation) && !entry.targets.is_empty()))
}
fn main() -> Result<()> {
    let cli = arguments(std::env::args_os())?;
    let c = cli.case;
    let id = cli.id;
    if cli.mode == "generate" {
        let bytes = source(c)?;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&cli.path)?;
        file.write_all(&bytes)?;
        return emit(&json!({"case":id,"bytes":bytes.len(),"producer":"original documented-wire synthetic stress; not stock GIMP"}));
    }
    let bytes = read_bounded(&cli.path, MAX_INPUT_BYTES)?;
    let mut runs = Vec::new();
    runs.try_reserve_exact(cli.reps)?;
    for run in 0..=cli.reps {
        let start = Instant::now();
        let imported = match photocraft_io::import("synthetic-stress.xcf", &bytes) {
            Ok(v) => v,
            Err(e) => {
                emit(&json!({"case":id,"outcome":"import_error","error_debug":format!("{e:?}"),"error":e.to_string()}))?;
                return Err(e.into());
            }
        };
        let import_ms = start.elapsed().as_secs_f64() * 1000.0;
        let warnings = imported.warnings;
        let mut doc = imported.document;
        if doc.size.width != c.width || doc.size.height != c.height {
            return Err("input dimensions do not match selected case".into());
        }
        let prepare = Instant::now();
        if let Err(e) = photocraft_engine::import_prepare::prepare_imported_document(&mut doc, &photocraft_engine::jobs::JobCtx::new()) {
            emit(&json!({"case":id,"outcome":"prepare_error","error_debug":format!("{e:?}"),"error":e.to_string(),"import_ms":import_ms}))?;
            return Err(e.into());
        }
        let prepare_ms = prepare.elapsed().as_secs_f64() * 1000.0;
        let render = Instant::now();
        let output = photocraft_compose::flatten(&doc);
        let render_ms = render.elapsed().as_secs_f64() * 1000.0;
        let total_ms = start.elapsed().as_secs_f64() * 1000.0;
        if output.px.iter().flatten().any(|v| !v.is_finite()) {
            return Err("nonfinite native output".into());
        }
        if c.graph && !graph_applied(&doc) {
            return Err("graph was not translated into applied native targets; no graph timing acceptance".into());
        }
        let report = match &doc.metadata.import_conversion {
            Some(photocraft_doc::import_conversion::ImportConversion::Report(r)) => serde_json::to_value(r)?,
            _ => Value::Null,
        };
        std::hint::black_box(&output);
        if run > 0 {
            runs.push(json!({"run":run,"import_ms":import_ms,"prepare_ms":prepare_ms,"render_ms":render_ms,"total_ms":total_ms}));
        }
        if run == cli.reps {
            emit(
                &json!({"schema_version":1,"case":id,"width":c.width,"height":c.height,"source_precision":if c.float{"F32 linear HDR -0.25..1.75"}else{"U16 encoded"},"graph":c.graph,"source_bytes":bytes.len(),"outcome":"prepared_and_rendered","native_depth":format!("{:?}",doc.depth),"warnings":warnings,"conversion_report":report,"warmups":1,"runs":runs,"scope":"synthetic documented-wire XCF -> native import -> ordinary native preparation -> first CPU flatten; source read excluded, retained source included in process RSS"}),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::SampleType;
    #[test]
    fn partial_tiles_high_depth_and_native_graph_prepare_actual_pixels() {
        for float in [false, true] {
            for graph in [false, true] {
                let c = Case { width: 96, height: 65, float, graph };
                let bytes = source(c).unwrap();
                let mut doc = photocraft_io::import("original.xcf", &bytes).unwrap().document;
                assert_eq!(doc.depth, if float { SampleType::F32 } else { SampleType::U16 });
                if graph {
                    assert!(graph_applied(&doc));
                    assert!(doc.layers[0].surface().is_none());
                } else {
                    let raw = doc.layers[0].surface().unwrap();
                    assert!((raw.sample_channel(0, 0, 0) - if float { -0.25 } else { 0.0 }).abs() < 1e-6);
                    let value = ((95 * 17 + 64 * 31) % 1024) as f32 / 1023.0;
                    let expected = if float { value * 2.0 - 0.25 } else { (value * 65535.0).round() / 65535.0 };
                    assert!((raw.sample_channel(95, 64, 0) - expected).abs() < 1e-6, "partial edge tile");
                    if float {
                        assert!(raw.read_region(doc.bounds()).iter().any(|v| *v > 1.0));
                    }
                }
                photocraft_engine::import_prepare::prepare_imported_document(&mut doc, &photocraft_engine::jobs::JobCtx::new()).unwrap();
                assert!(doc.layers[0].surface().is_some());
                let output = photocraft_compose::flatten(&doc);
                assert_eq!(output.px.len(), 96 * 65);
                assert!(output.px.iter().flatten().all(|v| v.is_finite()));
                assert!(output.px.iter().any(|v| v[3] > 0.0), "real prepared pixels, not empty cache");
            }
        }
    }
    #[test]
    fn cli_and_wire_errors_are_bounded() {
        for args in [
            vec!["bench"],
            vec!["bench", "measure", "smoke-graph-f32", "x", "0"],
            vec!["bench", "measure", "smoke-graph-f32", "x", "6"],
            vec!["bench", "generate", "smoke-graph-f32", "x", "1"],
            vec!["bench", "measure", "smoke-graph-f32", "x", "1", "extra"],
        ] {
            assert!(arguments(args.into_iter().map(OsString::from)).is_err());
        }
        let mut wire = Wire::new(16).unwrap();
        wire.pointer(0).unwrap();
        assert!(wire.patch(usize::MAX).is_err());
        assert!(wire.patch(1).is_err());
        assert!(source(Case { width: 0, height: 1, float: false, graph: false }).is_err());
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&[7; 17]).unwrap();
        assert!(read_bounded(file.path(), 16).is_err());
        assert_eq!(read_bounded(file.path(), 17).unwrap(), vec![7; 17]);
    }
}
