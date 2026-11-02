//! Native model metadata, prompts, defaults and model slots. Files contain only `meta`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result, Task};

/// Licence class of the model a template drives, shown in the picker and gated by preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum License {
    /// Apache-2.0 / MIT: usable by anyone for anything.
    Permissive,
    /// Open weights with conditions (revenue caps, moderation duties).
    Community,
    /// Non-commercial / research-only.
    Research,
}

/// A model file slot and its folder in the existing weights layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSlot {
    /// The placeholder whose value is the file name (`unet`, `clip`, `vae`, `lora`…).
    pub placeholder: String,
    /// Subfolder under Models folder.
    pub folder: String,
    pub default: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Defaults {
    pub steps: u32,
    pub guidance: f32,
}

impl Default for Defaults {
    fn default() -> Self {
        Self { steps: 20, guidance: 1.0 }
    }
}

/// Reference image limits for the native request, besides its main picture.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct References {
    /// At most this many (0 = the template takes none).
    pub max: u32,
    /// At least this many (a reference-driven template).
    pub min: u32,
}

/// Output sizes a text-to-image template offers (empty = not an image template).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Sizes {
    /// `"1:1"`, `"3:2"`, … in the order the picker shows them.
    pub aspects: Vec<String>,
    /// Shorter and longer side limits (pixels) and the grid both sides snap to.
    pub min: u32,
    pub max: u32,
    pub step: u32,
}

/// How many variations one request may ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Counts {
    pub max: u32,
    pub default: u32,
}

impl Default for Counts {
    fn default() -> Self {
        Self { max: 4, default: 1 }
    }
}

/// A run's expected duration on a warm server: `base + perMegapixel × megapixels +
/// perReference × extra reference images` seconds per variation (measured on the reference
/// machine, see `docs/comfy/benchmarks.md`). A reference-driven text-to-image template's first
/// reference stands in for the picture and is in `base`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Estimate {
    pub base: f32,
    pub per_megapixel: f32,
    pub per_reference: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    /// `family/task`, e.g. `qwen-edit-2511/fill`. Lower-case letters, digits, `-`, `.`, `/`.
    pub id: String,
    pub name: String,
    pub family: String,
    pub task: Task,
    pub license: License,
    #[serde(default)]
    pub license_note: String,
    #[serde(default)]
    pub needs_image: bool,
    #[serde(default)]
    pub needs_mask: bool,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub models: Vec<ModelSlot>,
    /// Additional native request parameters supported by this template.
    #[serde(default)]
    pub parameters: Vec<String>,
    /// How the user's prompt is wrapped before it reaches the model: `{prompt}` is replaced by
    /// the text (instruction-following editors want "change X to …", not a bare noun phrase).
    /// Empty = the prompt as typed.
    #[serde(default)]
    pub prompt_format: String,
    /// The wrapper used instead when the prompt already is an instruction (it starts with an
    /// imperative verb such as "remove" or "replace", see [`IMPERATIVE_OPENERS`]); empty = use
    /// `prompt_format` for those too.
    #[serde(default)]
    pub prompt_format_imperative: String,
    /// For text-to-image templates whose model can output transparency: how the prompt asks
    /// for it (`{prompt}` replaced), used when the caller wants an RGBA result. Empty = the
    /// model cannot, and a request for transparency is refused.
    #[serde(default)]
    pub prompt_format_transparent: String,
    /// Expand templates: how much of the picture next to the new canvas goes into the request,
    /// as a fraction of the canvas's longer side (`generate.expand`'s `margin` default). Models
    /// that regenerate the whole picture in context want the whole picture (1.0): shown a strip
    /// of it, they paint a second subject into the new area. Absent: the engine's 0.25.
    #[serde(default)]
    pub expand_margin: Option<f32>,
    /// Fill templates: the selection is flattened to mid-grey in the request image before it
    /// is sent, so the model sees a hole to paint rather than the old object to copy (a swap
    /// from a reference image wants this; a fill that continues what is there does not). The
    /// caller's `erase` overrides it.
    #[serde(default)]
    pub erase_selection: bool,
    /// Fill templates with a removal adapter trained on a painted highlight: the colour the
    /// engine paints the selection in the request picture ("red"), sending the adapter's
    /// trigger sentence instead of the removal wording (ShotoPhopEX).
    #[serde(default)]
    pub highlight: Option<String>,
    #[serde(default)]
    pub notes: String,
    /// Who made the model ("Alibaba Qwen", "Krea", "Meta"), for the picker.
    #[serde(default)]
    pub creator: String,
    /// One line for the picker ("the fast local edit model").
    #[serde(default)]
    pub description: String,
    /// Free-text badges for the picker (it adds the time estimate itself).
    #[serde(default)]
    pub badges: Vec<String>,
    #[serde(default)]
    pub references: References,
    #[serde(default)]
    pub sizes: Sizes,
    #[serde(default)]
    pub counts: Counts,
    #[serde(default)]
    pub estimated_seconds: Estimate,
    /// The Generate panel pages that offer this template: `image`, `fill`, `edit`, `expand`,
    /// `matte`, `split`, `select`, `enhance`.
    #[serde(default)]
    pub pages: Vec<String>,
}

impl Meta {
    /// Expected seconds for `count` variations at `width × height` with `references` reference
    /// images on a warm server (0 when the template has no estimate). One formula for the UI,
    /// the CLI and scripts.
    pub fn estimate_seconds(&self, width: u32, height: u32, count: u32, references: u32) -> f32 {
        let e = &self.estimated_seconds;
        let mp = f64::from(width) * f64::from(height) / 1_000_000.0;
        let free = u32::from(!self.needs_image && self.references.min > 0);
        let extra = f64::from(references.saturating_sub(free));
        let one = f64::from(e.base) + f64::from(e.per_megapixel) * mp + f64::from(e.per_reference) * extra;
        (one * f64::from(count.max(1))).clamp(0.0, 1.0e6) as f32
    }
}

/// A parsed template.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    pub meta: Meta,
}

#[derive(Deserialize)]
struct File {
    meta: Meta,
}

const BUILTIN: &[&str] = &[
    include_str!("../workflows/qwen-edit-2511-fill.json"),
    include_str!("../workflows/qwen-edit-2511-fill-lightning-8.json"),
    include_str!("../workflows/qwen-edit-2511-fill-lightning-4.json"),
    include_str!("../workflows/qwen-edit-2511-fill-guided.json"),
    include_str!("../workflows/qwen-edit-2511-fill-ref-lightning-8.json"),
    include_str!("../workflows/qwen-edit-2511-fill-ref.json"),
    include_str!("../workflows/qwen-edit-2511-expand-lightning-8.json"),
    include_str!("../workflows/qwen-edit-2511-expand.json"),
    include_str!("../workflows/qwen-edit-2511-edit-lightning-8.json"),
    include_str!("../workflows/qwen-edit-2511-edit.json"),
    include_str!("../workflows/qwen-2.1-fill.json"),
    include_str!("../workflows/qwen-2.1-expand.json"),
    include_str!("../workflows/krea2-turbo-image.json"),
    include_str!("../workflows/qwen-2.1-image.json"),
    include_str!("../workflows/qwen-2.1-matte.json"),
    include_str!("../workflows/sam3.1-segment.json"),
    include_str!("../workflows/sam3.1-segment-point.json"),
    include_str!("../workflows/qwen-layered-split.json"),
    include_str!("../workflows/enhance-edit.json"),
    include_str!("../workflows/enhance-image.json"),
    include_str!("../workflows/qwen-edit-2511-image-ref.json"),
    include_str!("../workflows/krea2-turbo-image-style.json"),
    include_str!("../workflows/qwen-2.1-edit.json"),
    include_str!("../workflows/qwen-edit-2511-angles.json"),
    include_str!("../workflows/sam3.1-segment-box.json"),
    include_str!("../workflows/qwen-edit-2511-remove-lightning-8.json"),
    include_str!("../workflows/krea2-turbo-edit.json"),
    include_str!("../workflows/qwen-2.1-image-ref.json"),
    include_str!("../workflows/moss-image-image.json"),
    include_str!("../workflows/moss-image-image-base.json"),
];

/// Verbs a prompt can open with when it is already an edit instruction ("remove the car",
/// "make the sky stormy") rather than a description of new content ("a red boat").
pub const IMPERATIVE_OPENERS: &[&str] = &[
    "add",
    "remove",
    "delete",
    "erase",
    "replace",
    "change",
    "make",
    "turn",
    "put",
    "fill",
    "paint",
    "convert",
    "swap",
    "move",
    "give",
    "extend",
    "clean",
    "restore",
    "fix",
    "repair",
    "cover",
    "color",
    "colour",
    "recolor",
    "recolour",
    "blur",
    "sharpen",
    "brighten",
    "darken",
    "transform",
    "draw",
    "place",
    "insert",
    "render",
    "create",
    "generate",
    "show",
    "hide",
    "open",
    "close",
    "rotate",
    "flip",
    "crop",
    "redraw",
    "repaint",
    "retouch",
    "smooth",
    "straighten",
    "upscale",
    "enhance",
    "edit",
    "modify",
    "adjust",
    "apply",
    "let",
    "keep",
    "set",
    "write",
];

/// Does the prompt start with one of [`IMPERATIVE_OPENERS`] (case-insensitive, first word)?
pub fn is_imperative(prompt: &str) -> bool {
    let first = prompt.trim().split(|c: char| !c.is_alphanumeric()).next().unwrap_or("").to_ascii_lowercase();
    !first.is_empty() && IMPERATIVE_OPENERS.contains(&first.as_str())
}

/// Every compiled-in template. Parsing cannot fail for shipped files (a test checks it); a file
/// that fails anyway is skipped rather than taking the others down.
pub fn builtin() -> Vec<Template> {
    BUILTIN.iter().filter_map(|src| Template::parse(src).ok()).collect()
}

/// The built-in template with this id.
pub fn find(id: &str) -> Result<Template> {
    builtin().into_iter().find(|t| t.meta.id == id).ok_or_else(|| {
        let known: Vec<String> = builtin().into_iter().map(|t| t.meta.id).collect();
        Error::Template(format!("unknown template `{id}`; available: {}", known.join(", ")))
    })
}

impl Template {
    /// Parse native model metadata and validate its identity and reference limits.
    pub fn parse(src: &str) -> Result<Template> {
        let f: File = serde_json::from_str(src).map_err(|e| Error::Template(format!("not a template file: {e}")))?;
        let m = &f.meta;
        if m.id.is_empty() || !m.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '/' | '_')) {
            return Err(Error::Template(format!("bad template id `{}`", m.id)));
        }
        if m.references.min > m.references.max {
            return Err(Error::Template(format!("{}: references need min <= max", m.id)));
        }
        Ok(Template { meta: f.meta })
    }

    /// The prompt as the model should see it: `meta.promptFormatImperative` when the prompt
    /// already is an instruction (see [`is_imperative`]) and the template has one, else
    /// `meta.promptFormat`; an empty format passes the prompt through.
    pub fn format_prompt(&self, prompt: &str) -> String {
        let prompt = prompt.trim();
        let imperative = self.meta.prompt_format_imperative.trim();
        let f = if is_imperative(prompt) && !imperative.is_empty() { imperative } else { self.meta.prompt_format.trim() };
        if f.is_empty() {
            prompt.to_string()
        } else if f.contains("{prompt}") {
            f.replace("{prompt}", prompt)
        } else {
            format!("{f} {prompt}")
        }
    }

    /// Bindings for the model slots: the template defaults, overridden by `overrides`
    /// (`placeholder → file name`). Unknown override names are an error so typos surface.
    pub fn model_bindings(&self, overrides: &[(String, String)]) -> Result<BTreeMap<String, Value>> {
        let mut out: BTreeMap<String, Value> = self.meta.models.iter().map(|s| (s.placeholder.clone(), Value::String(s.default.clone()))).collect();
        for (k, v) in overrides {
            if !out.contains_key(k) {
                let known: Vec<&str> = self.meta.models.iter().map(|s| s.placeholder.as_str()).collect();
                return Err(Error::Request(format!("template `{}` has no model slot `{k}` (slots: {})", self.meta.id, known.join(", "))));
            }
            if v.is_empty() || v.contains(['/', '\\', '\0']) || v.contains("..") {
                return Err(Error::Request(format!("`{k}` must be a plain model file name, got `{v}`")));
            }
            out.insert(k.clone(), Value::String(v.clone()));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_builtin_is_meta_only_and_valid() {
        assert_eq!(builtin().len(), BUILTIN.len());
        for src in BUILTIN {
            let v: Value = serde_json::from_str(src).unwrap();
            assert_eq!(v.as_object().unwrap().len(), 1);
            let t = Template::parse(src).unwrap();
            assert!(t.meta.references.min <= t.meta.references.max);
            assert!(!t.meta.family.is_empty());
        }
    }
    #[test]
    fn prompts_and_overrides_are_preserved() {
        assert!(is_imperative("Remove the car"));
        assert!(!is_imperative("a red boat"));
        let t = find("qwen-edit-2511/fill").unwrap();
        assert!(t.format_prompt("a red boat").starts_with("Add a red boat"));
        assert!(t.format_prompt("remove the car").starts_with("remove the car"));
        assert!(t.model_bindings(&[("unet".into(), "custom.safetensors".into())]).is_ok());
        assert!(t.model_bindings(&[("unet".into(), "../bad".into())]).is_err());
        assert!(t.model_bindings(&[("unknown".into(), "x".into())]).is_err());
        assert!(Template::parse(r#"{"meta":{}}"#).is_err());
    }
}
