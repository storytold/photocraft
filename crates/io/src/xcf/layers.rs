//! Native layer admission, metadata retention and hierarchy construction.

use super::{
    AlphaChannel, BlendMode, Channel, ConversionDisposition, ConversionEntry, ConversionReason, Image, Interrupt, IoError, LabelColor, Layer, LayerContent,
    LayerMask, MAX_DEPTH, NativeTargetRef, Node, PendingLayerEffects, PixelFormat, Pixels, Prop, Rect, Result, Surface, be_f32, be_i32, be_u32, blend_mode,
    conversion, finish, invalid, label, metadata, parasites, paths, place, props, resources, text, write_pixels,
};

impl<'a> Image<'a> {
    /// Disclose omitted optional metadata after its native owner exists.
    fn layer_metadata_limits(&mut self, outcome: usize, name_limited: bool, path_limited: bool, effects_limited: bool) -> Result<()> {
        if name_limited {
            self.metadata_limit(outcome, "xcf.layer_name", "Layer name omitted after the shared metadata allowance was exhausted")?;
        }
        if path_limited {
            self.metadata_limit(
                outcome,
                "xcf.item_path",
                "Layer item path omitted after the shared metadata allowance was exhausted; retained as a top-level layer",
            )?;
        }
        if effects_limited {
            self.metadata_limit(outcome, "xcf.layer_effect", "Layer effect pointers omitted after optional metadata retention limits were reached")?;
        }
        Ok(())
    }

    /// Bound retained hierarchy metadata before allocating it; omitted paths leave usable layers at the top level.
    fn retained_item_path(&mut self, bytes: &[u8]) -> (Vec<u32>, bool) {
        let count = (bytes.len() / 4).min(MAX_DEPTH + 1);
        let Some(remaining) = self.meta_budget.checked_sub(count.saturating_mul(4)) else { return (Vec::new(), true) };
        let path = bytes.as_chunks::<4>().0.iter().take(count).map(|word| u32::from_be_bytes(*word)).collect();
        self.meta_budget = remaining;
        (path, false)
    }

    /// Decode layers in source order, then build the native hierarchy with the existing bounded-path policy.
    pub(super) fn import_layers(&mut self, pointers: &[usize], fmt: PixelFormat, mask_fmt: PixelFormat, ctl: &Interrupt<'_>) -> Result<(Vec<Layer>, bool)> {
        let total = pointers.len().max(1) as f32;

        // Layers, topmost first in the file.
        let mut nodes: Vec<Node> = Vec::new();
        let mut linear_compositing = false;
        for (i, &ptr) in pointers.iter().enumerate() {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            ctl.progress(0.05 + 0.85 * i as f32 / total);
            if let Some((node, linear)) = self.layer(i, ptr, fmt, mask_fmt, ctl)? {
                linear_compositing |= linear;
                nodes.push(node);
            }
        }
        let mut roots: Vec<Node> = Vec::new();
        for (i, mut node) in nodes.into_iter().enumerate() {
            // Default path: the item's position at the top level, as a file without groups has it.
            if node.path.is_empty() {
                node.path = vec![i as u32];
            }
            if node.path.len() > MAX_DEPTH {
                self.warnings.layer(&node.layer.name, "is nested deeper than the reader allows and was moved to the top level");
                self.outcomes.detail(
                    node.outcome,
                    "xcf.item_path",
                    ConversionReason::NativeLimitation,
                    "Source hierarchy exceeds 64 components".into(),
                    "Layer moved to the native top level; source identity retains the ordinal and bounded path prefix".into(),
                )?;
                node.path.truncate(1);
            }
            place(&mut roots, node, &mut self.warnings, &mut self.outcomes)?;
        }
        Ok((finish(roots), linear_compositing))
    }

    /// Decode saved selection and alpha coverage, retaining duplicate markers and applying native view hints once.
    pub(super) fn import_channels(&mut self, pointers: &[usize], mask_fmt: PixelFormat, ctl: &Interrupt<'_>) -> Result<(Vec<AlphaChannel>, Option<Surface>)> {
        let precision = self.precision;
        // Channels: the selection and the extras.
        let mut channels = Vec::new();
        let mut selection = None;
        for (ordinal, &ptr) in pointers.iter().enumerate() {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let Channel { name, properties: cprops, pixels: px, effects: coverage_effects, name_limited, effects_limited } = self.channel(ptr, ctl)?;
            let mut surface = Surface::new(mask_fmt);
            write_pixels(&mut surface, &px, 0, 0, &mut self.budget)?;
            let is_selection = cprops.iter().any(|p| p.kind == 4);
            let native_selection = is_selection && selection.is_none();
            let channel_outcome = self.outcomes.push(ConversionEntry {
                feature_code: if is_selection { "xcf.selection" } else { "xcf.channel" }.into(),
                source: conversion::source(
                    if is_selection { "selection" } else { "channel" },
                    ordinal,
                    ptr,
                    &name,
                    &[],
                    cprops.iter().find(|p| p.kind == 20).and_then(|p| be_u32(p.data)),
                ),
                targets: vec![NativeTargetRef {
                    channel_index: if native_selection { None } else { Some(channels.len() as u32) },
                    label: if native_selection { "Document selection" } else { "Alpha channel" }.into(),
                    ..Default::default()
                }],
                disposition: if precision.reduces_precision() { ConversionDisposition::NativeApproximation } else { ConversionDisposition::NativeEquivalent },
                reason: ConversionReason::NativePolicy,
                source_description: "Stored channel coverage samples".into(),
                native_description: "Native coverage surface".into(),
                attachments: Vec::new(),
            })?;
            if name_limited {
                self.metadata_limit(channel_outcome, "xcf.channel_name", "Channel name omitted after the shared metadata allowance was exhausted")?;
            }
            if effects_limited {
                self.metadata_limit(
                    channel_outcome,
                    "xcf.coverage_effect",
                    "Channel effect pointers omitted after the shared metadata allowance was exhausted",
                )?;
            }
            self.precision_underflow(channel_outcome, &px)?;
            self.inventory_coverage_effects(channel_outcome, &coverage_effects, ctl)?;
            let mut channel = AlphaChannel::new(name, surface);
            let handled = metadata::channel_overlay(&mut channel, &cprops);
            for property in &cprops {
                if !matches!(property.kind, 4 | 20) && !handled.contains(&property.kind) {
                    let reason = match property.kind {
                        3 | 8 => ConversionReason::NativePolicy,
                        6 | 14 | 16 | 33 | 38 => ConversionReason::InvalidOptionalData,
                        9 | 28 | 32 | 34 | 42 => ConversionReason::NativeLimitation,
                        _ => ConversionReason::MissingMapping,
                    };
                    self.outcomes.property_with_reason(channel_outcome, property.kind, property.offset, property.data, reason)?;
                }
            }
            if native_selection {
                selection = Some(channel.surface);
            } else {
                // Preserve ambiguous additional selection records as ordinary saved alpha channels.
                if is_selection {
                    self.outcomes.detail(
                        channel_outcome,
                        "xcf.duplicate_selection",
                        ConversionReason::InvalidOptionalData,
                        "More than one source channel is marked as the active selection".into(),
                        "First selection kept; additional coverage retained as an ordinary alpha channel".into(),
                    )?;
                }
                if metadata::flag(&cprops, 8) {
                    self.initial_view.visible_alpha_channels.push(channels.len());
                }
                if metadata::active(&cprops, 3) {
                    if matches!(self.initial_view.target, crate::initial_view::NativeImportTarget::AlphaChannel(_)) {
                        self.outcomes.detail(
                            channel_outcome,
                            "xcf.active_channels",
                            ConversionReason::NativeLimitation,
                            "Multiple source channels selected".into(),
                            "Native initial view targets one alpha channel; original markers remain inert".into(),
                        )?;
                    } else {
                        self.initial_view.target = crate::initial_view::NativeImportTarget::AlphaChannel(channels.len());
                    }
                }
                channels.push(channel);
            }
        }
        Ok((channels, selection))
    }

    /// Describe floating attachments and the single historical movement set without changing their native layer semantics.
    pub(super) fn layer_identity_notices(
        &mut self,
        outcome: usize,
        properties: &[Prop<'a>],
        floating: bool,
        gray_floating: bool,
        legacy_linked: bool,
    ) -> Result<()> {
        if floating {
            let attachment = properties.iter().rev().find(|p| p.kind == 5).map(|p| resources::attachment(p.data, self.c.wide));
            let (reason, source) = match attachment {
                Some(Ok(pointer)) => (ConversionReason::NativePolicy, format!("Floating pixels; source attachment record {pointer}")),
                _ => (ConversionReason::InvalidOptionalData, "Floating pixels with an invalid optional attachment record".into()),
            };
            self.outcomes.notice(
                outcome,
                "xcf.floating_selection",
                reason,
                (
                    source,
                    if gray_floating {
                        "Separate positioned native raster layer; gray coverage repeated into RGB, attachment retained inert; original channel/mask unchanged"
                    } else {
                        "Separate positioned native raster layer; attachment retained inert, without merging or anchoring"
                    }
                    .into(),
                ),
                Some("is a floating selection and opens as a separate editable layer"),
                &mut self.warnings,
            )?;
        }
        if legacy_linked {
            self.outcomes.detail(
                outcome,
                "xcf.legacy_linked_movement",
                ConversionReason::NativePolicy,
                "Member of the source's single legacy linked movement set".into(),
                "Native link_group 1; ordinary PhotoCraft movement, linking and unlinking apply".into(),
            )?;
        }
        Ok(())
    }

    /// Report inert linked-resource and placement records without resolving a locator or applying placement twice.
    pub(super) fn layer_link(&mut self, outcome: usize, properties: &[Prop<'a>], hptr: usize) -> Result<()> {
        let metadata = properties.iter().rev().find(|p| p.kind == 48).map(|p| resources::link(p.data));
        let (reason, source) = match metadata {
            Some(Ok(metadata)) => (
                if hptr == 0 { ConversionReason::ResourceUnavailable } else { ConversionReason::NativePolicy },
                format!(
                    "Link resource {}x{}, flags {}, nonempty locator {} bytes retained inert; {}",
                    metadata.width,
                    metadata.height,
                    metadata.flags,
                    metadata.locator.len(),
                    if hptr == 0 { "no stored pixel hierarchy" } else { "stored pixel hierarchy available" },
                ),
            ),
            _ => (ConversionReason::InvalidOptionalData, "Invalid optional link-resource configuration retained inert".into()),
        };
        self.outcomes.notice(
            outcome,
            "xcf.link_layer",
            reason,
            (
                source,
                if hptr == 0 {
                    "Native raster layer has no stored pixels; source resource is unavailable until explicitly supplied by the user"
                } else {
                    "Stored pixels preserved as an editable native raster; no locator resolution, automatic refresh or filesystem watch"
                }
                .into(),
            ),
            Some(if hptr == 0 {
                "is a link layer without stored pixels; its source must be supplied explicitly"
            } else {
                "is a link layer and opens as stored editable pixels"
            }),
            &mut self.warnings,
        )?;
        if let Some(property) = properties.iter().rev().find(|p| p.kind == 49) {
            let (reason, source) = match resources::transform(property.data) {
                Ok(transform) => (
                    ConversionReason::NativePolicy,
                    format!(
                        "Source placement offset {:?}, interpolation {}, projective matrix {:?}",
                        transform.offset, transform.interpolation, transform.matrix
                    ),
                ),
                Err(_) => (ConversionReason::InvalidOptionalData, "Invalid optional link placement retained inert".into()),
            };
            self.outcomes.detail(outcome, "xcf.link_transform", reason, source,
                "Stored drawable pixels already contain the source placement; native layer offsets are preserved and the source transform is not applied a second time".into())?;
        }
        Ok(())
    }

    /// Record source blend and coverage policies while leaving the already constructed native layer unchanged.
    pub(super) fn layer_compositing(&mut self, layer: &Layer, outcome: usize, layer_mode: u32, group: bool, composite_mode: i32) -> Result<()> {
        let (blend, note) = blend_mode(layer_mode);
        // These describe the native translation, not a promise of GIMP formula equivalence.
        if layer_mode != 0 && layer_mode != 28 && !(group && layer_mode == 61) {
            let human = note.map(|note| {
                format!(
                    "uses GIMP's {note} mode, which opens as {}",
                    if blend == BlendMode::Normal { "Normal".into() } else { format!("Photoshop's {blend:?}") }
                )
            });
            self.outcomes.notice(
                outcome,
                "xcf.blend_mode",
                if blend == BlendMode::Normal { ConversionReason::MissingMapping } else { ConversionReason::NativePolicy },
                (
                    format!("GIMP mode {layer_mode}{}", note.map(|s| format!(" ({s})")).unwrap_or_default()),
                    format!("Native {:?}; PhotoCraft blend formulas and clamping apply", layer.blend),
                ),
                human.as_deref(),
                &mut self.warnings,
            )?;
        }
        if composite_mode.unsigned_abs() > 1 {
            let human = match composite_mode.unsigned_abs() {
                3 => Some("composites with Clip to layer, which is not reproduced"),
                4 => Some("composites with Intersection, which is not reproduced"),
                _ => None,
            };
            self.outcomes.notice(
                outcome,
                "xcf.composite_mode",
                ConversionReason::NativePolicy,
                (
                    format!(
                        "GIMP coverage mode {composite_mode} ({})",
                        match composite_mode.unsigned_abs() {
                            2 => "Clip to backdrop",
                            3 => "Clip to layer",
                            4 => "Intersection",
                            _ => "unknown",
                        }
                    ),
                    "Native source-over coverage; source coverage does not enable a native clipping mask".into(),
                ),
                human,
                &mut self.warnings,
            )?;
        }
        Ok(())
    }

    /// Translate unmodified stored text into editable native text, retaining saved pixels on optional-data failures.
    pub(super) fn layer_text(&mut self, layer: &mut Layer, properties: &[Prop<'a>], position: (i32, i32), outcome: usize) -> Result<()> {
        let (x, y) = position;
        let modified = properties.iter().find(|p| p.kind == 26).and_then(|p| be_u32(p.data)).is_some_and(|flags| flags & 2 != 0);
        let config = properties
            .iter()
            .filter(|p| p.kind == 21)
            .flat_map(|p| parasites(p.data))
            .filter(|(name, _)| matches!(name.as_str(), "gimp-text-layer" | "plug_in_gdyntext/data"))
            .min_by_key(|(name, _)| name != "gimp-text-layer");
        if modified {
            self.outcomes.notice(
                outcome,
                "xcf.text",
                ConversionReason::NativePolicy,
                (
                    "Source text pixels were modified independently of the saved text configuration".into(),
                    "Native raster preserves modified pixels; original text configuration remains inert".into(),
                ),
                Some("has modified text pixels and opens as a native raster layer"),
                &mut self.warnings,
            )?;
        } else if let Some((config_name, config)) = config {
            // Decline repeated large configurations before font lookup and layout
            // once even their source-sized retained text cannot fit the shared budget.
            if config.len() > self.meta_budget {
                return self.text_retention_fallback(outcome);
            }
            let decoded = if let Some(error) = &self.native_color_error {
                Err(IoError::Unsupported(format!("native text colors cannot be converted: {error}")))
            } else if config_name == "plug_in_gdyntext/data" {
                text::decode_gdyntext(&config, self.resolution_dpi, (x, y))
            } else {
                text::decode(&config, self.resolution_dpi, (x, y))
            };
            match decoded {
                Ok(decoded) => {
                    let Some(remaining) = self.meta_budget.checked_sub(text::retained_allocation(&decoded.layer)) else {
                        return self.text_retention_fallback(outcome);
                    };
                    self.meta_budget = remaining;
                    layer.content = LayerContent::Text(decoded.layer);
                    self.outcomes.notice(
                        outcome,
                        "xcf.text",
                        ConversionReason::NativePolicy,
                        (
                            "Editable source text, typography and placement".into(),
                            "Native editable text; PhotoCraft font matching, shaping and rasterization apply".into(),
                        ),
                        Some("opens as editable native text with PhotoCraft typography"),
                        &mut self.warnings,
                    )?;
                    for note in decoded.native_policy {
                        self.outcomes.detail(
                            outcome,
                            "xcf.text.formatting",
                            ConversionReason::NativePolicy,
                            note,
                            "Native text placement and editable affine data apply".into(),
                        )?;
                    }
                    for note in decoded.source_only {
                        self.outcomes.detail(
                            outcome,
                            "xcf.text.formatting",
                            ConversionReason::MissingMapping,
                            note,
                            "Supported native typography is editable; additional source formatting remains inert".into(),
                        )?;
                    }
                }
                Err(error) => self.outcomes.notice(
                    outcome,
                    "xcf.text",
                    if matches!(error, IoError::Unsupported(_)) { ConversionReason::MissingMapping } else { ConversionReason::InvalidOptionalData },
                    (
                        format!("Source text configuration could not be translated: {error}"),
                        "Stored pixels preserved as a native raster; source configuration remains inert".into(),
                    ),
                    Some("has unsupported or invalid text configuration and opens as pixels"),
                    &mut self.warnings,
                )?,
            }
        } else {
            self.outcomes.notice(
                outcome,
                "xcf.text",
                ConversionReason::InvalidOptionalData,
                ("Source text marker has no saved text configuration".into(), "Stored pixels preserved as a native raster".into()),
                Some("has no saved text configuration and opens as pixels"),
                &mut self.warnings,
            )?;
        }
        Ok(())
    }

    /// Optional editability cannot multiply one source record beyond the retained metadata allowance.
    fn text_retention_fallback(&mut self, outcome: usize) -> Result<()> {
        self.outcomes.notice(
            outcome,
            "xcf.text",
            ConversionReason::RetentionLimit,
            (
                "Editable source text exceeds the remaining native metadata allowance".into(),
                "Stored pixels preserved as a native raster; text configuration remains inert".into(),
            ),
            Some("has text exceeding the native metadata allowance and opens as pixels"),
            &mut self.warnings,
        )
    }

    /// Retain only unmapped layer records with their original policy/error classifications and shared retention budget.
    pub(super) fn retain_layer_properties(&mut self, outcome: usize, properties: &[Prop<'a>]) -> Result<()> {
        for property in properties {
            if property.kind == 21 {
                self.outcomes.parasites(outcome, property.offset, property.data, &["gimphoto-clipped"])?;
            } else if !matches!(property.kind, 6 | 7 | 8 | 9 | 10 | 11 | 15 | 20 | 28 | 29 | 30 | 31 | 32 | 33 | 34 | 35 | 36 | 37) {
                let reason = match property.kind {
                    5 => {
                        if resources::attachment(property.data, self.c.wide).is_ok() {
                            ConversionReason::NativePolicy
                        } else {
                            ConversionReason::InvalidOptionalData
                        }
                    }
                    48 => {
                        if resources::link(property.data).is_ok() {
                            ConversionReason::NativePolicy
                        } else {
                            ConversionReason::InvalidOptionalData
                        }
                    }
                    49 => {
                        if resources::transform(property.data).is_ok() {
                            ConversionReason::NativePolicy
                        } else {
                            ConversionReason::InvalidOptionalData
                        }
                    }
                    2 | 12 | 13 => ConversionReason::NativePolicy,
                    42 => ConversionReason::NativeLimitation,
                    _ => ConversionReason::MissingMapping,
                };
                self.outcomes.property_with_reason(outcome, property.kind, property.offset, property.data, reason)?;
            }
        }
        Ok(())
    }

    /// Capture one native layer/mask editing target after its content is available, without persisting view rules.
    pub(super) fn layer_initial_view(&mut self, layer: &Layer, properties: &[Prop<'a>]) {
        if metadata::active(properties, 2) {
            self.initial_view.selected_layers.push(layer.id);
            if self.initial_view.active_layer.is_none() {
                self.initial_view.active_layer = Some(layer.id);
                self.initial_view.show_layer_mask = layer.mask.is_some() && metadata::flag(properties, 13);
                if layer.mask.is_some() && metadata::flag(properties, 12) {
                    self.initial_view.target = crate::initial_view::NativeImportTarget::LayerMask;
                }
            }
        }
    }

    /// Decode a layer's declared sample layout and restore the image model even when decoding fails or is cancelled.
    pub(super) fn layer_hierarchy(&mut self, hptr: usize, kind: u32, alpha: bool, floating: bool, name: &str, ctl: &Interrupt<'_>) -> Result<Pixels> {
        let source_base_type = if floating {
            match kind {
                0 | 1 => 0,
                2 | 3 => 1,
                _ => 2,
            }
        } else {
            self.base_type
        };
        let source_channels = match (source_base_type, alpha) {
            (2, false) => 1,
            (2, true) => 2,
            (1, false) => 1,
            (1, true) => 2,
            (_, false) => 3,
            (_, true) => 4,
        };
        let saved_base_type = self.base_type;
        self.base_type = source_base_type;
        // Gray samples remain color even for a floating selection attached to a
        // channel or mask. The hierarchy validates its alpha plane as coverage.
        let decoded = self.hierarchy(hptr, source_channels, false, ctl);
        self.base_type = saved_base_type;
        decoded.map_err(|error| match error {
            IoError::Cancelled => IoError::Cancelled,
            error => invalid(format!("layer '{name}': {error}")),
        })
    }

    /// Construct a native group or raster with source expansion state and the existing non-group pass-through fallback.
    pub(super) fn native_layer(name: String, fmt: PixelFormat, group: bool, expanded: bool, blend: BlendMode) -> Layer {
        if group {
            let mut g = Layer::group(name, Vec::new());
            if let LayerContent::Group(gr) = &mut g.content {
                gr.expanded = expanded;
            }
            g.blend = if blend == BlendMode::PassThrough { BlendMode::PassThrough } else { blend };
            g
        } else {
            let mut l = Layer::raster(name, fmt);
            l.blend = if blend == BlendMode::PassThrough { BlendMode::Normal } else { blend };
            l
        }
    }

    /// Reads a layer structure, and whether GIMP composites it in linear light.
    pub(super) fn layer(&mut self, ordinal: usize, ptr: usize, fmt: PixelFormat, mask_fmt: PixelFormat, ctl: &Interrupt<'_>) -> Result<Option<(Node, bool)>> {
        let mut c = self.c.at(ptr);
        let (lw, lh, kind) = (c.u32()?, c.u32()?, c.u32()?);
        let (name, name_limited) = self.metadata_name(&mut c, "Layer")?;
        let lprops = props(&mut c)?;
        let hptr = c.ptr()?;
        let mptr = c.ptr()?;
        let (effect_pointers, effects_limited) = if self.version >= 20 { self.optional_effect_pointers(&mut c, ctl)? } else { (Vec::new(), false) };
        let (type_rgb, alpha) = match kind {
            0 => (true, false),
            1 => (true, true),
            2 | 4 => (false, false),
            3 | 5 => (false, true),
            k => return Err(invalid(format!("unknown layer type {k} on '{name}'"))),
        };

        let mut layer_mode = 0u32;
        let mut opacity = 1.0f32;
        // PROP_FLOAT_OPACITY wins over PROP_OPACITY whatever their order in the file.
        let mut float_opacity = None;
        let mut visible = true;
        let (mut x, mut y) = (0i32, 0i32);
        let mut apply_mask = true;
        let mut group = false;
        let mut expanded = true;
        let mut path = Vec::new();
        let mut path_limited = false;
        let mut text = false;
        let mut vector = false;
        let mut link = false;
        let mut floating = false;
        let mut legacy_linked = false;
        let mut clipped = false;
        let mut composite_mode = 0i32;
        let mut composite_space = 0i32;
        let mut blend_space = 0i32;
        let mut locks = photocraft_doc::Locks::default();
        let mut tag = LabelColor::None;
        let mut tattoo = None;
        for p in &lprops {
            let v = be_u32(p.data).unwrap_or(0);
            match p.kind {
                5 => floating = true,
                6 => opacity = (v.min(255) as f32) / 255.0,
                33 => float_opacity = be_f32(p.data).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0)).or(float_opacity),
                7 => layer_mode = v,
                8 => visible = v != 0,
                9 => legacy_linked = v != 0,
                10 => locks.transparency = v != 0,
                11 => apply_mask = v != 0,
                15 => {
                    x = be_i32(p.data).unwrap_or(0);
                    y = p.data.get(4..).and_then(be_i32).unwrap_or(0);
                }
                26 => text = true,
                20 => tattoo = (v != 0).then_some(v),
                28 => locks.pixels = v != 0,
                29 => group = true,
                30 => {
                    let (retained, limited) = self.retained_item_path(p.data);
                    path = retained;
                    path_limited |= limited;
                }
                31 => expanded = v & 1 != 0,
                32 => locks.position = v != 0,
                34 => tag = label(v),
                35 => composite_mode = be_i32(p.data).unwrap_or(0),
                36 => composite_space = be_i32(p.data).unwrap_or(0),
                37 => blend_space = be_i32(p.data).unwrap_or(0),
                47 => vector = true,
                48 => link = true,
                21 => {
                    for (pname, _) in parasites(p.data) {
                        match pname.as_str() {
                            "gimphoto-clipped" => clipped = true,
                            "gimp-text-layer" | "plug_in_gdyntext/data" => text = true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        let gray_floating = floating && matches!(kind, 2 | 3) && self.base_type != 1;
        if !gray_floating && type_rgb != (self.base_type != 1) && self.base_type != 2 {
            return Err(invalid(format!("layer '{name}' does not match the image's colour model")));
        }

        let (blend, _) = blend_mode(layer_mode);
        if composite_mode.unsigned_abs() == 2 {
            self.clipped_to_backdrop += 1;
        }

        let mut layer = Self::native_layer(name.clone(), fmt, group, expanded, blend);
        layer.opacity = float_opacity.unwrap_or(opacity);
        layer.visible = visible;
        layer.locks = locks;
        layer.label = tag;
        layer.clipped = clipped;
        // Historical XCF stores one boolean movement set, not arbitrary independent groups.
        layer.link_group = legacy_linked.then_some(1);

        let outcome = self.outcomes.push(ConversionEntry {
            feature_code: "xcf.layer".into(),
            source: conversion::source("layer", ordinal, ptr, &name, &path, tattoo),
            targets: vec![NativeTargetRef { layer_id: Some(layer.id), ..Default::default() }],
            disposition: if text || vector || link || self.precision.reduces_precision() {
                ConversionDisposition::NativeApproximation
            } else {
                ConversionDisposition::NativeEquivalent
            },
            reason: ConversionReason::NativePolicy,
            source_description: "Layer structure, offsets, opacity, visibility, locks and stored pixel samples".into(),
            native_description: if group { "Native group and its properties" } else { "Native raster layer and its properties" }.into(),
            attachments: Vec::new(),
        })?;
        self.layer_metadata_limits(outcome, name_limited, path_limited, effects_limited)?;
        self.layer_identity_notices(outcome, &lprops, floating, gray_floating, legacy_linked)?;
        if link {
            self.layer_link(outcome, &lprops, hptr)?;
        }
        self.layer_compositing(&layer, outcome, layer_mode, group, composite_mode)?;
        let linear = composite_space.unsigned_abs() == 1 || blend_space.unsigned_abs() == 1;
        if linear || composite_space.unsigned_abs() > 2 || blend_space.unsigned_abs() > 2 {
            self.outcomes.detail(
                outcome,
                "xcf.compositing_space",
                ConversionReason::NativePolicy,
                format!("GIMP blend space {blend_space}, composite space {composite_space}"),
                "Native document encoding and PhotoCraft compositing rules".into(),
            )?;
        }

        if !group && hptr != 0 {
            let mut px = self.layer_hierarchy(hptr, kind, alpha, floating, &name, ctl)?;
            if (px.width, px.height) != (lw, lh) {
                return Err(invalid(format!("layer '{name}' pixels are {}x{} where {lw}x{lh} were declared", px.width, px.height)));
            }
            if !alpha {
                // An opaque layer: add the alpha channel the document's pixel format carries.
                let n = px.channels;
                let mut data = Vec::with_capacity(px.data.len() / n * (n + 1));
                for p in px.data.chunks_exact(n) {
                    data.extend_from_slice(p);
                    data.push(1.0);
                }
                px = Pixels {
                    width: px.width,
                    height: px.height,
                    channels: n + 1,
                    data,
                    underflow: px.underflow,
                    coverage_clamped: px.coverage_clamped,
                    palette_fallback: px.palette_fallback,
                };
            }
            if gray_floating {
                px = resources::gray_to_rgb(px, &mut self.budget, ctl)?;
            }
            let mut surface = Surface::new(fmt);
            write_pixels(&mut surface, &px, x, y, &mut self.budget)?;
            self.precision_underflow(outcome, &px)?;
            layer.content = LayerContent::Raster(surface);
            if !alpha {
                layer.locks.transparency = true;
            }
        }
        if vector
            && !group
            && let Some(property) = lprops.iter().rev().find(|p| p.kind == 47)
        {
            match paths::vector_style(property.data) {
                Ok(style) => {
                    let strings = style.source_only.iter().try_fold(
                        std::mem::size_of::<paths::VectorStyle>().saturating_add(style.source_only.capacity().saturating_mul(std::mem::size_of::<String>())),
                        |bytes, note| bytes.checked_add(note.capacity()),
                    );
                    // Reused source records still create distinct owned diagnostics.
                    if let Some(remaining) = strings.and_then(|strings| self.meta_budget.checked_sub(strings)) {
                        self.meta_budget = remaining;
                        self.pending_vectors.push((layer.id, outcome, style));
                    } else {
                        self.metadata_limit(
                            outcome,
                            "xcf.vector_layer",
                            "Editable vector metadata exceeds the remaining allowance; saved raster pixels retained",
                        )?;
                    }
                }
                Err(error) => self.outcomes.notice(
                    outcome,
                    "xcf.vector_layer",
                    if matches!(error, IoError::Unsupported(_)) { ConversionReason::MissingMapping } else { ConversionReason::InvalidOptionalData },
                    (
                        format!("Source vector style could not be translated: {error}"),
                        "Stored pixels preserved as a native raster; source style remains inert".into(),
                    ),
                    Some("has unsupported or invalid vector paint and opens as pixels"),
                    &mut self.warnings,
                )?,
            }
        }
        if text && !group {
            self.layer_text(&mut layer, &lprops, (x, y), outcome)?;
        }
        self.retain_layer_properties(outcome, &lprops)?;
        if mptr != 0 {
            let Channel { name: mname, properties: mprops, pixels: px, effects: coverage_effects, name_limited, effects_limited } = self.channel(mptr, ctl)?;
            let mut surface = Surface::new(mask_fmt);
            write_pixels(&mut surface, &px, x, y, &mut self.budget)?;
            let mask_outcome = self.outcomes.push(ConversionEntry {
                feature_code: "xcf.layer_mask".into(),
                source: conversion::source("layer_mask", ordinal, mptr, &mname, &path, mprops.iter().find(|p| p.kind == 20).and_then(|p| be_u32(p.data))),
                targets: vec![NativeTargetRef { layer_id: Some(layer.id), label: "Layer mask".into(), ..Default::default() }],
                disposition: if self.precision.reduces_precision() {
                    ConversionDisposition::NativeApproximation
                } else {
                    ConversionDisposition::NativeEquivalent
                },
                reason: ConversionReason::NativePolicy,
                source_description: "Stored mask coverage and enabled state".into(),
                native_description: "Native linked layer mask".into(),
                attachments: Vec::new(),
            })?;
            if name_limited {
                self.metadata_limit(mask_outcome, "xcf.mask_name", "Layer mask name omitted after the shared metadata allowance was exhausted")?;
            }
            if effects_limited {
                self.metadata_limit(mask_outcome, "xcf.coverage_effect", "Mask effect pointers omitted after the shared metadata allowance was exhausted")?;
            }
            self.precision_underflow(mask_outcome, &px)?;
            self.inventory_coverage_effects(mask_outcome, &coverage_effects, ctl)?;
            layer.mask = Some(LayerMask { surface, enabled: apply_mask, linked: true, density: 1.0, feather: 0.0 });
            for property in &mprops {
                let reason = match property.kind {
                    6 | 8 | 9 | 14 | 16 | 20 | 28 | 32 | 33 | 34 | 38 | 42 => ConversionReason::NativeLimitation,
                    _ => ConversionReason::MissingMapping,
                };
                self.outcomes.property_with_reason(mask_outcome, property.kind, property.offset, property.data, reason)?;
            }
        }
        self.layer_initial_view(&layer, &lprops);
        if !effect_pointers.is_empty() {
            let bounds = x.checked_add(lw as i32).zip(y.checked_add(lh as i32)).map_or(Rect::EMPTY, |(right, bottom)| Rect::new(x, y, right, bottom));
            self.pending_effects.push(PendingLayerEffects { layer: layer.id, owner: outcome, bounds, pointers: effect_pointers });
        }
        Ok(Some((Node { layer, path, group, children: Vec::new(), outcome }, linear)))
    }
}
