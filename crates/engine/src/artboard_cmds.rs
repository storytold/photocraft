//! Artboards: Layer › New › Artboard (from scratch, a group or the selected layers), artboard
//! properties (board rect, background, preset), View › Clear Selected Artboard Guides, and
//! File › Export › Artboards to Files / to PDF.
//!
//! An artboard is a top-level group with [`photocraft_doc::Artboard`] set; the compositor clips
//! its children to the board and paints its background. Moving the group (Move tool,
//! `layer.translate`) moves the board with its contents. The canvas grows to fit every board,
//! in every direction, like Photoshop's auto-expanding artboard canvas ([`fit_canvas`]).

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::responsive::{
    Anchor, Dimensions, HorizontalAnchor, LayerLayoutRule, ResizeMode, StackAlignment, StackDirection, StackRule, TextFlow, VerticalAnchor,
};
use photocraft_doc::{Artboard, ArtboardBackground, Document, Layer, LayerContent, LayerId, Size};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;
use serde_json::{Value, json};

const MAX_ARTBOARD_DIAGNOSTICS: usize = 1_000;
const MAX_ARTBOARD_COLLISION_COMPARISONS: usize = 100_000;

use crate::commands::{CommandSpec, color_param, int, int_i32};
use crate::file_cmds::{SaveOpts, encode, f64_param, join, native_doc, sanitize, save_doc, stem, str_param, write_file};
use crate::{EngineError, Result, Session};

/// Artboard size presets (name, width, height) in pixels, Photoshop's common devices and pages.
pub const PRESETS: &[(&str, u32, u32)] = &[
    ("iPhone 14", 390, 844),
    ("iPhone 14 Pro Max", 430, 932),
    ("Android 1080p", 360, 640),
    ("iPad Pro 12.9", 1024, 1366),
    ("Web 1280", 1280, 800),
    ("Web 1366", 1366, 768),
    ("Web 1920", 1920, 1080),
    ("Instagram Post", 1080, 1080),
    ("Letter", 612, 792),
    ("A4", 595, 842),
];

/// Semantic layout roles stored in native PhotoCraft documents as namespaced metadata.
pub const SEMANTIC_ROLES: &[(&str, &str)] = &[
    ("auto", "Auto (suggested)"),
    ("background", "Background"),
    ("headline", "Headline"),
    ("body", "Body text"),
    ("footer", "Footer"),
    ("logo", "Logo"),
    ("cta", "Call to action"),
    ("badge", "Badge"),
    ("price", "Price"),
    ("disclaimer", "Disclaimer"),
    ("image_mark", "Image mark"),
    ("decoration", "Decoration"),
    ("free", "Free placement"),
];
const ROLE_META_PREFIX: &str = "photocraft.artboard-role.";
const ROLE_SOURCE_META_PREFIX: &str = "photocraft.artboard-role-source.";
const MAX_ROLE_BULK_TARGETS: usize = 10_000;

fn role_meta_key(id: LayerId) -> String {
    format!("{ROLE_META_PREFIX}{}", id.0)
}

fn role_source_meta_key(id: LayerId) -> String {
    format!("{ROLE_SOURCE_META_PREFIX}{}", id.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SemanticRoleSuggestion {
    pub role: &'static str,
    pub confidence: f32,
    pub evidence: &'static str,
}

pub fn explicit_semantic_role(doc: &Document, id: LayerId) -> Option<&'static str> {
    let value = doc.metadata.text.iter().rev().find(|(key, _)| key == &role_meta_key(id)).map(|(_, value)| value.as_str())?;
    SEMANTIC_ROLES.iter().find(|(role, _)| *role == value).map(|(role, _)| *role)
}

/// Explicit per-layer role, or a conservative suggestion based on editable layer data.
pub fn semantic_role(doc: &Document, id: LayerId) -> &'static str {
    explicit_semantic_role(doc, id).filter(|role| *role != "auto").unwrap_or_else(|| inferred_semantic_role(doc, id).role)
}

pub fn semantic_role_source(doc: &Document, id: LayerId) -> &'static str {
    if explicit_semantic_role(doc, id).is_none_or(|role| role == "auto") {
        return "suggestion";
    }
    match doc.metadata.text.iter().rev().find(|(key, _)| key == &role_source_meta_key(id)).map(|(_, value)| value.as_str()) {
        Some("file") => "file",
        Some("manual") => "manual",
        _ => "document",
    }
}

pub fn semantic_role_suggestion(doc: &Document, id: LayerId) -> SemanticRoleSuggestion {
    if let Some(role) = explicit_semantic_role(doc, id)
        && role != "auto"
    {
        return SemanticRoleSuggestion { role, confidence: 1.0, evidence: "Explicit role" };
    }
    inferred_semantic_role(doc, id)
}

fn inferred_semantic_role(doc: &Document, id: LayerId) -> SemanticRoleSuggestion {
    let Some(layer) = doc.layer(id) else { return SemanticRoleSuggestion { role: "free", confidence: 0.0, evidence: "Layer unavailable" } };
    let lower_name = layer.name.to_ascii_lowercase();
    match &layer.content {
        LayerContent::Text(text) => {
            let lower_text = text.text.to_ascii_lowercase();
            if ["logo", "brand", "mark"].iter().any(|needle| lower_name.contains(needle)) || (lower_text.contains("photocraft") && text.size_pt <= 24.0) {
                SemanticRoleSuggestion { role: "logo", confidence: 0.96, evidence: "Layer name or text" }
            } else if ["cta", "button", "action", "call to action"].iter().any(|needle| lower_name.contains(needle)) {
                SemanticRoleSuggestion { role: "cta", confidence: 0.96, evidence: "Layer name" }
            } else if lower_name.contains("footer") {
                SemanticRoleSuggestion { role: "footer", confidence: 0.94, evidence: "Layer name" }
            } else if ["title", "headline", "heading", "eyebrow"].iter().any(|needle| lower_name.contains(needle)) {
                SemanticRoleSuggestion { role: "headline", confidence: 0.96, evidence: "Layer name" }
            } else if text.size_pt >= 28.0 {
                SemanticRoleSuggestion { role: "headline", confidence: 0.72, evidence: "Font size" }
            } else {
                SemanticRoleSuggestion { role: "body", confidence: 0.58, evidence: "Text layer" }
            }
        }
        LayerContent::Shape(_) => {
            let bounds = crate::layer_multi_cmds::layer_bounds(layer);
            let board = doc.artboard_of(id).and_then(|board| doc.layer(board)).and_then(Layer::artboard).map(|a| a.rect);
            if let (Some(bounds), Some(board)) = (bounds, board)
                && i64::from(bounds.width()) * 10 >= i64::from(board.width()) * 9
                && i64::from(bounds.height()) * 10 >= i64::from(board.height()) * 9
            {
                SemanticRoleSuggestion { role: "background", confidence: 0.92, evidence: "Covers most of artboard" }
            } else {
                SemanticRoleSuggestion { role: "decoration", confidence: 0.52, evidence: "Shape layer" }
            }
        }
        _ if ["background", "bg"].iter().any(|needle| lower_name.contains(needle)) => {
            SemanticRoleSuggestion { role: "background", confidence: 0.88, evidence: "Layer name" }
        }
        _ => SemanticRoleSuggestion { role: "free", confidence: 0.24, evidence: "No strong role signal" },
    }
}

fn set_semantic_role(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.semanticRole.set";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = optional_layer_param(p, cmd)?.or(d.active_layer).ok_or_else(|| bad(cmd, "select a layer inside an artboard"))?;
    if d.doc.artboard_of(id).is_none() {
        return Err(bad(cmd, "semantic roles can only be assigned to layers inside an artboard"));
    }
    let role = p.get("role").and_then(Value::as_str).ok_or_else(|| bad(cmd, "`role` is required"))?;
    if !SEMANTIC_ROLES.iter().any(|(known, _)| *known == role) {
        return Err(bad(cmd, format!("unknown role `{role}`")));
    }
    let apply_to_similar = match p.get("applyToSimilar") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err(bad(cmd, "`applyToSimilar` must be a boolean")),
    };
    let board = d.doc.artboard_of(id).ok_or_else(|| bad(cmd, "the selected layer is not inside an artboard"))?;
    let source = d.doc.layer(id).ok_or_else(|| bad(cmd, "the selected layer no longer exists"))?;
    let source_kind = source.content.kind_name();
    let source_suggestion = inferred_semantic_role(&d.doc, id).role;
    let mut targets = vec![id];
    if apply_to_similar {
        for (_, _, candidate) in d.doc.walk() {
            if candidate.id != id
                && d.doc.artboard_of(candidate.id) == Some(board)
                && candidate.content.kind_name() == source_kind
                && inferred_semantic_role(&d.doc, candidate.id).role == source_suggestion
            {
                if targets.len() >= MAX_ROLE_BULK_TARGETS {
                    return Err(bad(cmd, format!("similar-layer role changes are capped at {MAX_ROLE_BULK_TARGETS} layers")));
                }
                targets.push(candidate.id);
            }
        }
    }
    s.edit("Set Artboard Semantic Role", |doc, _| {
        for target in &targets {
            let key = role_meta_key(*target);
            let source_key = role_source_meta_key(*target);
            doc.metadata.text.retain(|(k, _)| k != &key && k != &source_key);
            if role != "auto" {
                doc.metadata.text.push((key, role.to_owned()));
                doc.metadata.text.push((source_key, "manual".to_owned()));
            }
        }
        Ok(())
    })?;
    let role = s.active().map(|st| semantic_role(&st.doc, id)).unwrap_or("free");
    Ok(json!({"layer": id.0, "role": role, "updatedLayers": targets.iter().map(|target| target.0).collect::<Vec<_>>(), "updatedCount": targets.len()}))
}

fn set_layout_rule(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.layoutRule.set";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = optional_layer_param(p, cmd)?.or(d.active_layer).ok_or_else(|| bad(cmd, "select a layer inside an artboard"))?;
    let Some(board_id) = d.doc.artboard_of(id) else {
        return Err(bad(cmd, "layout rules can only be assigned to layers inside an artboard"));
    };
    let variant_id = d.doc.responsive.variants.iter().any(|variant| variant.artboard == board_id).then_some(board_id);
    let parse_size = |name: &str| -> Result<Option<Dimensions>> {
        let Some(value) = p.get(name) else { return Ok(None) };
        if value.is_null() {
            return Ok(None);
        }
        let width = value
            .get("width")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(|| bad(cmd, format!("`{name}.width` must be a positive 32-bit integer")))?;
        let height = value
            .get("height")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(|| bad(cmd, format!("`{name}.height` must be a positive 32-bit integer")))?;
        Ok(Some(Dimensions { width, height }))
    };
    let choice = |key: &str, default: &str, choices: &[&str]| -> Result<String> {
        let value = match p.get(key) {
            None => default,
            Some(Value::String(value)) => value.as_str(),
            Some(_) => return Err(bad(cmd, format!("`{key}` must be a string"))),
        };
        if choices.contains(&value) { Ok(value.to_owned()) } else { Err(bad(cmd, format!("`{key}` must be one of {}", choices.join(", ")))) }
    };
    let horizontal = match choice("horizontal", "left", &["left", "center", "right"])?.as_str() {
        "center" => HorizontalAnchor::Center,
        "right" => HorizontalAnchor::Right,
        _ => HorizontalAnchor::Left,
    };
    let vertical = match choice("vertical", "top", &["top", "center", "bottom"])?.as_str() {
        "center" => VerticalAnchor::Center,
        "bottom" => VerticalAnchor::Bottom,
        _ => VerticalAnchor::Top,
    };
    let mode = match choice("mode", "fixed", &["fixed", "stretch", "fill", "fit"])?.as_str() {
        "stretch" => ResizeMode::Stretch,
        "fill" => ResizeMode::Fill,
        "fit" => ResizeMode::Fit,
        _ => ResizeMode::Fixed,
    };
    let text_flow = match choice("textFlow", "preserve", &["preserve", "rewrap", "rewrap_auto_height", "rewrap_and_shrink"])?.as_str() {
        "rewrap" => TextFlow::Rewrap,
        "rewrap_auto_height" => TextFlow::RewrapAutoHeight,
        "rewrap_and_shrink" => TextFlow::RewrapAndShrink,
        _ => TextFlow::Preserve,
    };
    let min_font_size_pt = match p.get("minFontSizePt") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|v| v.is_finite() && *v > 0.0 && *v <= 1000.0)
                .ok_or_else(|| bad(cmd, "`minFontSizePt` must be finite and between 0 and 1000"))? as f32,
        ),
    };
    let stack = if let Some(value) = p.get("stack") {
        let direction = match value.get("direction").and_then(Value::as_str).unwrap_or("vertical") {
            "vertical" => StackDirection::Vertical,
            "horizontal" => StackDirection::Horizontal,
            _ => return Err(bad(cmd, "`stack.direction` must be vertical or horizontal")),
        };
        let alignment = match value.get("alignment").and_then(Value::as_str).unwrap_or("start") {
            "start" => StackAlignment::Start,
            "center" => StackAlignment::Center,
            "end" => StackAlignment::End,
            "stretch" => StackAlignment::Stretch,
            _ => return Err(bad(cmd, "`stack.alignment` must be start, center, end, or stretch")),
        };
        let gap = value
            .get("gap")
            .and_then(Value::as_f64)
            .filter(|gap| gap.is_finite() && (0.0..=1_000_000.0).contains(gap))
            .ok_or_else(|| bad(cmd, "`stack.gap` must be finite and between 0 and 1,000,000"))? as f32;
        Some(StackRule { direction, alignment, gap })
    } else {
        None
    };
    let rule = LayerLayoutRule {
        layer: id,
        anchor: Anchor { horizontal, vertical },
        mode,
        min_size: parse_size("minSize")?,
        max_size: parse_size("maxSize")?,
        text_flow,
        min_font_size_pt,
        stack,
    };
    if let (Some(min), Some(max)) = (rule.min_size, rule.max_size)
        && (min.width > max.width || min.height > max.height)
    {
        return Err(bad(cmd, "`maxSize` must be at least `minSize`"));
    }
    s.edit("Set Responsive Layer Rule", |doc, _| {
        if let Some(variant) = variant_id {
            doc.responsive.overrides.retain(|existing| existing.variant != variant || existing.layer != id);
            doc.responsive.overrides.push(photocraft_doc::responsive::LayerRuleOverride { variant, layer: id, rule: rule.clone() });
        } else {
            doc.responsive.rules.retain(|existing| existing.layer != id);
            doc.responsive.rules.push(rule.clone());
        }
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "rule": rule}))
}

fn set_safe_area(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.safeArea.set";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let board = optional_layer_param(p, cmd)?.or_else(|| active_artboard(s)).ok_or_else(|| bad(cmd, "select an artboard"))?;
    if d.doc.layer(board).and_then(Layer::artboard).is_none() {
        return Err(bad(cmd, "the selected layer is not an artboard"));
    }
    let name = p
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= 256)
        .ok_or_else(|| bad(cmd, "`name` must contain 1 to 256 bytes"))?
        .to_owned();
    let inset = |key: &str| -> Result<f32> {
        let value = match p.get(key) {
            None => 0.0,
            Some(value) => value.as_f64().ok_or_else(|| bad(cmd, format!("`{key}` must be a number")))?,
        };
        if value.is_finite() && (0.0..=0.5).contains(&value) { Ok(value as f32) } else { Err(bad(cmd, format!("`{key}` must be between 0 and 0.5"))) }
    };
    let mut required_roles = Vec::new();
    if let Some(roles) = p.get("requiredRoles") {
        let Some(values) = roles.as_array() else {
            return Err(bad(cmd, "`requiredRoles` must be an array of role names"));
        };
        for role in values {
            let Some(role) = role.as_str() else {
                return Err(bad(cmd, "every required role must be a string"));
            };
            if role.len() > 64 {
                return Err(bad(cmd, "required role names are limited to 64 bytes"));
            }
            required_roles.push(role.to_owned());
        }
        required_roles.sort();
        required_roles.dedup();
        if required_roles.len() > 64 {
            return Err(bad(cmd, "at most 64 required roles are supported"));
        }
    }
    let area = photocraft_doc::responsive::ArtboardSafeArea {
        artboard: board,
        name: name.clone(),
        insets: photocraft_doc::responsive::SafeInsets { top: inset("top")?, right: inset("right")?, bottom: inset("bottom")?, left: inset("left")? },
        required_roles,
    };
    s.edit("Set Artboard Safe Area", |doc, _| {
        if let Some(existing) = doc.responsive.safe_areas.iter_mut().find(|existing| existing.artboard == board && existing.name == name) {
            *existing = area.clone();
        } else {
            if doc.responsive.safe_areas.len() >= photocraft_doc::responsive::MAX_RESPONSIVE_SAFE_AREAS {
                return Err(bad(cmd, "this document already has the maximum of 65 safe areas"));
            }
            doc.responsive.safe_areas.push(area.clone());
        }
        Ok(())
    })?;
    Ok(
        json!({"artboard":board.0,"name":name,"insets":{"top":area.insets.top,"right":area.insets.right,"bottom":area.insets.bottom,"left":area.insets.left},"requiredRoles":area.required_roles}),
    )
}

fn delete_safe_area(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.safeArea.delete";
    let board = optional_layer_param(p, cmd)?.or_else(|| active_artboard(s)).ok_or_else(|| bad(cmd, "select an artboard"))?;
    let name = p
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= 256)
        .ok_or_else(|| bad(cmd, "`name` must contain 1 to 256 bytes"))?
        .to_owned();
    let removed = s.edit("Delete Artboard Safe Area", |doc, _| {
        let previous = doc.responsive.safe_areas.len();
        doc.responsive.safe_areas.retain(|area| area.artboard != board || area.name != name);
        if doc.responsive.safe_areas.len() == previous {
            return Err(bad(cmd, "the named safe area does not exist on this artboard"));
        }
        Ok(previous - doc.responsive.safe_areas.len())
    })?;
    Ok(json!({"artboard":board.0,"name":name,"deleted":removed}))
}

fn diagnose_artboard(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.diagnose";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = optional_layer_param(p, cmd)?.or_else(|| active_artboard(s)).ok_or_else(|| bad(cmd, "select an artboard"))?;
    diagnose_document_artboard(&d.doc, id, d.revision)
}

pub fn diagnose_document_artboard(doc: &Document, id: LayerId, revision: u64) -> Result<Value> {
    let board = doc.layer(id).and_then(Layer::artboard).ok_or_else(|| bad("layer.artboard.diagnose", "the selected layer is not an artboard"))?;
    let rect = board.rect;
    let mut overflow = Vec::new();
    let mut semantic: Vec<(LayerId, &'static str, Rect)> = Vec::new();
    let mut truncated = false;
    for (path, _, layer) in doc.walk() {
        if layer.id == id || doc.artboard_of(layer.id) != Some(id) || !path_is_visible(doc, &path) {
            continue;
        }
        let Some(bounds) = crate::layer_multi_cmds::layer_bounds(layer) else { continue };
        let role = semantic_role(doc, layer.id);
        let left = (i64::from(rect.x0) - i64::from(bounds.x0)).max(0);
        let top = (i64::from(rect.y0) - i64::from(bounds.y0)).max(0);
        let right = (i64::from(bounds.x1) - i64::from(rect.x1)).max(0);
        let bottom = (i64::from(bounds.y1) - i64::from(rect.y1)).max(0);
        if left + top + right + bottom > 0 {
            if overflow.len() < MAX_ARTBOARD_DIAGNOSTICS {
                overflow.push(json!({"layer":layer.id.0,"role":role,"left":left,"top":top,"right":right,"bottom":bottom}));
            } else {
                truncated = true;
            }
        }
        if matches!(role, "headline" | "body" | "logo" | "cta" | "badge" | "price" | "disclaimer" | "image_mark") {
            semantic.push((layer.id, role, bounds));
        }
    }
    semantic.sort_unstable_by_key(|entry| entry.2.x0);
    let mut collisions = Vec::new();
    let mut comparisons = 0usize;
    'pairs: for i in 0..semantic.len() {
        for other in semantic.iter().skip(i + 1).take_while(|entry| entry.2.x0 < semantic[i].2.x1) {
            if comparisons >= MAX_ARTBOARD_COLLISION_COMPARISONS {
                truncated = true;
                break 'pairs;
            }
            comparisons += 1;
            if !semantic[i].2.intersect(&other.2).is_empty() {
                if collisions.len() < MAX_ARTBOARD_DIAGNOSTICS {
                    collisions.push(json!({"first":semantic[i].0.0,"firstRole":semantic[i].1,"second":other.0.0,"secondRole":other.1}));
                } else {
                    truncated = true;
                    break 'pairs;
                }
            }
        }
    }
    let safe_areas: Vec<Value> = doc
        .responsive
        .safe_areas
        .iter()
        .filter(|area| area.artboard == id)
        .map(|area| {
            let inset = area.insets;
            let x0 = f64::from(rect.x0) + f64::from(rect.width()) * f64::from(inset.left);
            let x1 = f64::from(rect.x1) - f64::from(rect.width()) * f64::from(inset.right);
            let y0 = f64::from(rect.y0) + f64::from(rect.height()) * f64::from(inset.top);
            let y1 = f64::from(rect.y1) - f64::from(rect.height()) * f64::from(inset.bottom);
            let mut violations = Vec::new();
            for (layer, role, bounds) in &semantic {
                if area.required_roles.iter().any(|required| required == role)
                    && (f64::from(bounds.x0) < x0 || f64::from(bounds.y0) < y0 || f64::from(bounds.x1) > x1 || f64::from(bounds.y1) > y1)
                {
                    if violations.len() < MAX_ARTBOARD_DIAGNOSTICS {
                        violations.push(json!({"layer":layer.0,"role":role}));
                    } else {
                        truncated = true;
                        break;
                    }
                }
            }
            json!({"name":area.name,"violations":violations})
        })
        .collect();
    Ok(json!({"artboard":id.0,"revision":revision,"overflow":overflow,"collisions":collisions,"safeAreas":safe_areas,"truncated":truncated}))
}

/// Visibility is inherited through groups. Diagnostics describe rendered content, so a hidden
/// layer or any layer below a hidden group must not create overflow, collision, or safe-area findings.
fn path_is_visible(doc: &Document, path: &[usize]) -> bool {
    let mut siblings = doc.layers.as_slice();
    for (depth, index) in path.iter().enumerate() {
        let Some(layer) = siblings.get(*index) else { return false };
        if !layer.visible {
            return false;
        }
        if depth + 1 < path.len() {
            let Some(children) = layer.children() else { return false };
            siblings = children;
        }
    }
    !path.is_empty()
}

fn set_responsive_master(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.master.set";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = optional_layer_param(p, cmd)?.or_else(|| active_artboard(s)).ok_or_else(|| bad(cmd, "select an artboard"))?;
    if d.doc.layer(id).and_then(Layer::artboard).is_none() {
        return Err(bad(cmd, "the selected layer is not an artboard"));
    }
    s.edit("Set Responsive Master Artboard", |doc, _| {
        if let Some(selected) = doc.responsive.variants.iter().find(|variant| variant.artboard == id).cloned() {
            let old_master = doc.responsive.master.unwrap_or(selected.master);
            let inverse: std::collections::HashMap<_, _> = selected.layers.iter().map(|mapping| (mapping.master_layer, mapping.variant_layer)).collect();
            let mut rules = Vec::new();
            for mut rule in std::mem::take(&mut doc.responsive.rules) {
                if let Some(layer) = inverse.get(&rule.layer) {
                    rule.layer = *layer;
                    rules.push(rule);
                }
            }
            for mut entry in doc.responsive.overrides.iter().filter(|entry| entry.variant == id).cloned() {
                entry.rule.layer = entry.layer;
                rules.retain(|rule| rule.layer != entry.layer);
                rules.push(entry.rule);
            }
            doc.responsive.overrides.retain(|entry| entry.variant != id);
            for variant in &mut doc.responsive.variants {
                if variant.artboard == id {
                    continue;
                }
                if variant.master == old_master {
                    variant.master = id;
                    for mapping in &mut variant.layers {
                        if let Some(new_master_layer) = inverse.get(&mapping.master_layer) {
                            mapping.master_layer = *new_master_layer;
                        }
                    }
                }
            }
            doc.responsive.variants.retain(|variant| variant.artboard != id);
            doc.responsive.rules = rules;
        } else {
            let old_master = doc.responsive.master;
            if let Some(old_master) = old_master.filter(|old| *old != id)
                && let (Some(old_layer), Some(new_layer)) = (doc.layer(old_master).cloned(), doc.layer(id).cloned())
            {
                let mut pairs = Vec::new();
                collect_layer_pairs(&old_layer, &new_layer, &mut pairs);
                let layer_map: std::collections::HashMap<_, _> = pairs.into_iter().collect();
                doc.responsive.rules.retain_mut(|rule| {
                    let Some(layer) = layer_map.get(&rule.layer) else { return false };
                    rule.layer = *layer;
                    true
                });
                for variant in &mut doc.responsive.variants {
                    variant.master = id;
                    variant.layers.retain_mut(|mapping| {
                        let Some(master_layer) = layer_map.get(&mapping.master_layer) else { return false };
                        mapping.master_layer = *master_layer;
                        true
                    });
                }
            } else {
                for variant in &mut doc.responsive.variants {
                    variant.master = id;
                }
            }
        }
        doc.responsive.master = Some(id);
        Ok(())
    })?;
    Ok(json!({"master": id.0}))
}

fn delete_variant(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.variant.delete";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = optional_layer_param(p, cmd)?.or_else(|| active_artboard(s)).ok_or_else(|| bad(cmd, "select a variant artboard"))?;
    if !d.doc.responsive.variants.iter().any(|variant| variant.artboard == id) {
        return Err(bad(cmd, "the selected artboard is not a responsive variant"));
    }
    let neighbors = crate::layer_multi_cmds::deletion_neighbours(&d.doc, id);
    let next = s.edit("Delete Responsive Variant", |doc, active| {
        doc.remove(id).ok_or(EngineError::NoLayer(id))?;
        if doc.layers.is_empty() {
            return Err(EngineError::Other("a document must keep at least one layer".into()));
        }
        fit_canvas(doc);
        let next = neighbors.iter().copied().find(|candidate| doc.layer(*candidate).is_some()).or_else(|| doc.layers.last().map(|layer| layer.id));
        *active = next;
        Ok(next)
    })?;
    if let Some(next) = next {
        crate::layer_multi_cmds::set_selection(s, vec![next], Some(next), Some(next))?;
    }
    Ok(json!({"deleted":id.0,"selected":next.map(|id|id.0)}))
}

fn rename_variant(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.variant.rename";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = optional_layer_param(p, cmd)?.or_else(|| active_artboard(s)).ok_or_else(|| bad(cmd, "select a variant artboard"))?;
    let name = p
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= 256)
        .ok_or_else(|| bad(cmd, "`name` must contain 1 to 256 UTF-8 bytes"))?
        .to_owned();
    if !d.doc.responsive.variants.iter().any(|variant| variant.artboard == id) {
        return Err(bad(cmd, "the selected artboard is not a responsive variant"));
    }
    s.edit("Rename Responsive Variant", |doc, _| {
        let layer = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        layer.name.clone_from(&name);
        let variant = doc.responsive.variants.iter_mut().find(|variant| variant.artboard == id).ok_or_else(|| bad(cmd, "variant no longer exists"))?;
        variant.name.clone_from(&name);
        Ok(())
    })?;
    Ok(json!({"layer":id.0,"name":name}))
}

fn reset_variant_rule(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.layoutRule.reset";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let layer = optional_layer_param(p, cmd)?.or(d.active_layer).ok_or_else(|| bad(cmd, "select a layer inside a variant artboard"))?;
    let board = d.doc.artboard_of(layer).ok_or_else(|| bad(cmd, "select a layer inside a variant artboard"))?;
    let variant = d
        .doc
        .responsive
        .variants
        .iter()
        .find(|variant| variant.artboard == board)
        .ok_or_else(|| bad(cmd, "the selected artboard is not a responsive variant"))?;
    let master_layer = variant
        .layers
        .iter()
        .find(|mapping| mapping.variant_layer == layer)
        .map(|mapping| mapping.master_layer)
        .ok_or_else(|| bad(cmd, "this layer has no corresponding master layer"))?;
    if !d.doc.responsive.overrides.iter().any(|entry| entry.variant == board && entry.layer == layer) {
        return Err(bad(cmd, "this layer has no variant override to reset"));
    }
    let inherited = d.doc.responsive.rules.iter().find(|rule| rule.layer == master_layer).cloned();
    s.edit("Reset Variant Layout Override", |doc, _| {
        doc.responsive.overrides.retain(|entry| entry.variant != board || entry.layer != layer);
        Ok(())
    })?;
    Ok(json!({"layer":layer.0,"masterLayer":master_layer.0,"inheritedRule":inherited}))
}

/// Gap Photoshop leaves between a new artboard and the rightmost existing one.
const GAP: i32 = 100;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn optional_layer_param(p: &Value, cmd: &str) -> Result<Option<LayerId>> {
    match p.get("layer") {
        None => Ok(None),
        Some(value) => value.as_u64().map(LayerId).map(Some).ok_or_else(|| bad(cmd, "`layer` must be an unsigned 64-bit layer ID")),
    }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_artboards(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.has_artboards() { Ok(()) } else { Err("the document has no artboards".into()) }
}

fn export_artboards(s: &Session) -> std::result::Result<(), String> {
    native_doc(s)?;
    has_artboards(s)
}

/// The active layer's artboard (the layer itself or its top-level ancestor).
fn active_artboard(s: &Session) -> Option<LayerId> {
    let d = s.active()?;
    d.doc.artboard_of(d.active_layer?)
}

fn cycle_responsive_variant(s: &mut Session, direction: isize) -> Result<Value> {
    let command_id = if direction < 0 { "layer.artboard.variant.previous" } else { "layer.artboard.variant.next" };
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let active = state.active_layer.ok_or_else(|| bad(command_id, "select an artboard or a layer on one"))?;
    let selected_board = state.doc.artboard_of(active).ok_or_else(|| bad(command_id, "the selection is not inside an artboard"))?;
    let master = state.doc.responsive.variants.iter().find(|variant| variant.artboard == selected_board).map_or(selected_board, |variant| variant.master);
    if state.doc.layer(master).and_then(Layer::artboard).is_none() {
        return Err(bad(command_id, "the responsive master is missing"));
    }
    let mut boards = vec![master];
    boards.extend(
        state
            .doc
            .responsive
            .variants
            .iter()
            .filter(|variant| variant.master == master && state.doc.layer(variant.artboard).and_then(Layer::artboard).is_some())
            .map(|variant| variant.artboard),
    );
    let Some(current) = boards.iter().position(|board| *board == selected_board) else {
        return Err(bad(command_id, "the selected artboard is not part of the master's variant set"));
    };
    if boards.len() < 2 {
        return Err(bad(command_id, "the master has no responsive variants"));
    }
    let next = if direction < 0 { (current + boards.len() - 1) % boards.len() } else { (current + 1) % boards.len() };
    let target = boards[next];
    crate::layer_multi_cmds::select(s, &json!({"layer": target.0}))?;
    Ok(json!({"layer":target.0,"master":master.0,"index":next,"count":boards.len()}))
}

fn next_responsive_variant(s: &mut Session, _: &Value) -> Result<Value> {
    cycle_responsive_variant(s, 1)
}

fn previous_responsive_variant(s: &mut Session, _: &Value) -> Result<Value> {
    cycle_responsive_variant(s, -1)
}

fn has_active_artboard(s: &Session) -> std::result::Result<(), String> {
    active_artboard(s).map(|_| ()).ok_or_else(|| "select an artboard (or a layer inside one)".into())
}

fn has_plain_group(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    let l = d.doc.layer(id).ok_or("no active layer")?;
    if !l.is_group() || l.artboard().is_some() {
        return Err("select a layer group".into());
    }
    if !d.doc.layers.iter().any(|t| t.id == id) {
        return Err("artboards must be top-level: select a group that is not nested".into());
    }
    Ok(())
}

fn has_free_layers(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let sel = d.selected_layers();
    if sel.is_empty() {
        return Err("select one or more layers".into());
    }
    if sel.iter().any(|id| d.doc.artboard_of(*id).is_some()) {
        return Err("the selection is already in an artboard".into());
    }
    Ok(())
}

fn preset_size(name: &str) -> Option<(u32, u32)> {
    PRESETS.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(name)).map(|(_, w, h)| (*w, *h))
}

fn background_param(p: &Value, cmd: &str) -> Result<Option<ArtboardBackground>> {
    let custom = p.get("color").map(|_| {
        let c = color_param(p, "color", [1.0; 4]);
        ArtboardBackground::Custom(Color::rgba(c[0], c[1], c[2], c[3]))
    });
    Ok(match p.get("background").and_then(Value::as_str) {
        None => custom,
        Some("white") => Some(ArtboardBackground::White),
        Some("black") => Some(ArtboardBackground::Black),
        Some("transparent") => Some(ArtboardBackground::Transparent),
        Some("custom" | "other") => Some(custom.unwrap_or(ArtboardBackground::White)),
        Some(o) => return Err(bad(cmd, format!("unknown background `{o}` (white|black|transparent|custom)"))),
    })
}

/// A board rect from `x, y, width, height`, or a bad-params error naming them when an edge falls
/// outside the 32-bit coordinate range. `Rect::from_xywh` would clamp the far edge to `i32::MAX`
/// (silently shrinking the board) and wrap a width past `i32::MAX` negative (#931).
fn board_rect(cmd: &str, [x, y, w, h]: [i64; 4]) -> Result<Rect> {
    if w <= 0 || h <= 0 {
        return Err(bad(cmd, format!("the artboard size must be positive (width = {w}, height = {h})")));
    }
    let span = |a: i64, len: i64| Some((i32::try_from(a).ok()?, i32::try_from(a.checked_add(len)?).ok()?));
    match (span(x, w), span(y, h)) {
        (Some((x0, x1)), Some((y0, y1))) => Ok(Rect::new(x0, y0, x1, y1)),
        _ => Err(bad(
            cmd,
            format!("the artboard [x, y, width, height] = [{x}, {y}, {w}, {h}] reaches outside the 32-bit coordinate range ({}..={})", i32::MIN, i32::MAX),
        )),
    }
}

/// `rect` as `[x, y, w, h]` or `x`/`y`/`width`/`height` keys over `base` (`[x, y, w, h]`, wide so
/// a default position past the edge reaches [`board_rect`] instead of overflowing).
fn rect_param(p: &Value, base: [i64; 4], cmd: &str) -> Result<Rect> {
    if let Some(Value::Array(a)) = p.get("rect") {
        let v: Vec<i64> = a.iter().filter_map(|x| x.as_i64().or_else(|| x.as_f64().filter(|f| f.is_finite()).map(|f| f.round() as i64))).collect();
        let Ok(v) = <[i64; 4]>::try_from(v) else {
            return Err(bad(cmd, "\"rect\" must be [x, y, width, height] with a positive size"));
        };
        return board_rect(cmd, v);
    }
    let x = int_i32(cmd, p, "x")?.map_or(base[0], i64::from);
    let y = int_i32(cmd, p, "y")?.map_or(base[1], i64::from);
    let w = int(p, "width").unwrap_or(base[2]);
    let h = int(p, "height").unwrap_or(base[3]);
    board_rect(cmd, [x, y, w, h])
}

/// `r` as `[x, y, w, h]` for [`rect_param`].
fn xywh(r: Rect) -> [i64; 4] {
    [i64::from(r.x0), i64::from(r.y0), i64::from(r.width()), i64::from(r.height())]
}

/// Layer › Duplicate on an artboard (#1531): Photoshop never stacks the copy on the original,
/// it puts it right of the board, a [`GAP`] away, sliding further right past any board in the
/// way. Moves the copy, layer `id` (just inserted on top of its original), there with its
/// contents and grows the canvas to show it; anything that isn't an artboard is left alone.
pub(crate) fn place_copy(doc: &mut Document, id: LayerId) -> Result<()> {
    let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let Some(r) = l.artboard().map(|a| a.rect) else { return Ok(()) };
    let name = l.name.clone();
    let boards: Vec<Rect> = doc.artboards().iter().filter(|b| b.0 != id).map(|b| b.2.rect).collect();
    let (y0, y1, w) = (i64::from(r.y0), i64::from(r.y1), i64::from(r.width()));
    // In i64: `x1 + GAP` passes i32::MAX for a board at the far right edge (#931).
    let mut x = i64::from(r.x1) + i64::from(GAP);
    // Each round clears every board in the way, so one round per board settles it.
    for _ in 0..=boards.len() {
        let blocked =
            boards.iter().filter(|b| i64::from(b.x0) < x + w && i64::from(b.x1) > x && i64::from(b.y0) < y1 && i64::from(b.y1) > y0).map(|b| b.x1).max();
        match blocked {
            Some(edge) => x = i64::from(edge) + i64::from(GAP),
            None => break,
        }
    }
    let fits = x.checked_add(w).is_some_and(|x1| x1 <= i64::from(i32::MAX));
    let dx = i32::try_from(x - i64::from(r.x0)).ok().filter(|_| fits).ok_or_else(|| {
        EngineError::Other(format!("no room for a copy of artboard \"{name}\" right of it; duplicate it in place (\"inPlace\": true) and move it"))
    })?;
    // Grow the canvas first, so shapes on the board re-render uncut where they land.
    let x1 = i64::from(doc.size.width).max(x + w);
    doc.size = Size::new(x1.clamp(1, 300_000) as u32, doc.size.height);
    let snapshot = doc.clone();
    let copy = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    crate::commands::translate_layer(&snapshot, copy, dx, 0);
    crate::vector_cmds::translate_vectors(&snapshot, copy, f64::from(dx), 0.0);
    fit_canvas(doc);
    Ok(())
}

/// Grow the canvas so every artboard fits. Photoshop's artboard documents grow in every
/// direction: a board left of or above the origin shifts the whole document (boards, their
/// contents, guides, paths…) right/down by its overhang, then the canvas grows to show it. An
/// axis whose boards span more than the largest canvas, or a shift that would push pixels past
/// the 32-bit range, stays unshifted: the board stays partly off-canvas, as one past the size
/// limit does.
pub(crate) fn fit_canvas(doc: &mut Document) {
    let max = i64::from(crate::document_preset_cmds::MAX_DIMENSION);
    let boards: Vec<Rect> = doc.artboards().iter().map(|b| b.2.rect).collect();
    // The canvas and every board, per axis (`lo` ≤ 0 is the canvas origin or a board past it).
    let span = |edges: &dyn Fn(&Rect) -> (i32, i32), len: u32| {
        boards.iter().map(edges).fold((0, i64::from(len)), |(lo, hi), (a, b)| (lo.min(i64::from(a)), hi.max(i64::from(b))))
    };
    let (x0, x1) = span(&|r| (r.x0, r.x1), doc.size.width);
    let (y0, y1) = span(&|r| (r.y0, r.y1), doc.size.height);
    let shift = |lo: i64, hi: i64| if hi - lo <= max { i32::try_from(-lo).unwrap_or(0) } else { 0 };
    let (mut sx, mut sy) = (shift(x0, x1), shift(y0, y1));
    // `translate_doc` checks before it moves anything, so a refusal leaves the document as is.
    if (sx, sy) != (0, 0) && crate::image_cmds::translate_doc(doc, "artboard", sx, sy).is_err() {
        (sx, sy) = (0, 0);
    }
    let size = Size::new((x1 + i64::from(sx)).clamp(1, max) as u32, (y1 + i64::from(sy)).clamp(1, max) as u32);
    if size != doc.size {
        doc.size = size;
        // Shape pixels are cut at the canvas: a board moved past the old edge rendered its
        // shapes against it, leaving them missing or clipped (#2389).
        crate::canvas_geom::refresh(doc, crate::canvas_geom::Refresh::Shapes);
    }
}

fn next_artboard_name(doc: &Document) -> String {
    let names: Vec<&str> = doc.artboards().into_iter().map(|b| b.1).collect();
    // `names.len() + 1` candidates always include a free one.
    (1..=names.len() + 1).map(|n| format!("Artboard {n}")).find(|n| !names.contains(&n.as_str())).unwrap_or_else(|| "Artboard".into())
}

fn set_artboard(l: &mut Layer, a: Artboard) {
    if let LayerContent::Group(g) = &mut l.content {
        g.artboard = Some(a);
        g.expanded = true;
    }
    // Artboards composite like pass-through groups.
    l.clipped = false;
}

fn new_artboard(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.new.artboard";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let preset = p.get("preset").and_then(Value::as_str).unwrap_or("").to_string();
    let (pw, ph) = match preset.as_str() {
        "" => (d.doc.size.width, d.doc.size.height),
        name => preset_size(name).ok_or_else(|| bad(cmd, format!("unknown preset `{name}`")))?,
    };
    // Default placement: right of the rightmost artboard, else the canvas origin.
    let right = d.doc.artboards().iter().map(|b| b.2.rect.x1).max();
    let top = d.doc.artboards().iter().map(|b| b.2.rect.y0).min().unwrap_or(0);
    // In i64: `x1 + GAP` passes i32::MAX for a board at the far right edge (#931).
    let base = match right {
        Some(x) => [i64::from(x) + i64::from(GAP), i64::from(top), i64::from(pw), i64::from(ph)],
        None => [0, 0, i64::from(pw), i64::from(ph)],
    };
    // With no room right of the rightmost board, ask for a position rather than pick another
    // spot that may overlap a board; an explicit `x` (or `rect`) still works.
    let explicit_x = p.get("rect").is_some() || p.get("x").is_some_and(|v| !v.is_null());
    if let Some(edge) = right.filter(|_| !explicit_x && base[0] > i64::from(i32::MAX)) {
        return Err(bad(cmd, format!("no room for a new artboard right of the rightmost one (its right edge is x = {edge}); pass \"x\" to place it")));
    }
    // A preset decides the size (dialogs send the canvas size alongside it).
    let rect = if preset.is_empty() || p.get("rect").is_some() {
        rect_param(p, base, cmd)?
    } else {
        rect_param(&json!({"x": p.get("x"), "y": p.get("y")}), base, cmd)?
    };
    let background = background_param(p, cmd)?.unwrap_or_default();
    let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
    let id = s.edit("New Artboard", |doc, active| {
        let mut g = Layer::group(name.unwrap_or_else(|| next_artboard_name(doc)), vec![]);
        set_artboard(&mut g, Artboard { rect, background, preset });
        let id = doc.insert_above(None, g);
        fit_canvas(doc);
        *active = Some(id);
        Ok(id)
    })?;
    crate::layer_multi_cmds::set_selection(s, vec![id], Some(id), Some(id))?;
    Ok(json!({"layer": id.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

fn artboard_from_group(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = p.get("layer").and_then(Value::as_u64).map(LayerId).or(d.active_layer).ok_or(EngineError::Other("no active layer".into()))?;
    let canvas = d.doc.bounds();
    let background = background_param(p, "layer.new.artboardFromGroup")?.unwrap_or_default();
    let rect = s.edit("Artboard from Group", |doc, active| {
        if !doc.layers.iter().any(|t| t.id == id && t.is_group() && t.artboard().is_none()) {
            return Err(EngineError::Other("artboards are made from a top-level layer group".into()));
        }
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let rect = crate::layer_multi_cmds::layer_bounds(l).unwrap_or(canvas);
        set_artboard(l, Artboard { rect, background, preset: String::new() });
        fit_canvas(doc);
        *active = Some(id);
        Ok(rect)
    })?;
    Ok(json!({"layer": id.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

fn artboard_from_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let sel = crate::layer_multi_cmds::selected(s);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let background = background_param(p, "layer.new.artboardFromLayers")?.unwrap_or_default();
    let (gid, rect) = s.edit("Artboard from Layers", |doc, active| {
        let ids = crate::layer_multi_cmds::top_level(doc, &sel);
        if ids.is_empty() || ids.iter().any(|id| doc.artboard_of(*id).is_some()) {
            return Err(EngineError::Other("select layers that are not in an artboard".into()));
        }
        let mut children = Vec::with_capacity(ids.len());
        for id in &ids {
            children.push(doc.remove(*id).ok_or(EngineError::NoLayer(*id))?);
        }
        let rect = children.iter().filter_map(crate::layer_multi_cmds::layer_bounds).reduce(|a, b| a.union(&b)).unwrap_or(canvas);
        let mut g = Layer::group(name.unwrap_or_else(|| next_artboard_name(doc)), children);
        set_artboard(&mut g, Artboard { rect, background, preset: String::new() });
        let gid = doc.insert_above(None, g);
        crate::layer_multi_cmds::check_group_depth(doc, "Artboard from Layers")?;
        fit_canvas(doc);
        *active = Some(gid);
        Ok((gid, rect))
    })?;
    crate::layer_multi_cmds::set_selection(s, vec![gid], Some(gid), Some(gid))?;
    Ok(json!({"layer": gid.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

/// Properties panel / Artboard tool: board position and size, background, preset, name.
/// Changing X/Y moves the board with its contents; changing W/H resizes it from the top left.
fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.set";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = match p.get("layer").and_then(Value::as_u64) {
        Some(v) => LayerId(v),
        None => active_artboard(s).ok_or_else(|| bad(cmd, "no artboard selected"))?,
    };
    let old = d.doc.layer(id).and_then(Layer::artboard).cloned().ok_or_else(|| bad(cmd, "the layer is not an artboard"))?;
    let preset = p.get("preset").and_then(Value::as_str).map(str::to_string);
    let mut base = xywh(old.rect);
    if let Some(name) = preset.as_deref().filter(|n| !n.is_empty()) {
        let (w, h) = preset_size(name).ok_or_else(|| bad(cmd, format!("unknown preset `{name}`")))?;
        (base[2], base[3]) = (i64::from(w), i64::from(h));
    }
    let rect = rect_param(p, base, cmd)?;
    let background = background_param(p, cmd)?;
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let move_contents = p.get("moveContents").and_then(Value::as_bool).unwrap_or(true);
    // The offset between two in-range edges can need 33 bits (#931); layers move by i32 only.
    let (dx, dy) = (i64::from(rect.x0) - i64::from(old.rect.x0), i64::from(rect.y0) - i64::from(old.rect.y0));
    let (dx, dy) = match (i32::try_from(dx), i32::try_from(dy)) {
        (Ok(dx), Ok(dy)) => (dx, dy),
        _ if !move_contents => (0, 0),
        _ => {
            return Err(bad(
                cmd,
                format!(
                    "moving the artboard with its contents by ({dx}, {dy}) is outside the 32-bit range; move it in smaller steps or pass \"moveContents\": false"
                ),
            ));
        }
    };
    s.edit("Edit Artboard", |doc, _| {
        if move_contents && (dx, dy) != (0, 0) {
            let snapshot = doc.clone();
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            crate::commands::translate_layer(&snapshot, l, dx, dy);
            crate::vector_cmds::translate_vectors(&snapshot, l, f64::from(dx), f64::from(dy));
        }
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if let Some(n) = name {
            l.name = n;
        }
        let a = l.artboard_mut().ok_or(EngineError::NoLayer(id))?;
        a.rect = rect;
        if let Some(b) = background {
            a.background = b;
        }
        match preset {
            Some(pr) => a.preset = pr,
            // A hand-edited size no longer matches the preset.
            None if rect.width() != old.rect.width() || rect.height() != old.rect.height() => a.preset.clear(),
            None => {}
        }
        if let Some(variant_name) = doc.layer(id).map(|layer| layer.name.clone())
            && let Some(variant) = doc.responsive.variants.iter_mut().find(|variant| variant.artboard == id)
        {
            variant.name = variant_name;
        }
        fit_canvas(doc);
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

/// The Artboard tool's "+" buttons: a board the size of board `layer` on one of its sides, a
/// [`GAP`] away (further out past any board in the way), empty or (`duplicate`, the ⌥-click) a
/// copy with its contents. A board left of or above the canvas origin shifts the whole document
/// right/down so the canvas can grow to show it. One undo step.
fn add_beside(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.addBeside";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = match p.get("layer").and_then(Value::as_u64) {
        Some(v) => LayerId(v),
        None => active_artboard(s).ok_or_else(|| bad(cmd, "no artboard selected"))?,
    };
    let src = d.doc.layer(id).and_then(Layer::artboard).cloned().ok_or_else(|| bad(cmd, "the layer is not an artboard"))?;
    let side = p.get("side").and_then(Value::as_str).unwrap_or("right");
    let duplicate = p.get("duplicate").and_then(Value::as_bool).unwrap_or(false);
    let [x0, y0, w, h] = xywh(src.rect);
    let gap = i64::from(GAP);
    // In i64: edges ± a board and a gap can pass the i32 range (#931); `board_rect` checks.
    let (mut x, mut y) = match side {
        "right" => (x0 + w + gap, y0),
        "left" => (x0 - gap - w, y0),
        "bottom" => (x0, y0 + h + gap),
        "top" => (x0, y0 - gap - h),
        o => return Err(bad(cmd, format!("unknown side `{o}` (left|right|top|bottom)"))),
    };
    let boards: Vec<Rect> = d.doc.artboards().iter().map(|b| b.2.rect).collect();
    // Each round clears every board in the way, so one round per board settles it.
    for _ in 0..=boards.len() {
        let blocked: Vec<&Rect> =
            boards.iter().filter(|b| i64::from(b.x0) < x + w && i64::from(b.x1) > x && i64::from(b.y0) < y + h && i64::from(b.y1) > y).collect();
        if blocked.is_empty() {
            break;
        }
        match side {
            "right" => x = blocked.iter().map(|b| i64::from(b.x1)).max().unwrap_or(x) + gap,
            "left" => x = blocked.iter().map(|b| i64::from(b.x0)).min().unwrap_or(x) - gap - w,
            "bottom" => y = blocked.iter().map(|b| i64::from(b.y1)).max().unwrap_or(y) + gap,
            _ => y = blocked.iter().map(|b| i64::from(b.y0)).min().unwrap_or(y) - gap - h,
        }
    }
    let rect = board_rect(cmd, [x, y, w, h])?;
    // The canvas starts at (0, 0): a board past it moves everything right/down instead.
    let (sx, sy) = (rect.x0.min(0).checked_neg(), rect.y0.min(0).checked_neg());
    let (Some(sx), Some(sy)) = (sx, sy) else {
        return Err(bad(cmd, format!("no room for an artboard {side} of \"{}\"", d.doc.layer(id).map_or("", |l| l.name.as_str()))));
    };
    let grown = |len: u32, by: i32| (i64::from(len) + i64::from(by)).clamp(1, 300_000) as u32;
    let label = if duplicate { "Duplicate Artboard" } else { "New Artboard" };
    let (nid, rect) = s.edit(label, |doc, active| {
        if (sx, sy) != (0, 0) {
            crate::image_cmds::translate_doc(doc, cmd, sx, sy)?;
            doc.size = Size::new(grown(doc.size.width, sx), grown(doc.size.height, sy));
            crate::canvas_geom::refresh(doc, crate::canvas_geom::Refresh::Shapes);
        }
        let (Some(x0), Some(y0), Some(x1), Some(y1)) = (rect.x0.checked_add(sx), rect.y0.checked_add(sy), rect.x1.checked_add(sx), rect.y1.checked_add(sy))
        else {
            return Err(bad(cmd, "the new artboard is outside the 32-bit coordinate range"));
        };
        let rect = Rect::new(x0, y0, x1, y1);
        let nid = if duplicate {
            let src_rect = doc.layer(id).and_then(Layer::artboard).map(|a| a.rect).ok_or(EngineError::NoLayer(id))?;
            let (dx, dy) = (rect.x0.saturating_sub(src_rect.x0), rect.y0.saturating_sub(src_rect.y0));
            let copy = crate::commands::layer_copy(doc, id)?;
            let nid = doc.insert_above(Some(id), copy);
            // Grow the canvas first, so shapes on the board re-render uncut where they land.
            let fit = |len: u32, edge: i32| i64::from(len).max(i64::from(edge)).clamp(1, 300_000) as u32;
            doc.size = Size::new(fit(doc.size.width, rect.x1), fit(doc.size.height, rect.y1));
            let snapshot = doc.clone();
            let l = doc.layer_mut(nid).ok_or(EngineError::NoLayer(nid))?;
            crate::commands::translate_layer(&snapshot, l, dx, dy);
            crate::vector_cmds::translate_vectors(&snapshot, l, f64::from(dx), f64::from(dy));
            nid
        } else {
            let mut g = Layer::group(next_artboard_name(doc), vec![]);
            set_artboard(&mut g, Artboard { rect, background: src.background, preset: src.preset.clone() });
            doc.insert_above(Some(id), g)
        };
        fit_canvas(doc);
        *active = Some(nid);
        Ok((nid, rect))
    })?;
    crate::layer_multi_cmds::set_selection(s, vec![nid], Some(nid), Some(nid))?;
    Ok(json!({"layer": nid.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

fn semantic_transform_for(role: &str, source: Rect, current_board: Rect, target: Rect, proportional: Affine) -> Affine {
    let dx = (i64::from(target.x0) - i64::from(current_board.x0)) as f64;
    let dy = (i64::from(target.y0) - i64::from(current_board.y0)) as f64;
    match role {
        "background" => {
            let sx = f64::from(target.width()) / f64::from(source.width().max(1));
            let sy = f64::from(target.height()) / f64::from(source.height().max(1));
            Affine { m: [sx, 0.0, 0.0, sy, f64::from(target.x0) - f64::from(current_board.x0) * sx, f64::from(target.y0) - f64::from(current_board.y0) * sy] }
        }
        "cta" | "footer" | "price" => {
            Affine::translate((i64::from(target.x1) - i64::from(current_board.x1)) as f64, (i64::from(target.y1) - i64::from(current_board.y1)) as f64)
        }
        "badge" => Affine::translate((i64::from(target.x1) - i64::from(current_board.x1)) as f64, dy),
        "disclaimer" => Affine::translate(dx, (i64::from(target.y1) - i64::from(current_board.y1)) as f64),
        "headline" | "body" | "logo" => Affine::translate(dx, dy),
        _ => proportional,
    }
}

fn rule_transform(rule: &LayerLayoutRule, bounds: Rect, current_board: Rect, target: Rect) -> Affine {
    let anchor_delta = |start: i32, end: i32, target_start: i32, target_end: i32, anchor: &str| -> f64 {
        match anchor {
            "center" => (f64::from(target_start) + f64::from(target_end) - f64::from(start) - f64::from(end)) * 0.5,
            "end" => f64::from(target_end) - f64::from(end),
            _ => f64::from(target_start) - f64::from(start),
        }
    };
    let (mut sx, mut sy, mut tx, mut ty) = match rule.mode {
        ResizeMode::Fixed => (
            1.0,
            1.0,
            anchor_delta(
                current_board.x0,
                current_board.x1,
                target.x0,
                target.x1,
                match rule.anchor.horizontal {
                    HorizontalAnchor::Left => "start",
                    HorizontalAnchor::Center => "center",
                    HorizontalAnchor::Right => "end",
                },
            ),
            anchor_delta(
                current_board.y0,
                current_board.y1,
                target.y0,
                target.y1,
                match rule.anchor.vertical {
                    VerticalAnchor::Top => "start",
                    VerticalAnchor::Center => "center",
                    VerticalAnchor::Bottom => "end",
                },
            ),
        ),
        ResizeMode::Stretch => {
            let x0 = f64::from(target.x0) + f64::from(bounds.x0) - f64::from(current_board.x0);
            let x1 = f64::from(target.x1) - f64::from(current_board.x1) + f64::from(bounds.x1);
            let y0 = f64::from(target.y0) + f64::from(bounds.y0) - f64::from(current_board.y0);
            let y1 = f64::from(target.y1) - f64::from(current_board.y1) + f64::from(bounds.y1);
            let sx = ((x1 - x0) / f64::from(bounds.width().max(1))).max(0.0);
            let sy = ((y1 - y0) / f64::from(bounds.height().max(1))).max(0.0);
            (sx, sy, x0 - f64::from(bounds.x0) * sx, y0 - f64::from(bounds.y0) * sy)
        }
        ResizeMode::Fill | ResizeMode::Fit => {
            let x_scale = f64::from(target.width()) / f64::from(current_board.width().max(1));
            let y_scale = f64::from(target.height()) / f64::from(current_board.height().max(1));
            let scale = if rule.mode == ResizeMode::Fill { x_scale.max(y_scale) } else { x_scale.min(y_scale) };
            let ax = match rule.anchor.horizontal {
                HorizontalAnchor::Left => f64::from(target.x0) - f64::from(current_board.x0) * scale,
                HorizontalAnchor::Center => {
                    ((f64::from(target.x0) + f64::from(target.x1)) - (f64::from(current_board.x0) + f64::from(current_board.x1)) * scale) * 0.5
                }
                HorizontalAnchor::Right => f64::from(target.x1) - f64::from(current_board.x1) * scale,
            };
            let ay = match rule.anchor.vertical {
                VerticalAnchor::Top => f64::from(target.y0) - f64::from(current_board.y0) * scale,
                VerticalAnchor::Center => {
                    ((f64::from(target.y0) + f64::from(target.y1)) - (f64::from(current_board.y0) + f64::from(current_board.y1)) * scale) * 0.5
                }
                VerticalAnchor::Bottom => f64::from(target.y1) - f64::from(current_board.y1) * scale,
            };
            (scale, scale, ax, ay)
        }
    };
    let clamp_axis = |scale: f64, translation: f64, start: i32, end: i32, min: Option<u32>, max: Option<u32>, center: bool, trailing: bool| {
        let source_size = f64::from(end.saturating_sub(start).max(1));
        let size = source_size * scale.abs();
        // A layer must not collapse to a zero-area transform when its preserved edge margins
        // exceed a much smaller target board. Keeping a one-pixel floor also makes repeated
        // reflow to that same target idempotent.
        let min_size = min.map_or(1.0, |value| f64::from(value).max(1.0));
        let max_size = max.map_or(f64::INFINITY, |value| f64::from(value).max(min_size));
        let constrained = size.max(min_size).min(max_size);
        let anchor_coordinate = if center {
            (f64::from(start) + f64::from(end)) * 0.5
        } else if trailing {
            f64::from(end)
        } else {
            f64::from(start)
        };
        let anchor = anchor_coordinate * scale + translation;
        let sign = if scale < 0.0 { -1.0 } else { 1.0 };
        let new_scale = sign * constrained / source_size;
        (new_scale, anchor - anchor_coordinate * new_scale)
    };
    let (x_scale, x_translation) = clamp_axis(
        sx,
        tx,
        bounds.x0,
        bounds.x1,
        rule.min_size.map(|v| v.width),
        rule.max_size.map(|v| v.width),
        rule.anchor.horizontal == HorizontalAnchor::Center,
        rule.anchor.horizontal == HorizontalAnchor::Right,
    );
    let (y_scale, y_translation) = clamp_axis(
        sy,
        ty,
        bounds.y0,
        bounds.y1,
        rule.min_size.map(|v| v.height),
        rule.max_size.map(|v| v.height),
        rule.anchor.vertical == VerticalAnchor::Center,
        rule.anchor.vertical == VerticalAnchor::Bottom,
    );
    sx = x_scale;
    sy = y_scale;
    tx = x_translation;
    ty = y_translation;
    Affine { m: [sx, 0.0, 0.0, sy, tx, ty] }
}

fn copy_semantic_roles(source_doc: &Document, source: &Layer, doc: &mut Document, target: &Layer) {
    if let Some(role) = explicit_semantic_role(source_doc, source.id)
        && role != "auto"
    {
        let key = role_meta_key(target.id);
        doc.metadata.text.retain(|(k, _)| k != &key);
        doc.metadata.text.push((key, role.to_owned()));
        let source_key = role_source_meta_key(target.id);
        doc.metadata.text.retain(|(k, _)| k != &source_key);
        if let Some(source) = source_doc.metadata.text.iter().rev().find(|(k, _)| k == &role_source_meta_key(source.id)).map(|(_, value)| value.clone()) {
            doc.metadata.text.push((source_key, source));
        }
    }
    if let (Some(source_children), Some(target_children)) = (source.children(), target.children()) {
        for (source_child, target_child) in source_children.iter().zip(target_children.iter()) {
            copy_semantic_roles(source_doc, source_child, doc, target_child);
        }
    }
}

fn collect_layer_pairs(source: &Layer, target: &Layer, out: &mut Vec<(LayerId, LayerId)>) {
    out.push((source.id, target.id));
    if let (Some(source_children), Some(target_children)) = (source.children(), target.children()) {
        for (source_child, target_child) in source_children.iter().zip(target_children) {
            collect_layer_pairs(source_child, target_child, out);
        }
    }
}

struct SemanticReflow<'a> {
    source_doc: &'a Document,
    source_rect: Rect,
    current_board: Rect,
    target_rect: Rect,
    proportional: Affine,
    dpi: f32,
    rules: &'a std::collections::HashMap<LayerId, LayerLayoutRule>,
    text_engine: Option<photocraft_text::TextEngine>,
    job: crate::jobs::JobCtx,
    visited: usize,
    total: usize,
}

fn reflow_stack_children(children: &mut [Layer], stack: &StackRule, dpi: f32, doc: &Document, job: &crate::jobs::JobCtx) -> Result<()> {
    let Some(parent_bounds) = children.iter().filter_map(crate::layer_multi_cmds::layer_bounds).reduce(|a, b| a.union(&b)) else { return Ok(()) };
    let mut cursor: Option<f64> = None;
    let child_count = children.len();
    for (index, child) in children.iter_mut().enumerate() {
        job.check()?;
        let Some(bounds) = crate::layer_multi_cmds::layer_bounds(child) else { continue };
        let locks = doc.effective_locks(child.id);
        if locks.position || locks.all {
            cursor = Some(match stack.direction {
                StackDirection::Vertical => f64::from(bounds.y1) + f64::from(stack.gap),
                StackDirection::Horizontal => f64::from(bounds.x1) + f64::from(stack.gap),
            });
            continue;
        }
        let (mut sx, mut sy) = (1.0f64, 1.0f64);
        let (mut x, mut y) = (f64::from(bounds.x0), f64::from(bounds.y0));
        match stack.direction {
            StackDirection::Vertical => {
                if let Some(next) = cursor {
                    y = next;
                }
                x = match stack.alignment {
                    StackAlignment::Start | StackAlignment::Stretch => f64::from(parent_bounds.x0),
                    StackAlignment::Center => (f64::from(parent_bounds.x0) + f64::from(parent_bounds.x1) - f64::from(bounds.width())) * 0.5,
                    StackAlignment::End => f64::from(parent_bounds.x1) - f64::from(bounds.width()),
                };
                if stack.alignment == StackAlignment::Stretch {
                    sx = f64::from(parent_bounds.width()) / f64::from(bounds.width().max(1));
                }
                cursor = Some(y + f64::from(bounds.height()) + f64::from(stack.gap));
            }
            StackDirection::Horizontal => {
                if let Some(next) = cursor {
                    x = next;
                }
                y = match stack.alignment {
                    StackAlignment::Start | StackAlignment::Stretch => f64::from(parent_bounds.y0),
                    StackAlignment::Center => (f64::from(parent_bounds.y0) + f64::from(parent_bounds.y1) - f64::from(bounds.height())) * 0.5,
                    StackAlignment::End => f64::from(parent_bounds.y1) - f64::from(bounds.height()),
                };
                if stack.alignment == StackAlignment::Stretch {
                    sy = f64::from(parent_bounds.height()) / f64::from(bounds.height().max(1));
                }
                cursor = Some(x + f64::from(bounds.width()) + f64::from(stack.gap));
            }
        }
        if index + 1 < child_count && cursor.is_some_and(|axis| !axis.is_finite() || axis < f64::from(i32::MIN) || axis > f64::from(i32::MAX)) {
            return Err(EngineError::Other("stack gap places a layer outside the document coordinate range".into()));
        }
        let tx = x - f64::from(bounds.x0) * sx;
        let ty = y - f64::from(bounds.y0) * sy;
        if ![sx, sy, tx, ty].into_iter().all(f64::is_finite) {
            return Err(EngineError::Other("stack layout produced non-finite geometry".into()));
        }
        if sx == 1.0 && sy == 1.0 {
            let dx = (x - f64::from(bounds.x0)).round();
            let dy = (y - f64::from(bounds.y0)).round();
            if dx < f64::from(i32::MIN) || dx > f64::from(i32::MAX) || dy < f64::from(i32::MIN) || dy > f64::from(i32::MAX) {
                return Err(EngineError::Other("stack translation exceeds the document coordinate range".into()));
            }
            let (dx, dy) = (dx as i32, dy as i32);
            if (dx, dy) != (0, 0) {
                crate::commands::translate_layer(doc, child, dx, dy);
                crate::vector_cmds::translate_vectors(doc, child, f64::from(dx), f64::from(dy));
            }
            continue;
        }
        let affine = Affine { m: [sx, 0.0, 0.0, sy, tx, ty] };
        let homography = photocraft_algo::transform::Homography([sx, 0.0, tx, 0.0, sy, ty, 0.0, 0.0, 1.0]);
        crate::transform_cmds::transform_layer(None, Default::default(), child, &homography, Some(affine), photocraft_algo::transform::Interp::Bicubic)?;
        crate::canvas_geom::transform_layer_geometry(child, &affine, false, dpi);
    }
    Ok(())
}

fn semantic_reflow_child(source: &Layer, target: &mut Layer, reflow: &mut SemanticReflow<'_>) -> Result<()> {
    reflow.job.check()?;
    reflow.visited = reflow.visited.saturating_add(1);
    let progress = reflow.visited.min(reflow.total.max(1)) as f32 / reflow.total.max(1) as f32;
    reflow.job.progress(0.05 + 0.85 * progress, "Reflowing responsive layout");
    let locks = reflow.source_doc.effective_locks(source.id);
    if locks.position || locks.all {
        return Ok(());
    }
    if let (Some(source_children), Some(target_children)) = (source.children(), target.children_mut()) {
        for (source, target) in source_children.iter().zip(target_children.iter_mut()) {
            semantic_reflow_child(source, target, reflow)?;
        }
        if let Some(stack) = reflow.rules.get(&source.id).and_then(|rule| rule.stack.as_ref()) {
            reflow_stack_children(target_children, stack, reflow.dpi, reflow.source_doc, &reflow.job)?;
        }
        return Ok(());
    }
    let Some(bounds) = crate::layer_multi_cmds::layer_bounds(source) else { return Ok(()) };
    let role = semantic_role(reflow.source_doc, source.id);
    let transform_bounds = crate::layer_multi_cmds::layer_bounds(target).unwrap_or(bounds);
    let explicit_rule = reflow.rules.get(&source.id);
    let affine = explicit_rule.map_or_else(
        || semantic_transform_for(role, reflow.source_rect, reflow.current_board, reflow.target_rect, reflow.proportional),
        |rule| rule_transform(rule, transform_bounds, reflow.current_board, reflow.target_rect),
    );
    let homography = photocraft_algo::transform::Homography([affine.m[0], affine.m[2], affine.m[4], affine.m[1], affine.m[3], affine.m[5], 0.0, 0.0, 1.0]);
    crate::transform_cmds::transform_layer(None, Default::default(), target, &homography, Some(affine), photocraft_algo::transform::Interp::Bicubic)?;
    crate::canvas_geom::transform_layer_geometry(target, &affine, false, reflow.dpi);
    if (matches!(role, "headline" | "body") || explicit_rule.is_some_and(|rule| rule.text_flow != TextFlow::Preserve))
        && let LayerContent::Text(text) = &mut target.content
        && let photocraft_doc::text::TextShape::Box { x, y, height, .. } = text.shape
    {
        let left = bounds.x0.saturating_sub(reflow.source_rect.x0).max(0);
        let right = reflow.source_rect.x1.saturating_sub(bounds.x1).max(0);
        let width = (i64::from(reflow.target_rect.width()) - i64::from(left) - i64::from(right)).max(1) as f32;
        text.shape = photocraft_doc::text::TextShape::Box { x, y, width, height };
        if let Some(rule) = explicit_rule
            && matches!(rule.text_flow, TextFlow::RewrapAndShrink | TextFlow::RewrapAutoHeight)
        {
            // Measure the full wrapped content once. Lazily build one deterministic bundled-font
            // engine per variant operation, then reuse it for every paragraph in that operation.
            text.shape = photocraft_doc::text::TextShape::Box { x, y, width, height: 300_000.0 };
            let engine = reflow.text_engine.get_or_insert_with(photocraft_text::TextEngine::new);
            let layout = engine.layout(text, reflow.dpi);
            let measured =
                layout.lines.iter().map(|line| line.baseline + line.descent).filter(|height| height.is_finite()).fold(1.0f32, f32::max).min(300_000.0);
            if rule.text_flow == TextFlow::RewrapAndShrink
                && measured > height
                && let LayerContent::Text(source_text) = &source.content
            {
                let ratio = (height / measured).clamp(0.0, 1.0);
                let floor = rule.min_font_size_pt.unwrap_or(source_text.size_pt).min(source_text.size_pt);
                for run in &mut text.runs {
                    let original_size = run.style.size_pt;
                    run.style.size_pt = (original_size * ratio).max(floor.min(original_size));
                }
                if text.runs.is_empty() {
                    text.size_pt = (text.size_pt * ratio).max(floor);
                } else {
                    text.sync_summary();
                }
            }
            text.shape =
                photocraft_doc::text::TextShape::Box { x, y, width, height: if rule.text_flow == TextFlow::RewrapAutoHeight { measured } else { height } };
        }
        crate::type_cmds::refresh(reflow.source_doc, text);
    }
    Ok(())
}

/// Create an editable, proportionally fitted copy of the active artboard at a new size.
/// The semantic mode preserves text size, wraps paragraph boxes to the target width and anchors
/// recognizable roles; proportional mode remains available for an exact uniform fit.
fn create_variant(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.createVariant";
    let source = p.get("layer").and_then(Value::as_u64).map(LayerId).or_else(|| active_artboard(s)).ok_or_else(|| bad(cmd, "select an artboard"))?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let original = d.doc.layer(source).and_then(Layer::artboard).cloned().ok_or_else(|| bad(cmd, "the source layer is not an artboard"))?;
    if d.doc.responsive.variants.len() >= photocraft_doc::responsive::MAX_RESPONSIVE_VARIANTS {
        return Err(bad(cmd, "this document already has the maximum of 64 responsive variants"));
    }
    let width = int(p, "width").ok_or_else(|| bad(cmd, "`width` must be a positive integer"))?;
    let height = int(p, "height").ok_or_else(|| bad(cmd, "`height` must be a positive integer"))?;
    let area = width.checked_mul(height).ok_or_else(|| bad(cmd, "the target artboard is too large"))?;
    if width == 0 || height == 0 || width > 300_000 || height > 300_000 || area > 36_000_000 {
        return Err(bad(cmd, "target dimensions must be positive, at most 300,000 px per side, and no more than 36 megapixels"));
    }
    let mut duplicate = d.doc.layer(source).ok_or(EngineError::NoLayer(source))?.duplicate();
    let layout = p.get("layout").and_then(Value::as_str).unwrap_or("semantic").to_owned();
    if !matches!(layout.as_str(), "semantic" | "proportional") {
        return Err(bad(cmd, "`layout` must be `semantic` or `proportional`"));
    }
    let duplicate_name = p
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{} ({}×{})", d.doc.layer(source).map_or("Artboard", |l| l.name.as_str()), width, height));
    duplicate.name = duplicate_name;
    let original_rect = original.rect;
    let dpi = d.doc.resolution_dpi;
    let source_doc = d.doc.clone();
    let source_variant = source_doc.responsive.variants.iter().find(|variant| variant.artboard == source).cloned();
    let master_rules: std::collections::HashMap<_, _> = source_doc.responsive.rules.iter().cloned().map(|rule| (rule.layer, rule)).collect();
    let mut rule_map = std::collections::HashMap::new();
    if let Some(variant) = &source_variant {
        for mapping in &variant.layers {
            if let Some(rule) = master_rules.get(&mapping.master_layer) {
                rule_map.insert(mapping.variant_layer, rule.clone());
            }
        }
        for override_rule in source_doc.responsive.overrides.iter().filter(|entry| entry.variant == source) {
            rule_map.insert(override_rule.layer, override_rule.rule.clone());
        }
    } else {
        rule_map = master_rules;
    }
    let master = source_doc
        .responsive
        .variants
        .iter()
        .find(|variant| variant.artboard == source)
        .map(|variant| variant.master)
        .or(source_doc.responsive.master)
        .unwrap_or(source);
    crate::jobs::edit_job(
        s,
        "Create Artboard Variant",
        move |doc, active, job| {
            let copy = doc.insert_above(Some(source), duplicate);
            place_copy(doc, copy)?;
            let copy_rect = doc.layer(copy).and_then(Layer::artboard).map(|a| a.rect).ok_or(EngineError::NoLayer(copy))?;
            let source_layer = source_doc.layer(source).ok_or(EngineError::NoLayer(source))?;
            let copy_layer = doc.layer(copy).ok_or(EngineError::NoLayer(copy))?.clone();
            copy_semantic_roles(&source_doc, source_layer, doc, &copy_layer);
            let mut pairs = Vec::new();
            collect_layer_pairs(source_layer, &copy_layer, &mut pairs);
            let scale = ((width as f64) / f64::from(original_rect.width())).min((height as f64) / f64::from(original_rect.height()));
            let tx = f64::from(copy_rect.x0) + (width as f64 - f64::from(original_rect.width()) * scale) * 0.5 - f64::from(copy_rect.x0) * scale;
            let ty = f64::from(copy_rect.y0) + (height as f64 - f64::from(original_rect.height()) * scale) * 0.5 - f64::from(copy_rect.y0) * scale;
            let affine = Affine { m: [scale, 0.0, 0.0, scale, tx, ty] };
            let h = photocraft_algo::transform::Homography([scale, 0.0, tx, 0.0, scale, ty, 0.0, 0.0, 1.0]);
            let mut reflow = SemanticReflow {
                source_doc: &source_doc,
                source_rect: original_rect,
                current_board: copy_rect,
                target_rect: board_rect(cmd, [i64::from(copy_rect.x0), i64::from(copy_rect.y0), width, height])?,
                proportional: affine,
                dpi,
                rules: &rule_map,
                text_engine: None,
                job: job.clone(),
                visited: 0,
                total: source_doc.walk().len().max(1),
            };
            let layer = doc.layer_mut(copy).ok_or(EngineError::NoLayer(copy))?;
            if layout == "semantic" {
                if let (Some(source_children), Some(target_children)) = (source_layer.children(), layer.children_mut()) {
                    let count = source_children.len().max(1);
                    for (index, (source_child, target_child)) in source_children.iter().zip(target_children.iter_mut()).enumerate() {
                        job.check()?;
                        job.progress(0.05 + 0.85 * (index as f32 / count as f32), "Reflowing responsive layout");
                        semantic_reflow_child(source_child, target_child, &mut reflow)?;
                    }
                    if let Some(stack) = rule_map.get(&source_layer.id).and_then(|rule| rule.stack.as_ref()) {
                        reflow_stack_children(target_children, stack, dpi, &source_doc, &reflow.job)?;
                    }
                }
            } else {
                crate::transform_cmds::transform_layer(None, Default::default(), layer, &h, Some(affine), photocraft_algo::transform::Interp::Bicubic)?;
                crate::canvas_geom::transform_layer_geometry(layer, &affine, false, dpi);
            }
            job.check()?;
            let board = layer.artboard_mut().ok_or(EngineError::NoLayer(copy))?;
            board.rect = board_rect(cmd, [i64::from(copy_rect.x0), i64::from(copy_rect.y0), width, height])?;
            board.preset.clear();
            doc.responsive.master.get_or_insert(master);
            doc.responsive.variants.retain(|variant| variant.artboard != copy);
            let layer_map: std::collections::HashMap<_, _> = pairs
                .iter()
                .map(|(source_id, copy_id)| {
                    let master_id = source_variant
                        .as_ref()
                        .and_then(|variant| variant.layers.iter().find(|mapping| mapping.variant_layer == *source_id).map(|mapping| mapping.master_layer))
                        .unwrap_or(*source_id);
                    (*source_id, (master_id, *copy_id))
                })
                .collect();
            let mappings = pairs
                .iter()
                .filter_map(|(source_id, _)| {
                    layer_map.get(source_id).map(|(master_layer, variant_layer)| photocraft_doc::responsive::LayerMapping {
                        master_layer: *master_layer,
                        variant_layer: *variant_layer,
                    })
                })
                .collect();
            for previous in source_doc.responsive.overrides.iter().filter(|entry| entry.variant == source) {
                if let Some((_, new_layer)) = layer_map.get(&previous.layer) {
                    let mut rule = previous.rule.clone();
                    rule.layer = *new_layer;
                    doc.responsive.overrides.push(photocraft_doc::responsive::LayerRuleOverride { variant: copy, layer: *new_layer, rule });
                }
            }
            doc.responsive.variants.push(photocraft_doc::responsive::ResponsiveVariant {
                artboard: copy,
                master,
                name: doc.layer(copy).map_or_else(String::new, |l| l.name.clone()),
                layers: mappings,
            });
            fit_canvas(doc);
            *active = Some(copy);
            let rect = doc.layer(copy).and_then(Layer::artboard).map(|artboard| artboard.rect).ok_or(EngineError::NoLayer(copy))?;
            let scale = ((width as f64) / f64::from(original_rect.width())).min((height as f64) / f64::from(original_rect.height()));
            job.progress(1.0, "Responsive variant ready");
            Ok((copy, rect, scale))
        },
        move |(created, rect, scale)| json!({"layer": created.0, "source": source.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()], "scale": scale}),
    )
}

fn keep_variant_layer_ids(master: &Layer, variant: &mut Layer, previous: &std::collections::HashMap<LayerId, LayerId>) {
    if let Some(id) = previous.get(&master.id) {
        variant.id = *id;
    }
    if let (Some(master_children), Some(variant_children)) = (master.children(), variant.children_mut()) {
        for (master_child, variant_child) in master_children.iter().zip(variant_children) {
            keep_variant_layer_ids(master_child, variant_child, previous);
        }
    }
}

fn preserve_variant_only_layers(previous: &Layer, refreshed: &mut Layer, mapped_ids: &std::collections::HashSet<LayerId>, preserved: &mut Vec<Layer>) {
    let Some(previous_children) = previous.children() else { return };
    let Some(refreshed_children) = refreshed.children_mut() else {
        preserved.extend(previous_children.iter().cloned());
        return;
    };
    for child in previous_children {
        if !mapped_ids.contains(&child.id) {
            preserved.push(child.clone());
            continue;
        }
        if let Some(target) = refreshed_children.iter_mut().find(|target| target.id == child.id) {
            preserve_variant_only_layers(child, target, mapped_ids, preserved);
        } else {
            // The master changed this mapped layer's structure or content type. Keep its old
            // subtree on the artboard instead of silently dropping local variant content.
            preserved.push(child.clone());
        }
    }
}

fn update_variants(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.variants.update";
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let master_id = optional_layer_param(p, cmd)?
        .or(state.doc.responsive.master)
        .or_else(|| active_artboard(s))
        .ok_or_else(|| bad(cmd, "set or select a master artboard"))?;
    let master =
        state.doc.layer(master_id).filter(|layer| layer.artboard().is_some()).cloned().ok_or_else(|| bad(cmd, "the selected master ID is not an artboard"))?;
    let master_rect = master.artboard().map(|board| board.rect).ok_or_else(|| bad(cmd, "the master layer is not an artboard"))?;
    let variants: Vec<_> = state.doc.responsive.variants.iter().filter(|variant| variant.master == master_id).cloned().collect();
    if variants.is_empty() {
        return Err(bad(cmd, "the master artboard has no variants to update"));
    }
    let source_doc = state.doc.clone();
    let dpi = source_doc.resolution_dpi;
    let master_rules: std::collections::HashMap<_, _> = source_doc.responsive.rules.iter().cloned().map(|rule| (rule.layer, rule)).collect();
    let updated_count = variants.len();
    crate::jobs::edit_job(
        s,
        "Update Responsive Artboards",
        move |doc, _, job| {
            let (mut added, mut removed) = (0usize, 0usize);
            let mut conflicts = Vec::new();
            for (variant_index, variant) in variants.iter().enumerate() {
                job.check()?;
                job.progress(0.02 + 0.9 * (variant_index as f32 / updated_count.max(1) as f32), "Updating responsive variants");
                let Some(previous_board) = source_doc.layer(variant.artboard) else { continue };
                let Some(previous_artboard) = previous_board.artboard() else { continue };
                let target_rect = previous_artboard.rect;
                let mut refreshed = master.duplicate();
                let previous: std::collections::HashMap<_, _> = variant.layers.iter().map(|mapping| (mapping.master_layer, mapping.variant_layer)).collect();
                keep_variant_layer_ids(&master, &mut refreshed, &previous);
                refreshed.name.clone_from(&previous_board.name);
                if let Some(board) = refreshed.artboard_mut() {
                    board.rect = master_rect;
                    board.background = previous_artboard.background;
                }

                let mut rules = master_rules.clone();
                let inverse: std::collections::HashMap<_, _> = variant.layers.iter().map(|mapping| (mapping.variant_layer, mapping.master_layer)).collect();
                for override_rule in source_doc.responsive.overrides.iter().filter(|entry| entry.variant == variant.artboard) {
                    if let Some(master_layer) = inverse.get(&override_rule.layer) {
                        let mut rule = override_rule.rule.clone();
                        rule.layer = *master_layer;
                        rules.insert(*master_layer, rule);
                    }
                }
                let scale = (f64::from(target_rect.width()) / f64::from(master_rect.width().max(1)))
                    .min(f64::from(target_rect.height()) / f64::from(master_rect.height().max(1)));
                let tx = f64::from(target_rect.x0) + (f64::from(target_rect.width()) - f64::from(master_rect.width()) * scale) * 0.5
                    - f64::from(master_rect.x0) * scale;
                let ty = f64::from(target_rect.y0) + (f64::from(target_rect.height()) - f64::from(master_rect.height()) * scale) * 0.5
                    - f64::from(master_rect.y0) * scale;
                let proportional = Affine { m: [scale, 0.0, 0.0, scale, tx, ty] };
                let mut reflow = SemanticReflow {
                    source_doc: &source_doc,
                    source_rect: master_rect,
                    current_board: master_rect,
                    target_rect,
                    proportional,
                    dpi,
                    rules: &rules,
                    text_engine: None,
                    job: job.clone(),
                    visited: 0,
                    total: source_doc.walk().len().max(1),
                };
                semantic_reflow_child(&master, &mut refreshed, &mut reflow)?;
                if let Some(board) = refreshed.artboard_mut() {
                    board.rect = target_rect;
                }

                let mut pairs = Vec::new();
                collect_layer_pairs(&master, &refreshed, &mut pairs);
                let mappings: Vec<_> = pairs
                    .iter()
                    .map(|(master_layer, variant_layer)| photocraft_doc::responsive::LayerMapping {
                        master_layer: *master_layer,
                        variant_layer: *variant_layer,
                    })
                    .collect();
                let current_master_ids: std::collections::HashSet<_> = mappings.iter().map(|mapping| mapping.master_layer).collect();
                added = added.saturating_add(mappings.iter().filter(|mapping| !previous.contains_key(&mapping.master_layer)).count());
                removed = removed.saturating_add(variant.layers.iter().filter(|mapping| !current_master_ids.contains(&mapping.master_layer)).count());
                for stale in variant.layers.iter().filter(|mapping| !current_master_ids.contains(&mapping.master_layer)) {
                    conflicts.push(
                    json!({"variant":variant.artboard.0,"kind":"removed_master_layer","masterLayer":stale.master_layer.0,"variantLayer":stale.variant_layer.0}),
                );
                }

                // Keep explicit semantic-role edits made on existing variants. New variant layers
                // inherit the master's role through copy_semantic_roles below.
                let preserved_roles: Vec<_> = variant
                    .layers
                    .iter()
                    .filter_map(|mapping| explicit_semantic_role(&source_doc, mapping.variant_layer).map(|role| (mapping.variant_layer, role.to_owned())))
                    .collect();
                let old_ids: std::collections::HashSet<_> = variant.layers.iter().map(|mapping| mapping.variant_layer).collect();
                for stale in source_doc.responsive.overrides.iter().filter(|entry| entry.variant == variant.artboard && !inverse.contains_key(&entry.layer)) {
                    conflicts.push(json!({"variant":variant.artboard.0,"kind":"orphaned_override","layer":stale.layer.0}));
                }
                doc.metadata.text.retain(|(key, _)| !old_ids.iter().any(|id| key == &role_meta_key(*id)));
                copy_semantic_roles(&source_doc, &master, doc, &refreshed);
                for (id, role) in preserved_roles {
                    let key = role_meta_key(id);
                    doc.metadata.text.retain(|(existing, _)| existing != &key);
                    doc.metadata.text.push((key, role));
                }
                if let Some(current) = doc.responsive.variants.iter_mut().find(|entry| entry.artboard == variant.artboard) {
                    current.layers = mappings;
                }
                let valid_ids: std::collections::HashSet<_> = pairs.iter().map(|(_, variant_layer)| *variant_layer).collect();
                doc.responsive.overrides.retain(|entry| entry.variant != variant.artboard || valid_ids.contains(&entry.layer));
                let mapped_ids: std::collections::HashSet<_> = variant.layers.iter().map(|mapping| mapping.variant_layer).collect();
                let mut preserved_local_layers = Vec::new();
                preserve_variant_only_layers(previous_board, &mut refreshed, &mapped_ids, &mut preserved_local_layers);
                if let Some(children) = refreshed.children_mut() {
                    for local_layer in preserved_local_layers {
                        conflicts.push(json!({"variant":variant.artboard.0,"kind":"preserved_variant_layer","layer":local_layer.id.0,"name":local_layer.name}));
                        children.push(local_layer);
                    }
                }
                if let Some(board_layer) = doc.layer_mut(variant.artboard) {
                    *board_layer = refreshed.clone();
                }
            }
            job.check()?;
            fit_canvas(doc);
            job.progress(1.0, "Responsive variants updated");
            Ok((added, removed, conflicts))
        },
        move |(added, removed, conflicts)| json!({"master":master_id.0,"updated":updated_count,"addedLayers":added,"removedLayers":removed,"overridesPreserved":true,"conflicts":conflicts}),
    )
}

fn clear_artboard_guides(s: &mut Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = active_artboard(s).ok_or(EngineError::Other("no artboard selected".into()))?;
    let r = d.doc.layer(id).and_then(Layer::artboard).map(|a| a.rect).ok_or(EngineError::NoLayer(id))?;
    let inside_x = |x: &f64| *x > f64::from(r.x0) && *x < f64::from(r.x1);
    let inside_y = |y: &f64| *y > f64::from(r.y0) && *y < f64::from(r.y1);
    let n = d.doc.guides.vertical.iter().filter(|x| inside_x(x)).count() + d.doc.guides.horizontal.iter().filter(|y| inside_y(y)).count();
    if n == 0 {
        return Ok(json!({"cleared": 0}));
    }
    s.edit("Clear Selected Artboard Guides", |doc, _| {
        doc.guides.vertical.retain(|x| !inside_x(x));
        doc.guides.horizontal.retain(|y| !inside_y(y));
        doc.guides.prune_colors();
        Ok(())
    })?;
    Ok(json!({"cleared": n}))
}

/// One artboard as its own document: the board's size, its group moved to the origin. `None`
/// when `id` is not an artboard; an error when the board is too far from the origin to move.
pub fn artboard_document(doc: &Document, id: LayerId) -> Result<Option<Document>> {
    let Some(l) = doc.layer(id) else { return Ok(None) };
    let Some(r) = l.artboard().map(|a| a.rect) else { return Ok(None) };
    // A board starting at i32::MIN needs an offset of 2^31 to reach the origin, one past
    // i32::MAX (#931).
    let (Some(dx), Some(dy)) = (r.x0.checked_neg(), r.y0.checked_neg()) else {
        return Err(EngineError::Other(format!("artboard `{}` at ({}, {}) is too far from the origin to export", l.name, r.x0, r.y0)));
    };
    let mut group = l.clone();
    group.visible = true;
    crate::commands::translate_layer(doc, &mut group, dx, dy);
    crate::vector_cmds::translate_vectors(doc, &mut group, f64::from(dx), f64::from(dy));
    let mut one = doc.clone();
    one.name = l.name.clone();
    one.size = Size::new(r.width(), r.height());
    one.layers = vec![group];
    one.channels.clear();
    one.selection = None;
    one.quick_mask = None;
    one.layer_comps.clear();
    one.last_applied_comp = None;
    one.last_document_state = None;
    let shift = |v: &[f64], o: i32, len: u32| v.iter().map(|g| g - f64::from(o)).filter(|g| *g > 0.0 && *g < f64::from(len)).collect::<Vec<_>>();
    one.guides.vertical = shift(&doc.guides.vertical, r.x0, r.width());
    one.guides.horizontal = shift(&doc.guides.horizontal, r.y0, r.height());
    Ok(Some(one))
}

/// [`artboard_document`] for each chosen board, all built before any file is written so a
/// board that can't be exported fails the export without leaving a partial set behind.
fn chosen_documents(doc: &Document, p: &Value) -> Result<Vec<Document>> {
    chosen_artboards(doc, p).into_iter().filter_map(|id| artboard_document(doc, id).transpose()).collect()
}

/// The artboards an export covers: `"artboards": [ids]`, else all, top of the Layers panel first.
fn chosen_artboards(doc: &Document, p: &Value) -> Vec<LayerId> {
    let all: Vec<LayerId> = doc.artboards().into_iter().rev().map(|b| b.0).collect();
    match p.get("artboards") {
        Some(Value::Array(a)) => {
            let want: Vec<u64> = a.iter().filter_map(Value::as_u64).collect();
            all.into_iter().filter(|id| want.contains(&id.0)).collect()
        }
        _ => all,
    }
}

/// File › Export › Artboards to Files: `<prefix>_<index:04>_<artboard name>.<format>` per board.
fn artboards_to_files(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.artboardsToFiles";
    let dir = str_param(p, "dir", cmd).or_else(|_| str_param(p, "output", cmd))?.to_string();
    let format = p.get("format").and_then(Value::as_str).unwrap_or("png").trim_start_matches('.').to_ascii_lowercase();
    let save = SaveOpts::from_params(s, p, cmd)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let prefix = p.get("prefix").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&d.doc.name));
    let doc = d.doc.clone();
    let mut files = Vec::new();
    for (i, one) in chosen_documents(&doc, p)?.into_iter().enumerate() {
        // Names may be identical, or collide after sanitizing or filesystem case folding.
        let name = if prefix.is_empty() { format!("{i:04}_{}", sanitize(&one.name)) } else { format!("{}_{i:04}_{}", sanitize(&prefix), sanitize(&one.name)) };
        let path = join(&dir, &format!("{name}.{format}"));
        save_doc(&one, &path, save)?;
        files.push(path);
    }
    if files.is_empty() {
        return Err(bad(cmd, "no matching artboards"));
    }
    Ok(json!({"files": files}))
}

/// A minimal PDF with one page per JPEG image: (width px, height px, dpi, JPEG bytes).
/// Pages are sized so the image prints at its resolution.
pub fn raster_pdf(pages: &[(u32, u32, f32, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    // Objects: 1 catalog, 2 page tree, then per page: page, contents, image.
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 3 + i * 3)).collect();
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).as_bytes());
    for (i, (w, h, dpi, jpeg)) in pages.iter().enumerate() {
        let k = 72.0 / f64::from(dpi.max(1.0));
        let (pw, ph) = (f64::from(*w) * k, f64::from(*h) * k);
        let (contents, image) = (4 + i * 3, 5 + i * 3);
        obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.3} {ph:.3}] /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {contents} 0 R >>"
            )
            .as_bytes(),
        );
        let stream = format!("q {pw:.3} 0 0 {ph:.3} 0 0 cm /Im0 Do Q");
        obj(&mut out, format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()).as_bytes());
        let mut img = format!(
            "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            jpeg.len()
        )
        .into_bytes();
        img.extend_from_slice(jpeg);
        img.extend_from_slice(b"\nendstream");
        obj(&mut out, &img);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

/// File › Export › Artboards to PDF: one page per board (flattened over white, JPEG-encoded).
fn artboards_to_pdf(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.artboardsToPdf";
    let path = str_param(p, "path", cmd)?.to_string();
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    let quality = f64_param(p, "quality").or(Some(10.0));
    let mut pages = Vec::new();
    for one in chosen_documents(&doc, p)? {
        let buf = photocraft_compose::flatten(&one).over_background([1.0, 1.0, 1.0]);
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
        let mut flat = Document::new(&one.name, one.size, ColorMode::Rgb, SampleType::U8);
        let mut surf = Surface::new(fmt);
        let vals: Vec<f32> = buf.px.iter().flat_map(|px| photocraft_raster::from_rgba(&fmt, *px)).collect();
        surf.write_region(one.bounds(), &vals);
        flat.layers.push(Layer::new("Background", LayerContent::Raster(surf)));
        let (jpeg, _) = encode(&flat, "page.jpg", quality)?;
        pages.push((one.size.width, one.size.height, doc.resolution_dpi, jpeg));
    }
    if pages.is_empty() {
        return Err(bad(cmd, "no matching artboards"));
    }
    write_file(&path, &raster_pdf(&pages))?;
    Ok(json!({"path": path, "pages": pages.len()}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    let mut specs = vec![
        spec!(
            "layer.new.artboard",
            "Artboard…",
            &["Layer", "New"],
            r##"{"rect":[x,y,w,h]? | "x","y","width","height"? (default: canvas size, right of the last board),"preset":"iPhone 14|Web 1920|A4|…"?,"name":str?,"background":"white|black|transparent|custom"="white","color":[r,g,b]|"#rrggbb"? (custom)} → {layer, rect}"##,
            has_doc,
            new_artboard
        ),
        spec!(
            "layer.new.artboardFromGroup",
            "Artboard from Group…",
            &["Layer", "New"],
            r##"{"layer":id? (a top-level group; default active),"background":…?} → {layer, rect}"##,
            has_plain_group,
            artboard_from_group
        ),
        spec!(
            "layer.new.artboardFromLayers",
            "Artboard from Layers…",
            &["Layer", "New"],
            r##"{"name":str?,"background":…?} (the selected layers) → {layer, rect}"##,
            has_free_layers,
            artboard_from_layers
        ),
        spec!(
            "layer.artboard.set",
            "Edit Artboard",
            &[],
            r##"{"layer":id? (default: the active artboard),"x","y","width","height"?|"rect":[x,y,w,h]?,"preset":str?,"background":"white|black|transparent|custom"?,"color":…?,"name":str?,"moveContents":bool=true} → {layer, rect}"##,
            has_active_artboard,
            set_props
        ),
        spec!(
            "layer.artboard.createVariant",
            "Create Artboard Variant",
            &[],
            r##"{"layer":id? (default: active artboard),"width":positive integer,"height":positive integer,"name":str?,"layout":"semantic|proportional"="semantic"} → {layer, source, rect, scale}. Semantic keeps type size, wraps paragraph text and uses per-layer roles; proportional uniformly scales content."##,
            has_active_artboard,
            create_variant
        ),
        spec!(
            "layer.artboard.semanticRole.set",
            "Set Artboard Semantic Role",
            &[],
            "{\"layer\":id? (default active),\"role\":semantic role,\"applyToSimilar\":bool=false}; optional bulk change is bounded to 10,000 matching layers in the same artboard; returns updated layer IDs/count",
            has_active_artboard,
            set_semantic_role
        ),
        spec!(
            "layer.artboard.layoutRule.set",
            "Set Responsive Layout Rule",
            &[],
            r##"{"layer":id? (active layer),"horizontal":"left|center|right","vertical":"top|center|bottom","mode":"fixed|stretch|fill|fit","minSize":{"width":px,"height":px}? ,"maxSize":…?,"textFlow":"preserve|rewrap|rewrap_auto_height|rewrap_and_shrink","minFontSizePt":number?}"##,
            has_active_artboard,
            set_layout_rule
        ),
        spec!(
            "layer.artboard.master.set",
            "Set Responsive Master Artboard",
            &[],
            "{\"layer\":artboard id? (default active artboard)}",
            has_artboards,
            set_responsive_master
        ),
        spec!(
            "layer.artboard.variant.delete",
            "Delete Responsive Variant",
            &[],
            "{\"layer\":variant artboard id? (default active artboard)}",
            has_artboards,
            delete_variant
        ),
        spec!(
            "layer.artboard.variant.rename",
            "Rename Responsive Variant",
            &[],
            "{\"layer\":variant artboard id?,\"name\":1..256 UTF-8 bytes}",
            has_artboards,
            rename_variant
        ),
        spec!(
            "layer.artboard.layoutRule.reset",
            "Reset Variant Layout Override",
            &[],
            "{\"layer\":variant layer ID? (default active layer)}; remove this variant's override and inherit the master rule",
            has_active_artboard,
            reset_variant_rule
        ),
        spec!(
            "layer.artboard.variants.update",
            "Update Responsive Variants",
            &[],
            "{\"layer\":master artboard ID? (default saved master)}; refresh every mapped layer while keeping variant layer IDs, backgrounds, semantic-role edits, and per-layer rule overrides",
            has_artboards,
            update_variants
        ),
        spec!(
            "layer.artboard.safeArea.set",
            "Set Artboard Safe Area",
            &[],
            "{\"layer\":artboard id?,\"name\":str,\"top\":0..0.5,\"right\":0..0.5,\"bottom\":0..0.5,\"left\":0..0.5,\"requiredRoles\":[role]?}",
            has_artboards,
            set_safe_area
        ),
        spec!(
            "layer.artboard.safeArea.delete",
            "Delete Artboard Safe Area",
            &[],
            "{\"layer\":artboard id?,\"name\":safe-area name}",
            has_artboards,
            delete_safe_area
        ),
        spec!(
            "view.clearSelectedArtboardGuides",
            "Clear Selected Artboard Guides",
            &["View"],
            "{} (guides inside the active artboard)",
            has_active_artboard,
            |s, _| clear_artboard_guides(s)
        ),
        spec!(
            "layer.artboard.addBeside",
            "Add Artboard Beside",
            &[],
            r##"{"layer":id? (default: the active artboard),"side":"left|right|top|bottom"="right","duplicate":bool=false (copy the board with its contents instead of an empty one)} → {layer, rect} (the Artboard tool's "+" buttons: same size, a 100 px gap away, past any board in the way)"##,
            has_active_artboard,
            add_beside
        ),
        spec!(
            "file.export.artboardsToFiles",
            "Artboards to Files…",
            &["File", "Export"],
            r##"{"dir":folder,"format":"png|jpg|psd|tiff|…"="png","prefix":str=document name ("" = none),"artboards":[id]? (default all),"quality":0..12?} → {files}"##,
            export_artboards,
            artboards_to_files
        ),
        spec!(
            "file.export.artboardsToPdf",
            "Artboards to PDF…",
            &["File", "Export"],
            r##"{"path":str (.pdf),"artboards":[id]?,"quality":0..12=10} → {path, pages} (one raster page per board)"##,
            export_artboards,
            artboards_to_pdf
        ),
    ];
    specs.extend([
        CommandSpec {
            id: "layer.artboard.variant.next",
            label: "Next Artboard Variant",
            menu: &["View"],
            shortcut: Some("Cmd+Alt+PageDown"),
            params: "{} → select the next artboard in the active responsive set (master, then its variants)",
            enabled: has_artboards,
            run: next_responsive_variant,
            journal: false,
        },
        CommandSpec {
            id: "layer.artboard.variant.previous",
            label: "Previous Artboard Variant",
            menu: &["View"],
            shortcut: Some("Cmd+Alt+PageUp"),
            params: "{} → select the previous artboard in the active responsive set (master, then its variants)",
            enabled: has_artboards,
            run: previous_responsive_variant,
            journal: false,
        },
    ]);
    specs.push(CommandSpec {
        id: "layer.artboard.diagnose",
        label: "Diagnose Artboard Layout",
        menu: &[],
        shortcut: None,
        params: "{\"layer\":artboard id? (default: active artboard)} → {overflow, collisions, safeAreas, revision}",
        enabled: has_artboards,
        run: diagnose_artboard,
        journal: false,
    });
    specs
}

#[cfg(test)]
#[path = "artboard_cmds/tests.rs"]
mod tests;
