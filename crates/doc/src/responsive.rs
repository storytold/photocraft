//! Persistent responsive-artboard rules. This module stores the layout program as document data;
//! solving and diagnostics live in the engine and therefore remain shared by UI, CLI and agents.

use serde::{Deserialize, Serialize};

use crate::LayerId;

pub const RESPONSIVE_SCHEMA_VERSION: u32 = 1;
pub const MAX_RESPONSIVE_VARIANTS: usize = 64;
pub const MAX_RESPONSIVE_SAFE_AREAS: usize = 65;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HorizontalAnchor {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAnchor {
    #[default]
    Top,
    Center,
    Bottom,
}

/// Horizontal and vertical anchor positions as a stable 3×3 semantic point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    #[serde(default)]
    pub horizontal: HorizontalAnchor,
    #[serde(default)]
    pub vertical: VerticalAnchor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResizeMode {
    /// Keep the original dimensions and move according to the selected anchor.
    #[default]
    Fixed,
    /// Stretch to preserve the distances from both opposing edges.
    Stretch,
    /// Uniformly cover the target frame; excess content may overflow.
    Fill,
    /// Uniformly fit inside the target frame.
    Fit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFlow {
    #[default]
    Preserve,
    Rewrap,
    /// Reflow paragraph text to the target width and grow the box to fit the measured lines.
    RewrapAutoHeight,
    RewrapAndShrink,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StackDirection {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StackAlignment {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StackRule {
    #[serde(default)]
    pub direction: StackDirection,
    #[serde(default)]
    pub alignment: StackAlignment,
    /// Gap in document pixels; validated as finite and non-negative by the engine.
    pub gap: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerLayoutRule {
    pub layer: LayerId,
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default)]
    pub mode: ResizeMode,
    #[serde(default)]
    pub min_size: Option<Dimensions>,
    #[serde(default)]
    pub max_size: Option<Dimensions>,
    #[serde(default)]
    pub text_flow: TextFlow,
    /// Lower font-size bound (points) for `rewrap_and_shrink` overflow fallback.
    #[serde(default)]
    pub min_font_size_pt: Option<f32>,
    #[serde(default)]
    pub stack: Option<StackRule>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerRuleOverride {
    pub variant: LayerId,
    pub layer: LayerId,
    pub rule: LayerLayoutRule,
}

/// Insets are fractions of the board dimensions, each constrained to `0..=0.5`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SafeInsets {
    #[serde(default)]
    pub top: f32,
    #[serde(default)]
    pub right: f32,
    #[serde(default)]
    pub bottom: f32,
    #[serde(default)]
    pub left: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtboardSafeArea {
    pub artboard: LayerId,
    pub name: String,
    #[serde(default)]
    pub insets: SafeInsets,
    /// Interactive semantic roles required to remain inside this zone.
    #[serde(default)]
    pub required_roles: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsiveVariant {
    pub artboard: LayerId,
    pub master: LayerId,
    #[serde(default)]
    pub name: String,
    /// Stable correspondence from master layers to this variant's duplicated layers.
    #[serde(default)]
    pub layers: Vec<LayerMapping>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerMapping {
    pub master_layer: LayerId,
    pub variant_layer: LayerId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponsiveLayout {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub master: Option<LayerId>,
    #[serde(default)]
    pub variants: Vec<ResponsiveVariant>,
    #[serde(default)]
    pub rules: Vec<LayerLayoutRule>,
    #[serde(default)]
    pub overrides: Vec<LayerRuleOverride>,
    #[serde(default)]
    pub safe_areas: Vec<ArtboardSafeArea>,
}

const fn schema_version() -> u32 {
    RESPONSIVE_SCHEMA_VERSION
}

impl Default for ResponsiveLayout {
    fn default() -> Self {
        Self { schema_version: RESPONSIVE_SCHEMA_VERSION, master: None, variants: Vec::new(), rules: Vec::new(), overrides: Vec::new(), safe_areas: Vec::new() }
    }
}

impl ResponsiveLayout {
    pub fn remove_layers(&mut self, ids: &std::collections::HashSet<LayerId>) {
        if self.master.is_some_and(|id| ids.contains(&id)) {
            self.master = None;
        }
        self.variants.retain(|variant| !ids.contains(&variant.artboard) && !ids.contains(&variant.master));
        for variant in &mut self.variants {
            variant.layers.retain(|mapping| !ids.contains(&mapping.master_layer) && !ids.contains(&mapping.variant_layer));
        }
        self.rules.retain(|rule| !ids.contains(&rule.layer));
        self.overrides.retain(|entry| !ids.contains(&entry.variant) && !ids.contains(&entry.layer) && !ids.contains(&entry.rule.layer));
        self.safe_areas.retain(|area| !ids.contains(&area.artboard));
    }

    /// Check resource bounds and numeric invariants before accepting persisted data.
    pub fn validate(&self, layer_count: usize) -> std::result::Result<(), &'static str> {
        use std::collections::HashSet;
        if self.schema_version != RESPONSIVE_SCHEMA_VERSION {
            return Err("unsupported responsive layout schema version");
        }
        if self.variants.len() > MAX_RESPONSIVE_VARIANTS {
            return Err("too many responsive variants");
        }
        let mut variant_ids = HashSet::with_capacity(self.variants.len());
        for variant in &self.variants {
            if !variant_ids.insert(variant.artboard) {
                return Err("duplicate responsive variant artboard");
            }
            let mut master_layers = HashSet::with_capacity(variant.layers.len());
            let mut variant_layers = HashSet::with_capacity(variant.layers.len());
            for mapping in &variant.layers {
                if !master_layers.insert(mapping.master_layer) || !variant_layers.insert(mapping.variant_layer) {
                    return Err("duplicate responsive layer mapping");
                }
            }
        }
        if self.rules.len() > layer_count {
            return Err("responsive rule count exceeds layer count");
        }
        let mut rule_layers = HashSet::with_capacity(self.rules.len());
        if self.rules.iter().any(|rule| !rule_layers.insert(rule.layer)) {
            return Err("duplicate responsive layer rule");
        }
        let map_limit = MAX_RESPONSIVE_VARIANTS.saturating_mul(layer_count);
        if self.variants.iter().map(|variant| variant.layers.len()).fold(0usize, usize::saturating_add) > map_limit {
            return Err("responsive layer mapping count exceeds limit");
        }
        let max_overrides = MAX_RESPONSIVE_VARIANTS.saturating_mul(layer_count);
        if self.overrides.len() > max_overrides {
            return Err("responsive override count exceeds limit");
        }
        let mut override_pairs = HashSet::with_capacity(self.overrides.len());
        for entry in &self.overrides {
            if !variant_ids.contains(&entry.variant) || entry.rule.layer != entry.layer || !override_pairs.insert((entry.variant, entry.layer)) {
                return Err("invalid or duplicate responsive layer override");
            }
        }
        if self.safe_areas.len() > MAX_RESPONSIVE_SAFE_AREAS {
            return Err("too many artboard safe areas");
        }
        for rule in self.rules.iter().chain(self.overrides.iter().map(|o| &o.rule)) {
            if let (Some(min), Some(max)) = (rule.min_size, rule.max_size)
                && (min.width > max.width || min.height > max.height)
            {
                return Err("responsive maximum size is smaller than minimum size");
            }
            if rule.min_font_size_pt.is_some_and(|v| !v.is_finite() || v <= 0.0) {
                return Err("invalid minimum font size");
            }
            if rule.stack.as_ref().is_some_and(|s| !s.gap.is_finite() || !(0.0..=1_000_000.0).contains(&s.gap)) {
                return Err("invalid stack gap");
            }
        }
        let mut safe_area_names = HashSet::with_capacity(self.safe_areas.len());
        for safe in &self.safe_areas {
            if !safe_area_names.insert((safe.artboard, safe.name.as_str())) {
                return Err("duplicate safe-area name on artboard");
            }
            let i = safe.insets;
            if [i.top, i.right, i.bottom, i.left].into_iter().any(|v| !v.is_finite() || !(0.0..=0.5).contains(&v)) {
                return Err("safe-area insets must be finite fractions between zero and one half");
            }
            if safe.name.len() > 256 || safe.required_roles.len() > 64 || safe.required_roles.iter().any(|r| r.len() > 64) {
                return Err("safe-area metadata exceeds limits");
            }
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.master.is_none() && self.variants.is_empty() && self.rules.is_empty() && self.overrides.is_empty() && self.safe_areas.is_empty()
    }

    /// Rewrites every stored layer reference after `.pcraft` loading or layer duplication.
    pub fn remap_ids(&mut self, map: &std::collections::HashMap<LayerId, LayerId>) {
        self.master = self.master.and_then(|id| map.get(&id).copied());
        self.variants.retain_mut(|variant| {
            let Some(artboard) = map.get(&variant.artboard).copied() else { return false };
            let Some(master) = map.get(&variant.master).copied() else { return false };
            variant.artboard = artboard;
            variant.master = master;
            variant.layers.retain_mut(|mapping| {
                let (Some(master_layer), Some(variant_layer)) = (map.get(&mapping.master_layer), map.get(&mapping.variant_layer)) else { return false };
                mapping.master_layer = *master_layer;
                mapping.variant_layer = *variant_layer;
                true
            });
            true
        });
        self.rules.retain_mut(|rule| {
            let Some(layer) = map.get(&rule.layer).copied() else { return false };
            rule.layer = layer;
            true
        });
        self.overrides.retain_mut(|entry| {
            let (Some(variant), Some(layer), Some(rule_layer)) = (map.get(&entry.variant), map.get(&entry.layer), map.get(&entry.rule.layer)) else {
                return false;
            };
            entry.variant = *variant;
            entry.layer = *layer;
            entry.rule.layer = *rule_layer;
            true
        });
        self.safe_areas.retain_mut(|area| {
            let Some(id) = map.get(&area.artboard).copied() else { return false };
            area.artboard = id;
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_empty_layout_defaults_to_current_schema() {
        let layout: ResponsiveLayout = serde_json::from_str("{}").unwrap();
        assert_eq!(layout.schema_version, RESPONSIVE_SCHEMA_VERSION);
        assert!(layout.is_empty());
    }

    #[test]
    fn validates_bounds_and_rejects_non_finite_values() {
        let mut layout = ResponsiveLayout::default();
        layout.safe_areas.push(ArtboardSafeArea {
            artboard: LayerId(1),
            name: "social".into(),
            insets: SafeInsets { top: f32::NAN, ..SafeInsets::default() },
            required_roles: vec![],
        });
        assert!(layout.validate(1).is_err());
        let mut layout = ResponsiveLayout::default();
        layout
            .variants
            .resize(MAX_RESPONSIVE_VARIANTS + 1, ResponsiveVariant { artboard: LayerId(1), master: LayerId(1), name: String::new(), layers: Vec::new() });
        assert!(layout.validate(1).is_err());
        let mut layout = ResponsiveLayout::default();
        layout.safe_areas.resize(
            MAX_RESPONSIVE_SAFE_AREAS + 1,
            ArtboardSafeArea { artboard: LayerId(1), name: "safe".into(), insets: SafeInsets::default(), required_roles: vec![] },
        );
        assert!(layout.validate(1).is_err());
        let mut layout = ResponsiveLayout::default();
        let area = ArtboardSafeArea { artboard: LayerId(1), name: "same".into(), insets: SafeInsets::default(), required_roles: vec![] };
        layout.safe_areas = vec![area.clone(), area];
        assert!(layout.validate(1).is_err());
    }

    #[test]
    fn id_remapping_prunes_dangling_references() {
        let mut layout = ResponsiveLayout { master: Some(LayerId(10)), ..ResponsiveLayout::default() };
        layout.variants.push(ResponsiveVariant { artboard: LayerId(11), master: LayerId(10), name: "square".into(), layers: Vec::new() });
        layout.rules.push(LayerLayoutRule {
            layer: LayerId(12),
            anchor: Anchor::default(),
            mode: ResizeMode::Fixed,
            min_size: None,
            max_size: None,
            text_flow: TextFlow::Preserve,
            min_font_size_pt: None,
            stack: None,
        });
        let map = [(LayerId(10), LayerId(20)), (LayerId(11), LayerId(21))].into_iter().collect();
        layout.remap_ids(&map);
        assert_eq!(layout.master, Some(LayerId(20)));
        assert_eq!(layout.variants[0].master, LayerId(20));
        assert!(layout.rules.is_empty());
    }

    #[test]
    fn deleting_a_master_prunes_dependent_variant_and_rules() {
        let mut layout = ResponsiveLayout { master: Some(LayerId(1)), ..ResponsiveLayout::default() };
        layout.variants.push(ResponsiveVariant { artboard: LayerId(2), master: LayerId(1), name: "variant".into(), layers: Vec::new() });
        layout.rules.push(LayerLayoutRule {
            layer: LayerId(3),
            anchor: Anchor::default(),
            mode: ResizeMode::Fixed,
            min_size: None,
            max_size: None,
            text_flow: TextFlow::Preserve,
            min_font_size_pt: None,
            stack: None,
        });
        layout.remove_layers(&[LayerId(1), LayerId(3)].into_iter().collect());
        assert!(layout.master.is_none());
        assert!(layout.variants.is_empty());
        assert!(layout.rules.is_empty());
    }
}
