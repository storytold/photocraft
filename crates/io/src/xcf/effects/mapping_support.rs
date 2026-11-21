//! Helpers used by XCF's native effect mapping adapters.

#[cfg(test)]
use super::Argument;
use super::StoredEffect;

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
    /// Construct a visible, full-strength synthetic stored effect with neutral envelope fields for independent mapping tests.
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
}

#[derive(Debug)]
pub(in crate::xcf) struct MappingFailure {
    pub reason: photocraft_doc::import_conversion::ConversionReason,
    pub message: String,
}

impl MappingFailure {
    /// Classify an unavailable resource separately from an unfinished native mapping.
    pub fn resource(message: impl Into<String>) -> Self {
        Self { reason: photocraft_doc::import_conversion::ConversionReason::ResourceUnavailable, message: message.into() }
    }
}

impl From<String> for MappingFailure {
    /// Turn an owned adapter diagnostic into an explicit missing-mapping disposition for retained source data.
    fn from(message: String) -> Self {
        Self { reason: photocraft_doc::import_conversion::ConversionReason::MissingMapping, message }
    }
}

impl From<&str> for MappingFailure {
    /// Preserve a borrowed adapter diagnostic through the same owned missing-mapping conversion.
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
