//! Stored XCF configuration decoded as bounded data, then translated to native filters.
//!
//! Source operation names are never executable command identifiers. The import policy
//! maps an exact supported identity to an ordinary native command and parameters.

use photocraft_doc::SmartFilter;

mod color;
mod config;
mod core_composites;
mod curves_and_quantization;
mod envelope;
mod graph;
mod graph_composites;
mod graph_core;
mod graph_displacement;
mod graph_math;
mod graph_objects;
mod graph_porter;
mod graph_resources;
mod graph_schema;
mod graph_stock;
mod graph_tone;
mod residual_coverage;
mod residual_geometry;
mod residual_render;
mod residual_spatial;
mod residual_tone;
mod residual_transform;
mod resources;
mod schema;
mod spatial;
mod tone;

pub(super) use config::parse_config;
pub(super) use envelope::{decode, read_pointers};
pub(super) use resources::literal_rgb;

/// Owner-local geometry after native embedding, and the native document policy.
pub(super) struct MappingContext<'a> {
    pub mode: photocraft_color::ColorMode,
    pub linear: bool,
    pub owner_bounds: photocraft_geom::Rect,
    /// Bounds the native filter evaluates, after any ordinary owner-local embedding.
    pub native_filter_bounds: photocraft_geom::Rect,
    pub target_profile: Option<std::sync::Arc<Vec<u8>>>,
    /// Resolution of the actual native construction frame, for pixel-to-point type.
    pub resolution_dpi: f32,
    /// Existing import allowance remaining before any generated resource allocation.
    pub remaining_generation_bytes: usize,
    /// Existing copied/native parameter allowance, before encoding generated resources.
    pub remaining_metadata_bytes: usize,
    pub interrupt: photocraft_raster::Interrupt<'a>,
}

/// One parsed effect; file order and argument order remain explicit.
#[derive(Debug)]
pub(super) struct StoredEffect<'a> {
    pub name: &'a str,
    pub icon: &'a str,
    pub operation: &'a str,
    pub version: Option<&'a str>,
    pub arguments: Vec<Argument<'a>>,
    pub visible: bool,
    pub opacity: f32,
    pub mode: i32,
    pub blend_space: i32,
    pub composite_space: i32,
    pub composite_mode: i32,
    pub clip: bool,
    pub region: u32,
    pub mask_offset: usize,
    pub record_offset: usize,
    pub raw: &'a [u8],
    pub unknown_properties: Vec<SourceProperty<'a>>,
    pub errors: Vec<String>,
}

#[derive(Debug)]
pub(super) struct SourceProperty<'a> {
    pub offset: usize,
    pub kind: u32,
    pub bytes: &'a [u8],
}

#[derive(Debug)]
pub(super) struct Argument<'a> {
    pub name: &'a str,
    pub value: ArgumentValue<'a>,
}

#[derive(Debug)]
pub(super) enum ArgumentValue<'a> {
    Int(i32),
    UInt(u32),
    Bool(bool),
    Float(f32),
    /// Numeric graph literals are text, not an XCF binary32 argument.
    Literal(f64),
    /// Portable graph text colors, with an explicit source transfer encoding.
    /// Never constructed by the binary XCF argument decoder.
    LiteralColor {
        rgba: [f32; 4],
        linear: bool,
    },
    String(&'a str),
    Enum(i32),
    Config(Config<'a>),
    Color(Option<ColorLiteral<'a>>),
    Unknown {
        kind: u32,
        bytes: &'a [u8],
    },
    Invalid {
        kind: u32,
        bytes: &'a [u8],
        message: String,
    },
}

/// Typed source color bytes and their own profile. No implicit document ICC.
#[derive(Debug)]
pub(super) struct ColorLiteral<'a> {
    pub encoding: &'a str,
    pub pixels: &'a [u8],
    pub profile: &'a [u8],
}

/// A bounded configuration syntax tree. It contains no commands or evaluation API.
#[derive(Debug)]
pub(super) struct Config<'a> {
    pub raw: &'a str,
    pub forms: Vec<ConfigExpr<'a>>,
}

#[derive(Debug)]
pub(super) enum ConfigExpr<'a> {
    Symbol(&'a str),
    String(String),
    /// GimpConfig quotes may carry escaped binary color/profile data. These are
    /// bytes, not source text or executable instructions.
    Bytes(Vec<u8>),
    Number(f64),
    List(Vec<ConfigExpr<'a>>),
}

impl<'a> ConfigExpr<'a> {
    pub fn as_symbol(&self) -> Option<&'a str> {
        if let Self::Symbol(value) = self { Some(value) } else { None }
    }
    pub fn as_number(&self) -> Option<f64> {
        if let Self::Number(value) = self { Some(*value) } else { None }
    }
    pub fn as_list(&self) -> Option<&[Self]> {
        if let Self::List(value) = self { Some(value) } else { None }
    }
    pub fn head(&self) -> Option<&'a str> {
        self.as_list()?.first()?.as_symbol()
    }
}

impl<'a> Config<'a> {
    /// Repeated groups remain ordered, including channel/curve/levels groups.
    pub fn fields(&self, name: &str) -> impl Iterator<Item = &[ConfigExpr<'a>]> {
        self.forms.iter().filter_map(move |form| {
            let list = form.as_list()?;
            if list.first()?.as_symbol() == Some(name) { list.get(1..) } else { None }
        })
    }
}

impl<'a> StoredEffect<'a> {
    /// Resolve only the frozen compatibility aliases for one-time native mapping.
    /// Keep `operation` itself unchanged for source identity and historical reporting.
    pub fn canonical_operation(&self) -> &str {
        match self.operation {
            "gegl:abs" => "gegl:absolute",
            "gegl:color-reduction" => "gegl:dither",
            "gegl:invert" => "gegl:invert-linear",
            "gegl:motion-blur" => "gegl:motion-blur-linear",
            "gegl:little-planet" => "gegl:stereographic-projection",
            operation => operation,
        }
    }
    #[cfg(test)]
    pub fn test(operation: &'a str, arguments: Vec<Argument<'a>>) -> Self {
        Self {
            name: "",
            icon: "",
            operation,
            version: None,
            arguments,
            visible: true,
            opacity: 1.0,
            mode: 0,
            blend_space: 0,
            composite_space: 0,
            composite_mode: 0,
            clip: false,
            region: 0,
            mask_offset: 0,
            record_offset: 0,
            raw: &[],
            unknown_properties: Vec::new(),
            errors: Vec::new(),
        }
    }
    pub fn argument(&self, name: &str) -> std::result::Result<Option<&ArgumentValue<'a>>, String> {
        let mut matches = self.arguments.iter().filter(|arg| arg.name == name);
        let first = matches.next().map(|arg| &arg.value);
        if matches.next().is_some() {
            return Err(format!("duplicate stored effect argument {name}"));
        }
        Ok(first)
    }
    pub fn number(&self, name: &str) -> std::result::Result<Option<f64>, String> {
        match self.argument(name)? {
            None => Ok(None),
            Some(ArgumentValue::Float(value)) => Ok(Some(f64::from(*value))),
            Some(ArgumentValue::Literal(value)) => Ok(Some(*value)),
            Some(ArgumentValue::Int(value)) => Ok(Some(f64::from(*value))),
            Some(ArgumentValue::UInt(value)) => Ok(Some(f64::from(*value))),
            _ => Err(format!("effect argument {name} is not a number")),
        }
    }
    pub fn boolean(&self, name: &str) -> std::result::Result<Option<bool>, String> {
        match self.argument(name)? {
            None => Ok(None),
            Some(ArgumentValue::Bool(value)) => Ok(Some(*value)),
            _ => Err(format!("effect argument {name} is not a boolean")),
        }
    }
    pub fn enum_value(&self, name: &str) -> std::result::Result<Option<i32>, String> {
        match self.argument(name)? {
            None => Ok(None),
            Some(ArgumentValue::Enum(value)) => Ok(Some(*value)),
            _ => Err(format!("effect argument {name} is not an enum")),
        }
    }
    pub fn config(&self, name: &str) -> std::result::Result<Option<&Config<'a>>, String> {
        match self.argument(name)? {
            None => Ok(None),
            Some(ArgumentValue::Config(value)) => Ok(Some(value)),
            _ => Err(format!("effect argument {name} is not configuration text")),
        }
    }
}

/// Native commands only. Each helper owns an exact finite identity batch.
#[derive(Debug, Default)]
pub(super) struct NativeMapping {
    /// Transient request for ordinary RGB Smart contents inside a Gray owner.
    /// Materialized as native document mode/profile; never persisted as a rule.
    pub working_rgb: bool,
    pub filters: Vec<SmartFilter>,
    /// Ordinary resources needed by generated native fills or filter structures.
    pub patterns: Vec<photocraft_doc::Pattern>,
    /// Ordinary native layer styles, evaluated at the native pipeline location.
    /// Mixed filter/style stage ordering requires an owner-local native wrapper.
    pub layer_effects: Vec<photocraft_doc::effects::Effect>,
    /// Transient native placement request, materialized as ordinary smart contents.
    /// No source operation identity is stored as a transform rule.
    pub quarter_turns: u8,
    /// Clockwise ordinary native placement; mutually exclusive with quarter_turns.
    pub rotation_degrees: f64,
    /// Fixed native document construction for a mapping that needs more than a filter.
    /// Never serialized as an executable plan; the importer materializes native layers.
    pub structure: Option<NativeStructure>,
    pub notes: Vec<String>,
}

#[derive(Debug)]
pub(super) enum NativeStructure {
    Replace {
        layer: Box<photocraft_doc::Layer>,
        patterns: Vec<photocraft_doc::Pattern>,
    },
    Composite {
        branches: Vec<NativeBranch>,
    },
    /// Import-time construction only. Index zero is the current native input;
    /// each step adds one result and may reference only earlier native results.
    /// Source node names/text never enter this representation or native saves.
    Stages {
        steps: Vec<NativeStage>,
        output: usize,
    },
}

/// Ordinary native channel selection used only while constructing static data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeMaskComponent {
    Alpha,
    /// Native Apply Image composite policy: arithmetic mean of color channels.
    Composite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeMapCoordinates {
    Offsets,
    Positions,
}

#[derive(Debug)]
pub(super) enum NativeStage {
    /// One-time ordinary native alignment using actual earlier native extents.
    Align {
        input: usize,
        target: usize,
        horizontal: f64,
        vertical: f64,
        margin_x: f64,
        margin_y: f64,
        snap: bool,
    },
    /// Convert a captured native XY resource once into an embedded native Displace map.
    Displace {
        input: usize,
        map: usize,
        coordinates: NativeMapCoordinates,
        scale: f32,
        wrap: bool,
        metadata_limit: usize,
    },
    /// Capture an earlier native result as an ordinary opaque channel raster.
    Channel {
        input: usize,
        component: NativeMaskComponent,
    },
    /// Attach a static ordinary native pixel mask; no live source graph binding.
    Mask {
        input: usize,
        mask: usize,
        component: NativeMaskComponent,
        invert: bool,
        scale: f32,
    },
    Apply {
        input: usize,
        mapping: Box<NativeMapping>,
    },
    Join {
        below: usize,
        above: usize,
        blend: photocraft_color::BlendMode,
        opacity: f32,
    },
    /// Ordinary Photoshop-style knockout: upper transparency punches through
    /// the native backdrop while its fill is hidden. No source blend callback.
    Knockout {
        below: usize,
        above: usize,
        opacity: f32,
    },
}

pub(super) fn validate_stages(steps: &[NativeStage], output: usize) -> std::result::Result<(), MappingFailure> {
    if steps.len() > 64 || output > steps.len() {
        return Err("Native construction plan exceeds its step or output bounds".into());
    }
    for (index, step) in steps.iter().enumerate() {
        let available = index + 1; // Includes the original native input at zero.
        match step {
            NativeStage::Align { input, target, horizontal, vertical, margin_x, margin_y, .. } => {
                if *input >= available
                    || *target >= available
                    || !(-2.0..=3.0).contains(horizontal)
                    || !(-2.0..=3.0).contains(vertical)
                    || ![horizontal, vertical, margin_x, margin_y].iter().all(|value| value.is_finite())
                {
                    return Err("Native alignment requires earlier inputs and finite bounded controls".into());
                }
            }
            NativeStage::Displace { input, map, scale, metadata_limit, .. } => {
                if *input >= available
                    || *map >= available
                    || !scale.is_finite()
                    || !(0.0..=5000.0).contains(scale)
                    || *metadata_limit == 0
                    || *metadata_limit > super::MAX_METADATA_BYTES
                {
                    return Err("Native displacement requires earlier inputs and finite bounded scale".into());
                }
            }
            NativeStage::Channel { input, .. } => {
                if *input >= available {
                    return Err("Native channel capture requires an earlier input".into());
                }
            }
            NativeStage::Mask { input, mask, scale, .. } => {
                if *input >= available || *mask >= available || !scale.is_finite() || !(0.0..=1.0).contains(scale) {
                    return Err("Native pixel mask requires earlier inputs and finite normalized scale".into());
                }
            }
            NativeStage::Apply { input, mapping } => {
                if *input >= available || matches!(mapping.structure, Some(NativeStructure::Stages { .. })) {
                    return Err("Native construction requires earlier inputs and a flat stage list".into());
                }
            }
            NativeStage::Join { below, above, blend, opacity } => {
                if *below >= available
                    || *above >= available
                    || *blend == photocraft_color::BlendMode::PassThrough
                    || !opacity.is_finite()
                    || !(0.0..=1.0).contains(opacity)
                {
                    return Err("Native construction join requires earlier inputs, a layer blend mode and finite normalized opacity".into());
                }
            }
            NativeStage::Knockout { below, above, opacity } => {
                if *below >= available || *above >= available || !opacity.is_finite() || !(0.0..=1.0).contains(opacity) {
                    return Err("Native knockout requires earlier inputs and finite normalized opacity".into());
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct NativeBranch {
    pub filters: Vec<SmartFilter>,
    pub blend: photocraft_color::BlendMode,
    pub clipped: bool,
    pub opacity: f32,
    pub fill_opacity: f32,
    pub advanced: photocraft_doc::AdvancedBlending,
    /// Ordinary smart-object placement in owner-local native document coordinates.
    pub transform: photocraft_doc::Affine,
    /// Temporary owner-local native canvas. Input placement is rebased into it;
    /// filters run there, then ordinary outer placement restores this origin.
    pub filter_frame: Option<photocraft_geom::Rect>,
    /// Ordinary native Smart Object projective placement, in owner-local coordinates.
    pub perspective: Option<[f64; 9]>,
    /// Coverage lives in owner-local document coordinates, as on any native layer.
    pub mask: Option<photocraft_doc::LayerMask>,
    pub vector_mask: Option<photocraft_doc::VectorMask>,
    pub layer_effects: Vec<photocraft_doc::effects::Effect>,
    /// Styles applied to the completed native styled branch, through ordinary embedded contents.
    pub post_layer_effects: Vec<photocraft_doc::effects::Effect>,
    /// A direct native fill instead of another copy of the current input.
    pub fill: Option<photocraft_doc::Fill>,
}

impl Default for NativeBranch {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            blend: photocraft_color::BlendMode::Normal,
            clipped: false,
            opacity: 1.0,
            fill_opacity: 1.0,
            advanced: photocraft_doc::AdvancedBlending::default(),
            transform: photocraft_doc::Affine::IDENTITY,
            filter_frame: None,
            perspective: None,
            mask: None,
            vector_mask: None,
            layer_effects: Vec::new(),
            post_layer_effects: Vec::new(),
            fill: None,
        }
    }
}

/// None: identity belongs to another helper. Err: known identity cannot be safely
/// mapped for these arguments; preserve/report it without changing siblings.
pub(super) type MappingResult = Option<std::result::Result<NativeMapping, MappingFailure>>;

#[derive(Debug)]
pub(super) struct MappingFailure {
    pub reason: photocraft_doc::import_conversion::ConversionReason,
    pub message: String,
}

impl MappingFailure {
    pub fn resource(message: impl Into<String>) -> Self {
        Self { reason: photocraft_doc::import_conversion::ConversionReason::ResourceUnavailable, message: message.into() }
    }
}

impl From<String> for MappingFailure {
    fn from(message: String) -> Self {
        Self { reason: photocraft_doc::import_conversion::ConversionReason::MissingMapping, message }
    }
}

impl From<&str> for MappingFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

pub(super) fn native_filter(command: &str, params: serde_json::Value) -> SmartFilter {
    SmartFilter { command: command.into(), params, blend: photocraft_color::BlendMode::Normal, opacity: 1.0, visible: true }
}

pub(super) fn map(effect: &StoredEffect<'_>, context: &MappingContext) -> std::result::Result<NativeMapping, MappingFailure> {
    context
        .interrupt
        .check()
        .map_err(|error| MappingFailure { reason: photocraft_doc::import_conversion::ConversionReason::NativePolicy, message: error.to_string() })?;
    if !effect.errors.is_empty() || effect.arguments.iter().any(|argument| matches!(argument.value, ArgumentValue::Invalid { .. })) {
        return Err(MappingFailure {
            reason: photocraft_doc::import_conversion::ConversionReason::InvalidOptionalData,
            message: "Invalid stored effect configuration is retained without application".into(),
        });
    }
    let expected = if effect.operation == "gegl:exposure" { "1:0" } else { "0:0" };
    if effect.version.is_some_and(|version| version != expected) {
        return Err(format!("Stored operation version is not mapped for {}", effect.operation).into());
    }
    let mut seen = std::collections::HashSet::new();
    for argument in &effect.arguments {
        if !seen.insert(argument.name) {
            return Err(MappingFailure {
                reason: photocraft_doc::import_conversion::ConversionReason::InvalidOptionalData,
                message: format!("Duplicate stored operation argument {}", argument.name),
            });
        }
        if schema::fields(effect.operation).is_some_and(|fields| !fields.contains(&argument.name)) {
            return Err(format!("Stored operation argument {} is not mapped for {}", argument.name, effect.operation).into());
        }
    }
    tone::map(effect, context)
        .or_else(|| graph::map(effect, context))
        .or_else(|| curves_and_quantization::map(effect, context))
        .or_else(|| color::map(effect, context))
        .or_else(|| resources::map(effect, context))
        .or_else(|| spatial::map(effect, context))
        .or_else(|| residual_tone::map(effect, context))
        .or_else(|| residual_geometry::map(effect, context))
        .or_else(|| residual_spatial::map(effect, context))
        .or_else(|| core_composites::map(effect, context))
        .or_else(|| residual_transform::map(effect, context))
        .or_else(|| residual_coverage::map(effect, context))
        .or_else(|| residual_render::map(effect, context))
        .unwrap_or_else(|| Err(format!("Native mapping is unfinished for stored operation {}", effect.operation).into()))
}

#[cfg(test)]
mod schema_tests {
    use super::*;
    use photocraft_doc::import_conversion::ConversionReason;

    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: photocraft_color::ColorMode::Rgb,
            linear: false,
            owner_bounds: photocraft_geom::Rect::new(0, 0, 2, 1),
            native_filter_bounds: photocraft_geom::Rect::new(0, 0, 2, 1),
            target_profile: None,
            resolution_dpi: 72.0,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }

    #[test]
    fn unknown_stored_controls_are_never_silently_applied_as_native_defaults() {
        for operation in ["gegl:grid", "gegl:video-degradation", "gegl:gaussian-blur", "gimp:fill-source"] {
            let effect = StoredEffect::test(operation, vec![Argument { name: "future-control", value: ArgumentValue::Float(0.75) }]);
            let failure = map(&effect, &context()).unwrap_err();
            assert_eq!(failure.reason, ConversionReason::MissingMapping, "{operation}");
            assert!(failure.message.contains("future-control"));
        }
    }

    #[test]
    fn duplicate_known_arguments_are_invalid_but_schema_aliases_are_explicit() {
        let effect = StoredEffect::test(
            "gegl:gaussian-blur",
            vec![Argument { name: "std-dev-x", value: ArgumentValue::Float(1.0) }, Argument { name: "std-dev-x", value: ArgumentValue::Float(2.0) }],
        );
        assert_eq!(map(&effect, &context()).unwrap_err().reason, ConversionReason::InvalidOptionalData);
        assert_eq!(schema::fields("gegl:motion-blur"), schema::fields("gegl:motion-blur-linear"));
        assert_eq!(schema::fields("gegl:invert"), schema::fields("gegl:invert-linear"));
        assert!(schema::fields("user-installed:operation").is_none());
    }

    #[test]
    fn frozen_aliases_map_native_data_without_changing_source_identity() {
        for (alias, canonical) in [
            ("gegl:abs", "gegl:absolute"),
            ("gegl:color-reduction", "gegl:dither"),
            ("gegl:invert", "gegl:invert-linear"),
            ("gegl:motion-blur", "gegl:motion-blur-linear"),
            ("gegl:little-planet", "gegl:stereographic-projection"),
        ] {
            let source = StoredEffect::test(alias, Vec::new());
            let normal = StoredEffect::test(canonical, Vec::new());
            assert_eq!(source.canonical_operation(), canonical);
            let mapped = map(&source, &context()).unwrap_or_else(|error| panic!("{alias}: {}", error.message));
            let expected = map(&normal, &context()).unwrap_or_else(|error| panic!("{canonical}: {}", error.message));
            assert_eq!(mapped.filters, expected.filters, "{alias}");
            assert_eq!(mapped.patterns, expected.patterns, "{alias}");
            assert_eq!(mapped.structure.is_some(), expected.structure.is_some(), "{alias}");
            assert_eq!(source.operation, alias, "historical source identity must remain intact");
        }
    }
}
