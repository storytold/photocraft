//! GIMP XCF import. `craft-xcf` owns XCF parsing, decompression and pixel decoding; this module
//! maps its neutral document representation into PhotoCraft's model.

mod color;
mod effects;
mod graph_lowering;
mod historical;
mod metadata;
mod metadata_packets;
mod paths;
mod raster;
mod report;
mod saved_view;
#[cfg(test)]
mod tests;
mod text_source;
pub(crate) mod transport;

#[cfg(test)]
use craft_xcf::{gunzip, is_gzipped_xcf};

use craft_xcf::{BaseType, TreeNode};
use craft_xcf::{records, samples::SampleOptions};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, LabelColor, Layer, LayerContent, LayerMask};
use photocraft_geom::Size;
use photocraft_raster::Interrupt;

use crate::ora::Warnings;
use crate::{ImportResult, IoError};
use raster::TileWriter;

type Result<T> = std::result::Result<T, IoError>;

#[cfg(test)]
pub(crate) const MAX_SIDE: u32 = craft_xcf::MAX_SIDE;
pub(crate) const MAX_PIXEL_BYTES: usize = craft_xcf::MAX_PIXEL_BYTES;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Xcf(message.into())
}

fn map_error(error: craft_xcf::Error) -> IoError {
    match error {
        craft_xcf::Error::Invalid(message) => invalid(message),
        craft_xcf::Error::Cancelled => IoError::Cancelled,
    }
}

fn blend_mode(mode: u32) -> (BlendMode, Option<&'static str>) {
    use BlendMode as B;
    match mode {
        0 | 28 => (B::Normal, None),
        1 => (B::Dissolve, None),
        2 | 29 => (B::Normal, Some("Behind")),
        3 | 30 => (B::Multiply, None),
        4 | 31 => (B::Screen, None),
        5 | 19 => (B::SoftLight, Some("GIMP's legacy Soft light")),
        45 => (B::SoftLight, Some("GIMP's Soft light")),
        23 => (B::Overlay, None),
        6 | 32 => (B::Difference, None),
        7 | 33 => (B::LinearDodge, None),
        8 | 34 => (B::Subtract, None),
        9 | 35 => (B::Darken, None),
        10 | 36 => (B::Lighten, None),
        11 | 37 => (B::Hue, Some("HSV Hue")),
        12 | 38 => (B::Saturation, Some("HSV Saturation")),
        13 | 39 => (B::Color, Some("HSL Color")),
        14 | 40 => (B::Luminosity, Some("HSV Value")),
        15 | 41 => (B::Divide, None),
        16 | 42 => (B::ColorDodge, None),
        17 | 43 => (B::ColorBurn, None),
        18 | 44 => (B::HardLight, None),
        20 | 46 => (B::Normal, Some("Grain extract")),
        21 | 47 => (B::Normal, Some("Grain merge")),
        22 | 57 => (B::Normal, Some("Color erase")),
        24 => (B::Hue, Some("LCH Hue")),
        25 => (B::Saturation, Some("LCH Chroma")),
        26 => (B::Color, Some("LCH Color")),
        27 => (B::Luminosity, Some("LCH Lightness")),
        48 => (B::VividLight, None),
        49 => (B::PinLight, None),
        50 => (B::LinearLight, None),
        51 => (B::HardMix, None),
        52 => (B::Exclusion, None),
        53 => (B::LinearBurn, None),
        54 => (B::DarkerColor, Some("Luma darken only")),
        55 => (B::LighterColor, Some("Luma lighten only")),
        56 => (B::Luminosity, Some("Luminance")),
        58 => (B::Normal, Some("Erase")),
        59 => (B::Normal, Some("Merge")),
        60 => (B::Normal, Some("Split")),
        61 => (B::PassThrough, None),
        _ => (B::Normal, Some("an unknown mode")),
    }
}

fn label(tag: u32) -> LabelColor {
    match tag {
        1 => LabelColor::Blue,
        2 => LabelColor::Green,
        3 => LabelColor::Yellow,
        4 => LabelColor::Orange,
        6 => LabelColor::Red,
        7 => LabelColor::Violet,
        8 => LabelColor::Gray,
        _ => LabelColor::None,
    }
}

struct Mapper<'a, 'c> {
    xcf: &'a craft_xcf::Xcf<'a>,
    ctl: &'c Interrupt<'c>,
    fmt: PixelFormat,
    mask_fmt: PixelFormat,
    budget: usize,
    source_budget: usize,
    record_budget: records::Budget,
    options: SampleOptions,
    report: report::Report<'a>,
    view: saved_view::SavedView,
    resolution: (f32, f32),
    text_budget: text_source::NativeBudget,
    text_colors: Option<text_source::NativeColors>,
    clipped_to_backdrop: usize,
    linear_compositing: bool,
    warnings: Warnings,
}

impl Mapper<'_, '_> {
    fn layer(&mut self, index: usize) -> Result<(Layer, bool)> {
        self.ctl.check().map_err(|_| IoError::Cancelled)?;
        let image = self.xcf.image();
        let source = image.layers.get(index).ok_or_else(|| invalid("tree references a missing layer"))?;
        let name = source.name.as_str();
        if source.floating_selection {
            self.warnings.layer(name, "is a floating selection and was not imported (anchor it in GIMP first)");
        }
        let (blend, note) = blend_mode(source.mode);
        if let Some(note) = note {
            self.warnings.layer(
                name,
                &format!(
                    "uses GIMP's {note} mode, which opens as {}",
                    if blend == BlendMode::Normal { "Normal".to_string() } else { format!("Photoshop's {blend:?}") }
                ),
            );
        }
        match source.composite_mode.unsigned_abs() {
            0 | 1 => {}
            2 => self.clipped_to_backdrop += 1,
            3 => self.warnings.layer(name, "composites with Clip to layer, which is not reproduced"),
            4 => self.warnings.layer(name, "composites with Intersection, which is not reproduced"),
            _ => {}
        }
        if source.vector {
            self.warnings.layer(name, "is a vector layer and opens as pixels");
        }
        if source.link {
            self.warnings.layer(name, "is a link layer and opens as pixels");
        }

        let mut layer = if source.group {
            let mut group = Layer::group(name, Vec::new());
            if let LayerContent::Group(content) = &mut group.content {
                content.expanded = source.expanded;
            }
            group.blend = blend;
            group
        } else {
            let mut raster = Layer::raster(name, self.fmt);
            raster.blend = if blend == BlendMode::PassThrough { BlendMode::Normal } else { blend };
            raster
        };
        layer.opacity = source.opacity;
        layer.visible = source.visible;
        layer.locks.transparency = source.lock_alpha;
        layer.locks.pixels = source.lock_content;
        layer.locks.position = source.lock_position;
        layer.label = label(source.color_tag);
        layer.clipped = source.clipped;

        if !source.group && source.has_pixels() {
            let opaque = !source.has_alpha();
            let mut writer = TileWriter::new(self.fmt, (source.x, source.y), &image.colormap, opaque, false, self.budget);
            self.xcf
                .visit_layer_tiles(index, self.options, &mut self.source_budget, &|| !self.ctl.cancelled(), |tile| writer.write(tile))
                .map_err(map_error)?
                .ok_or_else(|| invalid("layer pixels are missing"))?;
            let (surface, clamped) = writer.finish(&mut self.budget);
            layer.locks.transparency |= opaque;
            if clamped {
                self.warnings.layer(name, "has alpha samples outside the supported range");
            }
            layer.content = LayerContent::Raster(surface);
        }
        let mut writer = TileWriter::new(self.mask_fmt, (source.x, source.y), &[], false, true, self.budget);
        if self
            .xcf
            .visit_layer_mask_tiles(index, self.options, &mut self.source_budget, &|| !self.ctl.cancelled(), |tile| writer.write(tile))
            .map_err(map_error)?
            .is_some()
        {
            let (surface, clamped) = writer.finish(&mut self.budget);
            if clamped {
                self.warnings.layer(name, "has mask samples outside the supported range");
            }
            layer.mask = Some(LayerMask { surface, enabled: source.apply_mask, linked: true, density: 1.0, feather: 0.0 });
        }
        let linear = source.composite_space.unsigned_abs() == 1 || source.blend_space.unsigned_abs() == 1;
        self.linear_compositing |= linear;
        let properties = self.xcf.layer_properties(index, records::Limits::default(), &mut self.record_budget, &|| !self.ctl.cancelled()).map_err(map_error)?;
        self.view.layer(&properties, index, &layer, &mut self.record_budget, self.ctl)?;
        if !source.group && !source.floating_selection {
            self.map_text(&properties, index, source, &mut layer)?;
        }
        self.report.layer_effects(
            self.xcf,
            index,
            &mut layer,
            &mut effects::coverage::Admission {
                retained: &mut self.budget,
                source: &mut self.source_budget,
                metadata: &mut self.record_budget,
                options: self.options,
            },
            &mut self.warnings,
            self.ctl,
        )?;
        Ok((layer, linear))
    }

    fn map_text(&mut self, properties: &records::Scan<records::Property<'_>>, index: usize, source: &craft_xcf::Layer, layer: &mut Layer) -> Result<()> {
        use text_source::{Mapping, NativeOptions, TextSource};

        if !source.text && !properties.records.iter().any(|property| property.kind == 21) {
            return Ok(());
        }
        let text = text_source::read_layer(properties, &mut self.record_budget, self.ctl)?;
        if !source.text && matches!(text, TextSource::Missing) {
            return Ok(());
        }
        let options = NativeOptions { resolution: self.resolution, offset: (source.x, source.y), colors: self.text_colors.as_ref() };
        let mapping = if source.link {
            Mapping::Preserved { reason: "the link record takes precedence over its text configuration".into(), source_reason: None }
        } else {
            text_source::map_layer(layer, &text, options, &mut self.text_budget, &mut self.record_budget, self.ctl)?
        };
        match &mapping {
            Mapping::Missing => self.warnings.layer(&layer.name, "is a text layer and opens as pixels"),
            Mapping::Converted { notes } => {
                for note in notes {
                    self.warnings.layer(&layer.name, note);
                }
            }
            Mapping::Preserved { reason, .. } => self.warnings.layer(&layer.name, &format!("is a text layer and opens as pixels ({reason})")),
        }
        self.report.text(index, layer, &text, &mapping)
    }

    fn build(&mut self, tree: &[TreeNode]) -> Result<Vec<Layer>> {
        let mut out = Vec::with_capacity(tree.len());
        for node in tree.iter().rev() {
            let (mut layer, _) = self.layer(node.layer)?;
            if !node.children.is_empty() {
                let children = self.build(&node.children)?;
                if let LayerContent::Group(group) = &mut layer.content {
                    group.children = children;
                }
            }
            out.push(layer);
        }
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult> {
    import_with_samples(name, bytes, ctl, SampleOptions::default())
}

pub(crate) fn import_with_samples(name: &str, bytes: &[u8], ctl: &Interrupt<'_>, options: SampleOptions) -> Result<ImportResult> {
    let xcf = craft_xcf::Xcf::parse_with(bytes, &|| !ctl.cancelled()).map_err(map_error)?;
    let image = xcf.image();
    let request = xcf.historical_sample_order_request();
    if options.historical_order == crate::HistoricalXcfOrder::RequireExplicit
        && let Some(request) = request
    {
        return Err(IoError::HistoricalXcfSampleOrderRequired(crate::HistoricalXcfSampleOrderRequest {
            version: request.version,
            bits_per_sample: request.precision.storage.bytes as u32 * 8,
        }));
    }
    let mode = if image.base_type == BaseType::Gray { ColorMode::Grayscale } else { ColorMode::Rgb };
    let mode = if image.base_type == BaseType::Indexed { ColorMode::Rgb } else { mode };
    let depth = match (image.precision.float, image.precision.bytes) {
        (false, 1) => SampleType::U8,
        (false, 4) => SampleType::F32,
        (false, _) => SampleType::U16,
        (true, _) => SampleType::F32,
    };
    let mut warnings = Warnings::default();
    if image.version > craft_xcf::KNOWN_VERSION {
        warnings.note(&format!("XCF version {} is newer than this reader (version {}); unknown parts were skipped", image.version, craft_xcf::KNOWN_VERSION));
    }
    if image.base_type == BaseType::Indexed {
        warnings.note("the indexed image opens as RGB");
    }
    if !image.precision.float && image.precision.bytes == 4 {
        warnings.note("32-bit integer samples open as native F32 with reduced integer precision");
    }
    let mut document = Document::new(name, Size::new(image.width, image.height), mode, depth);
    let mut record_budget = records::Budget::default();
    let image_metadata = metadata::read(&xcf, &mut record_budget, &mut warnings, ctl)?;
    let source_axes = metadata::resolution(&image_metadata, document.resolution_dpi);
    let color = color::select_profile(mode, xcf.source_precision(), metadata::assigned_profile(&image_metadata, &mut warnings))?;
    if let Some(reason) = &color.fallback_reason {
        warnings.note(&format!("the assigned XCF profile could not be used ({reason}); an implicit profile interprets the saved samples"));
    }
    document.icc_profile = color.tag_document.then(|| color.bytes.clone());
    metadata::apply(&mut document, &image_metadata, source_axes, image.version, &mut warnings, ctl)?;
    for source in &image.layers {
        if source.floating_selection {
            warnings.layer(&source.name, "is a floating selection and was not imported (anchor it in GIMP first)");
        }
    }
    let fmt = PixelFormat::new(mode, depth, true);
    let mask_fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
    let mut mapper = Mapper {
        xcf: &xcf,
        ctl,
        fmt,
        mask_fmt,
        budget: MAX_PIXEL_BYTES,
        source_budget: MAX_PIXEL_BYTES,
        record_budget,
        options,
        report: report::Report::new(bytes, image.version).with_native_context(&document),
        view: saved_view::SavedView::default(),
        resolution: source_axes,
        text_budget: text_source::NativeBudget::default(),
        text_colors: text_source::NativeColors::new(&color.bytes, mode).ok(),
        clipped_to_backdrop: 0,
        linear_compositing: false,
        warnings,
    };
    let tree = image.tree();
    for &index in &tree.too_deep {
        if let Some(layer) = image.layers.get(index) {
            mapper.warnings.layer(&layer.name, "is nested deeper than the reader allows and was moved to the top level");
        }
    }
    for &index in &tree.misplaced {
        if let Some(layer) = image.layers.get(index) {
            mapper.warnings.layer(&layer.name, "has an item path that leads to no group; opened at the top level");
        }
    }
    let layers = mapper.build(&tree.roots)?;

    let mut channels = Vec::new();
    let mut selection = None;
    for index in 0..image.channel_count {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let mut writer = TileWriter::new(mask_fmt, (0, 0), &[], false, true, mapper.budget);
        let (channel, _) =
            xcf.visit_channel_tiles(index, options, &mut mapper.source_budget, &|| !ctl.cancelled(), |tile| writer.write(tile)).map_err(map_error)?;
        let (surface, clamped) = writer.finish(&mut mapper.budget);
        if clamped {
            mapper.warnings.note(&format!("XCF channel '{}' has samples outside the supported range", channel.name));
        }
        if channel.selection && selection.is_none() {
            selection = Some(surface);
        } else {
            if channel.selection {
                mapper.warnings.note("additional XCF selection channels were imported as saved alpha channels");
            }
            let mut alpha = AlphaChannel::new(channel.name, surface);
            mapper.view.channel(&xcf, index, channels.len(), &mut alpha, &mut mapper.record_budget, ctl)?;
            channels.push(alpha);
        }
    }

    document.layers = layers.into();
    document.channels = channels;
    document.selection = selection;
    let selected_path = paths::import_saved_paths(&xcf, &mut mapper.record_budget, &mut document, &mut mapper.warnings, ctl)?;
    if mapper.clipped_to_backdrop > 0 && document.layers.first().is_some_and(|l| !l.locks.transparency || l.is_group()) {
        mapper.warnings.note(&format!(
            "{} layer(s) use a mode GIMP clips to the layers beneath; where those are transparent GIMP shows nothing and PhotoCraft shows the layer",
            mapper.clipped_to_backdrop
        ));
    }
    if !image.precision.linear && mapper.linear_compositing {
        mapper
            .warnings
            .note("GIMP composites this image in linear light; PhotoCraft composites in gamma-encoded RGB, so blends and soft edges differ slightly");
    }
    if document.layers.is_empty() {
        mapper.warnings.note("the XCF file has no layers");
    }
    if let Some(entry) = historical::evidence(request, options.historical_order) {
        mapper.report.append(entry)?;
    }
    mapper.report.attach(&mut document, &mut mapper.warnings);
    let initial_view = mapper.view.finish(&document, &image_metadata, selected_path, &mut mapper.warnings);
    ctl.progress(1.0);
    Ok(ImportResult { initial_view, document, warnings: mapper.warnings.finish(), source_read_only: true, preview_only: false })
}
