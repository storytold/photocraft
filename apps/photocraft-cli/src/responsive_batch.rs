//! `responsive-batch`: apply a set of named artboard sizes (and optional responsive templates)
//! to every layered document in a folder, ported to the clap CLI from the branch's original
//! hand-rolled subcommand table.

use std::io::Write;
use std::path::{Path, PathBuf};

use photocraft_automation::{Headless, files};
use photocraft_io::ExportOptions;
use serde_json::{Value, json};

use crate::{
    CliError, Result,
    args::ResponsiveBatchArgs,
    batch::{is_input, same_dir},
    output::warn_all,
};

struct ResponsivePreset {
    name: String,
    width: u32,
    height: u32,
    layout: String,
    template: Option<Value>,
}

const MAX_RESPONSIVE_BATCH_CONFIG_BYTES: usize = 1_048_576;

fn responsive_presets(path: &Path) -> Result<Vec<ResponsivePreset>> {
    let bytes = std::fs::read(path).map_err(|error| CliError::Message(format!("{}: {error}", path.display())))?;
    if bytes.len() > MAX_RESPONSIVE_BATCH_CONFIG_BYTES {
        return Err(CliError::Message("responsive preset config exceeds the 1 MiB limit".into()));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| CliError::Message(format!("responsive preset JSON: {error}")))?;
    let root = value.as_object().ok_or_else(|| CliError::Message("responsive preset config must be an object".into()))?;
    if root.keys().any(|key| key != "presets") {
        return Err(CliError::Message("responsive preset config only accepts the `presets` key".into()));
    }
    let entries = root.get("presets").and_then(Value::as_array).ok_or_else(|| CliError::Message("responsive preset config needs a `presets` array".into()))?;
    if entries.is_empty() || entries.len() > photocraft_doc::responsive::MAX_RESPONSIVE_VARIANTS {
        return Err(CliError::Message("responsive preset count must be between 1 and 64".into()));
    }
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut presets = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let object = entry.as_object().ok_or_else(|| CliError::Message(format!("preset {index} must be an object")))?;
        if object.keys().any(|key| !matches!(key.as_str(), "name" | "width" | "height" | "layout" | "template")) {
            return Err(CliError::Message(format!("preset {index} contains an unknown field")));
        }
        let name = object.get("name").and_then(Value::as_str).ok_or_else(|| CliError::Message(format!("preset {index} needs a string `name`")))?.trim();
        if name.is_empty() || name.len() > 128 {
            return Err(CliError::Message(format!("preset {index} name must contain 1..128 UTF-8 bytes")));
        }
        let width = object
            .get("width")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| CliError::Message(format!("preset `{name}` width must be an unsigned integer")))?;
        let height = object
            .get("height")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| CliError::Message(format!("preset `{name}` height must be an unsigned integer")))?;
        let area = width.checked_mul(height).ok_or_else(|| CliError::Message(format!("preset `{name}` dimensions overflow")))?;
        if width == 0 || height == 0 || width > 300_000 || height > 300_000 || area > 36_000_000 {
            return Err(CliError::Message(format!("preset `{name}` must be positive, at most 300,000 px per side, and no more than 36 megapixels")));
        }
        let layout = object.get("layout").and_then(Value::as_str).unwrap_or("semantic");
        if !matches!(layout, "semantic" | "proportional") {
            return Err(CliError::Message(format!("preset `{name}` layout must be `semantic` or `proportional`")));
        }
        let template = match object.get("template") {
            None | Some(Value::Null) => None,
            Some(Value::String(template_path)) if !template_path.trim().is_empty() => {
                let template_path = Path::new(template_path);
                let template_path = if template_path.is_absolute() { template_path.to_path_buf() } else { base.join(template_path) };
                let template_bytes = std::fs::read(&template_path).map_err(|error| CliError::Message(format!("{}: {error}", template_path.display())))?;
                if template_bytes.len() > MAX_RESPONSIVE_BATCH_CONFIG_BYTES {
                    return Err(CliError::Message(format!("{} exceeds the 1 MiB template limit", template_path.display())));
                }
                Some(
                    serde_json::from_slice(&template_bytes)
                        .map_err(|error| CliError::Message(format!("{}: invalid template JSON: {error}", template_path.display())))?,
                )
            }
            Some(_) => return Err(CliError::Message(format!("preset `{name}` template must be a non-empty path string"))),
        };
        presets.push(ResponsivePreset { name: name.to_owned(), width, height, layout: layout.to_owned(), template });
    }
    Ok(presets)
}

fn safe_file_stem(value: &str) -> String {
    let mut output = String::with_capacity(value.len().min(128));
    let mut separator = false;
    for character in value.chars().take(128) {
        if character.is_ascii_alphanumeric() {
            output.push(character);
            separator = false;
        } else if !separator && !output.is_empty() {
            output.push('_');
            separator = true;
        }
    }
    while output.ends_with('_') {
        output.pop();
    }
    output
}

/// Apply a set of named artboard sizes and optional responsive templates to every layered input.
pub(crate) fn responsive_batch(a: &ResponsiveBatchArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result {
    let (in_dir, out_dir) = (a.in_dir.as_path(), a.out.as_path());
    // Each preset writes its own artboard export, so an `--out` that is the `--in` folder would
    // replace the originals (#492).
    if same_dir(in_dir, out_dir) {
        return Err(CliError::OutIsIn(out_dir.to_path_buf()));
    }
    let presets = responsive_presets(&a.presets)?;
    let opts = ExportOptions::from(&a.export);
    // `--format .jpg` names outputs `<stem>.jpg`, as `--format jpg` does (#490).
    let format = a.export.format.as_deref().map(|f| f.strip_prefix('.').unwrap_or(f)).unwrap_or("png").to_ascii_lowercase();
    std::fs::create_dir_all(out_dir).map_err(|error| CliError::Message(format!("{}: {error}", out_dir.display())))?;
    let mut inputs: Vec<PathBuf> = std::fs::read_dir(in_dir)
        .map_err(|source| CliError::ReadDir { path: in_dir.to_path_buf(), source })?
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_input(p))
        .collect();
    inputs.sort();
    if inputs.is_empty() {
        return Err(CliError::Message(format!("{} contains no supported layered documents", in_dir.display())));
    }
    let (mut ok, mut failed) = (0usize, 0usize);
    for (index, input) in inputs.iter().enumerate() {
        let r = (|| -> Result<usize> {
            let mut h = Headless::trusted_local();
            let opened = h.open(input)?;
            warn_all(err, &serde_json::from_value::<Vec<String>>(opened["warnings"].clone()).unwrap_or_default());
            let state = h.session.active().ok_or_else(|| CliError::Message("opened document is unavailable".into()))?;
            let master = state
                .doc
                .responsive
                .master
                .filter(|id| state.doc.layer(*id).is_some_and(|layer| layer.artboard().is_some()))
                .or_else(|| state.doc.artboards().first().map(|board| board.0))
                .ok_or_else(|| CliError::Message(format!("{} has no artboard to use as a master", input.display())))?;
            let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| format!("document-{index}"));
            let export_dir = out_dir.join(format!("{index:04}"));
            std::fs::create_dir_all(&export_dir).map_err(|error| CliError::Message(format!("{}: {error}", export_dir.display())))?;
            let mut output_count = 0usize;
            let mut claims = photocraft_engine::file_cmds::OutputClaims::default();
            for preset in &presets {
                if let Some(template) = &preset.template {
                    h.command_run("file.import.responsiveTemplate", json!({"layer": master.0, "template": template}))
                        .map_err(|source| CliError::Message(format!("preset `{}` template: {source}", preset.name)))?;
                }
                let created = h
                    .command_run(
                        "layer.artboard.createVariant",
                        json!({"layer": master.0, "width": preset.width, "height": preset.height, "name": preset.name, "layout": preset.layout}),
                    )
                    .map_err(|source| CliError::Message(format!("preset `{}`: {source}", preset.name)))?;
                let artboard = created
                    .get("layer")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| CliError::Message(format!("preset `{}` returned no artboard ID", preset.name)))?;
                let state = h.session.active().ok_or_else(|| CliError::Message("active document disappeared during batch".into()))?;
                let one = photocraft_engine::artboard_cmds::artboard_document(&state.doc, photocraft_doc::LayerId(artboard))
                    .map_err(|source| CliError::Message(source.to_string()))?
                    .ok_or_else(|| CliError::Message(format!("preset `{}` artboard disappeared before export", preset.name)))?;
                let filename = format!("{}_{}.{}", safe_file_stem(&stem), safe_file_stem(&preset.name), format);
                let target = export_dir.join(filename);
                let target_text = target.to_string_lossy();
                claims.check(&target_text).map_err(CliError::OutputClaimed)?;
                let ws = files::save(&one, &target, Some(&format), &opts, None)?;
                claims.record(&target_text, &input.to_string_lossy());
                warn_all(err, &ws);
                output_count = output_count.saturating_add(1);
            }
            Ok(output_count)
        })();
        match r {
            Ok(count) => {
                ok += 1;
                let _ = writeln!(out, "ok    {} -> {} ({} preset exports)", input.display(), out_dir.display(), count);
            }
            Err(e) => {
                failed += 1;
                let _ = writeln!(err, "FAIL  {}: {e}", input.display());
            }
        }
    }
    let _ = writeln!(out, "{ok} succeeded, {failed} failed; {} presets", presets.len());
    if failed > 0 { Err(CliError::FilesFailed(failed)) } else { Ok(()) }
}
