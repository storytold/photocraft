//! OpenRaster 0.0.6: bounded ZIP/XML reads and editable baseline layer stacks.
//! Source specification: https://www.openraster.org/baseline/
use crate::{ExportOptions, ExportResult, ImportResult, IoError};
use photocraft_color::{BlendMode as B, ColorMode, SampleType};
use photocraft_doc::{Document, Group, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use std::collections::BTreeSet;
use std::io::{Cursor, Read, Write};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};
const BUDGET: u64 = 1 << 30;
const MAX_ENTRIES: usize = 4096;
fn bad(e: impl std::fmt::Display) -> IoError {
    IoError::Unsupported(format!("OpenRaster: {e}"))
}
fn xml(s: &str) -> Result<String, IoError> {
    if s.len() > 1 << 20 || s.chars().any(|c| !matches!(c,'\t'|'\n'|'\r'|' '..='\u{d7ff}'|'\u{e000}'..='\u{fffd}'|'\u{10000}'..='\u{10ffff}')) {
        return Err(bad("layer name contains invalid XML characters or exceeds name budget"));
    }
    Ok(s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;"))
}
fn blend(b: B) -> Option<&'static str> {
    Some(match b {
        B::Normal | B::PassThrough => "svg:src-over",
        B::Multiply => "svg:multiply",
        B::Screen => "svg:screen",
        B::Overlay => "svg:overlay",
        B::Darken => "svg:darken",
        B::Lighten => "svg:lighten",
        B::ColorDodge => "svg:color-dodge",
        B::ColorBurn => "svg:color-burn",
        B::HardLight => "svg:hard-light",
        B::SoftLight => "svg:soft-light",
        B::Difference => "svg:difference",
        B::Exclusion => "svg:exclusion",
        B::Hue => "svg:hue",
        B::Saturation => "svg:saturation",
        B::Color => "svg:color",
        B::Luminosity => "svg:luminosity",
        _ => return None,
    })
}
fn unblend(s: &str) -> Result<B, IoError> {
    for b in B::layer_modes().chain([B::Normal]) {
        if blend(b) == Some(s) {
            return Ok(b);
        }
    }
    Err(bad(format!("unsupported compositing operation {s}")))
}
fn add(zip: &mut ZipWriter<Cursor<Vec<u8>>>, name: &str, bytes: &[u8]) -> Result<(), IoError> {
    let current = zip.get_ref().ok_or_else(|| bad("ZIP writer is closed"))?.get_ref().len() as u64;
    if (bytes.len() as u64).saturating_add(current) > BUDGET {
        return Err(bad("output archive exceeds one GiB budget"));
    }
    // PNGs are compressed already. Stored entries keep memory/CPU costs predictable.
    zip.start_file(name, SimpleFileOptions::default().compression_method(CompressionMethod::Stored).large_file(bytes.len() as u64 > u64::from(u32::MAX)))
        .map_err(bad)?;
    zip.write_all(bytes).map_err(bad)
}
fn baseline(layers: &[Layer], depth: usize) -> bool {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return false;
    }
    layers.iter().all(|l| {
        blend(l.blend).is_some()
            && !l.clipped
            && l.excluded_channels == 0
            && l.blend_if == Default::default()
            && l.advanced == Default::default()
            && match &l.content {
                LayerContent::Adjustment(_) => false,
                LayerContent::Group(g) => {
                    g.artboard.is_none() && l.mask.is_none() && l.vector_mask.is_none() && l.effects.items.is_empty() && baseline(&g.children, depth + 1)
                }
                _ => true,
            }
    })
}
pub(crate) fn export(doc: &Document, opts: &ExportOptions) -> Result<ExportResult, IoError> {
    check_size(doc.size.width, doc.size.height)?;
    let mut pending = vec![(&doc.layers, 0usize)];
    let mut count = 0usize;
    while let Some((layers, depth)) = pending.pop() {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(bad("group depth exceeds limit"));
        }
        count = count.saturating_add(layers.len());
        if count > MAX_ENTRIES - 4 {
            return Err(bad("too many layers"));
        }
        for layer in layers {
            if let LayerContent::Group(group) = &layer.content {
                pending.push((&group.children, depth + 1));
            }
        }
    }
    if !doc.resolution_dpi.is_finite() || doc.resolution_dpi < 1.0 || doc.resolution_dpi > u32::MAX as f32 {
        return Err(bad("invalid document resolution"));
    }
    let mut warnings = Vec::new();
    if doc.resolution_dpi.fract() != 0.0 {
        warnings.push("OpenRaster resolution rounded to an integer ppi".into());
    }
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    add(&mut zip, "mimetype", b"image/openraster")?;
    let mut stack = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<image version=\"0.0.6\" w=\"{}\" h=\"{}\" xres=\"{}\" yres=\"{}\"><stack>",
        doc.size.width,
        doc.size.height,
        doc.resolution_dpi.round(),
        doc.resolution_dpi.round()
    );
    let merged = crate::flat::export_flat(doc, photocraft_codecs::Format::Png, opts)?;
    if baseline(&doc.layers, 0) {
        let mut state = LayerExport { id: 0, warnings: &mut warnings };
        export_layers(doc, &doc.layers, &mut zip, &mut stack, opts, &mut state, 0)?;
    } else {
        warnings.push("OpenRaster cannot represent these clipping, adjustment, artboard or advanced blend settings; output is a flattened composite. Use PhotoCraft or PSD to retain editability.".into());
        warnings.extend(merged.warnings.clone());
        add(&mut zip, "data/layer0.png", &merged.bytes)?;
        stack.push_str("<layer name=\"Composite\" src=\"data/layer0.png\" composite-op=\"svg:src-over\"/>");
    }
    stack.push_str("</stack></image>");
    add(&mut zip, "stack.xml", stack.as_bytes())?;
    add(&mut zip, "mergedimage.png", &merged.bytes)?;
    let thumb = photocraft_compose::thumbnail(doc, 256);
    let thumbnail = photocraft_codecs::Image::from_u8(thumb.width, thumb.height, photocraft_codecs::ChannelLayout::Rgba, thumb.pixels)?;
    add(&mut zip, "Thumbnails/thumbnail.png", &photocraft_codecs::encode(&thumbnail, photocraft_codecs::Format::Png, &opts.encode)?)?;
    warnings.push("OpenRaster keeps baseline raster layers/groups; editable masks, text, shapes, smart objects and effects are baked into PNG pixels, and content outside the canvas is cropped. Use PhotoCraft or PSD for full fidelity.".into());
    warnings.sort();
    warnings.dedup();
    Ok(ExportResult { bytes: zip.finish().map_err(bad)?.into_inner(), warnings })
}
struct LayerExport<'a> {
    id: usize,
    warnings: &'a mut Vec<String>,
}
fn export_layers(
    doc: &Document,
    layers: &[Layer],
    zip: &mut ZipWriter<Cursor<Vec<u8>>>,
    stack: &mut String,
    opts: &ExportOptions,
    state: &mut LayerExport<'_>,
    depth: usize,
) -> Result<(), IoError> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("group depth exceeds document limit"));
    }
    for layer in layers.iter().rev() {
        // ORA is top-first; the document model is bottom-first.
        if !layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity) {
            return Err(bad("invalid layer opacity"));
        }
        let attrs = format!(
            "name=\"{}\" opacity=\"{}\" visibility=\"{}\" composite-op=\"{}\"",
            xml(&layer.name)?,
            layer.opacity,
            if layer.visible { "visible" } else { "hidden" },
            blend(layer.blend).ok_or_else(|| bad("unsupported blend mode"))?
        );
        if let LayerContent::Group(group) = &layer.content {
            stack.push_str(&format!("<stack {attrs} isolation=\"{}\">", if layer.blend == B::PassThrough { "auto" } else { "isolate" }));
            export_layers(doc, &group.children, zip, stack, opts, state, depth + 1)?;
            stack.push_str("</stack>");
        } else {
            let mut isolated = doc.clone();
            let mut copy = layer.clone();
            copy.opacity = 1.0;
            copy.blend = B::Normal;
            copy.visible = true;
            isolated.layers = vec![copy];
            let png = crate::flat::export_flat(&isolated, photocraft_codecs::Format::Png, opts)?;
            state.warnings.extend(png.warnings.into_iter().filter(|w| !w.contains("layer(s) flattened")));
            let path = format!("data/layer{}.png", state.id);
            state.id = state.id.saturating_add(1);
            add(zip, &path, &png.bytes)?;
            stack.push_str(&format!("<layer {attrs} src=\"{path}\" x=\"0\" y=\"0\"/>"));
        }
        if stack.len() > 4 << 20 {
            return Err(bad("layer metadata exceeds XML budget"));
        }
    }
    Ok(())
}
fn check_size(w: u32, h: u32) -> Result<(), IoError> {
    photocraft_codecs::Limits { max_alloc: 256 << 20, max_width: 32768, max_height: 32768, ..Default::default() }.check(
        w,
        h,
        photocraft_codecs::ChannelLayout::Rgba,
        photocraft_codecs::SampleType::U16,
    )?;
    Ok(())
}
pub(crate) fn is_openraster(b: &[u8]) -> bool {
    if !b.starts_with(b"PK\x03\x04") || b.get(8..10) != Some(&[0, 0]) {
        return false;
    }
    let n = b.get(26..28).and_then(|v| <[u8; 2]>::try_from(v).ok()).map(u16::from_le_bytes).unwrap_or(0) as usize;
    let extra = b.get(28..30).and_then(|v| <[u8; 2]>::try_from(v).ok()).map(u16::from_le_bytes).unwrap_or(0) as usize;
    n == 8 && b.get(30..38) == Some(b"mimetype") && b.get(30 + n + extra..30 + n + extra + 16) == Some(b"image/openraster")
}
fn entry(zip: &mut ZipArchive<Cursor<&[u8]>>, name: &str, budget: &mut u64, cap: u64) -> Result<Vec<u8>, IoError> {
    let mut file = zip.by_name(name).map_err(bad)?;
    if !matches!(file.compression(), CompressionMethod::Stored | CompressionMethod::Deflated) {
        return Err(bad("unsupported ZIP compression"));
    }
    if file.size() > cap || file.size() > *budget {
        return Err(bad("ZIP entry exceeds decompression budget"));
    }
    let size = file.size();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(usize::try_from(size).map_err(bad)?).map_err(bad)?;
    file.by_ref().take(cap + 1).read_to_end(&mut bytes).map_err(bad)?;
    if bytes.len() as u64 != size {
        return Err(bad("ZIP entry length disagrees"));
    }
    *budget = budget.saturating_sub(size);
    Ok(bytes)
}
pub(crate) fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    if bytes.len() as u64 > BUDGET {
        return Err(bad("archive exceeds input budget"));
    }
    // Bound central-directory allocation before constructing the ZIP parser; Zip64 reads are
    // refused until they can receive an equally strict entry-count preflight.
    let tail = bytes.get(bytes.len().saturating_sub(65557)..).unwrap_or_default();
    let eocd = tail.windows(4).rposition(|v| v == b"PK\x05\x06").ok_or_else(|| bad("missing ZIP end record"))?;
    let count =
        tail.get(eocd + 10..eocd + 12).and_then(|v| <[u8; 2]>::try_from(v).ok()).map(u16::from_le_bytes).ok_or_else(|| bad("truncated ZIP directory"))?
            as usize;
    if count > MAX_ENTRIES {
        return Err(bad("too many archive entries or unsupported Zip64 archive"));
    }
    let end = tail.get(eocd..).ok_or_else(|| bad("truncated ZIP end record"))?;
    let word = |offset: usize| -> Result<u16, IoError> {
        Ok(u16::from_le_bytes(end.get(offset..offset + 2).ok_or_else(|| bad("short end record"))?.try_into().map_err(bad)?))
    };
    if end.len() != 22 + usize::from(word(20)?) || word(4)? != 0 || word(6)? != 0 || usize::from(word(8)?) != count {
        return Err(bad("unsupported multi-disk or inconsistent ZIP directory"));
    }
    if eocd >= 20 && tail.get(eocd - 20..eocd - 16) == Some(b"PK\x06\x07") {
        return Err(bad("Zip64 input is not supported"));
    }
    let directory_size = u32::from_le_bytes(end.get(12..16).ok_or_else(|| bad("short end record"))?.try_into().map_err(bad)?);
    if directory_size > 4 << 20 {
        return Err(bad("ZIP directory exceeds metadata budget"));
    }
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(bad)?;
    if zip.len() > MAX_ENTRIES {
        return Err(bad("too many archive entries"));
    }
    let mut names = BTreeSet::new();
    for i in 0..zip.len() {
        let file = zip.by_index(i).map_err(bad)?;
        if !names.insert(file.name().to_string()) {
            return Err(bad("duplicate archive paths"));
        }
    }
    let mut budget = BUDGET;
    if entry(&mut zip, "mimetype", &mut budget, 32)? != b"image/openraster" {
        return Err(bad("wrong mimetype"));
    }
    let xml_bytes = entry(&mut zip, "stack.xml", &mut budget, 4 << 20)?;
    let source = std::str::from_utf8(&xml_bytes).map_err(bad)?;
    let tree = roxmltree::Document::parse_with_options(source, roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 50000 }).map_err(bad)?;
    let root = tree.root_element();
    if !root.has_tag_name("image") {
        return Err(bad("missing image element"));
    }
    let w = number(root, "w", None)?;
    let h = number(root, "h", None)?;
    check_size(w, h)?;
    let stack = root.children().find(|n| n.has_tag_name("stack")).ok_or_else(|| bad("missing root stack"))?;
    let mut doc = Document::new(name, Size::new(w, h), ColorMode::Rgb, SampleType::U8);
    let dpi = number::<u32>(root, "xres", Some(72))?;
    let ydpi = number::<u32>(root, "yres", Some(72))?;
    if dpi == 0 || ydpi == 0 {
        return Err(bad("invalid resolution"));
    }
    doc.resolution_dpi = dpi as f32;
    let mut warnings = Vec::new();
    warnings.extend(crate::unequal_resolution_warning(f64::from(dpi), f64::from(ydpi)));
    doc.layers = import_layers(stack, &mut zip, &mut doc.icc_profile, &mut budget, 0)?;
    let mut mode = None;
    let mut depth = SampleType::U8;
    inspect_samples(&doc.layers, &mut mode, &mut depth, 0)?;
    doc.mode = mode.unwrap_or(ColorMode::Rgb);
    doc.depth = depth;
    let format = doc.pixel_format();
    harmonize(&mut doc.layers, format, 0)?;
    if doc.layers.is_empty() {
        return Err(bad("empty layer stack"));
    }
    Ok(ImportResult { document: doc, warnings, source_read_only: false, preview_only: false })
}
fn number<T: std::str::FromStr>(n: roxmltree::Node<'_, '_>, key: &str, default: Option<T>) -> Result<T, IoError> {
    if let Some(v) = n.attribute(key) { v.parse().map_err(|_| bad(format!("invalid {key}"))) } else { default.ok_or_else(|| bad(format!("missing {key}"))) }
}
fn import_layers(
    node: roxmltree::Node<'_, '_>,
    zip: &mut ZipArchive<Cursor<&[u8]>>,
    profile: &mut Option<std::sync::Arc<Vec<u8>>>,
    budget: &mut u64,
    depth: usize,
) -> Result<Vec<Layer>, IoError> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("group nesting exceeds document limit"));
    }
    let mut layers = Vec::new();
    for n in node.children().filter(|n| n.is_element()) {
        let name = n.attribute("name").unwrap_or("Layer");
        let mut layer = if n.has_tag_name("stack") {
            let children = import_layers(n, zip, profile, budget, depth + 1)?;
            Layer::new(name, LayerContent::Group(Group { children, expanded: true, artboard: None }))
        } else if n.has_tag_name("layer") {
            let path = n.attribute("src").ok_or_else(|| bad("missing layer path"))?;
            if !path.starts_with("data/") || path.split('/').any(|part| matches!(part, "" | "." | "..")) || path.contains('\\') {
                return Err(bad("unsafe layer path"));
            }
            let png = entry(zip, path, budget, 256 << 20)?;
            let limits = photocraft_codecs::Limits { max_alloc: (*budget).min(256 << 20), max_width: 32768, max_height: 32768, ..Default::default() };
            let image =
                photocraft_codecs::decode_as_with(photocraft_codecs::Format::Png, &png, &photocraft_codecs::DecodeOptions { limits, ..Default::default() })?;
            *budget = budget.checked_sub(u64::from(image.width()) * u64::from(image.height()) * 8).ok_or_else(|| bad("decoded layers exceed memory budget"))?;
            if let Some(icc) = &image.icc {
                if let Some(existing) = profile {
                    if existing.as_ref() != icc {
                        return Err(bad("layer ICC profiles differ; normalize them before export"));
                    }
                } else {
                    *profile = Some(std::sync::Arc::new(icc.clone()));
                }
            }
            let image = image.convert(image.layout().with_alpha(), image.sample_type());
            let mut imported = crate::flat::image_to_document(name, &image)?;
            let mut layer = imported.document.layers.pop().ok_or_else(|| bad("missing raster layer"))?;
            let x = number::<i32>(n, "x", Some(0))?;
            let y = number::<i32>(n, "y", Some(0))?;
            let right = x.checked_add(image.width() as i32).ok_or_else(|| bad("layer x extent overflow"))?;
            let bottom = y.checked_add(image.height() as i32).ok_or_else(|| bad("layer y extent overflow"))?;
            if x.unsigned_abs() > 1 << 20 || y.unsigned_abs() > 1 << 20 {
                return Err(bad("layer offsets exceed coordinate budget"));
            }
            if let LayerContent::Raster(surface) = &mut layer.content {
                let mut positioned = photocraft_raster::Surface::new(surface.format());
                positioned.write_interleaved(Rect::new(x, y, right, bottom), image.data());
                *surface = positioned;
            }
            layer.name = name.to_string();
            layer
        } else {
            return Err(bad("unsupported stack element"));
        };
        layer.opacity = number(n, "opacity", Some(1.0f32))?;
        if !layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity) {
            return Err(bad("invalid layer opacity"));
        }
        layer.visible = match n.attribute("visibility").unwrap_or("visible") {
            "visible" => true,
            "hidden" => false,
            _ => return Err(bad("invalid visibility")),
        };
        layer.blend = unblend(n.attribute("composite-op").unwrap_or("svg:src-over"))?;
        if n.has_tag_name("stack") && n.attribute("isolation") == Some("auto") {
            layer.blend = B::PassThrough;
        }
        layers.push(layer);
    }
    layers.reverse();
    Ok(layers)
}

fn inspect_samples(layers: &[Layer], mode: &mut Option<ColorMode>, depth: &mut SampleType, level: usize) -> Result<(), IoError> {
    if level > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("group depth exceeds limit"));
    }
    for layer in layers {
        match &layer.content {
            LayerContent::Raster(surface) => {
                if let Some(existing) = mode {
                    if *existing != surface.format().mode {
                        return Err(bad("mixed grayscale/RGB layer color models; convert layers to a common model first"));
                    }
                } else {
                    *mode = Some(surface.format().mode);
                }
                if surface.format().sample == SampleType::U16 {
                    *depth = SampleType::U16;
                }
            }
            LayerContent::Group(group) => inspect_samples(&group.children, mode, depth, level + 1)?,
            _ => {}
        }
    }
    Ok(())
}
fn harmonize(layers: &mut [Layer], format: photocraft_color::PixelFormat, level: usize) -> Result<(), IoError> {
    if level > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("group depth exceeds limit"));
    }
    for layer in layers {
        match &mut layer.content {
            LayerContent::Raster(surface) => {
                if surface.format() != format {
                    *surface = surface.convert(format);
                }
            }
            LayerContent::Group(group) => harmonize(&mut group.children, format, level + 1)?,
            _ => {}
        }
    }
    Ok(())
}
