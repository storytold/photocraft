//! Build ordinary native smart contents for one imported owner's filter stages.
//! No source operations or reports are evaluated here or stored as native commands.

use std::sync::Arc;

use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Affine, Document, Layer, LayerContent, LayerMask, SmartObject, SmartSource};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

use super::effects::{NativeMapping, NativeStage, NativeStructure};
use super::{MAX_SIDE, Result, invalid};

const MAX_STAGES: usize = 62; // Leave room for the base and outer native contents.

pub(super) struct NativeEnvelope {
    pub opacity: f32,
    pub blend: BlendMode,
    pub visible: bool,
    pub mask: Option<LayerMask>,
}

pub(super) struct NativeOwner {
    template: Arc<Document>,
    source: Arc<Vec<u8>>,
    // Import-private provenance, never serialized or derived from source identifiers.
    lineage: Arc<()>,
    origin: (i32, i32),
    stages: usize,
    added_depth: usize,
    existing_depth: usize,
}

/// Count ordinary native embedded contents before adding another owner wrapper.
/// Only in-memory native bundles are inspected; no locator is ever resolved.
fn nested_depth(layers: &[Layer], depth: usize, groups: usize, allowance: &mut u64, ctl: &Interrupt<'_>) -> Result<usize> {
    if depth > 64 || groups > super::MAX_DEPTH {
        return Err(invalid("native effect contents exceed the nested preparation limit"));
    }
    let mut deepest = depth;
    for layer in layers {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        match &layer.content {
            LayerContent::Group(group) => deepest = deepest.max(nested_depth(&group.children, depth, groups + 1, allowance, ctl)?),
            LayerContent::Smart(smart) => {
                let SmartSource::Embedded { bytes, .. } = &smart.source else {
                    return Err(invalid("effect owner requires self-contained native smart contents"));
                };
                if !photocraft_format::is_pcraft(bytes) {
                    return Err(invalid("effect owner nesting requires native embedded contents"));
                }
                if depth >= 64 {
                    return Err(invalid("native effect contents exceed the nested preparation limit"));
                }
                let options = photocraft_format::LoadOptions {
                    max_total_bytes: *allowance,
                    max_blob_bytes: usize::try_from(*allowance).unwrap_or(usize::MAX).min(super::MAX_PIXEL_BYTES),
                    ..Default::default()
                };
                let (document, bytes) =
                    photocraft_format::load_from_bytes_measured(bytes, &options).map_err(|e| invalid(format!("native effect nesting: {e}")))?;
                *allowance = allowance.checked_sub(bytes).ok_or_else(|| invalid("native effect nesting exceeds its load allowance"))?;
                deepest = deepest.max(nested_depth(&document.layers, depth + 1, 0, allowance, ctl)?);
            }
            _ => {}
        }
    }
    Ok(deepest)
}

/// Conservative admission for this builder's private native documents, before
/// format conversion clones metadata or creates compressed-object/ZIP buffers.
/// This bounds admitted representation bytes, not the format writer's whole peak memory.
fn encoded_preflight(doc: &Document, allowance: usize, ctl: &Interrupt<'_>) -> Result<()> {
    use std::collections::HashSet;
    use std::io::Write;
    macro_rules! metadata {
        ($state:expr, $value:expr) => {{
            let state = &mut *$state;
            let result = serde_json::to_writer_pretty(&mut *state, $value);
            state.ctl.check().map_err(|_| crate::IoError::Cancelled)?;
            result.map_err(|e| invalid(format!("native bundle metadata admission: {e}")))
        }};
    }
    struct Admission<'a, 'b> {
        left: usize,
        blobs: HashSet<usize>,
        tiles: HashSet<(usize, SampleType)>,
        nodes: usize,
        ctl: &'a Interrupt<'b>,
    }
    impl Write for Admission<'_, '_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.ctl.cancelled() {
                return Err(std::io::Error::other("native bundle admission cancelled"));
            }
            // Count pretty metadata at its maximum admitted group indentation.
            // Strings' internal newlines are escaped JSON, not indentation.
            let indent = bytes.iter().filter(|b| **b == b'\n').count().checked_mul(2 * (super::MAX_DEPTH + 20));
            let cost = indent.and_then(|n| n.checked_add(bytes.len())).ok_or_else(|| std::io::Error::other("native bundle metadata overflows"))?;
            self.left = self.left.checked_sub(cost).ok_or_else(|| std::io::Error::other("native bundle metadata exceeds remaining source allowance"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Admission<'_, '_> {
        fn charge(&mut self, bytes: usize) -> Result<()> {
            self.ctl.check().map_err(|_| crate::IoError::Cancelled)?;
            self.left =
                self.left.checked_sub(bytes).ok_or_else(|| invalid("native bundle representation exceeds remaining source allowance before serialization"))?;
            Ok(())
        }
        fn payload(&mut self, n: usize) -> Result<()> {
            // Existing native reader admits this conservative zstd frame bound;
            // fixed headroom includes ZIP entries/central records and blob hashes.
            let bytes = n.checked_add(n / 8).and_then(|n| n.checked_add(8192)).ok_or_else(|| invalid("native bundle payload admission overflows"))?;
            self.charge(bytes)
        }
        fn blob(&mut self, bytes: &Arc<Vec<u8>>) -> Result<()> {
            if self.blobs.insert(Arc::as_ptr(bytes) as usize) {
                self.payload(bytes.len())?;
            }
            Ok(())
        }
        fn optional_blob(&mut self, bytes: Option<&Arc<Vec<u8>>>) -> Result<()> {
            if let Some(bytes) = bytes {
                self.blob(bytes)?;
            }
            Ok(())
        }
        fn surface(&mut self, surface: &Surface) -> Result<()> {
            self.charge(8192)?;
            for (_, tile) in surface.tiles() {
                self.charge(2048)?; // hash, coordinates and maximum pretty indentation
                if self.tiles.insert((Arc::as_ptr(tile) as usize, surface.format().sample)) {
                    self.payload(tile.bytes().len())?;
                }
            }
            Ok(())
        }
        fn mask(&mut self, mask: Option<&LayerMask>) -> Result<()> {
            if let Some(mask) = mask {
                self.surface(&mask.surface)?;
            }
            Ok(())
        }
        fn layers(&mut self, layers: &[Layer], depth: usize) -> Result<()> {
            if depth > super::MAX_DEPTH {
                return Err(invalid("native bundle admission group depth exceeds limit"));
            }
            for layer in layers {
                self.nodes = self.nodes.checked_sub(1).ok_or_else(|| invalid("native bundle admission layer count exceeds limit"))?;
                self.charge(4096)?;
                metadata!(
                    self,
                    &(
                        &layer.name,
                        layer.visible,
                        layer.locks,
                        layer.blend,
                        layer.opacity,
                        layer.fill_opacity,
                        layer.clipped,
                        layer.label,
                        layer.psd_id,
                        layer.link_group,
                        layer.excluded_channels,
                        &layer.blend_if,
                        layer.advanced,
                        layer.xmp.as_deref(),
                    )
                )?;
                self.mask(layer.mask.as_ref())?;
                if let Some(mask) = &layer.vector_mask {
                    metadata!(self, &(&mask.path, mask.enabled, mask.linked, mask.density, mask.feather))?;
                }
                metadata!(self, &(&layer.effects.items, layer.effects.reference, layer.effects.enabled))?;
                self.optional_blob(layer.effects.psd_raw.as_ref())?;
                for (_, bytes) in &layer.psd_blocks {
                    self.charge(1024)?;
                    self.blob(bytes)?;
                }
                if let Some(cache) = &layer.fill_cache {
                    metadata!(self, &cache.fill)?;
                    self.surface(&cache.surface)?;
                }
                if layer.video.is_some() {
                    return Err(invalid("private native effect bundle does not admit video data"));
                }
                match &layer.content {
                    LayerContent::Raster(surface) => self.surface(surface)?,
                    LayerContent::Group(group) => {
                        metadata!(self, &group.artboard)?;
                        self.layers(&group.children, depth + 1)?;
                    }
                    LayerContent::Adjustment(adjustment) => metadata!(self, adjustment)?,
                    LayerContent::Fill(fill) => metadata!(self, fill)?,
                    LayerContent::Shape(shape) => {
                        metadata!(self, &(&shape.path, &shape.fill, &shape.stroke, &shape.live, shape.density, shape.feather))?;
                        self.optional_blob(shape.psd_raw.as_ref())?;
                        if let Some(cache) = &shape.cache {
                            self.surface(cache)?;
                        }
                    }
                    LayerContent::Text(text) => {
                        metadata!(
                            self,
                            &(
                                &text.text,
                                &text.font_family,
                                text.size_pt,
                                text.color,
                                text.transform,
                                &text.runs,
                                &text.paragraphs,
                                &text.shape,
                                text.orientation,
                                text.antialias,
                                &text.warp,
                            )
                        )?;
                        self.optional_blob(text.psd_raw.as_ref())?;
                        if let Some(cache) = &text.cache {
                            self.surface(cache)?;
                        }
                    }
                    LayerContent::Smart(smart) => {
                        match &smart.source {
                            SmartSource::Embedded { file_name, bytes } => {
                                metadata!(self, file_name)?;
                                self.blob(bytes)?;
                            }
                            SmartSource::Linked { .. } => return Err(invalid("private native effect bundle requires self-contained smart data")),
                        }
                        metadata!(self, &(&smart.smart_filters, smart.transform, &smart.warp, smart.stack_mode, smart.perspective, smart.filters_enabled))?;
                        self.mask(smart.filter_mask.as_ref())?;
                        self.optional_blob(smart.psd_raw.as_ref())?;
                        if let Some(cache) = &smart.cache {
                            self.surface(cache)?;
                        }
                    }
                }
            }
            Ok(())
        }
    }
    let mut admission = Admission { left: allowance.min(super::MAX_PIXEL_BYTES), blobs: HashSet::new(), tiles: HashSet::new(), nodes: super::MAX_LAYERS, ctl };
    // These fields are intentionally absent from private builder templates; do
    // not silently omit new variable metadata from this admission boundary.
    if doc.metadata != Default::default() || !doc.layer_comps.is_empty() || doc.last_document_state.is_some() {
        return Err(invalid("private native effect bundle contains unadmitted historical metadata or layer comps"));
    }
    admission.charge(8192)?;
    metadata!(
        &mut admission,
        &(&doc.name, doc.resolution_dpi, &doc.guides, doc.ruler_origin, doc.global_light, &doc.work_path, &doc.clipping_path, doc.quick_mask_unselected,)
    )?;
    metadata!(
        &mut admission,
        &(&doc.color_table, &doc.duotone, &doc.measurement, &doc.notes, &doc.text_styles, &doc.slices, &doc.variables, &doc.timeline, doc.file_encoding,)
    )?;
    admission.optional_blob(doc.icc_profile.as_ref())?;
    if let Some(selection) = &doc.selection {
        admission.surface(selection)?;
    }
    for channel in doc.channels.iter().chain(doc.quick_mask.iter()) {
        metadata!(&mut admission, &(&channel.name, channel.spot, channel.color, channel.opacity, channel.indicates))?;
        admission.surface(&channel.surface)?;
    }
    for path in &doc.paths {
        metadata!(&mut admission, &(&path.name, &path.path))?;
        admission.optional_blob(path.psd_raw.as_ref())?;
    }
    for pattern in &doc.patterns {
        metadata!(&mut admission, &(&pattern.id, &pattern.name, pattern.width, pattern.height))?;
        admission.surface(&pattern.surface)?;
    }
    admission.layers(&doc.layers, 0)?;
    ctl.check().map_err(|_| crate::IoError::Cancelled)
}

fn encoded(doc: &Document, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Arc<Vec<u8>>> {
    encoded_preflight(doc, *budget, ctl)?;
    let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).map_err(|e| invalid(format!("native effect contents: {e}")))?;
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    *budget = budget.checked_sub(bytes.len()).ok_or_else(|| invalid("native effect contents exceed the shared 1 GiB source allowance"))?;
    Ok(Arc::new(bytes))
}

fn surface_bytes(surface: &Surface) -> Result<usize> {
    surface.tiles().try_fold(surface.default_bytes().len(), |sum, (_, tile)| {
        sum.checked_add(tile.bytes().len()).ok_or_else(|| invalid("native structure surface allocation overflows"))
    })
}

/// Count serialized native parameters without allocating another JSON copy.
macro_rules! serialized_bytes {
    ($value:expr) => {{
        struct Counter(usize);
        impl std::io::Write for Counter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self.0.checked_add(bytes.len()).ok_or_else(|| std::io::Error::other("native parameters overflow"))?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut counter = Counter(0);
        serde_json::to_writer(&mut counter, $value).map_err(|e| invalid(format!("native structure parameters: {e}"))).map(|()| counter.0)
    }};
}

fn parameter_bytes(filters: &[photocraft_doc::SmartFilter]) -> Result<usize> {
    filters.iter().try_fold(0usize, |sum, filter| {
        sum.checked_add(serialized_bytes!(&filter.params)?)
            .and_then(|sum| sum.checked_add(filter.command.len()))
            .ok_or_else(|| invalid("native structure parameters overflow"))
    })
}

/// Admit only bounded ordinary native resources. The owning stage carries its
/// own resource table, so native save/reopen requires no source lookup.
pub(super) fn add_patterns(document: &mut Document, patterns: Vec<photocraft_doc::Pattern>, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<()> {
    if patterns.len() > 1024 {
        return Err(invalid("native effect declares too many patterns"));
    }
    for pattern in patterns {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        if pattern.width == 0
            || pattern.height == 0
            || pattern.width > photocraft_doc::pattern::MAX_EDGE
            || pattern.height > photocraft_doc::pattern::MAX_EDGE
            || u64::from(pattern.width) * u64::from(pattern.height) > photocraft_doc::pattern::MAX_PAINT_PIXELS
        {
            return Err(invalid("native effect pattern dimensions exceed paint limits"));
        }
        if let Some(existing) = document.patterns.iter().find(|existing| existing.id == pattern.id) {
            if existing != &pattern {
                return Err(invalid("native effect pattern conflicts with an existing native resource"));
            }
            continue;
        }
        let bytes = surface_bytes(&pattern.surface)?
            .checked_add(pattern.id.len())
            .and_then(|n| n.checked_add(pattern.name.len()))
            .ok_or_else(|| invalid("native pattern resource size overflows"))?;
        *budget = budget.checked_sub(bytes).ok_or_else(|| invalid("native pattern pixels exceed the shared source allowance"))?;
        document.patterns.push(pattern);
    }
    Ok(())
}

/// Admit ordinary editable typography before cloning/serializing a replacement.
/// Rendering remains the native text engine's responsibility.
fn direct_text_bytes(text: &photocraft_doc::TextLayer, ctl: &Interrupt<'_>) -> Result<usize> {
    use photocraft_doc::text::TextShape;
    let finite = |v: f32| v.is_finite() && v.abs() <= 1_000_000.;
    let color = |c: photocraft_color::Color| c.c.iter().all(|v| finite(*v)) && c.alpha.is_finite() && (0.0..=1.0).contains(&c.alpha);
    if text.text.len() > 65_536
        || text.font_family.len() > 4096
        || text.text.contains('\0')
        || text.font_family.contains('\0')
        || !text.size_pt.is_finite()
        || text.size_pt <= 0.
        || text.size_pt > 8192.
        || !color(text.color)
        || text.transform.m.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
        || text.runs.len() > 65_536
        || text.paragraphs.len() > 65_536
    {
        return Err(invalid("native replacement text exceeds finite typography limits"));
    }
    match &text.shape {
        TextShape::Point => {}
        TextShape::Box { x, y, width, height } if [*x, *y, *width, *height].iter().all(|v| finite(*v)) && *width >= 0. && *height >= 0. => {}
        _ => return Err(invalid("native replacement text requires a finite point or paragraph frame")),
    }
    for run in &text.runs {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let style = &run.style;
        if run.len > text.text.len()
            || style.font_family.len() > 4096
            || style.font_style.len() > 4096
            || style.postscript_name.as_ref().is_some_and(|v| v.len() > 4096)
            || !style.size_pt.is_finite()
            || style.size_pt <= 0.
            || style.size_pt > 8192.
            || !color(style.color)
            || [style.tracking, style.baseline_shift_pt, style.horizontal_scale, style.vertical_scale, style.kern].iter().any(|v| !finite(*v))
            || style.leading_pt.is_some_and(|v| !finite(v))
            || style.features.len() > 1024
            || style.variations.len() > 1024
            || style.features.iter().any(|v| v.tag.len() > 16)
            || style.variations.iter().any(|v| v.axis.len() > 16 || !finite(v.value))
            || style.language.as_ref().is_some_and(|v| v.len() > 4096)
        {
            return Err(invalid("native replacement character style exceeds finite typography limits"));
        }
    }
    for run in &text.paragraphs {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let style = &run.style;
        if run.len > text.text.len()
            || [style.first_line_indent_pt, style.start_indent_pt, style.end_indent_pt, style.space_before_pt, style.space_after_pt, style.auto_leading]
                .iter()
                .any(|v| !finite(*v))
        {
            return Err(invalid("native replacement paragraph style exceeds finite typography limits"));
        }
    }
    if text.warp.as_ref().is_some_and(|v| v.style.len() > 4096 || [v.value, v.horizontal_distortion, v.vertical_distortion].iter().any(|v| !finite(*v))) {
        return Err(invalid("native replacement warp exceeds finite typography limits"));
    }
    let bytes = serialized_bytes!(&(
        &text.text,
        &text.font_family,
        text.size_pt,
        text.color,
        text.transform,
        &text.runs,
        &text.paragraphs,
        &text.shape,
        text.orientation,
        text.antialias,
        &text.warp
    ))?;
    if bytes > super::MAX_METADATA_BYTES {
        return Err(invalid("native replacement text exceeds metadata allowance"));
    }
    let mut bytes = bytes.checked_add(std::mem::size_of::<photocraft_doc::TextLayer>()).ok_or_else(|| invalid("native replacement text size overflows"))?;
    if let Some(raw) = &text.psd_raw {
        bytes = bytes.checked_add(raw.capacity()).ok_or_else(|| invalid("native replacement text raw data overflows"))?;
    }
    if let Some(cache) = &text.cache {
        bytes = bytes.checked_add(surface_bytes(cache)?).ok_or_else(|| invalid("native replacement text cache overflows"))?;
    }
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    Ok(bytes)
}

fn direct_layer_bytes(layer: &Layer, depth: usize, remaining_nodes: &mut usize, ctl: &Interrupt<'_>) -> Result<usize> {
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    if depth > super::MAX_DEPTH {
        return Err(invalid("native replacement exceeds group depth"));
    }
    *remaining_nodes = remaining_nodes.checked_sub(1).ok_or_else(|| invalid("native replacement has too many layers"))?;
    let mut bytes = std::mem::size_of::<Layer>().checked_add(layer.name.len()).ok_or_else(|| invalid("native replacement size overflows"))?;
    let allocation = match &layer.content {
        LayerContent::Raster(surface) => surface_bytes(surface)?,
        LayerContent::Shape(shape) => {
            let mut allocation = serialized_bytes!(&shape.path)?;
            allocation = allocation
                .checked_add(serialized_bytes!(&shape.fill)?)
                .and_then(|v| v.checked_add(std::mem::size_of::<photocraft_doc::ShapeLayer>()))
                .ok_or_else(|| invalid("native replacement shape size overflows"))?;
            if let Some(cache) = &shape.cache {
                allocation = allocation.checked_add(surface_bytes(cache)?).ok_or_else(|| invalid("native replacement cache size overflows"))?;
            }
            allocation
        }
        LayerContent::Fill(fill) => serialized_bytes!(fill)?,
        LayerContent::Text(text) => direct_text_bytes(text, ctl)?,
        LayerContent::Group(group) => group.children.iter().try_fold(0usize, |sum, child| {
            sum.checked_add(direct_layer_bytes(child, depth + 1, remaining_nodes, ctl)?).ok_or_else(|| invalid("native replacement group size overflows"))
        })?,
        _ => return Err(invalid("native replacement requires direct raster, shape, fill, text, or grouped data")),
    };
    let style_bytes = serialized_bytes!(&layer.effects.items)?;
    bytes = bytes.checked_add(allocation).and_then(|v| v.checked_add(style_bytes)).ok_or_else(|| invalid("native replacement allocation overflows"))?;
    if let Some(mask) = &layer.mask {
        bytes = bytes.checked_add(surface_bytes(&mask.surface)?).ok_or_else(|| invalid("native replacement coverage overflows"))?;
    }
    if let Some(mask) = &layer.vector_mask {
        bytes = bytes.checked_add(serialized_bytes!(&mask.path)?).ok_or_else(|| invalid("native replacement vector coverage overflows"))?;
    }
    Ok(bytes)
}

/// Account for native values retained between independently converted nodes.
/// The first counter covers native resources plus their retained representation;
/// the second covers metadata only. Pixel payloads never consume metadata allowance.
/// Shared resources may be counted again conservatively; this is not a whole-RSS estimate.
pub(super) fn retained_mapping_bytes(mapping: &NativeMapping, ctl: &Interrupt<'_>) -> Result<(usize, usize)> {
    macro_rules! retained_metadata {
        ($state:expr, $value:expr) => {{
            let state = &mut *$state;
            let result = serde_json::to_writer(&mut *state, $value);
            state.ctl.check().map_err(|_| crate::IoError::Cancelled)?;
            result.map_err(|e| invalid(format!("native retained metadata: {e}")))
        }};
    }
    struct Retained<'a, 'b> {
        resource: usize,
        metadata: usize,
        nodes: usize,
        json_nodes: usize,
        ctl: &'a Interrupt<'b>,
    }
    impl std::io::Write for Retained<'_, '_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.ctl.cancelled() {
                return Err(std::io::Error::other("native retained metadata cancelled"));
            }
            self.metadata = self
                .metadata
                .checked_add(bytes.len())
                .filter(|n| *n <= super::MAX_METADATA_BYTES)
                .ok_or_else(|| std::io::Error::other("native retained metadata exceeds allowance"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Retained<'_, '_> {
        fn resource(&mut self, bytes: usize) -> Result<()> {
            self.ctl.check().map_err(|_| crate::IoError::Cancelled)?;
            self.resource = self
                .resource
                .checked_add(bytes)
                .filter(|n| *n <= super::MAX_PIXEL_BYTES)
                .ok_or_else(|| invalid("native retained resources exceed allowance"))?;
            Ok(())
        }
        fn vector<T>(&mut self, values: &Vec<T>) -> Result<()> {
            self.resource(values.capacity().checked_mul(std::mem::size_of::<T>()).ok_or_else(|| invalid("native retained vector overflows"))?)
        }
        fn json(&mut self, value: &serde_json::Value, depth: usize) -> Result<()> {
            if depth > 128 {
                return Err(invalid("native retained parameter depth exceeds limit"));
            }
            self.json_nodes = self.json_nodes.checked_sub(1).ok_or_else(|| invalid("native retained parameter count exceeds limit"))?;
            self.resource(std::mem::size_of::<serde_json::Value>())?;
            match value {
                serde_json::Value::String(text) => self.resource(text.capacity())?,
                serde_json::Value::Array(values) => {
                    self.vector(values)?;
                    for value in values {
                        self.json(value, depth + 1)?;
                    }
                }
                serde_json::Value::Object(values) => {
                    for (key, value) in values {
                        self.resource(
                            key.capacity().checked_add(8 * std::mem::size_of::<usize>()).ok_or_else(|| invalid("native retained object overflows"))?,
                        )?;
                        self.json(value, depth + 1)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        fn filters(&mut self, filters: &Vec<photocraft_doc::SmartFilter>) -> Result<()> {
            if filters.len() > 64 {
                return Err(invalid("native retained filter count exceeds limit"));
            }
            self.vector(filters)?;
            for filter in filters {
                self.resource(filter.command.capacity())?;
                self.json(&filter.params, 0)?;
            }
            retained_metadata!(self, filters)
        }
        fn surface(&mut self, surface: &Surface) -> Result<()> {
            self.resource(surface.default_bytes().len())?;
            for (_, tile) in surface.tiles() {
                self.resource(tile.bytes().len())?;
            }
            Ok(())
        }
        fn patterns(&mut self, patterns: &[photocraft_doc::Pattern]) -> Result<()> {
            if patterns.len() > 1024 {
                return Err(invalid("native retained patterns exceed limit"));
            }
            for pattern in patterns {
                self.surface(&pattern.surface)?;
                self.resource(std::mem::size_of::<photocraft_doc::Pattern>())?;
                self.resource(pattern.name.capacity())?;
                self.resource(pattern.id.capacity())?;
                retained_metadata!(self, &(&pattern.id, &pattern.name, pattern.width, pattern.height))?;
            }
            Ok(())
        }
        fn layer_metadata(&mut self, layer: &Layer, depth: usize) -> Result<()> {
            if depth > super::MAX_DEPTH {
                return Err(invalid("native retained layer depth exceeds limit"));
            }
            self.nodes = self.nodes.checked_sub(1).ok_or_else(|| invalid("native retained layer count exceeds limit"))?;
            retained_metadata!(self, &(&layer.name, &layer.blend_if, layer.advanced, layer.xmp.as_deref(), &layer.effects.items))?;
            if let Some(mask) = &layer.vector_mask {
                retained_metadata!(self, &(&mask.path, mask.enabled, mask.linked, mask.density, mask.feather))?;
            }
            if let Some(raw) = &layer.effects.psd_raw {
                self.resource(raw.capacity())?;
            }
            for (_, raw) in &layer.psd_blocks {
                self.resource(raw.capacity())?;
            }
            if let Some(cache) = &layer.fill_cache {
                retained_metadata!(self, &cache.fill)?;
                self.surface(&cache.surface)?;
            }
            if layer.video.is_some() {
                return Err(invalid("native retained replacement does not admit video"));
            }
            match &layer.content {
                LayerContent::Raster(_) => {}
                LayerContent::Fill(fill) => retained_metadata!(self, fill)?,
                LayerContent::Text(text) => {
                    direct_text_bytes(text, self.ctl)?;
                    retained_metadata!(
                        self,
                        &(
                            &text.text,
                            &text.font_family,
                            text.size_pt,
                            text.color,
                            text.transform,
                            &text.runs,
                            &text.paragraphs,
                            &text.shape,
                            text.orientation,
                            text.antialias,
                            &text.warp
                        )
                    )?;
                }
                LayerContent::Shape(shape) => {
                    retained_metadata!(self, &(&shape.path, &shape.fill, &shape.stroke, &shape.live, shape.density, shape.feather))?;
                    if let Some(raw) = &shape.psd_raw {
                        self.resource(raw.capacity())?;
                    }
                }
                LayerContent::Group(group) => {
                    retained_metadata!(self, &group.artboard)?;
                    for child in &group.children {
                        self.layer_metadata(child, depth + 1)?;
                    }
                }
                _ => return Err(invalid("native retained replacement requires direct native data")),
            }
            Ok(())
        }
    }
    let mut count =
        Retained { resource: std::mem::size_of::<NativeMapping>(), metadata: 0, nodes: super::MAX_LAYERS, json_nodes: super::MAX_METADATA_BYTES, ctl };
    count.filters(&mapping.filters)?;
    count.vector(&mapping.patterns)?;
    count.vector(&mapping.layer_effects)?;
    count.vector(&mapping.notes)?;
    retained_metadata!(&mut count, &(&mapping.layer_effects, &mapping.notes))?;
    count.patterns(&mapping.patterns)?;
    for note in &mapping.notes {
        count.resource(note.capacity())?;
    }
    match &mapping.structure {
        Some(NativeStructure::Replace { layer, patterns }) => {
            let mut nodes = super::MAX_LAYERS;
            count.resource(direct_layer_bytes(layer, 0, &mut nodes, ctl)?)?;
            count.layer_metadata(layer, 0)?;
            count.vector(patterns)?;
            count.patterns(patterns)?;
        }
        Some(NativeStructure::Composite { branches }) => {
            if branches.len() > 64 {
                return Err(invalid("native retained branch count exceeds limit"));
            }
            count.vector(branches)?;
            for branch in branches {
                count.filters(&branch.filters)?;
                count.vector(&branch.layer_effects)?;
                count.vector(&branch.post_layer_effects)?;
                retained_metadata!(&mut count, &(&branch.layer_effects, &branch.post_layer_effects, &branch.fill, branch.advanced))?;
                if let Some(mask) = &branch.mask {
                    count.surface(&mask.surface)?;
                }
                if let Some(mask) = &branch.vector_mask {
                    retained_metadata!(&mut count, &(&mask.path, mask.enabled, mask.linked, mask.density, mask.feather))?;
                }
            }
        }
        Some(NativeStructure::Stages { .. }) => return Err(invalid("retained node accounting requires a flat single native mapping")),
        None => {}
    }
    // Serialized metadata bounds strings, LUT text and nested native parameter data
    // without constructing another JSON buffer. Resource accounting includes it too.
    count.resource(count.metadata)?;
    Ok((count.resource, count.metadata))
}

fn checked_translate(rect: Rect, dx: i32, dy: i32) -> Result<()> {
    for (value, delta) in [(rect.x0, dx), (rect.x1, dx), (rect.y0, dy), (rect.y1, dy)] {
        value.checked_add(delta).ok_or_else(|| invalid("native effect placement exceeds document coordinate range"))?;
    }
    Ok(())
}

fn shift_surface(surface: &mut Surface, dx: i32, dy: i32) -> Result<()> {
    let bounds = surface.content_bounds();
    if !bounds.is_empty() {
        checked_translate(bounds, dx, dy)?;
    }
    *surface = photocraft_algo::resample::translate_surface(surface, dx, dy);
    Ok(())
}

fn move_layer(layer: &mut Layer, dx: i32, dy: i32, depth: usize, ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    if depth > super::MAX_DEPTH {
        return Err(invalid("native effect contents exceed the group depth limit"));
    }
    let shift = Affine::translate(f64::from(dx), f64::from(dy));
    if let Some(mask) = &mut layer.mask {
        shift_surface(&mut mask.surface, dx, dy)?;
    }
    if let Some(mask) = &mut layer.vector_mask {
        mask.path = mask.path.transform(&shift);
    }
    if let Some((x, y)) = &mut layer.effects.reference {
        *x += f64::from(dx);
        *y += f64::from(dy);
    }
    layer.fill_cache = None;
    match &mut layer.content {
        LayerContent::Raster(surface) => shift_surface(surface, dx, dy)?,
        LayerContent::Text(text) => {
            text.transform = shift.mul(&text.transform);
            text.cache = None;
        }
        LayerContent::Shape(shape) => {
            shape.path = shape.path.transform(&shift);
            shape.cache = None;
            shape.psd_raw = None;
            shape.live = None;
        }
        LayerContent::Group(group) => {
            for child in &mut group.children {
                move_layer(child, dx, dy, depth + 1, ctl)?;
            }
            if let Some(artboard) = &mut group.artboard {
                checked_translate(artboard.rect, dx, dy)?;
                artboard.rect = artboard.rect.translate(dx, dy);
            }
        }
        LayerContent::Smart(smart) => {
            smart.transform = shift.mul(&smart.transform);
            if let Some(matrix) = &mut smart.perspective {
                let [a, b, c, d, e, f, g, h, i] = *matrix;
                let (x, y) = (f64::from(dx), f64::from(dy));
                *matrix = [a + x * g, b + x * h, c + x * i, d + y * g, e + y * h, f + y * i, g, h, i];
            }
            if let Some(mask) = &mut smart.filter_mask {
                shift_surface(&mut mask.surface, dx, dy)?;
            }
            smart.cache = None;
        }
        // XCF does not produce native adjustment/fill content before this handoff.
        // Reject an unexpected spatial fill rather than silently misplacing it.
        LayerContent::Adjustment(_) => {}
        LayerContent::Fill(_) => return Err(invalid("native spatial fill needs an explicit placement conversion before effect embedding")),
    }
    Ok(())
}

impl NativeOwner {
    pub(super) fn new(document: &Document, layer: &Layer, bounds: Rect, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Self> {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let width = u32::try_from(i64::from(bounds.x1) - i64::from(bounds.x0)).map_err(|_| invalid("invalid native effect owner width"))?;
        let height = u32::try_from(i64::from(bounds.y1) - i64::from(bounds.y0)).map_err(|_| invalid("invalid native effect owner height"))?;
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(invalid("native effect owner dimensions exceed native document limits"));
        }
        let dx = bounds.x0.checked_neg().ok_or_else(|| invalid("native effect origin cannot be translated safely"))?;
        let dy = bounds.y0.checked_neg().ok_or_else(|| invalid("native effect origin cannot be translated safely"))?;
        let existing_depth = nested_depth(std::slice::from_ref(layer), 0, 0, &mut (super::MAX_PIXEL_BYTES as u64), ctl)?;
        if existing_depth >= 64 {
            return Err(invalid("native effect owner cannot add a wrapper beyond the native content depth"));
        }
        let mut input = Layer::new(layer.name.clone(), layer.content.clone());
        // The original owner's mask/blend/opacity/locks stay on its admitted outer layer.
        // Child native state remains part of an editable embedded group.
        move_layer(&mut input, dx, dy, 0, ctl)?;
        let mut template = Document::new("Native filter contents", Size::new(width, height), document.mode, document.depth);
        template.icc_profile = document.icc_profile.clone();
        template.resolution_dpi = document.resolution_dpi;
        template.global_light = document.global_light;
        template.patterns = document.patterns.clone();
        let mut base = template.clone();
        base.layers.push(input);
        let mut remaining = *budget;
        let source = encoded(&base, &mut remaining, ctl)?;
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        *budget = remaining;
        Ok(Self { template: Arc::new(template), source, lineage: Arc::new(()), origin: (bounds.x0, bounds.y0), stages: 0, added_depth: 0, existing_depth })
    }

    /// Snapshot the current accepted native stage. The immutable native bytes and
    /// private owner provenance are shared; subsequent pushes do not change this fork.
    pub(super) fn fork(&self) -> Self {
        Self {
            template: self.template.clone(),
            source: self.source.clone(),
            lineage: self.lineage.clone(),
            origin: self.origin,
            stages: self.stages,
            added_depth: self.added_depth,
            existing_depth: self.existing_depth,
        }
    }

    /// Combine actual native branch outputs, bottom then top, using ordinary native
    /// layer blending. The caller applies the complete imported operation's
    /// envelope separately and exactly once.
    pub(super) fn join(&mut self, below: &Self, above: &Self, blend: BlendMode, opacity: f32, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<()> {
        self.combine(below, above, (blend, opacity, false), budget, ctl)
    }

    /// Ordinary native shallow knockout: the upper image contributes coverage,
    /// while its fill is suppressed. No source compositing rule is retained.
    pub(super) fn knockout(&mut self, below: &Self, above: &Self, opacity: f32, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<()> {
        self.combine(below, above, (BlendMode::Normal, opacity, true), budget, ctl)
    }

    fn combine(&mut self, below: &Self, above: &Self, settings: (BlendMode, f32, bool), budget: &mut usize, ctl: &Interrupt<'_>) -> Result<()> {
        let (blend, opacity, knockout) = settings;
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let same_owner = |input: &Self| {
            Arc::ptr_eq(&self.lineage, &input.lineage)
                && self.origin == input.origin
                && self.template.size == input.template.size
                && self.template.mode == input.template.mode
                && self.template.depth == input.template.depth
                && self.template.icc_profile == input.template.icc_profile
                && self.existing_depth == input.existing_depth
        };
        if !same_owner(below) || !same_owner(above) {
            return Err(invalid("native branch join requires the same validated owner, frame and profile lineage"));
        }
        if blend == BlendMode::PassThrough {
            return Err(invalid("native branch join requires an ordinary layer blend, not group pass-through"));
        }
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(invalid("native branch join opacity must be finite in 0..1"));
        }
        let stages = self.stages.max(below.stages).max(above.stages).checked_add(1).ok_or_else(|| invalid("native branch stage count overflows"))?;
        let added_depth = below.added_depth.max(above.added_depth).checked_add(1).ok_or_else(|| invalid("native branch depth overflows"))?;
        let depth = self.existing_depth.checked_add(added_depth).and_then(|n| n.checked_add(1)).ok_or_else(|| invalid("native branch depth overflows"))?;
        if stages > MAX_STAGES || depth > 64 {
            return Err(invalid("native branch join exceeds the existing nested preparation limit"));
        }
        let mut joined = self.template.as_ref().clone();
        for (name, input, blend, opacity, knockout) in
            [("Native lower branch", below, BlendMode::Normal, 1., false), ("Native upper branch", above, blend, opacity, knockout)]
        {
            let mut layer = Layer::new(
                name,
                LayerContent::Smart(SmartObject::new(
                    SmartSource::Embedded { file_name: "native-branch.pcraft".into(), bytes: input.source.clone() },
                    Affine::IDENTITY,
                    None,
                )),
            );
            layer.blend = blend;
            layer.opacity = opacity;
            if knockout {
                layer.fill_opacity = 0.;
                layer.advanced.knockout = photocraft_doc::Knockout::Shallow;
            }
            joined.layers.push(layer);
        }
        // Existing admission prices the actual native blob allocations/metadata,
        // including distinct nested branch bytes; aliases share their native Arc.
        let mut remaining = *budget;
        let source = encoded(&joined, &mut remaining, ctl)?;
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        self.source = source;
        self.stages = stages;
        self.added_depth = added_depth;
        *budget = remaining;
        Ok(())
    }

    /// Keep the actual native input editable beneath a static native layer mask.
    pub(super) fn masked(&self, mask: LayerMask, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Self> {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let mut next = self.fork();
        next.push(
            NativeMapping {
                structure: Some(NativeStructure::Composite { branches: vec![super::effects::NativeBranch { mask: Some(mask), ..Default::default() }] }),
                notes: vec![
                    "Actual native auxiliary pixels become a static editable native layer mask; its actual native input contents remain editable.".into(),
                ],
                ..Default::default()
            },
            NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None },
            budget,
            ctl,
        )?;
        Ok(next)
    }

    /// Store a derived native channel image as ordinary editable raster data.
    pub(super) fn with_snapshot_raster(&self, surface: Surface, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Self> {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        if surface.format() != self.template.pixel_format() {
            return Err(invalid("native snapshot raster must preserve its owner's native format"));
        }
        let mut next = self.fork();
        next.push(
            NativeMapping {
                structure: Some(NativeStructure::Replace {
                    layer: Box::new(Layer::new("Native captured channel", LayerContent::Raster(surface))),
                    patterns: Vec::new(),
                }),
                ..Default::default()
            },
            NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None },
            budget,
            ctl,
        )?;
        Ok(next)
    }

    /// One source operation becomes one native stage. A compound Normal stage's
    /// opacity is a native filter mask over its whole chain, never the last filter alone.
    pub(super) fn push(&mut self, mut mapping: NativeMapping, envelope: NativeEnvelope, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Vec<String>> {
        if mapping.working_rgb {
            ctl.check().map_err(|_| crate::IoError::Cancelled)?;
            if self.template.mode != ColorMode::Grayscale {
                return Err(invalid("native RGB working contents require a Gray owner"));
            }
            mapping.working_rgb = false;
            let mut template = self.template.as_ref().clone();
            template.mode = ColorMode::Rgb;
            template.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
            let mut pending = Self {
                template: Arc::new(template),
                source: self.source.clone(),
                lineage: self.lineage.clone(),
                origin: self.origin,
                stages: self.stages,
                added_depth: self.added_depth,
                existing_depth: self.existing_depth,
            };
            let mut remaining = *budget;
            let mut notes = pending.push(mapping, envelope, &mut remaining, ctl)?;
            ctl.check().map_err(|_| crate::IoError::Cancelled)?;
            self.source = pending.source;
            self.stages = pending.stages;
            self.added_depth = pending.added_depth;
            *budget = remaining;
            notes.push("Independent native color components are stored in an ordinary RGB/sRGB Smart contents document inside the Gray owner. PhotoCraft's ordinary Smart source conversion, RGB working-color filter rules and final RGB-to-Gray conversion apply; the following owner stage retains its original Gray mode/profile. No source color evaluator or persistent working-mode rule is added".into());
            return Ok(notes);
        }
        let NativeEnvelope { opacity, blend, visible, mut mask } = envelope;
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let turns = mapping.quarter_turns;
        let degrees = mapping.rotation_degrees;
        if !degrees.is_finite() || degrees.abs() > 360.0 || (turns != 0 && degrees != 0.0) {
            return Err(invalid("native rotation requires one finite orientation in -360..360 degrees"));
        }
        let (turns, degrees) =
            if degrees != 0.0 && (degrees / 90.0).fract() == 0.0 { (((degrees / 90.0).rem_euclid(4.0)) as u8, 0.0) } else { (turns, degrees) };
        let rotated = turns != 0 || degrees != 0.0;
        if turns > 3 {
            return Err(invalid("native filter orientation requires 0..3 quarter turns"));
        }
        let stage_depth = if rotated { 3 } else { 1 };
        if self.stages >= MAX_STAGES || self.existing_depth + self.added_depth + stage_depth + 1 > 64 {
            return Err(invalid("native filter contents exceed the existing nested preparation limit"));
        }
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(invalid("invalid native filter opacity"));
        }
        if let Some(structure) = mapping.structure.take() {
            if rotated || !mapping.layer_effects.is_empty() {
                return Err(invalid("native document structures require a separate filter or orientation stage"));
            }
            let envelope = opacity != 1.0 || blend != BlendMode::Normal || mask.is_some();
            let structure_notes =
                self.push_structure(structure, mapping.filters, mapping.patterns, NativeEnvelope { opacity, blend, visible, mask }, budget, ctl)?;
            mapping.notes.extend(structure_notes);
            if envelope {
                mapping.notes.push("Source operation coverage, opacity and blend become a separate native layer over the unchanged input. Native alpha compositing approximates source effect interpolation for semitransparent pixels".into());
            }
            mapping.notes.push(
                "The operation is an ordinary editable native document structure; its unmodified input remains available as a hidden native layer".into(),
            );
            return Ok(mapping.notes);
        }
        if mapping.filters.len() > 1 && blend != BlendMode::Normal {
            return Err(invalid("compound native filters need an explicit mapping for this source stage blend mode"));
        }
        if !mapping.layer_effects.is_empty() && opacity < 1.0 && (!mapping.filters.is_empty() || mapping.layer_effects.len() > 1) {
            return Err(invalid("compound native filter/style opacity requires a separate complete-stage structure"));
        }
        if mapping.filters.is_empty() && mapping.layer_effects.is_empty() {
            return Ok(mapping.notes);
        }
        let compound = mapping.filters.len() > 1;
        let mut remaining = *budget;
        if compound && opacity < 1.0 {
            let scaled = scale_mask(mask, opacity, remaining, ctl)?;
            remaining =
                remaining.checked_sub(surface_bytes(&scaled.surface)?).ok_or_else(|| invalid("native compound mask exceeds remaining source allowance"))?;
            mask = Some(scaled);
            mapping.notes.push("Source operation opacity applies once through an ordinary native filter mask over the compound native chain".into());
        }
        for filter in &mut mapping.filters {
            filter.visible &= visible;
            if !compound {
                filter.opacity *= opacity;
                if blend != BlendMode::Normal {
                    if filter.blend != BlendMode::Normal {
                        return Err(invalid("native helper blend and source stage blend require separate native composition stages"));
                    }
                    filter.blend = blend;
                }
            }
        }
        for effect in &mut mapping.layer_effects {
            use photocraft_doc::effects::Effect;
            effect.set_enabled(effect.enabled() && visible);
            match effect {
                Effect::DropShadow(value) | Effect::InnerShadow(value) => value.common.opacity *= opacity,
                Effect::OuterGlow(value) | Effect::InnerGlow(value) => value.common.opacity *= opacity,
                Effect::Stroke(value) => value.common.opacity *= opacity,
                Effect::ColorOverlay { common, .. } | Effect::GradientOverlay { common, .. } | Effect::PatternOverlay { common, .. } => {
                    common.opacity *= opacity
                }
                Effect::Satin(value) => value.common.opacity *= opacity,
                Effect::BevelEmboss(value) => {
                    value.highlight.opacity *= opacity;
                    value.shadow.opacity *= opacity;
                }
            }
        }
        if !mapping.layer_effects.is_empty() && (blend != BlendMode::Normal || mask.is_some()) {
            return Err(invalid("native style stages need an explicit structure for a separate source blend or effect mask"));
        }
        if rotated && !mapping.layer_effects.is_empty() {
            return Err(invalid("native style orientation requires an explicit lighting and paint conversion"));
        }
        let mut stage_template = self.template.as_ref().clone();
        add_patterns(&mut stage_template, mapping.patterns, &mut remaining, ctl)?;
        let (rotation, rotated_size) =
            if degrees == 0.0 { orientation(self.template.size, turns) } else { arbitrary_orientation(self.template.size, degrees)? };
        let work = u64::from(rotated_size.width)
            .checked_add(2)
            .and_then(|width| u64::from(rotated_size.height).checked_add(2).and_then(|height| width.checked_mul(height)))
            .and_then(|pixels| pixels.checked_mul(self.template.pixel_format().channels().max(2) as u64 * 4))
            .ok_or_else(|| invalid("native rotation working extent overflows"))?;
        if rotated && (work > super::MAX_PIXEL_BYTES as u64 || work > *budget as u64) {
            return Err(invalid("native rotation expanded working extent exceeds the shared source allowance"));
        }
        let input = if !rotated {
            self.source.clone()
        } else {
            stage_template.size = rotated_size;
            let mut rotated = stage_template.clone();
            rotated.layers.push(Layer::new(
                "Native orientation",
                LayerContent::Smart(SmartObject::new(
                    SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: self.source.clone() },
                    rotation,
                    None,
                )),
            ));
            mask = mask
                .map(|mask| if degrees == 0.0 { rotate_mask(mask, self.template.size, turns, ctl) } else { warp_mask(mask, self.template.size, rotation, ctl) })
                .transpose()?;
            if let Some(mask) = &mask {
                remaining =
                    remaining.checked_sub(surface_bytes(&mask.surface)?).ok_or_else(|| invalid("native rotated coverage exceeds shared source allowance"))?;
            }
            mapping.notes.push(format!(
                "Native smart-object placement rotates {} degrees for the native directional filter, then restores the original document orientation; native image interpolation and rotated canvas edge rules apply",
                if degrees == 0.0 { f64::from(turns) * 90.0 } else { degrees }
            ));
            encoded(&rotated, &mut remaining, ctl)?
        };
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: input }, Affine::IDENTITY, None);
        smart.smart_filters = mapping.filters;
        smart.filter_mask = mask;
        let mut stage = Layer::new("Native filter stage", LayerContent::Smart(smart));
        stage.effects.items = mapping.layer_effects;
        let mut document = stage_template;
        document.layers.push(stage);
        let mut source = encoded(&document, &mut remaining, ctl)?;
        if rotated {
            let inverse = rotation.inverse().ok_or_else(|| invalid("native orientation placement is not invertible"))?;
            let mut restored = self.template.as_ref().clone();
            restored.layers.push(Layer::new(
                "Native restored orientation",
                LayerContent::Smart(SmartObject::new(SmartSource::Embedded { file_name: "oriented-filter.pcraft".into(), bytes: source }, inverse, None)),
            ));
            source = encoded(&restored, &mut remaining, ctl)?;
        }
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        self.source = source;
        self.stages += 1;
        self.added_depth += stage_depth;
        *budget = remaining;
        Ok(mapping.notes)
    }

    fn push_stages(
        &mut self,
        stages: (Vec<NativeStage>, usize),
        post_filters: Vec<photocraft_doc::SmartFilter>,
        patterns: Vec<photocraft_doc::Pattern>,
        envelope: NativeEnvelope,
        budget: &mut usize,
        ctl: &Interrupt<'_>,
    ) -> Result<Vec<String>> {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let (steps, output) = stages;
        super::effects::validate_stages(&steps, output).map_err(|failure| invalid(failure.message))?;
        let mut remaining = *budget;
        let mut values = Vec::new();
        values.try_reserve_exact(steps.len() + 1).map_err(|_| invalid("native stage snapshot allocation unavailable"))?;
        values.push(self.fork());
        let mut notes = Vec::new();
        for step in steps {
            ctl.check().map_err(|_| crate::IoError::Cancelled)?;
            let next = match step {
                NativeStage::Apply { input, mapping } => {
                    let mut next = values.get(input).ok_or_else(|| invalid("native stage input disappeared"))?.fork();
                    notes.extend(next.push(
                        *mapping,
                        NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None },
                        &mut remaining,
                        ctl,
                    )?);
                    next
                }
                NativeStage::Join { below, above, blend, opacity } => {
                    let below = values.get(below).ok_or_else(|| invalid("native lower stage input disappeared"))?;
                    let above = values.get(above).ok_or_else(|| invalid("native upper stage input disappeared"))?;
                    let mut next = self.fork();
                    next.join(below, above, blend, opacity, &mut remaining, ctl)?;
                    next
                }
                NativeStage::Knockout { below, above, opacity } => {
                    let below = values.get(below).ok_or_else(|| invalid("native lower knockout input disappeared"))?;
                    let above = values.get(above).ok_or_else(|| invalid("native upper knockout input disappeared"))?;
                    let mut next = self.fork();
                    next.knockout(below, above, opacity, &mut remaining, ctl)?;
                    next
                }
                NativeStage::Align { input, target, horizontal, vertical, margin_x, margin_y, snap } => {
                    let input = values.get(input).ok_or_else(|| invalid("native alignment input disappeared"))?;
                    let target = values.get(target).ok_or_else(|| invalid("native alignment target disappeared"))?;
                    let pixels = input.snapshot(&mut remaining, ctl)?;
                    let target_pixels = target.snapshot(&mut remaining, ctl)?;
                    let frame = Rect::new(0, 0, input.template.size.width as i32, input.template.size.height as i32);
                    let result = super::native_alignment::mapping(
                        &pixels,
                        &target_pixels,
                        frame,
                        super::native_alignment::Controls { horizontal, vertical, margin_x, margin_y, snap },
                        ctl,
                    );
                    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
                    let mapping = result.map_err(|failure| {
                        if failure.reason == photocraft_doc::import_conversion::ConversionReason::ResourceUnavailable {
                            crate::IoError::Unsupported(failure.message)
                        } else {
                            invalid(failure.message)
                        }
                    })?;
                    let mut next = input.fork();
                    notes.extend(next.push(
                        mapping,
                        NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None },
                        &mut remaining,
                        ctl,
                    )?);
                    next
                }
                NativeStage::Displace { input, map, coordinates, scale, wrap, metadata_limit } => {
                    let input = values.get(input).ok_or_else(|| invalid("native displacement input disappeared"))?;
                    let map = values.get(map).ok_or_else(|| invalid("native displacement map input disappeared"))?;
                    let pixels = map.snapshot(&mut remaining, ctl)?;
                    let frame = Rect::new(0, 0, input.template.size.width as i32, input.template.size.height as i32);
                    let mapping = super::native_displacement::mapping(
                        &pixels,
                        frame,
                        super::native_displacement::Controls { coordinates, scale, wrap, metadata_limit },
                        &mut remaining,
                        ctl,
                    )?;
                    let mut next = input.fork();
                    notes.extend(next.push(
                        mapping,
                        NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None },
                        &mut remaining,
                        ctl,
                    )?);
                    next
                }
                NativeStage::Channel { input, component } => {
                    let input = values.get(input).ok_or_else(|| invalid("native channel input disappeared"))?;
                    let pixels = input.snapshot(&mut remaining, ctl)?;
                    let frame = Rect::new(0, 0, input.template.size.width as i32, input.template.size.height as i32);
                    let channel = super::native_masks::channel(&pixels, frame, component, &mut remaining, ctl)?;
                    let next = input.with_snapshot_raster(channel, &mut remaining, ctl)?;
                    notes.push(
                        "Captured native channel pixels become an ordinary static raster resource; the actual input native contents remain editable.".into(),
                    );
                    next
                }
                NativeStage::Mask { input, mask, component, invert, scale } => {
                    let input = values.get(input).ok_or_else(|| invalid("native masked input disappeared"))?;
                    let mask = values.get(mask).ok_or_else(|| invalid("native mask input disappeared"))?;
                    let pixels = mask.snapshot(&mut remaining, ctl)?;
                    let frame = Rect::new(0, 0, input.template.size.width as i32, input.template.size.height as i32);
                    let mask = super::native_masks::mask(&pixels, frame, component, invert, scale, &mut remaining, ctl)?;
                    let next = input.masked(mask, &mut remaining, ctl)?;
                    notes.push(
                        "Actual native auxiliary pixels become a static editable native layer mask; its actual native input contents remain editable.".into(),
                    );
                    next
                }
            };
            values.push(next);
        }
        let selected = values.get(output).ok_or_else(|| invalid("native stage output disappeared"))?;
        let has_post = !post_filters.is_empty();
        let outer = envelope.opacity != 1. || envelope.blend != BlendMode::Normal || envelope.mask.is_some() || (!envelope.visible && has_post);
        let added_depth = self
            .added_depth
            .max(selected.added_depth)
            .checked_add(1 + usize::from(has_post) + usize::from(outer))
            .ok_or_else(|| invalid("native stages depth overflows"))?;
        let stages = self.stages.max(selected.stages).checked_add(1).ok_or_else(|| invalid("native stages count overflows"))?;
        if stages > MAX_STAGES || self.existing_depth.checked_add(added_depth).and_then(|n| n.checked_add(1)).is_none_or(|n| n > 64) {
            return Err(invalid("native stages exceed the existing nested preparation limit"));
        }
        let mut document = self.template.as_ref().clone();
        add_patterns(&mut document, patterns, &mut remaining, ctl)?;
        let active = envelope.visible || outer;
        let mut original = Layer::new(
            "Native unchanged construction input",
            LayerContent::Smart(SmartObject::new(
                SmartSource::Embedded { file_name: "native-original.pcraft".into(), bytes: self.source.clone() },
                Affine::IDENTITY,
                None,
            )),
        );
        original.visible = !active;
        document.layers.push(original);
        let mut result = Layer::new(
            "Native constructed output",
            LayerContent::Smart(SmartObject::new(
                SmartSource::Embedded { file_name: "native-construction.pcraft".into(), bytes: selected.source.clone() },
                Affine::IDENTITY,
                None,
            )),
        );
        result.visible = active;
        document.layers.push(result);
        let source = self.encode_structure(document, post_filters, envelope, &mut remaining, ctl)?;
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        self.source = source;
        self.stages = stages;
        self.added_depth = added_depth;
        *budget = remaining;
        Ok(notes)
    }

    fn push_structure(
        &mut self,
        structure: NativeStructure,
        post_filters: Vec<photocraft_doc::SmartFilter>,
        patterns: Vec<photocraft_doc::Pattern>,
        envelope: NativeEnvelope,
        budget: &mut usize,
        ctl: &Interrupt<'_>,
    ) -> Result<Vec<String>> {
        let structure = match structure {
            NativeStructure::Stages { steps, output } => return self.push_stages((steps, output), post_filters, patterns, envelope, budget, ctl),
            other => other,
        };
        let NativeEnvelope { opacity, blend, visible, mask } = envelope;
        let has_post = !post_filters.is_empty();
        let envelope = opacity != 1.0 || blend != BlendMode::Normal || mask.is_some() || (!visible && has_post);
        let branch_depth = match &structure {
            NativeStructure::Composite { branches } => {
                branches.iter().map(|branch| usize::from(branch.filter_frame.is_some()) + usize::from(!branch.post_layer_effects.is_empty())).max().unwrap_or(0)
            }
            NativeStructure::Replace { .. } => 0,
            NativeStructure::Stages { .. } => return Err(invalid("nested native stages were not flattened")),
        };
        let stage_depth = 1 + usize::from(envelope) + usize::from(has_post) + branch_depth;
        if self.existing_depth + self.added_depth + stage_depth + 1 > 64 {
            return Err(invalid("native structural envelope exceeds embedded preparation depth"));
        }
        let computed_visible = visible || envelope;
        let mut remaining = *budget;
        let mut document = self.template.as_ref().clone();
        add_patterns(&mut document, patterns, &mut remaining, ctl)?;
        let input = || {
            Layer::new(
                "Native original input",
                LayerContent::Smart(SmartObject::new(
                    SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: self.source.clone() },
                    Affine::IDENTITY,
                    None,
                )),
            )
        };
        let mut original = input();
        original.visible = !computed_visible;
        document.layers.push(original);
        match structure {
            NativeStructure::Replace { mut layer, patterns } => {
                // The replacement is already in owner-local native coordinates.
                let depth = nested_depth(std::slice::from_ref(&layer), 0, 0, &mut (super::MAX_PIXEL_BYTES as u64), ctl)?;
                if depth != 0 {
                    return Err(invalid("native replacement must contain direct editable native data rather than further embedded contents"));
                }
                let allocation = direct_layer_bytes(&layer, 0, &mut 65536, ctl)?;
                remaining = remaining.checked_sub(allocation).ok_or_else(|| invalid("native replacement allocation exceeds the shared source allowance"))?;
                layer.visible &= computed_visible;
                document.layers.push(*layer);
                add_patterns(&mut document, patterns, &mut remaining, ctl)?;
            }
            NativeStructure::Composite { branches } => {
                if branches.is_empty() || branches.len() > 64 {
                    return Err(invalid("native composite requires 1..64 explicit branches"));
                }
                for branch in branches {
                    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
                    if !branch.opacity.is_finite()
                        || !(0.0..=1.0).contains(&branch.opacity)
                        || !branch.fill_opacity.is_finite()
                        || !(0.0..=1.0).contains(&branch.fill_opacity)
                        || branch.filters.len() > 64
                        || branch.layer_effects.len() > 64
                        || branch.post_layer_effects.len() > 64
                    {
                        return Err(invalid("invalid native composite branch opacity or filter count"));
                    }
                    remaining = remaining
                        .checked_sub(parameter_bytes(&branch.filters)?)
                        .ok_or_else(|| invalid("native composite parameters exceed the shared source allowance"))?;
                    if branch.transform.m.iter().any(|value| !value.is_finite() || value.abs() > 1.0e10) || branch.transform.inverse().is_none() {
                        return Err(invalid("native composite placement is nonfinite, out of bounds, or singular"));
                    }
                    if let Some(matrix) = branch.perspective {
                        let inverse = photocraft_algo::transform::Homography(matrix).inverse();
                        if matrix.iter().any(|value| !value.is_finite() || value.abs() > 1.0e10)
                            || inverse.is_none_or(|inverse| inverse.0.iter().any(|value| !value.is_finite()))
                        {
                            return Err(invalid("native composite perspective is nonfinite, out of bounds, or singular"));
                        }
                    }
                    if let Some(mask) = &branch.mask {
                        remaining = remaining
                            .checked_sub(surface_bytes(&mask.surface)?)
                            .ok_or_else(|| invalid("native branch coverage exceeds the shared source allowance"))?;
                    }
                    if let Some(mask) = &branch.vector_mask {
                        let knots = mask
                            .path
                            .subpaths
                            .iter()
                            .try_fold(0usize, |sum, path| sum.checked_add(path.knots.len()).ok_or_else(|| invalid("native branch geometry overflows")))?;
                        let bytes =
                            knots.checked_mul(std::mem::size_of::<photocraft_doc::Knot>()).ok_or_else(|| invalid("native branch geometry overflows"))?;
                        remaining = remaining.checked_sub(bytes).ok_or_else(|| invalid("native branch geometry exceeds the shared source allowance"))?;
                    }
                    remaining = remaining
                        .checked_sub(serialized_bytes!(&branch.layer_effects)?)
                        .ok_or_else(|| invalid("native branch styles exceed the shared source allowance"))?;
                    let mut layer = if let Some(fill) = branch.fill {
                        if branch.transform != Affine::IDENTITY || branch.perspective.is_some() || branch.filter_frame.is_some() || !branch.filters.is_empty() {
                            return Err(invalid("native fill branches require identity placement and a separate native wrapper for filters"));
                        }
                        remaining = remaining.checked_sub(serialized_bytes!(&fill)?).ok_or_else(|| invalid("native branch fill exceeds shared allowance"))?;
                        Layer::new("Native fill branch", LayerContent::Fill(fill))
                    } else {
                        input()
                    };
                    layer.name = "Native composite branch".into();
                    layer.visible = computed_visible;
                    layer.opacity = branch.opacity;
                    layer.fill_opacity = branch.fill_opacity;
                    layer.advanced = branch.advanced;
                    layer.blend = branch.blend;
                    layer.clipped = branch.clipped;
                    layer.mask = branch.mask;
                    layer.vector_mask = branch.vector_mask;
                    layer.effects.items = branch.layer_effects;
                    if let Some(frame) = branch.filter_frame {
                        if branch.perspective.is_some() {
                            return Err(invalid("native filter frame with perspective is not represented"));
                        }
                        if frame.is_empty()
                            || frame.width() > MAX_SIDE
                            || frame.height() > MAX_SIDE
                            || frame.x0.checked_neg().is_none()
                            || frame.y0.checked_neg().is_none()
                        {
                            return Err(invalid("native filter frame exceeds supported dimensions or origin"));
                        }
                        let work = u64::from(frame.width())
                            .checked_add(2)
                            .and_then(|w| u64::from(frame.height()).checked_add(2).and_then(|h| w.checked_mul(h)))
                            .and_then(|n| n.checked_mul(self.template.pixel_format().channels().max(2) as u64 * 4))
                            .and_then(|n| n.checked_add((self.source.len() as u64).checked_mul(2)?))
                            .and_then(|n| n.checked_add(65536))
                            .ok_or_else(|| invalid("native filter frame working allowance overflows"))?;
                        if work > remaining as u64 || work > super::MAX_PIXEL_BYTES as u64 {
                            return Err(invalid("native filter frame working/encoded extent exceeds remaining source allowance"));
                        }
                        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
                        let mut framed = self.template.as_ref().clone();
                        framed.size = Size::new(frame.width(), frame.height());
                        framed.patterns = document.patterns.clone();
                        let mut framed_input = input();
                        if let LayerContent::Smart(smart) = &mut framed_input.content {
                            smart.transform = Affine::translate(-f64::from(frame.x0), -f64::from(frame.y0));
                            smart.smart_filters = branch.filters;
                        }
                        framed.layers.push(framed_input);
                        let bytes = encoded(&framed, &mut remaining, ctl)?;
                        layer.content = LayerContent::Smart(SmartObject::new(
                            SmartSource::Embedded { file_name: "native-filter-frame.pcraft".into(), bytes },
                            branch.transform.mul(&Affine::translate(f64::from(frame.x0), f64::from(frame.y0))),
                            None,
                        ));
                    } else if let LayerContent::Smart(smart) = &mut layer.content {
                        smart.transform = branch.transform;
                        smart.perspective = branch.perspective;
                        smart.smart_filters = branch.filters;
                    }
                    if !branch.post_layer_effects.is_empty() {
                        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
                        remaining = remaining
                            .checked_sub(serialized_bytes!(&branch.post_layer_effects)?)
                            .ok_or_else(|| invalid("native post-branch styles exceed shared allowance"))?;
                        let mut inner_document = self.template.as_ref().clone();
                        inner_document.patterns = document.patterns.clone();
                        let mut inner = Layer::new("Native pre-style branch", layer.content);
                        inner.effects = std::mem::take(&mut layer.effects);
                        layer.effects.enabled = true;
                        inner.fill_opacity = layer.fill_opacity;
                        inner_document.layers.push(inner);
                        let bytes = encoded(&inner_document, &mut remaining, ctl)?;
                        layer.content = LayerContent::Smart(SmartObject::new(
                            SmartSource::Embedded { file_name: "native-styled-branch.pcraft".into(), bytes },
                            Affine::IDENTITY,
                            None,
                        ));
                        layer.fill_opacity = 1.;
                        layer.effects.items = branch.post_layer_effects;
                        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
                    }
                    document.layers.push(layer);
                }
            }
            NativeStructure::Stages { .. } => return Err(invalid("nested native stages were not flattened")),
        }
        let source = self.encode_structure(document, post_filters, NativeEnvelope { opacity, blend, visible, mask }, &mut remaining, ctl)?;
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        self.source = source;
        self.stages += 1;
        self.added_depth += stage_depth;
        *budget = remaining;
        Ok(Vec::new())
    }

    fn encode_structure(
        &self,
        document: Document,
        post_filters: Vec<photocraft_doc::SmartFilter>,
        envelope: NativeEnvelope,
        remaining: &mut usize,
        ctl: &Interrupt<'_>,
    ) -> Result<Arc<Vec<u8>>> {
        let NativeEnvelope { opacity, blend, visible, mask } = envelope;
        let has_post = !post_filters.is_empty();
        let envelope = opacity != 1.0 || blend != BlendMode::Normal || mask.is_some() || (!visible && has_post);
        let mut source = encoded(&document, remaining, ctl)?;
        if has_post {
            if post_filters.len() > 64 {
                return Err(invalid("native structural post-filter count exceeds its limit"));
            }
            *remaining =
                remaining.checked_sub(parameter_bytes(&post_filters)?).ok_or_else(|| invalid("native structural post-filters exceed shared allowance"))?;
            let mut post = self.template.as_ref().clone();
            let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "native-structure.pcraft".into(), bytes: source }, Affine::IDENTITY, None);
            smart.smart_filters = post_filters;
            post.layers.push(Layer::new("Native structural post-filters", LayerContent::Smart(smart)));
            source = encoded(&post, remaining, ctl)?;
        }
        if envelope {
            let mut wrapped = self.template.as_ref().clone();
            wrapped.layers.push(Layer::new(
                "Native original input",
                LayerContent::Smart(SmartObject::new(
                    SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: self.source.clone() },
                    Affine::IDENTITY,
                    None,
                )),
            ));
            let mut output = Layer::new(
                "Native operation coverage",
                LayerContent::Smart(SmartObject::new(
                    SmartSource::Embedded { file_name: "native-operation.pcraft".into(), bytes: source },
                    Affine::IDENTITY,
                    None,
                )),
            );
            output.visible = visible;
            output.opacity = opacity;
            output.blend = blend;
            if let Some(mask) = &mask {
                *remaining =
                    remaining.checked_sub(surface_bytes(&mask.surface)?).ok_or_else(|| invalid("native stage coverage exceeds the shared source allowance"))?;
            }
            output.mask = mask;
            wrapped.layers.push(output);
            source = encoded(&wrapped, remaining, ctl)?;
        }
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        Ok(source)
    }

    /// Capture current ordinary native contents once for a static native resource.
    /// The native document remains editable and unchanged; returned pixels are owner-local.
    pub(super) fn snapshot(&self, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
        super::native_snapshot::capture(&self.source, &self.template, budget, ctl)
    }

    pub(super) fn content(&self) -> LayerContent {
        LayerContent::Smart(SmartObject::new(
            SmartSource::Embedded { file_name: "native-contents.pcraft".into(), bytes: self.source.clone() },
            Affine::translate(f64::from(self.origin.0), f64::from(self.origin.1)),
            None,
        ))
    }
}

pub(super) fn orientation(size: Size, turns: u8) -> (Affine, Size) {
    let (w, h) = (f64::from(size.width), f64::from(size.height));
    match turns {
        1 => (Affine { m: [0.0, 1.0, -1.0, 0.0, h, 0.0] }, Size::new(size.height, size.width)),
        2 => (Affine { m: [-1.0, 0.0, 0.0, -1.0, w, h] }, size),
        3 => (Affine { m: [0.0, -1.0, 1.0, 0.0, 0.0, w] }, Size::new(size.height, size.width)),
        _ => (Affine::IDENTITY, size),
    }
}

pub(super) fn arbitrary_orientation(size: Size, degrees: f64) -> Result<(Affine, Size)> {
    let rotation = Affine::rotate(degrees.to_radians());
    let corners = [(0.0, 0.0), (f64::from(size.width), 0.0), (f64::from(size.width), f64::from(size.height)), (0.0, f64::from(size.height))]
        .map(|(x, y)| rotation.apply(photocraft_geom::Point::new(x, y)));
    let left = corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min).floor();
    let top = corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min).floor();
    let width = corners.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max).ceil() - left;
    let height = corners.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max).ceil() - top;
    if !width.is_finite() || !height.is_finite() || width < 1.0 || height < 1.0 || width > f64::from(MAX_SIDE) || height > f64::from(MAX_SIDE) {
        return Err(invalid("native rotated canvas exceeds document limits"));
    }
    Ok((Affine::translate(-left, -top).mul(&rotation), Size::new(width as u32, height as u32)))
}

/// Existing native Free Transform resampling, rather than a source-specific sampler.
fn warp_mask(mut mask: LayerMask, size: Size, placement: Affine, ctl: &Interrupt<'_>) -> Result<LayerMask> {
    if mask.surface.channels() != 1 {
        return Err(invalid("native rotated effect coverage must be grayscale"));
    }
    let [a, b, c, d, e, f] = placement.m;
    let matrix = photocraft_algo::transform::Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0]);
    let rect = Rect::new(0, 0, size.width as i32, size.height as i32);
    let bytes = photocraft_algo::transform::warp_output_size(rect, &matrix)
        .and_then(|(w, h)| w.checked_mul(h)?.checked_mul(8))
        .ok_or_else(|| invalid("native coverage rotation extent overflows"))?;
    if bytes > super::MAX_PIXEL_BYTES as u64 {
        return Err(invalid("native coverage rotation exceeds its working allowance"));
    }
    let warped = photocraft_algo::transform::warp_surface_with(&mask.surface, rect, &matrix, photocraft_algo::transform::Interp::Bilinear, ctl)
        .map_err(|e| invalid(format!("native coverage rotation: {e}")))?
        .ok_or(crate::IoError::Cancelled)?;
    mask.surface = warped.try_convert(mask.surface.format()).map_err(|e| invalid(format!("native coverage rotation conversion: {e}")))?;
    Ok(mask)
}

/// Exact native pixel-grid rotation of a mask, preserving its sparse default.
/// One tile is the working set; no source sampling or renderer is introduced.
fn rotate_mask(mut mask: LayerMask, size: Size, turns: u8, ctl: &Interrupt<'_>) -> Result<LayerMask> {
    if turns == 0 {
        return Ok(mask);
    }
    if mask.surface.channels() != 1 {
        return Err(invalid("native effect coverage must be a one-channel surface"));
    }
    let mut out = Surface::with_default(mask.surface.format(), &mask.surface.default_pixel());
    let (w, h) = (i64::from(size.width), i64::from(size.height));
    for (coordinate, _) in mask.surface.tiles() {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let rect = coordinate.rect();
        let (tw, th) = (rect.width() as usize, rect.height() as usize);
        let old = mask.surface.read_region(rect);
        let mut values = vec![0.0; old.len()];
        let (x0, y0, x1, y1) = match turns {
            1 => (h - i64::from(rect.y1), i64::from(rect.x0), h - i64::from(rect.y0), i64::from(rect.x1)),
            2 => (w - i64::from(rect.x1), h - i64::from(rect.y1), w - i64::from(rect.x0), h - i64::from(rect.y0)),
            3 => (i64::from(rect.y0), w - i64::from(rect.x1), i64::from(rect.y1), w - i64::from(rect.x0)),
            _ => return Err(invalid("invalid native mask orientation")),
        };
        let convert = |v: i64| i32::try_from(v).map_err(|_| invalid("native rotated mask exceeds coordinate range"));
        let target = Rect::new(convert(x0)?, convert(y0)?, convert(x1)?, convert(y1)?);
        let stride = target.width() as usize;
        for y in 0..th {
            for x in 0..tw {
                let (tx, ty) = match turns {
                    1 => (th - 1 - y, x),
                    2 => (tw - 1 - x, th - 1 - y),
                    3 => (y, tw - 1 - x),
                    _ => return Err(invalid("invalid native mask orientation")),
                };
                let index = ty.checked_mul(stride).and_then(|v| v.checked_add(tx)).ok_or_else(|| invalid("native rotated mask index overflow"))?;
                let input =
                    y.checked_mul(tw).and_then(|v| v.checked_add(x)).and_then(|i| old.get(i)).copied().ok_or_else(|| invalid("native mask sample range"))?;
                *values.get_mut(index).ok_or_else(|| invalid("native rotated mask sample range"))? = input;
            }
        }
        out.write_region(target, &values);
        let side = photocraft_geom::TILE_SIZE as usize;
        let tile_bytes = side * side * mask.surface.format().bytes_per_pixel();
        if out.tile_count() > super::MAX_PIXEL_BYTES / tile_bytes {
            return Err(invalid("native rotated mask exceeds its 1 GiB pixel allowance"));
        }
    }
    out.prune();
    mask.surface = out;
    Ok(mask)
}

fn scale_mask(mask: Option<LayerMask>, opacity: f32, allowance: usize, ctl: &Interrupt<'_>) -> Result<LayerMask> {
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(invalid("native compound mask opacity must be finite normalized coverage"));
    }
    let Some(mask) = mask else {
        if allowance < std::mem::size_of::<f32>() {
            return Err(invalid("native compound mask exceeds remaining source allowance"));
        }
        return Ok(LayerMask {
            surface: Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[opacity]),
            ..LayerMask::reveal_all()
        });
    };
    if !mask.enabled {
        return scale_mask(None, opacity, allowance, ctl);
    }
    if mask.surface.channels() != 1 {
        return Err(invalid("native compound mask must be a one-channel surface"));
    }
    let side = photocraft_geom::TILE_SIZE as usize;
    let tile_bytes =
        side.checked_mul(side).and_then(|n| n.checked_mul(std::mem::size_of::<f32>())).ok_or_else(|| invalid("native compound mask tile size overflows"))?;
    let pixels = mask
        .surface
        .tile_count()
        .checked_mul(tile_bytes)
        .and_then(|n| n.checked_add(std::mem::size_of::<f32>()))
        .ok_or_else(|| invalid("native compound mask allocation overflows"))?;
    // One F32 tile is read/adjusted at a time in addition to the admitted output.
    let peak =
        pixels.checked_add(if mask.surface.tile_count() == 0 { 0 } else { tile_bytes }).ok_or_else(|| invalid("native compound mask work size overflows"))?;
    if peak > allowance || peak > super::MAX_PIXEL_BYTES {
        return Err(invalid("native compound F32 mask and tile scratch exceed remaining source allowance"));
    }
    let sample = mask.surface.default_pixel().first().copied().unwrap_or(0.0);
    if !sample.is_finite() || !(0.0..=1.0).contains(&sample) || !mask.density.is_finite() || !(0.0..=1.0).contains(&mask.density) {
        return Err(invalid("native effect mask must contain normalized finite coverage and density"));
    }
    let default = (1.0 - mask.density * (1.0 - sample)) * opacity;
    let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
    let mut surface = Surface::with_default(fmt, &[default]);
    for (coordinate, _) in mask.surface.tiles() {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let rect = coordinate.rect();
        let mut values = Vec::new();
        mask.values_into(rect, &mut values);
        for value in &mut values {
            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                return Err(invalid("native effect mask contains invalid coverage"));
            }
            *value *= opacity;
        }
        surface.try_write_region(rect, &values).map_err(|e| invalid(format!("native compound mask allocation: {e}")))?;
    }
    Ok(LayerMask { surface, ..LayerMask::reveal_all() })
}

#[cfg(test)]
mod tests {
    use super::super::effects::native_filter;
    use super::*;
    use photocraft_color::{Color, SampleType};
    use photocraft_doc::effects::{Effect, FxCommon};
    use serde_json::json;

    fn document() -> Document {
        let mut doc = Document::new("Native", Size::new(40, 30), ColorMode::Rgb, SampleType::F32);
        doc.resolution_dpi = 144.0;
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        doc
    }
    fn owner(doc: &Document) -> Layer {
        let mut layer = Layer::raster("Source owner", doc.pixel_format());
        layer.surface_mut().unwrap().write_region(Rect::new(-5, 7, -2, 9), &[0.2, 0.4, 0.7, 1.0].repeat(6));
        layer.opacity = 0.6;
        layer.blend = BlendMode::Multiply;
        layer.mask = Some(LayerMask::hide_all());
        layer
    }
    fn mapping(commands: &[&str]) -> NativeMapping {
        NativeMapping { filters: commands.iter().map(|id| native_filter(id, json!({}))).collect(), ..Default::default() }
    }
    fn embedded(content: &LayerContent) -> Document {
        let LayerContent::Smart(smart) = content else { panic!("smart") };
        let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("embedded") };
        photocraft_format::load_from_bytes(bytes).unwrap()
    }
    #[test]
    fn rgb_working_contents_keep_gray_input_and_following_gray_stage_native_on_reopen() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::new("Gray owner", Size::new(4, 3), ColorMode::Grayscale, depth);
            doc.icc_profile = Some(photocraft_cms::Builtin::GrayGamma22.profile().to_bytes());
            let mut input = Layer::raster("Editable Gray original", doc.pixel_format());
            input.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.25, 0.5]);
            let original = input.clone();
            let mut budget = 1 << 30;
            let mut builder = NativeOwner::new(&doc, &input, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
            let stage = NativeMapping {
                working_rgb: true,
                filters: vec![native_filter("image.adjustments.channelMixer", json!({"red":[100.,0.,0.,0.],"green":[0.,100.,0.,0.],"blue":[0.,0.,0.,0.]}))],
                ..Default::default()
            };
            builder.push(stage, NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
            let rgb = embedded(&builder.content());
            assert_eq!(rgb.mode, ColorMode::Rgb);
            assert_eq!(rgb.depth, depth);
            assert_eq!(rgb.icc_profile, Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()));
            let gray = embedded(&rgb.layers[0].content);
            assert_eq!(gray.mode, ColorMode::Grayscale);
            assert_eq!(gray.layers[0].content, original.content);
            assert_eq!(builder.template.mode, ColorMode::Grayscale);
            assert_eq!(builder.template.icc_profile, doc.icc_profile);
            builder
                .push(
                    mapping(&["image.adjustments.invert"]),
                    NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None },
                    &mut budget,
                    &Interrupt::NONE,
                )
                .unwrap();
            let after = embedded(&builder.content());
            assert_eq!(after.mode, ColorMode::Grayscale);
            assert_eq!(after.icc_profile, doc.icc_profile);
            assert_eq!(embedded(&after.layers[0].content), rgb);
            doc.layers.push(Layer::new("Native translated stages", builder.content()));
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened, doc);
            let reopened_gray = embedded(&reopened.layers[0].content);
            assert_eq!(embedded(&reopened_gray.layers[0].content), rgb);
        }
    }
    #[test]
    fn rgb_working_failure_and_late_cancel_preserve_budget_template_and_stages() {
        let doc = Document::new("Gray", Size::new(4, 3), ColorMode::Grayscale, SampleType::F32);
        let input = Layer::raster("Original", doc.pixel_format());
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &input, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
        let before = builder.source.clone();
        let template = builder.template.clone();
        let saved_budget = budget;
        let make = || NativeMapping { working_rgb: true, ..mapping(&["image.adjustments.invert"]) };
        let envelope = || NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None };
        let mut tiny = 1;
        assert!(builder.push(make(), envelope(), &mut tiny, &Interrupt::NONE).is_err());
        assert_eq!(tiny, 1);
        assert_eq!(builder.source, before);
        assert_eq!(builder.template, template);
        assert_eq!(builder.stages, 0);
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 3;
        assert!(builder.push(make(), envelope(), &mut budget, &Interrupt::cancel_only(&cancel)).is_err());
        assert!(checks.load(Ordering::Relaxed) >= 4);
        assert_eq!(budget, saved_budget);
        assert_eq!(builder.source, before);
        assert_eq!(builder.template, template);
        assert_eq!(builder.stages, 0);
        let rgb = document();
        let input = owner(&rgb);
        let mut rgb_budget = 1 << 30;
        let mut owner = NativeOwner::new(&rgb, &input, Rect::new(-5, 7, -2, 9), &mut rgb_budget, &Interrupt::NONE).unwrap();
        assert!(owner.push(make(), envelope(), &mut rgb_budget, &Interrupt::NONE).is_err());
    }
    #[test]
    fn native_filter_frame_rebases_parent_canvas_and_restores_outer_state_once() {
        use super::super::effects::NativeBranch;
        use photocraft_doc::effects::{Effect, FxCommon};
        let doc = document();
        let original = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &original, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let frame = Rect::new(-1, -2, 5, 2);
        let style = Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.7), color: photocraft_color::Color::rgb(0.2, 0.4, 0.6) };
        let mask =
            LayerMask { surface: Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]), ..LayerMask::reveal_all() };
        let mapping = NativeMapping {
            structure: Some(NativeStructure::Composite {
                branches: vec![NativeBranch {
                    filters: vec![super::super::effects::native_filter("filter.distort.twirl", serde_json::json!({"angle":90}))],
                    filter_frame: Some(frame),
                    transform: Affine::translate(10.0, 20.0),
                    opacity: 0.8,
                    fill_opacity: 0.6,
                    mask: Some(mask),
                    layer_effects: vec![style.clone()],
                    post_layer_effects: vec![style],
                    ..Default::default()
                }],
            }),
            ..Default::default()
        };
        builder.push(mapping, NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(builder.added_depth, 3);
        let outer = embedded(&builder.content());
        let branch = &outer.layers[1];
        assert_eq!(branch.opacity, 0.8);
        assert_eq!(branch.mask.as_ref().unwrap().value(0, 0), 0.5);
        let styled = embedded(&branch.content);
        let before_style = &styled.layers[0];
        assert!(before_style.mask.is_none());
        assert_eq!(before_style.effects.items.len(), 1);
        assert!(before_style.effects.enabled);
        let LayerContent::Smart(restored) = &before_style.content else { panic!("restored frame") };
        assert_eq!(restored.transform, Affine::translate(9.0, 18.0));
        assert!(restored.smart_filters.is_empty());
        let framed = embedded(&before_style.content);
        assert_eq!(framed.size, Size::new(6, 4));
        assert_eq!(framed.icc_profile, doc.icc_profile);
        let inner = &framed.layers[0];
        let LayerContent::Smart(input) = &inner.content else { panic!("framed input") };
        assert_eq!(input.transform, Affine::translate(1.0, 2.0));
        assert_eq!(input.smart_filters.len(), 1);
        assert_eq!(input.smart_filters[0].command, "filter.distort.twirl");
        assert!(inner.effects.items.is_empty());
        assert!(inner.mask.is_none());
        let base = embedded(&inner.content);
        assert_eq!(base.layers[0].surface().unwrap().pixel(0, 0), [0.2, 0.4, 0.7, 1.0]);
        let before = builder.source.clone();
        let before_budget = budget;
        for (frame, perspective) in [
            (Rect::new(0, 0, 3, 2), Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])),
            (Rect::new(0, 0, MAX_SIDE as i32, MAX_SIDE as i32), None),
            (Rect::new(i32::MIN, 0, 0, 1), None),
        ] {
            let mapping = NativeMapping {
                structure: Some(NativeStructure::Composite { branches: vec![NativeBranch { filter_frame: Some(frame), perspective, ..Default::default() }] }),
                ..Default::default()
            };
            assert!(
                builder
                    .push(mapping, NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE)
                    .is_err()
            );
            assert_eq!(builder.source, before);
            assert_eq!(budget, before_budget);
        }
        let checks = std::sync::atomic::AtomicU32::new(0);
        let cancel = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 2;
        let interrupted = NativeMapping {
            structure: Some(NativeStructure::Composite { branches: vec![NativeBranch { filter_frame: Some(Rect::new(0, 0, 3, 2)), ..Default::default() }] }),
            ..Default::default()
        };
        assert!(
            builder
                .push(
                    interrupted,
                    NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None },
                    &mut budget,
                    &Interrupt::cancel_only(&cancel)
                )
                .is_err()
        );
        assert_eq!(checks.load(std::sync::atomic::Ordering::Relaxed), 3);
        assert_eq!(builder.source, before);
        assert_eq!(budget, before_budget);
    }
    #[test]
    fn owner_embedding_rebases_content_and_preserves_outer_native_state() {
        let doc = document();
        let original = owner(&doc);
        let before = original.clone();
        let mut budget = 1 << 30;
        let builder = NativeOwner::new(&doc, &original, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(original, before);
        let content = builder.content();
        let LayerContent::Smart(smart) = &content else { panic!() };
        assert_eq!(smart.transform, Affine::translate(-5.0, 7.0));
        assert!(smart.cache.is_none());
        let base = embedded(&content);
        assert_eq!(base.size, Size::new(3, 2));
        assert_eq!(base.resolution_dpi, 144.0);
        assert_eq!(base.icc_profile, doc.icc_profile);
        let input = &base.layers[0];
        assert_eq!(input.opacity, 1.0);
        assert_eq!(input.blend, BlendMode::Normal);
        assert!(input.mask.is_none());
        assert_eq!(input.surface().unwrap().pixel(0, 0), [0.2, 0.4, 0.7, 1.0]);
        assert_eq!(input.surface().unwrap().pixel(-5, 7), [0.0; 4]);
    }
    #[test]
    fn compound_opacity_mixes_complete_chain_once_and_survives_native_reopen() {
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        builder
            .push(
                mapping(&["image.adjustments.invert", "image.adjustments.invert"]),
                NativeEnvelope { opacity: 0.25, blend: BlendMode::Normal, visible: true, mask: None },
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap();
        let stage = embedded(&builder.content());
        let LayerContent::Smart(smart) = &stage.layers[0].content else { panic!() };
        assert_eq!(smart.smart_filters.len(), 2);
        assert!(smart.smart_filters.iter().all(|filter| filter.opacity == 1.0 && filter.visible));
        let mask = smart.filter_mask.as_ref().unwrap();
        assert_eq!(mask.value(0, 0), 0.25);
        assert_eq!(mask.value(-100, 100), 0.25);
        // Two native inversions are identity. Partial opacity on the last
        // inversion alone would change the source; one mask over both does not.
        let base = embedded(&stage.layers[0].content);
        let pixels = base.layers[0].surface().unwrap();
        let mut intermediate = pixels.clone();
        intermediate.write_pixel(0, 0, &[0.8, 0.6, 0.3, 1.0]);
        let mixed = photocraft_compose::filter_mask::mix(pixels, pixels, mask);
        assert_eq!(mixed.pixel(0, 0), pixels.pixel(0, 0));
        let wrong = photocraft_compose::filter_mask::mix(&intermediate, pixels, mask);
        assert_ne!(wrong.pixel(0, 0), pixels.pixel(0, 0));
        let saved = photocraft_format::save_to_bytes(&stage, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), stage);
    }
    #[test]
    fn source_mask_density_default_and_disabled_state_are_native_data() {
        let mut source = LayerMask::reveal_all();
        source.density = 0.5;
        source.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.4]);
        source.surface.write_pixel(0, 0, &[0.0]);
        source.surface.write_pixel(1, 0, &[1.0]);
        let mask = scale_mask(Some(source), 0.25, super::super::MAX_PIXEL_BYTES, &Interrupt::NONE).unwrap();
        assert!((mask.value(0, 0) - 0.125).abs() < 1e-6);
        assert_eq!(mask.value(1, 0), 0.25);
        assert!((mask.value(-100, -100) - 0.175).abs() < 1e-6);
        let mut disabled = LayerMask::hide_all();
        disabled.enabled = false;
        assert_eq!(scale_mask(Some(disabled), 0.25, super::super::MAX_PIXEL_BYTES, &Interrupt::NONE).unwrap().value(0, 0), 0.25);
    }
    #[test]
    fn disabled_stages_styles_and_separate_mask_are_preserved() {
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let mut mask = LayerMask::hide_all();
        mask.surface.write_pixel(1, 0, &[1.0]);
        builder
            .push(
                mapping(&["image.adjustments.invert"]),
                NativeEnvelope { opacity: 0.6, blend: BlendMode::Multiply, visible: false, mask: Some(mask.clone()) },
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap();
        let stage = embedded(&builder.content());
        let LayerContent::Smart(smart) = &stage.layers[0].content else { panic!() };
        assert!(!smart.smart_filters[0].visible);
        assert_eq!(smart.smart_filters[0].opacity, 0.6);
        assert_eq!(smart.smart_filters[0].blend, BlendMode::Multiply);
        assert_eq!(smart.filter_mask, Some(mask));
        let style = Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.8), color: Color::rgb(1.0, 0.0, 0.0) };
        builder
            .push(
                NativeMapping { layer_effects: vec![style], ..Default::default() },
                NativeEnvelope { opacity: 0.5, blend: BlendMode::Normal, visible: false, mask: None },
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap();
        let stage = embedded(&builder.content());
        let Effect::ColorOverlay { common, .. } = &stage.layers[0].effects.items[0] else { panic!() };
        assert!(!common.enabled);
        assert_eq!(common.opacity, 0.4);
    }
    #[test]
    fn failures_preserve_builder_and_budget_and_do_not_publish_partial_stages() {
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let before = builder.source.clone();
        let saved_budget = budget;
        assert!(
            builder
                .push(
                    mapping(&["image.adjustments.invert", "image.adjustments.invert"]),
                    NativeEnvelope { opacity: 0.5, blend: BlendMode::Multiply, visible: true, mask: None },
                    &mut budget,
                    &Interrupt::NONE
                )
                .is_err()
        );
        assert_eq!(builder.source, before);
        assert_eq!(budget, saved_budget);
        assert_eq!(builder.stages, 0);
        let mut tiny = 1;
        assert!(
            builder
                .push(
                    mapping(&["image.adjustments.invert"]),
                    NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None },
                    &mut tiny,
                    &Interrupt::NONE
                )
                .is_err()
        );
        assert_eq!(tiny, 1);
        assert_eq!(builder.source, before);
        let cancelled = Interrupt::cancel_only(&|| true);
        assert!(matches!(
            builder.push(
                mapping(&["image.adjustments.invert"]),
                NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None },
                &mut budget,
                &cancelled
            ),
            Err(crate::IoError::Cancelled)
        ));
        assert_eq!(builder.source, before);
        assert_eq!(budget, saved_budget);
        assert!(NativeOwner::new(&doc, &source, Rect::new(i32::MIN, 0, i32::MIN + 1, 1), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget, saved_budget);
    }
    #[test]
    fn existing_native_depth_is_counted_before_adding_more_source_stages() {
        let doc = document();
        let mut layer = owner(&doc);
        let mut budget = 1 << 30;
        for _ in 0..3 {
            let mut builder = NativeOwner::new(&doc, &layer, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
            builder
                .push(
                    mapping(&["image.adjustments.invert"]),
                    NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None },
                    &mut budget,
                    &Interrupt::NONE,
                )
                .unwrap();
            layer.content = builder.content();
        }
        let builder = NativeOwner::new(&doc, &layer, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(builder.existing_depth, 6);
        let mut builder = builder;
        builder.stages = 58;
        builder.added_depth = 58;
        let before = builder.source.clone();
        let before_budget = budget;
        assert!(
            builder
                .push(
                    mapping(&["image.adjustments.invert"]),
                    NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None },
                    &mut budget,
                    &Interrupt::NONE
                )
                .is_err()
        );
        assert_eq!(builder.source, before);
        assert_eq!(budget, before_budget);
    }
    #[test]
    fn exact_native_quarter_turns_preserve_mask_defaults_coordinates_and_inverse_placement() {
        let mut mask = LayerMask::reveal_all();
        mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.4]);
        mask.surface.write_pixel(2, 1, &[0.9]);
        for (turns, point) in [(1, (1, 2)), (2, (1, 1)), (3, (1, 1))] {
            let rotated = rotate_mask(mask.clone(), Size::new(4, 3), turns, &Interrupt::NONE).unwrap();
            assert_eq!(rotated.surface.default_pixel(), vec![0.4]);
            assert_eq!(rotated.value(point.0, point.1), 0.9);
            let (placement, size) = orientation(Size::new(4, 3), turns);
            assert_eq!(placement.inverse().unwrap().mul(&placement), Affine::IDENTITY);
            assert_eq!(size, if turns == 2 { Size::new(4, 3) } else { Size::new(3, 4) });
        }
    }
    #[test]
    fn oriented_compound_stage_places_mask_and_opacity_inside_rotated_native_contents() {
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let mut source_mask = LayerMask::hide_all();
        source_mask.surface.write_pixel(1, 0, &[1.0]);
        let mut converted = mapping(&["image.adjustments.invert", "image.adjustments.invert"]);
        converted.quarter_turns = 1;
        builder
            .push(converted, NativeEnvelope { opacity: 0.25, blend: BlendMode::Normal, visible: true, mask: Some(source_mask) }, &mut budget, &Interrupt::NONE)
            .unwrap();
        assert_eq!(builder.added_depth, 3);
        let restored = embedded(&builder.content());
        assert_eq!(restored.size, Size::new(3, 2));
        let LayerContent::Smart(inverse) = &restored.layers[0].content else { panic!() };
        let filtered = embedded(&restored.layers[0].content);
        assert_eq!(filtered.size, Size::new(2, 3));
        let LayerContent::Smart(stage) = &filtered.layers[0].content else { panic!() };
        assert!(stage.smart_filters.iter().all(|filter| filter.opacity == 1.0));
        assert_eq!(stage.filter_mask.as_ref().unwrap().value(1, 1), 0.25);
        assert_eq!(stage.filter_mask.as_ref().unwrap().value(0, 0), 0.0);
        let oriented = embedded(&filtered.layers[0].content);
        let LayerContent::Smart(forward) = &oriented.layers[0].content else { panic!() };
        assert_eq!(inverse.transform.mul(&forward.transform), Affine::IDENTITY);
        let base = embedded(&oriented.layers[0].content);
        assert_eq!(base.layers[0].surface().unwrap().pixel(0, 0), [0.2, 0.4, 0.7, 1.0]);
    }
    #[test]
    fn fixed_native_replacement_preserves_editable_input_and_native_resource() {
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let mut surface = Surface::new(doc.pixel_format());
        surface.write_pixel(0, 0, &[0.9, 0.1, 0.2, 1.0]);
        let replacement = Layer::new("Editable replacement", LayerContent::Raster(surface.clone()));
        let pattern = photocraft_doc::Pattern::new("Original native pattern", surface, 1, 1);
        let mapping =
            NativeMapping { structure: Some(NativeStructure::Replace { layer: Box::new(replacement), patterns: vec![pattern.clone()] }), ..Default::default() };
        builder.push(mapping, NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
        let stage = embedded(&builder.content());
        assert_eq!(stage.layers.len(), 2);
        assert!(!stage.layers[0].visible);
        assert!(stage.layers[1].visible);
        assert_eq!(stage.layers[1].surface().unwrap().pixel(0, 0), [0.9, 0.1, 0.2, 1.0]);
        assert_eq!(stage.patterns.last(), Some(&pattern));
        let original = embedded(&stage.layers[0].content);
        assert_eq!(original.layers[0].surface().unwrap().pixel(0, 0), [0.2, 0.4, 0.7, 1.0]);
        let bytes = photocraft_format::save_to_bytes(&stage, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), stage);
    }

    #[test]
    fn composite_native_pattern_is_local_deduplicated_and_survives_reopen() {
        use super::super::effects::NativeBranch;
        let doc = document();
        let original = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &original, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let mut pixels = Surface::new(doc.pixel_format());
        pixels.write_pixel(0, 0, &[0.3, 0.7, 0.2, 1.0]);
        let pattern = photocraft_doc::Pattern::new("Native composite paint", pixels, 1, 1);
        let fill = photocraft_doc::Fill::Pattern { name: pattern.name.clone(), id: pattern.id.clone(), scale: 1.0, angle: 0.0, link: true, phase: (0.0, 0.0) };
        let mapping = NativeMapping {
            patterns: vec![pattern.clone(), pattern.clone()],
            structure: Some(NativeStructure::Composite { branches: vec![NativeBranch { fill: Some(fill), ..Default::default() }] }),
            ..Default::default()
        };
        builder.push(mapping, NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
        let stage = embedded(&builder.content());
        assert_eq!(stage.patterns.iter().filter(|value| value.id == pattern.id).count(), 1);
        let rendered = photocraft_compose::flatten(&stage);
        assert!(rendered.px.iter().all(|pixel| pixel.iter().zip([0.3, 0.7, 0.2, 1.0]).all(|(a, b)| (a - b).abs() <= 1.0 / 255.0)));
        let saved = photocraft_format::save_to_bytes(&stage, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(photocraft_compose::flatten(&reopened).px, rendered.px);
        let mut no_budget = 0;
        let before = builder.source.clone();
        let mapping = NativeMapping {
            patterns: vec![pattern],
            structure: Some(NativeStructure::Composite { branches: vec![NativeBranch::default()] }),
            ..Default::default()
        };
        assert!(
            builder
                .push(mapping, NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut no_budget, &Interrupt::NONE)
                .is_err()
        );
        assert_eq!(builder.source, before);
        assert_eq!(no_budget, 0);
    }

    #[test]
    fn native_composite_branches_are_explicit_bottom_to_top_and_disabled_stages_retain_input() {
        use super::super::effects::NativeBranch;
        for visible in [true, false] {
            let doc = document();
            let source = owner(&doc);
            let mut budget = 1 << 30;
            let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
            let mapping = NativeMapping {
                structure: Some(NativeStructure::Composite {
                    branches: vec![
                        NativeBranch { ..Default::default() },
                        NativeBranch {
                            filters: vec![native_filter("image.adjustments.invert", json!({}))],
                            blend: BlendMode::Screen,
                            opacity: 0.4,
                            ..Default::default()
                        },
                    ],
                }),
                ..Default::default()
            };
            builder.push(mapping, NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
            let stage = embedded(&builder.content());
            assert_eq!(stage.layers.len(), 3);
            assert_eq!(stage.layers[0].visible, !visible);
            assert_eq!(stage.layers[1].visible, visible);
            assert_eq!(stage.layers[2].visible, visible);
            assert_eq!(stage.layers[2].blend, BlendMode::Screen);
            assert_eq!(stage.layers[2].opacity, 0.4);
            let LayerContent::Smart(smart) = &stage.layers[2].content else { panic!() };
            assert_eq!(smart.smart_filters[0].command, "image.adjustments.invert");
            assert_eq!(embedded(&stage.layers[1].content), embedded(&stage.layers[2].content));
        }
    }

    #[test]
    fn native_structural_envelopes_keep_coverage_opacity_blend_and_unchanged_input_separate() {
        use super::super::effects::NativeBranch;
        let doc = document();
        let source = owner(&doc);
        for (opacity, blend, mask) in
            [(0.5, BlendMode::Normal, None), (1.0, BlendMode::Multiply, None), (1.0, BlendMode::Normal, Some(LayerMask::reveal_all()))]
        {
            let mut budget = 1 << 30;
            let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
            let mapping =
                NativeMapping { structure: Some(NativeStructure::Composite { branches: vec![NativeBranch { ..Default::default() }] }), ..Default::default() };
            let notes = builder.push(mapping, NativeEnvelope { opacity, blend, visible: true, mask: mask.clone() }, &mut budget, &Interrupt::NONE).unwrap();
            assert!(notes.iter().any(|note| note.contains("semitransparent")));
            let wrapped = embedded(&builder.content());
            assert_eq!(wrapped.layers.len(), 2);
            assert!(wrapped.layers[0].visible);
            assert_eq!(wrapped.layers[1].opacity, opacity);
            assert_eq!(wrapped.layers[1].blend, blend);
            assert_eq!(wrapped.layers[1].mask, mask);
            let computed = embedded(&wrapped.layers[1].content);
            assert!(!computed.layers[0].visible);
            assert!(computed.layers[1].visible);
            assert_eq!(builder.added_depth, 2);
        }
    }

    #[test]
    fn helper_component_blend_survives_normal_source_envelope() {
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let mut mapped = mapping(&["image.adjustments.invert"]);
        mapped.filters[0].blend = BlendMode::Luminosity;
        builder.push(mapped, NativeEnvelope { opacity: 0.5, blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
        let stage = embedded(&builder.content());
        let LayerContent::Smart(smart) = &stage.layers[0].content else { panic!() };
        assert_eq!(smart.smart_filters[0].blend, BlendMode::Luminosity);
        assert_eq!(smart.smart_filters[0].opacity, 0.5);
        let mut mapped = mapping(&["image.adjustments.invert"]);
        mapped.filters[0].blend = BlendMode::Luminosity;
        let before = builder.source.clone();
        let saved_budget = budget;
        assert!(
            builder
                .push(mapped, NativeEnvelope { opacity: 0.5, blend: BlendMode::Multiply, visible: true, mask: None }, &mut budget, &Interrupt::NONE)
                .is_err()
        );
        assert_eq!(builder.source, before);
        assert_eq!(budget, saved_budget);
    }
    #[test]
    fn transformed_branches_preserve_native_masks_styles_and_editable_source() {
        use super::super::effects::NativeBranch;
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let mut mask = LayerMask::hide_all();
        mask.surface.write_pixel(1, 1, &[1.0]);
        let vector_mask = photocraft_doc::VectorMask::new(photocraft_doc::Path::default());
        let style = Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.8), color: Color::rgb(0.1, 0.2, 0.3) };
        let converted = NativeMapping {
            structure: Some(NativeStructure::Composite {
                branches: vec![NativeBranch {
                    transform: Affine::translate(2.0, -1.0),
                    mask: Some(mask.clone()),
                    vector_mask: Some(vector_mask.clone()),
                    layer_effects: vec![style.clone()],
                    fill_opacity: 0.25,
                    advanced: photocraft_doc::AdvancedBlending { knockout: photocraft_doc::Knockout::Shallow, ..Default::default() },
                    ..Default::default()
                }],
            }),
            ..Default::default()
        };
        builder.push(converted, NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
        let document = embedded(&builder.content());
        let layer = &document.layers[1];
        assert_eq!(layer.mask, Some(mask));
        assert_eq!(layer.vector_mask, Some(vector_mask));
        assert_eq!(layer.effects.items, vec![style]);
        assert_eq!(layer.fill_opacity, 0.25);
        assert_eq!(layer.advanced.knockout, photocraft_doc::Knockout::Shallow);
        let LayerContent::Smart(smart) = &layer.content else { panic!() };
        assert_eq!(smart.transform, Affine::translate(2.0, -1.0));
        assert!(matches!(smart.source, SmartSource::Embedded { .. }));
        assert!(embedded(&layer.content).layers[0].surface().is_some());
    }

    #[test]
    fn post_branch_styles_embed_native_fill_stage_and_keep_outer_envelope_atomically() {
        use super::super::effects::NativeBranch;
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let before = builder.source.clone();
        let depth = builder.added_depth;
        let initial = Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.8), color: Color::rgb(1., 0., 0.) };
        let post = Effect::ColorOverlay { common: FxCommon::new(BlendMode::Screen, 0.3), color: Color::rgb(0., 1., 0.) };
        let converted = || NativeMapping {
            structure: Some(NativeStructure::Composite {
                branches: vec![NativeBranch {
                    transform: Affine::translate(1., -1.),
                    layer_effects: vec![initial.clone()],
                    post_layer_effects: vec![post.clone()],
                    fill_opacity: 0.,
                    opacity: 0.4,
                    blend: BlendMode::Multiply,
                    clipped: true,
                    advanced: photocraft_doc::AdvancedBlending { knockout: photocraft_doc::Knockout::Shallow, ..Default::default() },
                    mask: Some(LayerMask::hide_all()),
                    ..Default::default()
                }],
            }),
            ..Default::default()
        };
        let mut small = 8;
        assert!(
            builder
                .push(converted(), NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None }, &mut small, &Interrupt::NONE)
                .is_err()
        );
        assert_eq!(small, 8);
        assert_eq!(builder.source, before);
        assert_eq!(builder.added_depth, depth);
        builder.push(converted(), NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(builder.added_depth, depth + 2);
        let outer = embedded(&builder.content());
        let layer = &outer.layers[1];
        assert_eq!(layer.opacity, 0.4);
        assert_eq!(layer.blend, BlendMode::Multiply);
        assert!(layer.clipped);
        assert!(layer.mask.is_some());
        assert_eq!(layer.advanced.knockout, photocraft_doc::Knockout::Shallow);
        assert_eq!(layer.fill_opacity, 1.);
        assert_eq!(layer.effects.items, vec![post.clone()]);
        assert!(layer.effects.enabled);
        let inner = embedded(&layer.content);
        let layer = &inner.layers[0];
        assert_eq!(layer.opacity, 1.);
        assert_eq!(layer.blend, BlendMode::Normal);
        assert!(!layer.clipped);
        assert!(layer.mask.is_none());
        assert_eq!(layer.advanced.knockout, photocraft_doc::Knockout::None);
        assert_eq!(layer.fill_opacity, 0.);
        assert_eq!(layer.effects.items, vec![initial.clone()]);
        assert!(layer.effects.enabled);
        let LayerContent::Smart(sm) = &layer.content else { panic!() };
        assert_eq!(sm.transform, Affine::translate(1., -1.));
        let mut exhausted = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        exhausted.existing_depth = 62;
        let before = exhausted.source.clone();
        let allowance = budget;
        assert!(
            exhausted
                .push(converted(), NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE)
                .is_err()
        );
        assert_eq!(budget, allowance);
        assert_eq!(exhausted.source, before);
    }
    #[test]
    fn grouped_replacements_stay_native_and_charge_uncompressed_pixels_atomically() {
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let mut raster = Layer::raster("Original synthetic replacement", doc.pixel_format());
        raster.surface_mut().unwrap().write_pixel(0, 0, &[0.9, 0.2, 0.3, 1.0]);
        let group = Layer::group(
            "Native grouped generator",
            vec![Layer::new("Background", LayerContent::Fill(photocraft_doc::Fill::Solid(Color::rgb(0.1, 0.2, 0.3)))), raster],
        );
        let converted =
            || NativeMapping { structure: Some(NativeStructure::Replace { layer: Box::new(group.clone()), patterns: vec![] }), ..Default::default() };
        let before = builder.source.clone();
        let mut small = 100;
        let error = builder
            .push(converted(), NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut small, &Interrupt::NONE)
            .unwrap_err();
        assert!(error.to_string().contains("allocation"));
        assert_eq!(small, 100);
        assert_eq!(builder.source, before);
        builder.push(converted(), NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
        let document = embedded(&builder.content());
        assert_eq!(document.layers[1], group);
    }
    #[test]
    fn structural_post_filters_precede_one_complete_operation_envelope() {
        let doc = document();
        let source = owner(&doc);
        for visible in [true, false] {
            let mut budget = 1 << 30;
            let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
            let mut mask = LayerMask::hide_all();
            mask.surface.write_pixel(1, 1, &[1.0]);
            let mut mapped = mapping(&["image.adjustments.invert"]);
            mapped.structure = Some(NativeStructure::Replace {
                layer: Box::new(Layer::new("Editable solid", LayerContent::Fill(photocraft_doc::Fill::Solid(Color::rgb(0.8, 0.1, 0.2))))),
                patterns: vec![],
            });
            builder
                .push(mapped, NativeEnvelope { opacity: 0.4, blend: BlendMode::Normal, visible, mask: Some(mask.clone()) }, &mut budget, &Interrupt::NONE)
                .unwrap();
            let envelope = embedded(&builder.content());
            assert!(envelope.layers[0].visible);
            assert_eq!(envelope.layers[1].visible, visible);
            assert_eq!(envelope.layers[1].opacity, 0.4);
            assert_eq!(envelope.layers[1].mask, Some(mask));
            let post = embedded(&envelope.layers[1].content);
            let LayerContent::Smart(smart) = &post.layers[0].content else { panic!() };
            assert_eq!(smart.smart_filters.len(), 1);
            assert_eq!(smart.smart_filters[0].opacity, 1.0);
            assert!(smart.filter_mask.is_none());
            let generated = embedded(&post.layers[0].content);
            assert!(!generated.layers[0].visible);
            assert!(generated.layers[1].visible);
            assert!(matches!(generated.layers[1].content, LayerContent::Fill(_)));
            assert_eq!(builder.added_depth, 3);
        }
    }

    #[test]
    fn native_fill_branch_is_an_editable_underlay_with_no_invented_source_copy() {
        use super::super::effects::NativeBranch;
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let fill = photocraft_doc::Fill::Solid(Color::rgb(0.3, 0.6, 0.9));
        let mapped = NativeMapping {
            structure: Some(NativeStructure::Composite {
                branches: vec![NativeBranch { fill: Some(fill.clone()), ..Default::default() }, NativeBranch::default()],
            }),
            ..Default::default()
        };
        builder.push(mapped, NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
        let document = embedded(&builder.content());
        assert_eq!(document.layers.len(), 3);
        assert!(!document.layers[0].visible);
        assert_eq!(document.layers[1].content, LayerContent::Fill(fill));
        assert!(matches!(document.layers[2].content, LayerContent::Smart(_)));
    }
    #[test]
    fn arbitrary_native_rotation_uses_bounded_canvas_inverse_and_standard_mask_resampling() {
        let (placement, size) = arbitrary_orientation(Size::new(4, 2), 30.0).unwrap();
        assert_eq!(size, Size::new(5, 4));
        let inverse = placement.inverse().unwrap().mul(&placement);
        assert!(inverse.m.iter().zip(Affine::IDENTITY.m).all(|(a, b)| (a - b).abs() < 1e-12));
        assert!(arbitrary_orientation(Size::new(MAX_SIDE, MAX_SIDE), 45.0).is_err());
        let mut mask = LayerMask::reveal_all();
        mask.density = 0.8;
        mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.4]);
        mask.surface.write_pixel(2, 1, &[0.9]);
        // Independent native Free Transform invocation with the explicitly known
        // 30-degree matrix and expanded-canvas translation, not mapper output.
        let cosine = 3.0f64.sqrt() / 2.0;
        let matrix = photocraft_algo::transform::Homography([cosine, -0.5, 1.0, 0.5, cosine, 0.0, 0.0, 0.0, 1.0]);
        let expected = photocraft_algo::transform::warp_surface(&mask.surface, Rect::new(0, 0, 4, 2), &matrix, photocraft_algo::transform::Interp::Bilinear)
            .unwrap()
            .try_convert(mask.surface.format())
            .unwrap();
        let rotated = warp_mask(mask.clone(), Size::new(4, 2), placement, &Interrupt::NONE).unwrap();
        assert_eq!(rotated.surface, expected);
        assert_eq!(rotated.density, mask.density);
        assert_eq!(rotated.enabled, mask.enabled);
        let cancelled = Interrupt::cancel_only(&|| true);
        assert!(matches!(warp_mask(mask, Size::new(4, 2), placement, &cancelled), Err(crate::IoError::Cancelled)));
    }

    #[test]
    fn arbitrary_orientation_publishes_only_complete_native_placements_and_bounded_work() {
        let doc = document();
        let source = owner(&doc);
        let mut budget = 1 << 30;
        let mut builder = NativeOwner::new(&doc, &source, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let mut converted = mapping(&["image.adjustments.invert"]);
        converted.rotation_degrees = 30.0;
        let before = builder.source.clone();
        let mut tiny = 1;
        assert!(
            builder.push(converted, NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut tiny, &Interrupt::NONE).is_err()
        );
        assert_eq!(builder.source, before);
        assert_eq!(tiny, 1);
        let mut converted = mapping(&["image.adjustments.invert"]);
        converted.rotation_degrees = 30.0;
        let notes = builder
            .push(converted, NativeEnvelope { opacity: 1.0, blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE)
            .unwrap();
        assert!(notes.iter().any(|note| note.contains("interpolation")));
        let restored = embedded(&builder.content());
        let LayerContent::Smart(inverse) = &restored.layers[0].content else { panic!() };
        let filtered = embedded(&restored.layers[0].content);
        let oriented = embedded(&filtered.layers[0].content);
        let LayerContent::Smart(forward) = &oriented.layers[0].content else { panic!() };
        let identity = inverse.transform.mul(&forward.transform);
        assert!(identity.m.iter().zip(Affine::IDENTITY.m).all(|(a, b)| (a - b).abs() < 1e-12));
        assert_eq!(builder.added_depth, 3);
        assert_eq!(oriented.size, filtered.size);
        assert_eq!(restored.size, Size::new(3, 2));
    }
    #[test]
    fn compound_u8_mask_expansion_preflights_remaining_allowance_and_is_atomic() {
        let doc = document();
        let original = owner(&doc);
        let mut large = super::super::MAX_PIXEL_BYTES;
        let mut builder = NativeOwner::new(&doc, &original, Rect::new(-5, 7, -2, 9), &mut large, &Interrupt::NONE).unwrap();
        let source = builder.source.clone();
        let mut mask = LayerMask::hide_all();
        mask.surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        for (index, value) in [0.25, 0.5, 0.75].into_iter().enumerate() {
            mask.surface.write_pixel(index as i32 * photocraft_geom::TILE_SIZE, 0, &[value]);
        }
        assert_eq!(mask.surface.tile_count(), 3);
        let mask_before = mask.clone();
        let mut budget = 32_768;
        let before = budget;
        let result = builder.push(
            mapping(&["image.adjustments.invert", "image.adjustments.invert"]),
            NativeEnvelope { opacity: 0.25, blend: BlendMode::Normal, visible: true, mask: Some(mask.clone()) },
            &mut budget,
            &Interrupt::NONE,
        );
        assert!(result.is_err(), "three U8 tiles must not allocate three F32 tiles under a 32 KiB remaining allowance");
        assert_eq!(budget, before);
        assert_eq!(builder.source, source);
        assert_eq!(builder.stages, 0);
        assert_eq!(builder.added_depth, 0);
        assert_eq!(mask, mask_before);
    }

    #[test]
    fn scaled_mask_accounts_f32_tiles_scratch_defaults_density_and_invalid_state() {
        let mut mask = LayerMask::hide_all();
        mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), &[0.4]);
        mask.density = 0.5;
        for (index, value) in [0.25, 0.5, 0.75].into_iter().enumerate() {
            mask.surface.write_pixel(index as i32 * photocraft_geom::TILE_SIZE, 0, &[value]);
        }
        let side = photocraft_geom::TILE_SIZE as usize;
        let exact_peak = 4 * side * side * std::mem::size_of::<f32>() + std::mem::size_of::<f32>();
        assert!(scale_mask(Some(mask.clone()), 0.25, exact_peak - 1, &Interrupt::NONE).is_err());
        let scaled = scale_mask(Some(mask.clone()), 0.25, exact_peak, &Interrupt::NONE).unwrap();
        assert_eq!(scaled.surface.format().sample, SampleType::F32);
        for (x, y) in [(0, 0), (256, 0), (512, 0), (-1, -1)] {
            assert!((scaled.value(x, y) - 0.25 * mask.value(x, y)).abs() < 1e-6);
        }
        assert!(scale_mask(None, 0.25, 0, &Interrupt::NONE).is_err());
        assert!(scale_mask(None, f32::NAN, super::super::MAX_PIXEL_BYTES, &Interrupt::NONE).is_err());
        let mut invalid = LayerMask::reveal_all();
        invalid.surface = Surface::new(PixelFormat::RGBA8);
        assert!(scale_mask(Some(invalid), 0.25, super::super::MAX_PIXEL_BYTES, &Interrupt::NONE).is_err());
    }

    #[test]
    fn native_bundle_admission_counts_binary_aliases_before_zip_and_preserves_budget() {
        let mut doc = document();
        let shared = Arc::new(vec![0; 1 << 20]);
        for _ in 0..3 {
            doc.layers.push(Layer::new(
                "Shared native payload",
                LayerContent::Smart(SmartObject::new(
                    SmartSource::Embedded { file_name: "opaque-native-input.pcraft".into(), bytes: shared.clone() },
                    Affine::IDENTITY,
                    None,
                )),
            ));
        }
        // Actual private bundles can share one embedded allocation across many branches.
        encoded_preflight(&doc, 2 << 20, &Interrupt::NONE).unwrap();
        let mut tiny = 32_768;
        assert!(encoded(&doc, &mut tiny, &Interrupt::NONE).is_err());
        assert_eq!(tiny, 32_768);
        let mut enough = 2 << 20;
        let bytes = encoded(&doc, &mut enough, &Interrupt::NONE).unwrap();
        assert_eq!(enough, (2 << 20) - bytes.len(), "admission does not charge estimated ZIP bytes a second time");
        let before = enough;
        let cancel = || true;
        assert!(encoded(&doc, &mut enough, &Interrupt::cancel_only(&cancel)).is_err());
        assert_eq!(enough, before);
    }
    #[test]
    fn native_bundle_admission_covers_raster_mask_pattern_text_and_filter_metadata() {
        let mut doc = document();
        let mut pixels = Surface::new(PixelFormat::RGBA32F);
        pixels.write_pixel(0, 0, &[0.2, 0.4, 0.6, 0.5]);
        doc.layers.push(Layer::new("Native raster", LayerContent::Raster(pixels.clone())));
        assert!(encoded_preflight(&doc, 32_768, &Interrupt::NONE).is_err());
        doc.layers.clear();
        let mut layer = Layer::new("Native coverage", LayerContent::Fill(photocraft_doc::Fill::Solid(photocraft_color::Color::BLACK)));
        let mut mask = LayerMask::hide_all();
        mask.surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        mask.surface.write_pixel(0, 0, &[0.5]);
        layer.mask = Some(mask);
        doc.layers.push(layer);
        assert!(encoded_preflight(&doc, 32_768, &Interrupt::NONE).is_err());
        doc.layers.clear();
        doc.patterns.push(photocraft_doc::Pattern::new("Native pattern", pixels, 1, 1));
        assert!(encoded_preflight(&doc, 32_768, &Interrupt::NONE).is_err());
        doc.patterns.clear();
        let text = photocraft_doc::TextLayer { text: "\u{1}".repeat(65_536), ..Default::default() };
        doc.layers.push(Layer::new("Native text", LayerContent::Text(text)));
        assert!(encoded_preflight(&doc, 128 << 10, &Interrupt::NONE).is_err());
        doc.layers.clear();
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: Arc::new(Vec::new()) }, Affine::IDENTITY, None);
        smart.smart_filters.push(native_filter("image.adjustments.colorLookup", json!({"data":"x".repeat(128<<10)})));
        doc.layers.push(Layer::new("Native parameters", LayerContent::Smart(smart)));
        assert!(encoded_preflight(&doc, 64 << 10, &Interrupt::NONE).is_err());
    }

    fn fork_envelope() -> NativeEnvelope {
        NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None }
    }
    fn fork_native_filter(command: &str, params: serde_json::Value) -> NativeMapping {
        NativeMapping { filters: vec![native_filter(command, params)], ..Default::default() }
    }
    // Prepare only the ordinary native commands used by this independent builder
    // test. Production preparation remains the existing engine responsibility.
    fn prepare_fork_native(doc: &mut Document, depth: usize) {
        assert!(depth < 16);
        let format = doc.pixel_format();
        for layer in &mut doc.layers {
            if let LayerContent::Smart(smart) = &mut layer.content {
                let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("native embedded branch") };
                let mut inner = photocraft_format::load_from_bytes(bytes).unwrap();
                prepare_fork_native(&mut inner, depth + 1);
                let mut rendered = photocraft_compose::render(&inner, inner.bounds());
                for filter in &smart.smart_filters {
                    assert_eq!(filter.opacity, 1.);
                    assert_eq!(filter.blend, BlendMode::Normal);
                    assert!(filter.visible);
                    let adj = match filter.command.as_str() {
                        "image.adjustments.invert" => photocraft_doc::Adjustment::Invert,
                        "image.adjustments.exposure" => {
                            photocraft_doc::Adjustment::Exposure { exposure: filter.params["exposure"].as_f64().unwrap() as f32, offset: 0., gamma: 1. }
                        }
                        _ => panic!("unexpected native test command"),
                    };
                    photocraft_compose::adjust::apply_in(
                        &adj,
                        &mut rendered,
                        photocraft_compose::adjust::Transfer::for_document(doc.mode, doc.depth),
                        Some(doc.depth),
                        doc.mode,
                    );
                }
                let mut cached = Surface::new(format);
                let mut values = Vec::new();
                for pixel in rendered.px {
                    let mut row = [0.; 8];
                    let n = photocraft_raster::from_rgba_into(&format, pixel, &mut row);
                    values.extend_from_slice(&row[..n]);
                }
                cached.write_region(rendered.rect, &values);
                smart.cache = Some(cached);
            }
        }
    }
    #[test]
    fn fork_join_uses_current_noncommuting_native_stages_and_matches_native_blend() {
        let mut doc = Document::new("Native fork", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        let mut input = Layer::raster("Original", doc.pixel_format());
        input.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.1, 0.2, 0.3, 0.5]);
        let mut budget = 1 << 28;
        let mut root = NativeOwner::new(&doc, &input, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
        root.push(
            fork_native_filter("image.adjustments.exposure", json!({"exposure":1.,"offset":0.,"gamma":1.})),
            fork_envelope(),
            &mut budget,
            &Interrupt::NONE,
        )
        .unwrap();
        let below = root.fork();
        let mut above = root.fork();
        assert!(Arc::ptr_eq(&root.template, &below.template));
        assert!(Arc::ptr_eq(&root.template, &above.template));
        let source_before = below.source.clone();
        above.push(mapping(&["image.adjustments.invert"]), fork_envelope(), &mut budget, &Interrupt::NONE).unwrap();
        assert!(Arc::ptr_eq(&below.source, &source_before));
        assert!(!Arc::ptr_eq(&below.source, &above.source));
        root.join(&below, &above, BlendMode::Multiply, 0.35, &mut budget, &Interrupt::NONE).unwrap();
        let mut joined = embedded(&root.content());
        assert_eq!(joined.layers.len(), 2);
        assert_eq!(joined.layers[0].blend, BlendMode::Normal);
        assert_eq!(joined.layers[1].blend, BlendMode::Multiply);
        assert_eq!(joined.layers[1].opacity, 0.35);
        prepare_fork_native(&mut joined, 0);
        let actual = photocraft_compose::render(&joined, joined.bounds());
        let mut lower = photocraft_compose::Buffer { rect: doc.bounds(), px: vec![[0.1, 0.2, 0.3, 0.5]; 2] };
        let transfer = photocraft_compose::adjust::Transfer::for_document(doc.mode, doc.depth);
        photocraft_compose::adjust::apply_in(
            &photocraft_doc::Adjustment::Exposure { exposure: 1., offset: 0., gamma: 1. },
            &mut lower,
            transfer,
            Some(doc.depth),
            doc.mode,
        );
        let mut upper = lower.clone();
        photocraft_compose::adjust::apply_in(&photocraft_doc::Adjustment::Invert, &mut upper, transfer, Some(doc.depth), doc.mode);
        let mut reference = doc.clone();
        for (buffer, blend, opacity) in [(lower, BlendMode::Normal, 1.), (upper, BlendMode::Multiply, 0.35)] {
            let mut pixels = Surface::new(doc.pixel_format());
            pixels.write_region(buffer.rect, &buffer.px.iter().flatten().copied().collect::<Vec<_>>());
            let mut layer = Layer::new("Independent native branch", LayerContent::Raster(pixels));
            layer.blend = blend;
            layer.opacity = opacity;
            reference.layers.push(layer);
        }
        let expected = photocraft_compose::render(&reference, reference.bounds());
        assert_eq!(actual.px, expected.px);
        let saved = photocraft_format::save_to_bytes(&joined, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, expected.px);
    }
    #[test]
    fn fork_join_preserves_structural_resources_styles_and_gray_profile_contract() {
        let doc = document();
        let input = owner(&doc);
        let bounds = Rect::new(-5, 7, -2, 9);
        let mut budget = 1 << 28;
        let mut root = NativeOwner::new(&doc, &input, bounds, &mut budget, &Interrupt::NONE).unwrap();
        let below = root.fork();
        let mut above = root.fork();
        let pattern = photocraft_doc::Pattern::new("Editable native resource", input.surface().unwrap().clone(), 3, 2);
        let id = pattern.id.clone();
        above
            .push(
                NativeMapping {
                    patterns: vec![pattern],
                    structure: Some(NativeStructure::Composite {
                        branches: vec![super::super::effects::NativeBranch {
                            fill: Some(photocraft_doc::Fill::Pattern {
                                name: "Editable native resource".into(),
                                id: id.clone(),
                                scale: 1.,
                                angle: 0.,
                                link: true,
                                phase: (0., 0.),
                            }),
                            ..Default::default()
                        }],
                    }),
                    ..Default::default()
                },
                fork_envelope(),
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap();
        above
            .push(
                NativeMapping {
                    layer_effects: vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.8), color: Color::rgb(0.2, 0.4, 0.6) }],
                    ..Default::default()
                },
                fork_envelope(),
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap();
        let old_upper = above.source.clone();
        root.join(&below, &above, BlendMode::Normal, 1., &mut budget, &Interrupt::NONE).unwrap();
        let joint = embedded(&root.content());
        let upper = embedded(&joint.layers[1].content);
        assert_eq!(upper.layers[0].effects.items.len(), 1);
        let pattern_stage = embedded(&upper.layers[0].content);
        assert!(pattern_stage.patterns.iter().any(|p| p.id == id));
        assert!(Arc::ptr_eq(&old_upper, &above.source));
        // A working-RGB stage still returns to the original Gray/profile contract.
        let mut gray = Document::new("Gray", Size::new(2, 1), ColorMode::Grayscale, SampleType::F32);
        gray.icc_profile = Some(photocraft_cms::Builtin::GrayGamma22.profile().to_bytes());
        let mut pixels = Layer::raster("Gray pixels", gray.pixel_format());
        pixels.surface_mut().unwrap().fill_rect(gray.bounds(), &[0.2, 0.5]);
        let mut gray_root = NativeOwner::new(&gray, &pixels, gray.bounds(), &mut budget, &Interrupt::NONE).unwrap();
        let original = gray_root.fork();
        let mut rgb = gray_root.fork();
        let mut converted = mapping(&["image.adjustments.invert"]);
        converted.working_rgb = true;
        rgb.push(converted, fork_envelope(), &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(rgb.template.mode, ColorMode::Grayscale);
        assert_eq!(rgb.template.icc_profile, gray.icc_profile);
        gray_root.join(&original, &rgb, BlendMode::Normal, 1., &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(embedded(&gray_root.content()).mode, ColorMode::Grayscale);
    }
    #[test]
    fn fork_join_rejects_foreign_frame_profile_and_rolls_back_all_state() {
        let doc = document();
        let input = owner(&doc);
        let bounds = Rect::new(-5, 7, -2, 9);
        let mut budget = 1 << 28;
        let mut root = NativeOwner::new(&doc, &input, bounds, &mut budget, &Interrupt::NONE).unwrap();
        let below = root.fork();
        let other = NativeOwner::new(&doc, &input, bounds, &mut budget, &Interrupt::NONE).unwrap();
        let mut frame = root.fork();
        Arc::make_mut(&mut frame.template).size = Size::new(1, 1);
        let mut profile = root.fork();
        Arc::make_mut(&mut profile.template).icc_profile = None;
        let mut origin = root.fork();
        origin.origin = (0, 0);
        let before = root.source.clone();
        let state = (root.stages, root.added_depth, budget);
        for invalid in [&other, &frame, &profile, &origin] {
            let old_input = invalid.source.clone();
            assert!(root.join(&below, invalid, BlendMode::Normal, 1., &mut budget, &Interrupt::NONE).is_err());
            assert!(Arc::ptr_eq(&root.source, &before));
            assert_eq!((root.stages, root.added_depth, budget), state);
            assert!(Arc::ptr_eq(&invalid.source, &old_input));
        }
        assert!(root.join(&below, &below, BlendMode::PassThrough, 1., &mut budget, &Interrupt::NONE).is_err());
        for opacity in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(root.join(&below, &below, BlendMode::Normal, opacity, &mut budget, &Interrupt::NONE).is_err());
            assert!(Arc::ptr_eq(&root.source, &before));
            assert_eq!((root.stages, root.added_depth, budget), state);
        }
        let cancel = || true;
        assert!(matches!(root.join(&below, &below, BlendMode::Normal, 1., &mut budget, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
        assert!(Arc::ptr_eq(&root.source, &before));
        assert_eq!((root.stages, root.added_depth, budget), state);
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checks = AtomicUsize::new(0);
        let mid_cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 3;
        assert!(matches!(root.join(&below, &below, BlendMode::Normal, 1., &mut budget, &Interrupt::cancel_only(&mid_cancel)), Err(crate::IoError::Cancelled)));
        assert!(checks.load(Ordering::Relaxed) >= 4);
        assert!(Arc::ptr_eq(&root.source, &before));
        assert!(Arc::ptr_eq(&below.source, &before));
        assert_eq!((root.stages, root.added_depth, budget), state);
        let mut tiny = 0;
        assert!(root.join(&below, &below, BlendMode::Normal, 1., &mut tiny, &Interrupt::NONE).is_err());
        assert_eq!(tiny, 0);
        assert!(Arc::ptr_eq(&root.source, &before));
        let mut deep = root.fork();
        deep.added_depth = 63;
        assert!(root.join(&below, &deep, BlendMode::Normal, 1., &mut budget, &Interrupt::NONE).is_err());
        assert!(Arc::ptr_eq(&root.source, &before));
        assert_eq!((root.stages, root.added_depth, budget), state);
    }
    #[test]
    fn fork_join_fanout_charges_distinct_nested_bytes_and_max_input_depth() {
        let doc = document();
        let input = owner(&doc);
        let bounds = Rect::new(-5, 7, -2, 9);
        let mut budget = 1 << 28;
        let mut root = NativeOwner::new(&doc, &input, bounds, &mut budget, &Interrupt::NONE).unwrap();
        for _ in 0..4 {
            let mut below = root.fork();
            let mut above = root.fork();
            below.push(mapping(&["image.adjustments.invert"]), fork_envelope(), &mut budget, &Interrupt::NONE).unwrap();
            above
                .push(
                    fork_native_filter("image.adjustments.exposure", json!({"exposure":1.,"offset":0.,"gamma":1.})),
                    fork_envelope(),
                    &mut budget,
                    &Interrupt::NONE,
                )
                .unwrap();
            let before = budget;
            root.join(&below, &above, BlendMode::Screen, 1., &mut budget, &Interrupt::NONE).unwrap();
            assert_eq!(before - budget, root.source.len());
            assert_eq!(root.added_depth, below.added_depth.max(above.added_depth) + 1);
            let joint = embedded(&root.content());
            assert_eq!(nested_depth(&joint.layers, 0, 0, &mut (1 << 30), &Interrupt::NONE).unwrap(), root.existing_depth + root.added_depth);
            let mut low = below.source.len().max(above.source.len());
            let old = root.source.clone();
            let counts = (root.stages, root.added_depth);
            assert!(root.join(&below, &above, BlendMode::Screen, 1., &mut low, &Interrupt::NONE).is_err());
            assert_eq!(low, below.source.len().max(above.source.len()));
            assert!(Arc::ptr_eq(&root.source, &old));
            assert_eq!((root.stages, root.added_depth), counts);
        }
    }

    fn stage_apply(input: usize, mapping: NativeMapping) -> NativeStage {
        NativeStage::Apply { input, mapping: Box::new(mapping) }
    }
    fn stage_mapping(steps: Vec<NativeStage>, output: usize) -> NativeMapping {
        NativeMapping { structure: Some(NativeStructure::Stages { steps, output }), ..Default::default() }
    }
    fn stage_buffer_layer(buffer: &photocraft_compose::Buffer, format: PixelFormat) -> Layer {
        let mut surface = Surface::new(format);
        let mut samples = Vec::new();
        for pixel in &buffer.px {
            let mut data = [0.; 8];
            let n = photocraft_raster::from_rgba_into(&format, *pixel, &mut data);
            samples.extend_from_slice(&data[..n]);
        }
        surface.write_region(buffer.rect, &samples);
        Layer::new("Independent native stage", LayerContent::Raster(surface))
    }
    #[test]
    fn native_stages_apply_complete_mappings_and_outer_mask_blend_opacity_once() {
        let doc = Document::new("Native stages", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
        let mut input = Layer::raster("Original", doc.pixel_format());
        input.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.1, 0.2, 0.3, 0.5]);
        let mut budget = 1 << 28;
        let mut builder = NativeOwner::new(&doc, &input, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
        builder
            .push(
                fork_native_filter("image.adjustments.exposure", json!({"exposure":1.,"offset":0.,"gamma":1.})),
                fork_envelope(),
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap();
        let mut dim = fork_native_filter("image.adjustments.exposure", json!({"exposure":-1.,"offset":0.,"gamma":1.}));
        dim.notes.push("Internal native dimming policy".into());
        let mut mapping = stage_mapping(
            vec![
                stage_apply(0, mapping(&["image.adjustments.invert"])),
                stage_apply(0, dim),
                NativeStage::Join { below: 1, above: 2, blend: BlendMode::Screen, opacity: 0.6 },
            ],
            3,
        );
        mapping.filters = vec![native_filter("image.adjustments.invert", json!({}))];
        let mut mask = LayerMask::reveal_all();
        mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
        let notes = builder
            .push(mapping, NativeEnvelope { opacity: 0.4, blend: BlendMode::Multiply, visible: true, mask: Some(mask.clone()) }, &mut budget, &Interrupt::NONE)
            .unwrap();
        assert!(notes.iter().any(|note| note == "Internal native dimming policy"));
        let mut actual = embedded(&builder.content());
        assert_eq!(actual.layers[1].opacity, 0.4);
        assert_eq!(actual.layers[1].blend, BlendMode::Multiply);
        assert_eq!(actual.layers[1].mask, Some(mask.clone()));
        prepare_fork_native(&mut actual, 0);
        let actual_render = photocraft_compose::render(&actual, actual.bounds());
        let transfer = photocraft_compose::adjust::Transfer::for_document(doc.mode, doc.depth);
        let apply = |adj: &photocraft_doc::Adjustment, buffer: &mut photocraft_compose::Buffer| {
            photocraft_compose::adjust::apply_in(adj, buffer, transfer, Some(doc.depth), doc.mode)
        };
        let mut prior = photocraft_compose::Buffer { rect: doc.bounds(), px: vec![[0.1, 0.2, 0.3, 0.5]; 2] };
        apply(&photocraft_doc::Adjustment::Exposure { exposure: 1., offset: 0., gamma: 1. }, &mut prior);
        let mut lower = prior.clone();
        apply(&photocraft_doc::Adjustment::Invert, &mut lower);
        let mut upper = prior.clone();
        apply(&photocraft_doc::Adjustment::Exposure { exposure: -1., offset: 0., gamma: 1. }, &mut upper);
        let mut branches = doc.clone();
        branches.layers = vec![stage_buffer_layer(&lower, doc.pixel_format()), stage_buffer_layer(&upper, doc.pixel_format())];
        branches.layers[1].blend = BlendMode::Screen;
        branches.layers[1].opacity = 0.6;
        let mut combined = photocraft_compose::render(&branches, branches.bounds());
        apply(&photocraft_doc::Adjustment::Invert, &mut combined);
        let mut reference = doc.clone();
        reference.layers = vec![stage_buffer_layer(&prior, doc.pixel_format()), stage_buffer_layer(&combined, doc.pixel_format())];
        reference.layers[1].opacity = 0.4;
        reference.layers[1].blend = BlendMode::Multiply;
        reference.layers[1].mask = Some(mask);
        let expected = photocraft_compose::render(&reference, reference.bounds());
        for (a, b) in actual_render.px.iter().flatten().zip(expected.px.iter().flatten()) {
            assert!((a - b).abs() < 1e-6, "{a} != {b}");
        }
        let bytes = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, actual_render.px);
    }
    #[test]
    fn native_stages_keep_structures_resources_styles_and_selected_output_native() {
        let doc = document();
        let input = owner(&doc);
        let bounds = Rect::new(-5, 7, -2, 9);
        let mut budget = 1 << 28;
        let mut builder = NativeOwner::new(&doc, &input, bounds, &mut budget, &Interrupt::NONE).unwrap();
        let mut pixels = Surface::new(doc.pixel_format());
        pixels.write_pixel(0, 0, &[0.2, 0.4, 0.6, 1.]);
        let pattern = photocraft_doc::Pattern::new("Stage pattern", pixels, 1, 1);
        let id = pattern.id.clone();
        let structure = NativeMapping {
            patterns: vec![pattern],
            structure: Some(NativeStructure::Composite {
                branches: vec![super::super::effects::NativeBranch {
                    fill: Some(photocraft_doc::Fill::Pattern {
                        name: "Stage pattern".into(),
                        id: id.clone(),
                        scale: 1.,
                        angle: 0.,
                        link: true,
                        phase: (0., 0.),
                    }),
                    ..Default::default()
                }],
            }),
            ..Default::default()
        };
        let style = NativeMapping {
            layer_effects: vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.8), color: Color::rgb(0.1, 0.3, 0.5) }],
            ..Default::default()
        };
        builder.push(stage_mapping(vec![stage_apply(0, structure), stage_apply(1, style)], 2), fork_envelope(), &mut budget, &Interrupt::NONE).unwrap();
        let wrapper = embedded(&builder.content());
        assert!(!wrapper.layers[0].visible);
        assert!(wrapper.layers[1].visible);
        let styled = embedded(&wrapper.layers[1].content);
        assert_eq!(styled.layers[0].effects.items.len(), 1);
        let painted = embedded(&styled.layers[0].content);
        assert!(painted.patterns.iter().any(|pattern| pattern.id == id));
        assert!(matches!(&painted.layers[1].content,LayerContent::Fill(photocraft_doc::Fill::Pattern { id:found,.. }) if found==&id));
        // Selecting implicit input0 is permitted and still native/ordinary.
        let before = builder.source.clone();
        builder.push(stage_mapping(vec![], 0), fork_envelope(), &mut budget, &Interrupt::NONE).unwrap();
        let selected = embedded(&builder.content());
        let LayerContent::Smart(input) = &selected.layers[1].content else { panic!("native selected input") };
        let SmartSource::Embedded { bytes, .. } = &input.source else { panic!("native snapshot") };
        assert_eq!(bytes, &before);
    }
    #[test]
    fn native_stages_invalid_topology_later_failure_and_cancel_are_atomic() {
        let doc = document();
        let input = owner(&doc);
        let bounds = Rect::new(-5, 7, -2, 9);
        let mut budget = 1 << 28;
        let mut builder = NativeOwner::new(&doc, &input, bounds, &mut budget, &Interrupt::NONE).unwrap();
        let before = builder.source.clone();
        let state = (builder.stages, builder.added_depth, budget);
        let cases = vec![
            stage_mapping(vec![stage_apply(1, NativeMapping::default())], 1),
            stage_mapping(vec![NativeStage::Join { below: 0, above: 2, blend: BlendMode::Normal, opacity: 1. }], 1),
            stage_mapping(vec![], 1),
            stage_mapping(vec![stage_apply(0, stage_mapping(vec![], 0))], 1),
            stage_mapping(vec![NativeStage::Join { below: 0, above: 0, blend: BlendMode::PassThrough, opacity: 1. }], 1),
            stage_mapping((0..65).map(|_| stage_apply(0, NativeMapping::default())).collect(), 0),
            stage_mapping(
                vec![stage_apply(0, mapping(&["image.adjustments.invert"])), stage_apply(1, NativeMapping { quarter_turns: 4, ..Default::default() })],
                2,
            ),
        ];
        for case in cases {
            assert!(builder.push(case, fork_envelope(), &mut budget, &Interrupt::NONE).is_err());
            assert!(Arc::ptr_eq(&builder.source, &before));
            assert_eq!((builder.stages, builder.added_depth, budget), state);
        }
        let valid = || stage_mapping(vec![stage_apply(0, mapping(&["image.adjustments.invert"])), stage_apply(1, mapping(&["image.adjustments.invert"]))], 2);
        let mut tiny = 0;
        assert!(builder.push(valid(), fork_envelope(), &mut tiny, &Interrupt::NONE).is_err());
        assert_eq!(tiny, 0);
        assert!(Arc::ptr_eq(&builder.source, &before));
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 8;
        assert!(matches!(builder.push(valid(), fork_envelope(), &mut budget, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
        assert!(checks.load(Ordering::Relaxed) >= 9);
        assert!(Arc::ptr_eq(&builder.source, &before));
        assert_eq!((builder.stages, builder.added_depth, budget), state);
    }

    #[test]
    fn retained_mapping_resources_are_aggregate_without_charging_pixels_as_metadata() {
        let doc = document();
        let input = owner(&doc);
        let make = || NativeMapping {
            patterns: vec![photocraft_doc::Pattern::new("retained native pixels", input.surface().unwrap().clone(), 3, 2)],
            ..Default::default()
        };
        let first = make();
        let second = make();
        let a = retained_mapping_bytes(&first, &Interrupt::NONE).unwrap();
        let b = retained_mapping_bytes(&second, &Interrupt::NONE).unwrap();
        assert!(a.0 >= surface_bytes(&first.patterns[0].surface).unwrap());
        assert!(a.1 < 1024, "pixel tiles must not consume metadata allowance");
        let initial = a.0.max(b.0) + 1;
        assert!(a.0 <= initial && b.0 <= initial);
        let after_first = initial.checked_sub(a.0).unwrap();
        assert!(after_first.checked_sub(b.0).is_none(), "second converter must receive the reduced allowance");
        assert_eq!(retained_mapping_bytes(&first, &Interrupt::NONE).unwrap(), a);
        assert_eq!(initial, a.0.max(b.0) + 1);
    }

    #[test]
    fn retained_mapping_counts_direct_and_branch_native_data_and_is_cancellable() {
        let doc = document();
        let input = owner(&doc);
        let mut replacement = input.clone();
        replacement.name = "native replacement metadata".into();
        replacement.effects.items.push(Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.8), color: Color::rgb(0.2, 0.4, 0.6) });
        let direct = NativeMapping { structure: Some(NativeStructure::Replace { layer: Box::new(replacement), patterns: Vec::new() }), ..Default::default() };
        let direct_bytes = retained_mapping_bytes(&direct, &Interrupt::NONE).unwrap();
        assert!(direct_bytes.0 > surface_bytes(input.surface().unwrap()).unwrap());
        assert!(direct_bytes.1 > 32);
        let branch = super::super::effects::NativeBranch {
            mask: Some(LayerMask { surface: input.surface().unwrap().clone(), ..LayerMask::reveal_all() }),
            filters: fork_native_filter("filter.other.lut", serde_json::json!({"cube": "native LUT text".repeat(4096)})).filters,
            ..Default::default()
        };
        let composite = NativeMapping {
            structure: Some(NativeStructure::Composite { branches: vec![branch] }),
            notes: vec!["retained policy".into()],
            ..Default::default()
        };
        let branch_bytes = retained_mapping_bytes(&composite, &Interrupt::NONE).unwrap();
        assert!(branch_bytes.1 > 4096 * 15);
        assert!(branch_bytes.0 > branch_bytes.1);
        let cancel = || true;
        assert!(matches!(retained_mapping_bytes(&composite, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
        let nested = stage_mapping(Vec::new(), 0);
        assert!(retained_mapping_bytes(&nested, &Interrupt::NONE).is_err());
    }
    #[test]
    fn editable_native_text_replacement_is_bounded_persistent_and_failure_atomic() {
        let doc = document();
        let input = owner(&doc);
        let mut budget = 1 << 28;
        let mut builder = NativeOwner::new(&doc, &input, Rect::new(-5, 7, -2, 9), &mut budget, &Interrupt::NONE).unwrap();
        let native = photocraft_doc::TextLayer { text: "Editable café".into(), font_family: "Sans".into(), size_pt: 10., ..Default::default() };
        let make = |text| NativeMapping {
            structure: Some(NativeStructure::Replace { layer: Box::new(Layer::new("Native typography", LayerContent::Text(text))), patterns: Vec::new() }),
            ..Default::default()
        };
        let mapping = make(native.clone());
        let retained = retained_mapping_bytes(&mapping, &Interrupt::NONE).unwrap();
        assert!(retained.1 > native.text.len());
        builder.push(mapping, fork_envelope(), &mut budget, &Interrupt::NONE).unwrap();
        let saved = embedded(&builder.content());
        let LayerContent::Text(text) = &saved.layers[1].content else {
            panic!("native editable text");
        };
        assert_eq!((text.text.as_str(), text.font_family.as_str(), text.size_pt), ("Editable café", "Sans", 10.));
        assert!(text.cache.is_none());
        let bytes = photocraft_format::save_to_bytes(&saved, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert!(matches!(&reopened.layers[1].content,LayerContent::Text(text) if text.text == native.text));
        let before = builder.source.clone();
        let original_budget = budget;
        for corrupt in [f32::NAN, f32::INFINITY, -1.] {
            let mut text = native.clone();
            text.size_pt = corrupt;
            assert!(builder.push(make(text), fork_envelope(), &mut budget, &Interrupt::NONE).is_err());
            assert_eq!(budget, original_budget);
            assert!(Arc::ptr_eq(&before, &builder.source));
        }
        let mut too_long = native.clone();
        too_long.text = "a".repeat(65_537);
        assert!(builder.push(make(too_long), fork_envelope(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget, original_budget);
        assert!(Arc::ptr_eq(&before, &builder.source));
    }
    #[test]
    fn actual_native_knockout_branches_match_independent_native_documents_and_reopen() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let doc = Document::new("Native knockout", Size::new(2, 1), mode, depth);
                let mut input = Layer::raster("Actual lower image", doc.pixel_format());
                let values = if mode == ColorMode::Rgb { vec![0.7, 0.2, 0.3, 0.8] } else { vec![0.7, 0.8] };
                input.surface_mut().unwrap().fill_rect(doc.bounds(), &values);
                let mut upper = Layer::raster("Actual upper image", doc.pixel_format());
                let upper_values = if mode == ColorMode::Rgb { vec![0.1, 0.4, 0.9, 0.5] } else { vec![0.1, 0.5] };
                upper.surface_mut().unwrap().fill_rect(doc.bounds(), &upper_values);
                let mut budget = 1 << 28;
                let mut root = NativeOwner::new(&doc, &input, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
                let below = root.fork();
                let mut above = root.fork();
                let native = || NativeMapping {
                    structure: Some(NativeStructure::Replace { layer: Box::new(upper.clone()), patterns: Vec::new() }),
                    ..Default::default()
                };
                above.push(native(), fork_envelope(), &mut budget, &Interrupt::NONE).unwrap();
                root.knockout(&below, &above, 0.4, &mut budget, &Interrupt::NONE).unwrap();
                let mut actual = embedded(&root.content());
                assert_eq!(actual.layers[1].advanced.knockout, photocraft_doc::Knockout::Shallow);
                assert_eq!((actual.layers[1].fill_opacity, actual.layers[1].opacity), (0., 0.4));
                prepare_fork_native(&mut actual, 0);
                let mut expected = doc.clone();
                expected.layers.push(input.clone());
                let mut native_upper = upper.clone();
                native_upper.opacity = 0.4;
                native_upper.fill_opacity = 0.;
                native_upper.advanced.knockout = photocraft_doc::Knockout::Shallow;
                expected.layers.push(native_upper);
                let result = photocraft_compose::render(&actual, doc.bounds());
                let oracle = photocraft_compose::render(&expected, doc.bounds());
                for (got, want) in result.px.iter().zip(&oracle.px) {
                    for (got, want) in got.iter().zip(want) {
                        assert!((got - want).abs() < 1e-6, "{mode:?}/{depth:?}: {got} != {want}");
                    }
                }
                assert!(oracle.px[0][3] > 0. && oracle.px[0][3] < 0.8, "independent knockout must actually remove coverage");
                let saved = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
                assert_eq!(reopened.layers[1].advanced.knockout, photocraft_doc::Knockout::Shallow);
                assert_eq!(photocraft_compose::render(&reopened, doc.bounds()).px, result.px);
                // Exercise the same ordinary construction through the import-only
                // native stage list, with the outer envelope applied once.
                let mut staged = NativeOwner::new(&doc, &input, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
                staged
                    .push(
                        stage_mapping(vec![stage_apply(0, native()), NativeStage::Knockout { below: 0, above: 1, opacity: 0.4 }], 2),
                        fork_envelope(),
                        &mut budget,
                        &Interrupt::NONE,
                    )
                    .unwrap();
                let mut staged_doc = embedded(&staged.content());
                prepare_fork_native(&mut staged_doc, 0);
                // The plan output is an ordinary extra Smart contents boundary:
                // its cache quantizes at U8/U16 before the outer native render.
                let bytes = photocraft_format::save_to_bytes(&expected, &Default::default()).unwrap();
                let mut wrapped_oracle = doc.clone();
                wrapped_oracle.layers.push(Layer::new(
                    "Independent native Smart output",
                    LayerContent::Smart(SmartObject::new(
                        SmartSource::Embedded { file_name: "native-knockout-counterpart.pcraft".into(), bytes: Arc::new(bytes) },
                        Affine::IDENTITY,
                        None,
                    )),
                ));
                prepare_fork_native(&mut wrapped_oracle, 0);
                assert_eq!(photocraft_compose::render(&staged_doc, doc.bounds()).px, photocraft_compose::render(&wrapped_oracle, doc.bounds()).px);
            }
        }
    }
    #[test]
    fn native_knockout_rejects_foreign_opacity_budget_depth_and_cancel_atomically() {
        let doc = document();
        let input = owner(&doc);
        let bounds = Rect::new(-5, 7, -2, 9);
        let mut budget = 1 << 28;
        let mut root = NativeOwner::new(&doc, &input, bounds, &mut budget, &Interrupt::NONE).unwrap();
        let below = root.fork();
        let above = root.fork();
        let foreign = NativeOwner::new(&doc, &input, bounds, &mut budget, &Interrupt::NONE).unwrap();
        let before = root.source.clone();
        let state = (budget, root.stages, root.added_depth);
        for opacity in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(root.knockout(&below, &above, opacity, &mut budget, &Interrupt::NONE).is_err());
            assert_eq!((budget, root.stages, root.added_depth), state);
            assert!(Arc::ptr_eq(&before, &root.source));
        }
        assert!(root.knockout(&foreign, &above, 1., &mut budget, &Interrupt::NONE).is_err());
        let mut tiny = 0;
        assert!(root.knockout(&below, &above, 1., &mut tiny, &Interrupt::NONE).is_err());
        assert_eq!(tiny, 0);
        let mut deep = above.fork();
        deep.added_depth = 64;
        assert!(root.knockout(&below, &deep, 1., &mut budget, &Interrupt::NONE).is_err());
        let cancel = || true;
        assert!(matches!(root.knockout(&below, &above, 1., &mut budget, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 8;
        assert!(matches!(root.knockout(&below, &above, 1., &mut budget, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
        assert_eq!((budget, root.stages, root.added_depth), state);
        assert!(Arc::ptr_eq(&before, &root.source));
        assert!(Arc::ptr_eq(&below.source, &before));
        assert!(Arc::ptr_eq(&above.source, &before));
    }
    #[test]
    fn snapshot_reads_the_current_native_prefix_without_mutating_editable_builder() {
        let doc = document();
        let input = owner(&doc);
        let bounds = Rect::new(-5, 7, -2, 9);
        let mut budget = 1 << 28;
        let mut builder = NativeOwner::new(&doc, &input, bounds, &mut budget, &Interrupt::NONE).unwrap();
        builder.push(mapping(&["image.adjustments.invert"]), fork_envelope(), &mut budget, &Interrupt::NONE).unwrap();
        builder
            .push(
                fork_native_filter("image.adjustments.exposure", json!({"exposure":-1.,"offset":0.,"gamma":1.})),
                fork_envelope(),
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap();
        let before = builder.source.clone();
        let state = (builder.stages, builder.added_depth);
        let remaining = budget;
        let got = builder.snapshot(&mut budget, &Interrupt::NONE).unwrap();
        assert!(budget < remaining);
        assert!(Arc::ptr_eq(&before, &builder.source));
        assert_eq!(state, (builder.stages, builder.added_depth));
        let mut native = embedded(&builder.content());
        prepare_fork_native(&mut native, 0);
        let expected = photocraft_compose::render(&native, native.bounds());
        let mut expected_surface = Surface::new(doc.pixel_format());
        expected_surface
            .write_region(expected.rect, &expected.px.iter().flat_map(|pixel| photocraft_raster::from_rgba(&doc.pixel_format(), *pixel)).collect::<Vec<_>>());
        assert_eq!(got.read_region(native.bounds()), expected_surface.read_region(native.bounds()));
        let mut zero = 0;
        assert!(builder.snapshot(&mut zero, &Interrupt::NONE).is_err());
        assert_eq!(zero, 0);
        assert!(Arc::ptr_eq(&before, &builder.source));
    }
    #[test]
    fn captured_channel_and_auxiliary_mask_use_actual_native_pixels_and_persist() {
        use super::super::effects::NativeMaskComponent;
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let doc = Document::new("Captured native resources", Size::new(2, 1), mode, depth);
                let mut original = Layer::raster("Editable original", doc.pixel_format());
                let color = if mode == ColorMode::Rgb { vec![0.8, 0.3, 0.1, 0.8] } else { vec![0.8, 0.8] };
                original.surface_mut().unwrap().fill_rect(doc.bounds(), &color);
                let mut auxiliary = Layer::raster("Actual auxiliary", doc.pixel_format());
                let channels = doc.pixel_format().channels();
                let mut samples = vec![0.2; channels * 2];
                samples[channels - 1] = 0.25;
                samples[2 * channels - 1] = 0.75;
                auxiliary.surface_mut().unwrap().write_region(doc.bounds(), &samples);
                let stored = auxiliary.surface().unwrap().read_region(doc.bounds());
                let replace = || NativeMapping {
                    structure: Some(NativeStructure::Replace { layer: Box::new(auxiliary.clone()), patterns: Vec::new() }),
                    ..Default::default()
                };
                let mut budget = 1 << 28;
                let mut channel = NativeOwner::new(&doc, &original, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
                channel
                    .push(
                        stage_mapping(vec![stage_apply(0, replace()), NativeStage::Channel { input: 1, component: NativeMaskComponent::Alpha }], 2),
                        fork_envelope(),
                        &mut budget,
                        &Interrupt::NONE,
                    )
                    .unwrap();
                let got = channel.snapshot(&mut budget, &Interrupt::NONE).unwrap().read_region(doc.bounds());
                for (pixel, source) in got.chunks_exact(channels).zip(stored.chunks_exact(channels)) {
                    for color in &pixel[..channels - 1] {
                        assert!((*color - source[channels - 1]).abs() < 0.005);
                    }
                    assert_eq!(pixel[channels - 1], 1.);
                }
                let mut masked = NativeOwner::new(&doc, &original, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
                masked
                    .push(
                        stage_mapping(
                            vec![
                                stage_apply(0, replace()),
                                NativeStage::Mask { input: 0, mask: 1, component: NativeMaskComponent::Alpha, invert: true, scale: 0.5 },
                            ],
                            2,
                        ),
                        fork_envelope(),
                        &mut budget,
                        &Interrupt::NONE,
                    )
                    .unwrap();
                let got = masked.snapshot(&mut budget, &Interrupt::NONE).unwrap().read_region(doc.bounds());
                let mut mask_pixels = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                mask_pixels.write_region(doc.bounds(), &stored.chunks_exact(channels).map(|p| (1. - p[channels - 1]) * 0.5).collect::<Vec<_>>());
                let mut expected = doc.clone();
                let mut layer = original.clone();
                layer.mask = Some(LayerMask { surface: mask_pixels, ..LayerMask::reveal_all() });
                expected.layers.push(layer);
                let oracle = photocraft_compose::render(&expected, doc.bounds());
                for (pixel, oracle) in got.chunks_exact(channels).zip(&oracle.px) {
                    assert!((pixel[channels - 1] - oracle[3]).abs() < 0.01, "{mode:?}/{depth:?}: {pixel:?} versus {oracle:?}");
                }
                assert!(got[channels - 1] > got[2 * channels - 1]);
                let saved = embedded(&masked.content());
                let bytes = photocraft_format::save_to_bytes(&saved, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                let recoded = photocraft_format::save_to_bytes(&reopened, &Default::default()).unwrap();
                let mut remaining = 1 << 28;
                let pixels = super::super::native_snapshot::capture(&recoded, &doc, &mut remaining, &Interrupt::NONE).unwrap();
                assert_eq!(pixels.read_region(doc.bounds()), got);
                let before = masked.source.clone();
                let state = (budget, masked.stages, masked.added_depth);
                let invalid_plan =
                    stage_mapping(vec![NativeStage::Mask { input: 0, mask: 0, component: NativeMaskComponent::Alpha, invert: false, scale: f32::NAN }], 1);
                assert!(masked.push(invalid_plan, fork_envelope(), &mut budget, &Interrupt::NONE).is_err());
                assert_eq!((budget, masked.stages, masked.added_depth), state);
                assert!(Arc::ptr_eq(&before, &masked.source));
                let cancel = || true;
                let plan = stage_mapping(vec![NativeStage::Channel { input: 0, component: NativeMaskComponent::Composite }], 1);
                assert!(matches!(masked.push(plan, fork_envelope(), &mut budget, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
                assert_eq!((budget, masked.stages, masked.added_depth), state);
                assert!(Arc::ptr_eq(&before, &masked.source));
            }
        }
    }
    #[test]
    fn actual_auxiliary_displacement_captures_map_and_keeps_native_filter_editable() {
        use super::super::effects::NativeMapCoordinates;
        let doc = Document::new("Native auxiliary displacement", Size::new(3, 2), ColorMode::Rgb, SampleType::F32);
        let mut original = Layer::raster("Editable pixels", doc.pixel_format());
        original
            .surface_mut()
            .unwrap()
            .write_region(doc.bounds(), &[0.1, 0.2, 0.3, 1., 0.8, 0.2, 0.1, 0.5, 0.3, 0.7, 0.2, 1., 0.6, 0.3, 0.9, 1., 0.2, 0.9, 0.1, 0.7, 0.9, 0.8, 0.2, 1.]);
        let mut map = Layer::raster("Actual map", doc.pixel_format());
        map.surface_mut().unwrap().fill_rect(doc.bounds(), &[1., 0., 0., 1.]);
        let mut budget = 1 << 28;
        let mut actual = NativeOwner::new(&doc, &original, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
        let mut expected = actual.fork();
        let make = |metadata_limit| {
            stage_mapping(
                vec![
                    stage_apply(
                        0,
                        NativeMapping {
                            structure: Some(NativeStructure::Replace { layer: Box::new(map.clone()), patterns: Vec::new() }),
                            ..Default::default()
                        },
                    ),
                    NativeStage::Displace { input: 0, map: 1, coordinates: NativeMapCoordinates::Offsets, scale: 1., wrap: false, metadata_limit },
                ],
                2,
            )
        };
        // Independently author the ordinary Photoshop-compatible native map:
        // horizontal control1 has a 1.28px full normalized range, so +1px
        // encodes as .5 + 1/(2*1.28); vertical displacement is zero.
        let native_params = crate::smart_map::displace_embed_map(
            &json!({"horizontal":1.,"vertical":0.,"fit":"stretch","undefinedAreas":"repeat"}),
            &photocraft_algo::Image { rect: doc.bounds(), ch: 3, data: [0.890625, 0.5, 0.5].repeat(6) },
        )
        .unwrap();
        expected.push(fork_native_filter("filter.distort.displace", native_params), fork_envelope(), &mut budget, &Interrupt::NONE).unwrap();
        actual.push(make(super::super::MAX_METADATA_BYTES), fork_envelope(), &mut budget, &Interrupt::NONE).unwrap();
        let got = actual.snapshot(&mut budget, &Interrupt::NONE).unwrap();
        let want = expected.snapshot(&mut budget, &Interrupt::NONE).unwrap();
        // Stages add a native Smart contents boundary over the direct native
        // counterpart. Its F32 premultiply/unpremultiply round trip can differ
        // by a few ulps at partial alpha; this is not a displaced-pixel tolerance.
        for (got, want) in got.read_region(doc.bounds()).iter().zip(want.read_region(doc.bounds())) {
            assert!((got - want).abs() <= 4. * f32::EPSILON, "{got} versus {want}");
        }
        assert_ne!(got.read_region(doc.bounds()), original.surface().unwrap().read_region(doc.bounds()));
        let before = actual.source.clone();
        let state = (budget, actual.stages, actual.added_depth);
        assert!(actual.push(make(1), fork_envelope(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!((budget, actual.stages, actual.added_depth), state);
        assert!(Arc::ptr_eq(&before, &actual.source));
    }
}
