//! Portable, versioned responsive-layout templates.
//!
//! Templates intentionally identify layers by their child-index path inside an artboard, never
//! by document-local layer IDs. The commands exchange JSON data so UI, CLI and automation can
//! choose their own file picker/storage without introducing filesystem access into the engine.

use photocraft_doc::responsive::{Anchor, ArtboardSafeArea, Dimensions, LayerLayoutRule, ResizeMode, SafeInsets, StackRule, TextFlow};
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::artboard_cmds::{SEMANTIC_ROLES, explicit_semantic_role};
use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session};

const CMD_EXPORT: &str = "file.export.responsiveTemplate";
const CMD_IMPORT: &str = "file.import.responsiveTemplate";
const TEMPLATE_VERSION: u32 = 1;
const MAX_TEMPLATE_BYTES: usize = 1_048_576;
const MAX_ENTRIES: usize = 10_000;
const MAX_PATH_DEPTH: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Template {
    schema_version: u32,
    name: String,
    artboard: String,
    layers: Vec<LayerTemplate>,
    #[serde(default)]
    safe_areas: Vec<SafeAreaTemplate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LayerTemplate {
    path: Vec<usize>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    rule: Option<RuleTemplate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuleTemplate {
    #[serde(default)]
    anchor: Anchor,
    #[serde(default)]
    mode: ResizeMode,
    #[serde(default)]
    min_size: Option<Dimensions>,
    #[serde(default)]
    max_size: Option<Dimensions>,
    #[serde(default)]
    text_flow: TextFlow,
    #[serde(default)]
    min_font_size_pt: Option<f32>,
    #[serde(default)]
    stack: Option<StackRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SafeAreaTemplate {
    name: String,
    insets: SafeInsets,
    #[serde(default)]
    required_roles: Vec<String>,
}

fn bad(cmd: &str, message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.to_owned(), msg: message.into() }
}

fn selected_artboard(doc: &Document, params: &Value, cmd: &str) -> Result<LayerId> {
    let id = match params.get("layer") {
        Some(value) => LayerId(value.as_u64().ok_or_else(|| bad(cmd, "`layer` must be an unsigned layer ID"))?),
        None => return Err(bad(cmd, "`layer` is required")),
    };
    let board = doc.layer(id).and_then(Layer::artboard).map(|_| id).or_else(|| doc.artboard_of(id));
    board.ok_or_else(|| bad(cmd, "select an artboard or a layer inside one"))
}

fn content_layers(layer: &Layer) -> &[Layer] {
    match &layer.content {
        LayerContent::Group(group) => &group.children,
        _ => &[],
    }
}

fn walk_layers(layers: &[Layer], path: &mut Vec<usize>, out: &mut Vec<(Vec<usize>, LayerId)>) -> Result<()> {
    if path.len() >= MAX_PATH_DEPTH && !layers.is_empty() {
        return Err(bad(CMD_EXPORT, "artboard nesting exceeds the 256 layer path limit"));
    }
    for (index, layer) in layers.iter().enumerate() {
        if out.len() >= MAX_ENTRIES {
            return Err(bad(CMD_EXPORT, "artboard exceeds the 10,000 layer template limit"));
        }
        path.push(index);
        out.push((path.clone(), layer.id));
        walk_layers(content_layers(layer), path, out)?;
        path.pop();
    }
    Ok(())
}

fn find_path<'a>(board: &'a Layer, path: &[usize]) -> Option<&'a Layer> {
    if path.len() > MAX_PATH_DEPTH {
        return None;
    }
    let mut children = content_layers(board);
    let mut found = None;
    for index in path {
        found = children.get(*index);
        children = content_layers(found?);
    }
    found
}

fn validate_rule(rule: &RuleTemplate, cmd: &str) -> Result<()> {
    if let (Some(min), Some(max)) = (rule.min_size, rule.max_size)
        && (min.width == 0 || min.height == 0 || max.width == 0 || max.height == 0 || min.width > max.width || min.height > max.height)
    {
        return Err(bad(cmd, "rule size limits must be positive and maximum must be at least minimum"));
    }
    if rule.min_size.is_some_and(|size| size.width == 0 || size.height == 0)
        || rule.max_size.is_some_and(|size| size.width == 0 || size.height == 0)
        || rule.min_font_size_pt.is_some_and(|v| !v.is_finite() || !(0.1..=1000.0).contains(&v))
        || rule.stack.as_ref().is_some_and(|stack| !stack.gap.is_finite() || !(0.0..=1_000_000.0).contains(&stack.gap))
    {
        return Err(bad(cmd, "rule contains an invalid size, font size, or stack gap"));
    }
    Ok(())
}

fn export_template(session: &mut Session, params: &Value) -> Result<Value> {
    let cmd = CMD_EXPORT;
    let doc = &session.active().ok_or(EngineError::NoDocument)?.doc;
    let board_id = selected_artboard(doc, params, cmd)?;
    let board = doc.layer(board_id).ok_or(EngineError::NoLayer(board_id))?;
    let name = params.get("name").and_then(Value::as_str).unwrap_or(&board.name);
    if name.trim().is_empty() || name.len() > 256 {
        return Err(bad(cmd, "`name` must contain 1..256 UTF-8 bytes"));
    }
    let mut paths = Vec::new();
    walk_layers(content_layers(board), &mut Vec::new(), &mut paths)?;
    let mut layers = Vec::new();
    let is_variant = doc.responsive.variants.iter().any(|variant| variant.artboard == board_id);
    for (path, id) in paths {
        let role = explicit_semantic_role(doc, id).filter(|role| *role != "auto").map(str::to_owned);
        let source_rule = if is_variant {
            doc.responsive.overrides.iter().find(|override_rule| override_rule.variant == board_id && override_rule.layer == id).map(|item| &item.rule)
        } else {
            doc.responsive.rules.iter().find(|rule| rule.layer == id)
        };
        let rule = source_rule.map(|rule| RuleTemplate {
            anchor: rule.anchor,
            mode: rule.mode,
            min_size: rule.min_size,
            max_size: rule.max_size,
            text_flow: rule.text_flow,
            min_font_size_pt: rule.min_font_size_pt,
            stack: rule.stack.clone(),
        });
        if role.is_some() || rule.is_some() {
            layers.push(LayerTemplate { path, role, rule });
        }
    }
    let safe_areas = doc
        .responsive
        .safe_areas
        .iter()
        .filter(|area| area.artboard == board_id)
        .map(|area| SafeAreaTemplate { name: area.name.clone(), insets: area.insets, required_roles: area.required_roles.clone() })
        .collect();
    let layer_count = layers.len();
    let template = Template { schema_version: TEMPLATE_VERSION, name: name.to_owned(), artboard: board.name.clone(), layers, safe_areas };
    let data = serde_json::to_value(template).map_err(|error| EngineError::Other(format!("could not encode responsive template: {error}")))?;
    let bytes = serde_json::to_vec(&data).map_err(|error| EngineError::Other(format!("could not encode responsive template: {error}")))?.len();
    if bytes > MAX_TEMPLATE_BYTES {
        return Err(bad(cmd, "template exceeds the 1 MiB limit"));
    }
    Ok(json!({"template": data, "bytes": bytes, "layers": layer_count}))
}

fn import_template(session: &mut Session, params: &Value) -> Result<Value> {
    let cmd = CMD_IMPORT;
    let raw = params.get("template").ok_or_else(|| bad(cmd, "`template` JSON is required"))?;
    let bytes = serde_json::to_vec(raw).map_err(|error| bad(cmd, error.to_string()))?.len();
    if bytes > MAX_TEMPLATE_BYTES {
        return Err(bad(cmd, "template exceeds the 1 MiB limit"));
    }
    let template: Template = serde_json::from_value(raw.clone()).map_err(|error| bad(cmd, format!("invalid template: {error}")))?;
    if template.schema_version != TEMPLATE_VERSION {
        return Err(bad(cmd, format!("unsupported template schema version {}", template.schema_version)));
    }
    if template.name.trim().is_empty()
        || template.name.len() > 256
        || template.layers.len() > MAX_ENTRIES
        || template.safe_areas.len() > photocraft_doc::responsive::MAX_RESPONSIVE_VARIANTS + 1
    {
        return Err(bad(cmd, "template name or entry count exceeds its limit"));
    }
    if template.layers.iter().any(|entry| entry.path.is_empty() || entry.path.len() > MAX_PATH_DEPTH)
        || template.layers.iter().filter_map(|entry| entry.role.as_deref()).any(|role| !SEMANTIC_ROLES.iter().any(|(known, _)| *known == role))
    {
        return Err(bad(cmd, "template contains an invalid path or semantic role"));
    }
    for entry in &template.layers {
        if let Some(rule) = &entry.rule {
            validate_rule(rule, cmd)?;
        }
    }
    for area in &template.safe_areas {
        let inset = area.insets;
        if area.name.trim().is_empty()
            || area.name.len() > 128
            || [inset.top, inset.right, inset.bottom, inset.left].iter().any(|v| !v.is_finite() || !(0.0..=0.5).contains(v))
            || area.required_roles.len() > SEMANTIC_ROLES.len()
            || area.required_roles.iter().any(|role| !SEMANTIC_ROLES.iter().any(|(known, _)| known == role))
        {
            return Err(bad(cmd, "template contains an invalid safe area"));
        }
    }
    let doc = &session.active().ok_or(EngineError::NoDocument)?.doc;
    let board_id = selected_artboard(doc, params, cmd)?;
    let board = doc.layer(board_id).ok_or(EngineError::NoLayer(board_id))?;
    let retained_areas = doc.responsive.safe_areas.iter().filter(|area| area.artboard != board_id).count();
    if retained_areas.saturating_add(template.safe_areas.len()) > photocraft_doc::responsive::MAX_RESPONSIVE_VARIANTS + 1 {
        return Err(bad(cmd, "applying this template would exceed the document safe-area limit"));
    }
    let is_variant = doc.responsive.variants.iter().any(|variant| variant.artboard == board_id);
    let mut matched = Vec::new();
    let mut unmatched = 0usize;
    for entry in &template.layers {
        if let Some(layer) = find_path(board, &entry.path) {
            matched.push((layer.id, entry));
        } else {
            unmatched += 1;
        }
    }
    let mut safe_areas: Vec<ArtboardSafeArea> = template
        .safe_areas
        .iter()
        .map(|area| ArtboardSafeArea { artboard: board_id, name: area.name.clone(), insets: area.insets, required_roles: area.required_roles.clone() })
        .collect();
    let result_count = matched.len();
    session.edit("Apply Responsive Template", |doc, _| {
        for (id, entry) in &matched {
            let key = format!("photocraft.artboard-role.{}", id.0);
            let source_key = format!("photocraft.artboard-role-source.{}", id.0);
            doc.metadata.text.retain(|(old, _)| old != &key && old != &source_key);
            if let Some(role) = &entry.role
                && role != "auto"
            {
                doc.metadata.text.push((key, role.clone()));
                doc.metadata.text.push((source_key, "manual".to_owned()));
            }
            let imported_rule = entry.rule.as_ref().map(|rule| LayerLayoutRule {
                layer: *id,
                anchor: rule.anchor,
                mode: rule.mode,
                min_size: rule.min_size,
                max_size: rule.max_size,
                text_flow: rule.text_flow,
                min_font_size_pt: rule.min_font_size_pt,
                stack: rule.stack.clone(),
            });
            if is_variant {
                doc.responsive.overrides.retain(|existing| existing.variant != board_id || existing.layer != *id);
                if let Some(rule) = imported_rule {
                    doc.responsive.overrides.push(photocraft_doc::responsive::LayerRuleOverride { variant: board_id, layer: *id, rule });
                }
            } else {
                doc.responsive.rules.retain(|rule| rule.layer != *id);
                if let Some(rule) = imported_rule {
                    doc.responsive.rules.push(rule);
                }
            }
        }
        doc.responsive.safe_areas.retain(|area| area.artboard != board_id);
        doc.responsive.safe_areas.append(&mut safe_areas);
        Ok(())
    })?;
    Ok(json!({"artboard": board_id.0, "applied": result_count, "unmatched": unmatched, "safeAreas": template.safe_areas.len()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CMD_EXPORT,
            label: "Export Responsive Template",
            menu: &[],
            shortcut: None,
            params: "{\"layer\":artboard id,\"name\":str?} → {template, bytes, layers}; JSON format version 1",
            enabled: always,
            run: export_template,
            journal: false,
        },
        CommandSpec {
            id: CMD_IMPORT,
            label: "Import Responsive Template",
            menu: &[],
            shortcut: None,
            params: "{\"layer\":target artboard id,\"template\":versioned JSON} → {applied, unmatched, safeAreas}",
            enabled: always,
            run: import_template,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use photocraft_doc::{Artboard, Document, Layer};

    fn session_with_board() -> (Session, LayerId, LayerId) {
        let mut session = Session::new();
        let mut doc =
            Document::new("template", photocraft_doc::Size { width: 100, height: 80 }, photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut board = Layer::group("Board", Vec::new());
        let board_id = board.id;
        let layer = Layer::group("Headline", Vec::new());
        let layer_id = layer.id;
        if let LayerContent::Group(group) = &mut board.content {
            group.artboard = Some(Artboard { rect: photocraft_geom::Rect::new(0, 0, 100, 80), background: Default::default(), preset: String::new() });
            group.children.push(layer);
        }
        doc.layers.push(board);
        session.add_document(doc, None);
        (session, board_id, layer_id)
    }

    #[test]
    fn export_and_import_template_maps_layer_ids_by_path() {
        let (mut source, source_board, source_layer) = session_with_board();
        let role_set = source.execute("layer.artboard.semanticRole.set", json!({"layer":source_layer.0,"role":"headline"}));
        let rule_set = source.execute("layer.artboard.layoutRule.set", json!({"layer":source_layer.0,"mode":"fit"}));
        let safe_area_set = source.execute(
            "layer.artboard.safeArea.set",
            json!({"layer":source_board.0,"name":"Story","top":0.1,"right":0.05,"bottom":0.1,"left":0.05,"requiredRoles":["headline"]}),
        );
        assert!(role_set.is_ok() && rule_set.is_ok() && safe_area_set.is_ok());
        let exported = export_template(&mut source, &json!({"layer": source_board.0, "name":"Social"}));
        let template = exported.ok().and_then(|value| value.get("template").cloned());
        assert!(template.is_some());
        let (mut target, target_board, target_layer) = session_with_board();
        let result = import_template(&mut target, &json!({"layer":target_board.0,"template":template}));
        assert!(result.is_ok());
        let doc = target.active().map(|state| &state.doc);
        assert!(doc.and_then(|doc| explicit_semantic_role(doc, target_layer)) == Some("headline"));
        assert!(doc.is_some_and(|doc| doc.responsive.rules.iter().any(|rule| rule.layer == target_layer && rule.mode == ResizeMode::Fit)));
        assert!(doc.is_some_and(|doc| {
            doc.responsive.safe_areas.iter().any(|area| area.artboard == target_board && area.name == "Story" && area.required_roles == ["headline"])
        }));
        assert_ne!(source_layer, target_layer);
        assert!(template.is_some());
    }

    #[test]
    fn import_rejects_unknown_version_and_invalid_insets() {
        let (mut session, board, _) = session_with_board();
        let unknown = json!({"schemaVersion":2,"name":"x","artboard":"x","layers":[],"safeAreas":[]});
        assert!(import_template(&mut session, &json!({"layer":board.0,"template":unknown})).is_err());
        let invalid = json!({"schemaVersion":1,"name":"x","artboard":"x","layers":[],"safeAreas":[{"name":"bad","insets":{"top":0.8,"right":0,"bottom":0,"left":0},"requiredRoles":[]}]});
        assert!(import_template(&mut session, &json!({"layer":board.0,"template":invalid})).is_err());
    }

    #[test]
    fn importing_into_a_variant_creates_an_override_instead_of_a_master_rule() {
        let (mut session, board, layer) = session_with_board();
        let added = session.edit("fixture", |doc, _| {
            doc.responsive.variants.push(photocraft_doc::responsive::ResponsiveVariant {
                artboard: board,
                master: board,
                name: "Variant".into(),
                layers: Vec::new(),
            });
            Ok(())
        });
        assert!(added.is_ok());
        let template = json!({"schemaVersion":1,"name":"Social","artboard":"Board","layers":[{"path":[0],"rule":{"mode":"fit"}}],"safeAreas":[]});
        assert!(import_template(&mut session, &json!({"layer":board.0,"template":template})).is_ok());
        let doc = session.active().map(|state| &state.doc);
        assert!(
            doc.is_some_and(|doc| doc
                .responsive
                .overrides
                .iter()
                .any(|rule| rule.variant == board && rule.layer == layer && rule.rule.mode == ResizeMode::Fit))
        );
        assert!(doc.is_some_and(|doc| !doc.responsive.rules.iter().any(|rule| rule.layer == layer)));
    }

    #[test]
    fn export_rejects_layer_paths_deeper_than_the_template_limit() {
        let (mut session, board, _) = session_with_board();
        let mut nested = Layer::group("Deepest", Vec::new());
        for index in 0..MAX_PATH_DEPTH {
            nested = Layer::group(format!("Group {index}"), vec![nested]);
        }
        let added = session.edit("deep fixture", |doc, _| {
            let Some(board) = doc.layer_mut(board) else {
                return Err(EngineError::Other("test artboard disappeared".into()));
            };
            let LayerContent::Group(group) = &mut board.content else {
                return Err(EngineError::Other("test artboard is not a group".into()));
            };
            group.children.push(nested);
            Ok(())
        });
        assert!(added.is_ok());
        assert!(export_template(&mut session, &json!({"layer":board.0})).is_err());
    }
}
