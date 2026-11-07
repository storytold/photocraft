//! `mask.luminosity`: luminosity masks (Lights, Darks, Midtones at several degrees) and colour
//! masks narrowed by a tonal degree, built from the composite or the active layer.
//!
//! The masks are computed by [`photocraft_algo::luminosity`]. The result goes to one of three
//! places, each a single history step:
//!
//! * `layerMask`: the mask of the targeted layer(s), replacing a mask already there;
//! * `channel`: a new alpha channel (white = selected), ready to load as a selection;
//! * `selection`: the selection, combined with the current one by `operation`.

use photocraft_algo::luminosity::{self, MAX_DEGREE, Tone};
use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_doc::{AlphaChannel, Document, LayerMask};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "mask.luminosity";

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// What the mask follows: a tonal range, or a colour family (hue centre in degrees).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Tone(Tone),
    Colour(f32),
}

fn parse_kind(p: &Value) -> Result<(Kind, String)> {
    let name = match p.get("kind") {
        None | Some(Value::Null) => "lights",
        Some(Value::String(s)) => s.as_str(),
        Some(v) => return Err(bad(format!("kind: want a string (got {v})"))),
    };
    let lower = name.to_ascii_lowercase();
    if let Some(t) = Tone::parse(&lower) {
        return Ok((Kind::Tone(t), lower));
    }
    let centre = match lower.as_str() {
        "reds" => 0.0,
        "yellows" => 60.0,
        "greens" => 120.0,
        "cyans" => 180.0,
        "blues" => 240.0,
        "magentas" => 300.0,
        _ => return Err(bad(format!("unknown kind `{name}` (lights, darks, midtones, reds, yellows, greens, cyans, blues, magentas)"))),
    };
    Ok((Kind::Colour(centre), lower))
}

/// A degree 1..=[`MAX_DEGREE`], refused (never clamped) when it is anything else.
fn parse_degree(p: &Value, key: &str) -> Result<u32> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(1),
        Some(v) => v
            .as_u64()
            .filter(|d| (1..=u64::from(MAX_DEGREE)).contains(d))
            .map(|d| d as u32)
            .ok_or_else(|| bad(format!("{key}: want a whole number 1..{MAX_DEGREE} (got {v})"))),
    }
}

fn flag(p: &Value, key: &str, default: bool) -> bool {
    p.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// A channel name not yet used in the document: `base`, then `base (2)`, `base (3)`, …
fn unique_channel_name(doc: &Document, base: &str) -> String {
    if !doc.channels.iter().any(|c| c.name == base) {
        return base.to_string();
    }
    (2..=doc.channels.len().saturating_add(2))
        .map(|n| format!("{base} ({n})"))
        .find(|n| !doc.channels.iter().any(|c| &c.name == n))
        .unwrap_or_else(|| base.to_string())
}

fn title(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let (kind, kind_name) = parse_kind(p)?;
    let degree = parse_degree(p, "degree")?;
    // A colour mask may be narrowed by a tonal degree; a tonal mask has no use for one.
    let narrow = match (kind, p.get("tone")) {
        (Kind::Colour(_), Some(Value::String(t))) if !t.eq_ignore_ascii_case("none") => {
            let tone = Tone::parse(t).ok_or_else(|| bad(format!("tone: want lights, darks, midtones or none (got `{t}`)")))?;
            Some((tone, parse_degree(p, "toneDegree")?))
        }
        (Kind::Colour(_), None | Some(Value::Null)) => None,
        (Kind::Colour(_), Some(Value::String(_))) => None,
        (Kind::Colour(_), Some(v)) => return Err(bad(format!("tone: want a string (got {v})"))),
        (Kind::Tone(_), _) => None,
    };
    let output = p.get("output").and_then(Value::as_str).unwrap_or("layerMask");
    if !matches!(output, "layerMask" | "channel" | "selection") {
        return Err(bad(format!("unknown output `{output}` (layerMask, channel, selection)")));
    }
    let operation = p.get("operation").and_then(Value::as_str).unwrap_or("new");
    if !matches!(operation, "new" | "replace" | "add" | "subtract" | "intersect") {
        return Err(bad(format!("unknown operation `{operation}`")));
    }
    let name = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);

    // Resolve the layers a layer mask goes to before any pixels are read.
    let targets = if output == "layerMask" { crate::layer_multi_cmds::mask_targets(s, p)? } else { Vec::new() };

    let (area, px) = crate::selection_cmds::sample_pixels(s, flag(p, "sampleAllLayers", true), None)?;
    if area.is_empty() || px.is_empty() {
        return Err(bad("the canvas is empty"));
    }
    let mask: Vec<f32> = match (kind, narrow) {
        (Kind::Tone(t), _) => luminosity::tone_mask(&px, t, degree),
        (Kind::Colour(c), tone) => luminosity::colour_mask(&px, c, tone),
    };
    drop(px);

    let label = match (kind, narrow) {
        (Kind::Tone(_), _) => format!("{} {degree}", title(&kind_name)),
        (Kind::Colour(_), Some((t, d))) => format!(
            "{} {} {d}",
            title(&kind_name),
            title(match t {
                Tone::Lights => "lights",
                Tone::Darks => "darks",
                Tone::Midtones => "midtones",
            })
        ),
        (Kind::Colour(_), None) => title(&kind_name),
    };

    match output {
        "layerMask" => {
            let count = targets.len();
            s.edit("Luminosity Mask", |doc, _| {
                let mut surface_mask = crate::commands::mask_at_depth(LayerMask::hide_all(), doc.depth)?;
                surface_mask.surface.write_region(area, &mask);
                surface_mask.surface.prune();
                for id in &targets {
                    crate::commands::check_all_unlocked(doc, *id)?;
                    crate::extra_cmds::background_to_layer_for_mask(doc, *id);
                    doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?.mask = Some(surface_mask.clone());
                }
                Ok(())
            })?;
            Ok(json!({ "output": "layerMask", "kind": kind_name, "degree": degree, "layers": count }))
        }
        "channel" => {
            let index = s.edit("Luminosity Mask", |doc, _| {
                let channel_name = name.clone().unwrap_or_else(|| unique_channel_name(doc, &label));
                let mut surface = Surface::new(crate::channel_cmds::channel_format(doc));
                surface.write_region(area, &mask);
                surface.prune();
                doc.channels.push(AlphaChannel::new(channel_name, surface));
                Ok(doc.channels.len() - 1)
            })?;
            let channel_name = s.active().and_then(|d| d.doc.channels.get(index)).map(|c| c.name.clone()).unwrap_or_default();
            Ok(json!({ "output": "channel", "kind": kind_name, "degree": degree, "channel": index, "name": channel_name }))
        }
        _ => {
            let mode = SelectionMode::parse(if operation == "new" { "replace" } else { operation });
            let selected = s.edit("Luminosity Mask", |doc, _| {
                doc.selection = sel::combine(doc.selection.as_ref(), &mask, area, mode);
                Ok(doc.selection.is_some())
            })?;
            Ok(json!({ "output": "selection", "kind": kind_name, "degree": degree, "selected": selected }))
        }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Luminosity Mask…",
        menu: &[],
        shortcut: None,
        params: r##"{"kind":"lights|darks|midtones|reds|yellows|greens|cyans|blues|magentas"="lights","degree":1..6=1,"tone":"lights|darks|midtones|none"="none" (colour kinds only),"toneDegree":1..6=1,"output":"layerMask|channel|selection"="layerMask" (layerMask replaces the layer's mask),"operation":"new|add|subtract|intersect"="new" (selection output),"name":str? (channel output),"layer":id|name?,"sampleAllLayers":bool=true}"##,
        enabled: has_doc,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests;
