//! Multi-part EXR and channel groups as layers (a Maya/Arnold render writes one part per
//! AOV, or a single part with prefixed channel sets like `diffuse.R`): the entries of a
//! file listed for a chooser ([`entries`]), and the chosen ones imported as the layers of
//! one document ([`import`]), with an optional additive beauty precomp from the AOVs.

use craft_cms::Builtin;
use photocraft_codecs::{self as codecs, ChannelLayout, DecodeOptions, Image, SampleType};
use photocraft_color::{ColorMode, SampleType as DocSample};
use photocraft_doc::{Document, Layer, LayerContent, Size};
use photocraft_raster::Surface;

use crate::{ImportResult, IoError, flat::image_surface};

/// One selectable line of the chooser: a whole part, a prefixed channel group inside a
/// part, or a Cryptomatte layer (listed for information; its samples are not image layers).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    /// The name as shown and selected: the part name, the channel-group prefix, or the
    /// Cryptomatte layer name.
    pub name: String,
    /// The part's `view` attribute (stereo files), when it has one.
    pub view: Option<String>,
    pub width: u32,
    pub height: u32,
    /// Image channels that would import (4 for RGBA, 3 for RGB); Cryptomatte: stream count.
    pub channels: usize,
    /// Cryptomatte only: objects in the parsed manifest.
    pub objects: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Part,
    Group,
    Cryptomatte,
}

/// One channel group: where it lives and which exact channels make up its RGB(A).
#[derive(Clone, Debug)]
struct Group {
    /// The prefix before the last dot (`diffuse` of `diffuse.R`).
    prefix: String,
    part: usize,
    channels: Vec<String>,
    alpha: Option<String>,
}

/// The name an AOV normalizes to for the precomp check (`Direct_Diffuse` and
/// `directDiffuse` are the same AOV).
fn normalize(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// The additive AOVs a beauty precomp is built from: the light-path set Arnold writes.
const PRECOMP_REQUIRED: [&str; 5] = ["directdiffuse", "indirectdiffuse", "directspecular", "indirectspecular", "emission"];
const PRECOMP_OPTIONAL: [&str; 3] = ["transmission", "sss", "volume"];

/// The base channel name (`R` of `diffuse.R`, `R` of `R`).
fn channel_base(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// The prefixed RGB(A) channel sets of every flat part (`diffuse.R/G/B` make the group
/// `diffuse`). Unprefixed channels are no group, and a prefix ending in digits is a
/// Cryptomatte stream (`crypto_asset00`), not a group. Each prefix is listed once.
fn groups(bytes: &[u8]) -> Result<Vec<Group>, IoError> {
    let mut out: Vec<Group> = Vec::new();
    for part in codecs::exr_info(bytes, &Default::default())? {
        if part.deep {
            continue;
        }
        let mut found: Vec<Group> = Vec::new();
        for c in &part.channels {
            let Some((prefix, base)) = channel_base_split(&c.name) else { continue };
            if prefix.chars().last().is_some_and(|ch| ch.is_ascii_digit()) {
                continue;
            }
            if found.iter_mut().all(|g| g.prefix != prefix) {
                found.push(Group { prefix: prefix.to_string(), part: part.index, channels: Vec::new(), alpha: None });
            }
            let Some(group) = found.iter_mut().find(|g| g.prefix == prefix) else { continue };
            match base {
                "R" | "G" | "B" => group.channels.push(c.name.clone()),
                "A" => group.alpha = Some(c.name.clone()),
                _ => {}
            }
        }
        found.retain(|g| g.channels.len() == 3);
        out.extend(found);
    }
    Ok(out)
}

/// `(prefix, base)` when the channel name carries a dot (`diffuse.R`); `None` unprefixed.
fn channel_base_split(name: &str) -> Option<(&str, &str)> {
    let (prefix, base) = name.rsplit_once('.')?;
    (!prefix.is_empty()).then_some((prefix, base))
}

/// The name a part is listed, selected and imported as: its `name` attribute, or
/// `part <index>` when it has none (a single-part render usually carries no name).
fn part_display_name(part: &codecs::ExrPartInfo) -> String {
    part.name.clone().unwrap_or_else(|| format!("part {}", part.index))
}

/// Lists the entries of a multi-part or channel-grouped EXR for a chooser: flat parts in
/// file order, then the channel groups, then the Cryptomatte layers as information. A file
/// whose parts are all deep is refused (the deep decoder composites those).
pub fn entries(bytes: &[u8]) -> Result<Vec<Entry>, IoError> {
    let info = codecs::exr_info(bytes, &Default::default())?;
    if info.iter().all(|p| p.deep) {
        return Err(IoError::Unsupported("this EXR holds only deep parts; it opens through the deep decoder".into()));
    }
    let mut out = Vec::new();
    for part in &info {
        if part.deep {
            continue;
        }
        out.push(Entry {
            kind: EntryKind::Part,
            name: part_display_name(part),
            view: part.view.clone(),
            width: part.width,
            height: part.height,
            channels: part.channels.iter().filter(|c| ["R", "G", "B", "A", "Y"].contains(&channel_base(&c.name))).count(),
            objects: None,
        });
    }
    for group in groups(bytes)? {
        out.push(Entry {
            kind: EntryKind::Group,
            name: group.prefix.clone(),
            view: None,
            width: 0,
            height: 0,
            channels: 3 + usize::from(group.alpha.is_some()),
            objects: None,
        });
    }
    for layer in codecs::cryptomatte_layers(bytes, &Default::default())? {
        out.push(Entry {
            kind: EntryKind::Cryptomatte,
            name: layer.name.clone(),
            view: None,
            width: 0,
            height: 0,
            channels: layer.channels.len(),
            objects: Some(layer.manifest.len()),
        });
    }
    Ok(out)
}

/// What [`import`] turns into layers.
pub struct Selection {
    /// Part names to import as whole layers.
    pub parts: Vec<String>,
    /// Channel-group prefixes to import as layers.
    pub groups: Vec<String>,
    /// `"both"`, `"left"` or `"right"`: which stereo views of the parts to consider.
    pub view: String,
    /// Add a `beauty (precomp)` layer summed from the file's AOV groups (see
    /// [`PRECOMP_REQUIRED`]); fails naming the missing AOVs when the set is incomplete.
    pub precomp: bool,
}

/// An image as an importable layer: F16 becomes F32 (lossless, warned once by the caller), the
/// layout is RGBA (a gray part or group fills R, G and B alike, as every EXR opens as RGB,
/// #14069), the surface is built a band of rows at a time.
fn layer_surface(img: &Image, max: &mut (u32, u32)) -> Result<Surface, IoError> {
    max.0 = max.0.max(img.width());
    max.1 = max.1.max(img.height());
    let converted = img.convert(ChannelLayout::Rgba, SampleType::F32);
    image_surface(&converted, ChannelLayout::Rgba, SampleType::F32)
}

/// Imports the chosen entries as the layers of one document: parts whole, groups from their
/// own channels, F16 kept losslessly as F32, the document RGB and the linear-light EXR values
/// tagged linear sRGB.
/// `precomp` adds the additive sum of the file's AOV groups as a top layer and fails,
/// naming the missing AOVs, when the light-path set is incomplete.
pub fn import(name: &str, bytes: &[u8], sel: &Selection) -> Result<ImportResult, IoError> {
    let opts = DecodeOptions::default();
    let info = codecs::exr_info(bytes, &opts.limits)?;
    if info.is_empty() {
        return Err(IoError::Unsupported("the EXR has no parts".into()));
    }
    let all_groups = groups(bytes)?;
    let view_passes = |view: &Option<String>| view.as_ref().is_none_or(|v| sel.view.eq_ignore_ascii_case("both") || v.eq_ignore_ascii_case(&sel.view));
    // A named part wins over an unnamed one listed under the same fallback name ("part 0").
    let part_by_name = |want: &str| {
        let flat = || info.iter().filter(|p| !p.deep);
        flat()
            .find(|p| p.name.as_deref() == Some(want))
            .or_else(|| flat().find(|p| p.name.is_none() && part_display_name(p) == want))
            .ok_or_else(|| IoError::Unsupported(format!("no flat part named {want}")))
    };
    let mut chosen_parts: Vec<&codecs::ExrPartInfo> = Vec::new();
    for want in &sel.parts {
        let p = part_by_name(want)?;
        if !view_passes(&p.view) {
            return Err(IoError::Unsupported(format!("the part {want} is of the {:?} view, the filter is {}", p.view, sel.view)));
        }
        if !chosen_parts.iter().any(|c| c.index == p.index) {
            chosen_parts.push(p);
        }
    }
    let mut chosen_groups: Vec<&Group> = Vec::new();
    for want in &sel.groups {
        let g = all_groups.iter().find(|g| g.prefix == *want).ok_or_else(|| IoError::Unsupported(format!("no channel group named {want}")))?;
        let part_view = info.get(g.part).and_then(|p| p.view.clone());
        if !view_passes(&part_view) {
            return Err(IoError::Unsupported(format!("the group {} is of the {:?} view, the filter is {}", want, part_view, sel.view)));
        }
        if !chosen_groups.iter().any(|c| c.prefix == g.prefix) {
            chosen_groups.push(g);
        }
    }
    if chosen_parts.is_empty() && chosen_groups.is_empty() && !sel.precomp {
        return Err(IoError::Unsupported("no parts or groups selected".into()));
    }

    // Each chosen part (deduplicated above) decodes once; groups come from their part's named
    // channels.
    let mut layers: Vec<Layer> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut max = (1u32, 1u32);
    let mut f16_seen = false;

    for part in &chosen_parts {
        let img = codecs::decode_exr_part(bytes, part.index, &opts)?;
        if img.sample_type() == SampleType::F16 {
            f16_seen = true;
        }
        let surface = layer_surface(&img, &mut max)?;
        layers.push(Layer::new(part_display_name(part), LayerContent::Raster(surface)));
    }
    for group in &chosen_groups {
        let mut names = group.channels.clone();
        names.extend(group.alpha.clone());
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let img = codecs::decode_exr_channels(bytes, group.part, &refs, &opts)?;
        if img.sample_type() == SampleType::F16 {
            f16_seen = true;
        }
        let surface = layer_surface(&img, &mut max)?;
        layers.push(Layer::new(group.prefix.clone(), LayerContent::Raster(surface)));
    }

    if sel.precomp {
        let layer = precomp_layer(bytes, &all_groups, &opts, &mut max, &mut warnings)?;
        layers.push(layer);
    }

    if layers.is_empty() {
        return Err(IoError::Unsupported("nothing imported".into()));
    }
    if f16_seen {
        warnings.push("16-bit float samples are stored as 32-bit float".to_string());
    }
    let mut doc = Document::new(name, Size::new(max.0, max.1), ColorMode::Rgb, DocSample::F32);
    doc.layers = layers.into();
    // OpenEXR holds linear, scene-referred values: tag them linear sRGB, as File › Open does.
    doc.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
    Ok(ImportResult { document: doc, warnings, source_read_only: false, preview_only: false })
}

/// The additive beauty from the file's AOV groups: premultiplied RGB summed over the
/// light-path set, alpha the maximum coverage, straightened back. Fails naming the missing
/// AOVs when the required set is incomplete; AOVs beyond the set are ignored.
fn precomp_layer(bytes: &[u8], all_groups: &[Group], opts: &DecodeOptions, max: &mut (u32, u32), warnings: &mut Vec<String>) -> Result<Layer, IoError> {
    let have: Vec<String> = all_groups.iter().map(|g| normalize(&g.prefix)).collect();
    let missing: Vec<&str> = PRECOMP_REQUIRED.iter().copied().filter(|r| !have.iter().any(|h| h == *r)).collect();
    if !missing.is_empty() {
        return Err(IoError::Unsupported(format!("the AOV set is incomplete for a beauty precomp; missing: {}", missing.join(", "))));
    }
    let mut sum_rgb: Vec<f32> = Vec::new();
    let mut alpha: Vec<f32> = Vec::new();
    let mut size = (0u32, 0u32);
    for group in all_groups {
        let n = normalize(&group.prefix);
        if !PRECOMP_REQUIRED.contains(&n.as_str()) && !PRECOMP_OPTIONAL.contains(&n.as_str()) {
            continue;
        }
        let mut names = group.channels.clone();
        names.extend(group.alpha.clone());
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let img = codecs::decode_exr_channels(bytes, group.part, &refs, opts)?;
        let rgba = img.convert(ChannelLayout::Rgba, SampleType::F32);
        let (w, h) = rgba.dimensions();
        if size == (0, 0) {
            size = (w, h);
            sum_rgb = vec![0.0; w as usize * h as usize * 3];
            alpha = vec![0.0; w as usize * h as usize];
        } else if size != (w, h) {
            return Err(IoError::Unsupported("the AOV groups differ in size; a beauty precomp needs one common extent".into()));
        }
        let px = rgba.to_f32_samples().unwrap_or_default();
        for p in 0..(w as usize * h as usize) {
            // AOV channels are premultiplied by their coverage, as renderers write them:
            // the sum adds the stored RGB as-is, and alpha is the maximum coverage.
            let (r, g, b, a) = (px[p * 4], px[p * 4 + 1], px[p * 4 + 2], px[p * 4 + 3]);
            sum_rgb[p * 3] += r;
            sum_rgb[p * 3 + 1] += g;
            sum_rgb[p * 3 + 2] += b;
            alpha[p] = alpha[p].max(a);
        }
    }
    let (w, h) = size;
    if w == 0 || h == 0 {
        return Err(IoError::Unsupported("the AOV groups are empty".into()));
    }
    let mut out = vec![0f32; w as usize * h as usize * 4];
    for p in 0..(w as usize * h as usize) {
        let a = alpha[p];
        let (r, g, b) = if a > 0.0 { (sum_rgb[p * 3] / a, sum_rgb[p * 3 + 1] / a, sum_rgb[p * 3 + 2] / a) } else { (0.0, 0.0, 0.0) };
        out[p * 4] = r;
        out[p * 4 + 1] = g;
        out[p * 4 + 2] = b;
        out[p * 4 + 3] = a;
    }
    let img = Image::from_f32(w, h, ChannelLayout::Rgba, &out)?;
    max.0 = max.0.max(w);
    max.1 = max.1.max(h);
    let surface = layer_surface(&img, max)?;
    warnings.push("beauty (precomp) is the additive sum of the file's AOVs; alpha is the maximum coverage".to_string());
    Ok(Layer::new("beauty (precomp)", LayerContent::Raster(surface)))
}

// File › Open: every part, view and channel group as layers in folders (#14062).

/// The most bytes [`opens_layered`] reads from the start of a file to see its headers: the
/// metadata cap the band decoder applies too.
#[cfg(any(unix, windows))]
pub(crate) const HEADER_PREFIX_BYTES: u64 = 8 << 20;

/// One layer of the open, from channels of one decoded part (indices into its channel list).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Leaf {
    /// `R`, `G` and `B` (any of them; a missing one reads 0) with the set's `A`: a layer `RGB`.
    Rgb { rgb: [Option<usize>; 3], alpha: Option<usize> },
    /// One channel as grey (R = G = B = its value): `Y` (`YA` with the set's `A`) or a solo
    /// channel named after itself (`Z`, `U`, a lone `A`).
    Gray { name: String, value: usize, alpha: Option<usize> },
}

/// A node of the planned layer tree, listed top of the Layers panel first.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Folder { name: String, children: Vec<Node> },
    Leaf { part: usize, leaf: Leaf },
}

/// What File › Open makes of a file: the tree, and the canvas every part's data window sits in.
#[derive(Debug)]
struct Plan {
    nodes: Vec<Node>,
    /// The union of the parts' data windows: `(x, y, width, height)`.
    canvas: (i64, i64, u32, u32),
}

/// Whether File › Open builds the measured layer tree for these parts: they are all flat,
/// single-resolution scan lines of full-resolution float channels (32-bit, as measured, or
/// 16-bit: only the sample type differs, the tree doesn't depend on it), without chroma or
/// Cryptomatte data. Tiled, uint, chroma, Cryptomatte and deep files keep today's open (the
/// reference wasn't measured on them).
fn tree_eligible(parts: &[craft_exr::decode::PartInfo]) -> bool {
    !parts.is_empty()
        && parts.iter().all(|p| {
            !p.deep
                && !p.tiled
                && !p.text_attributes.iter().any(|(k, _)| k.starts_with("cryptomatte/"))
                && p.channels.iter().all(|c| {
                    matches!(c.sample, craft_exr::decode::SampleType::F32 | craft_exr::decode::SampleType::F16)
                        && c.sampling == (1, 1)
                        && !matches!(craft_exr::channels::base_channel_name(&c.name), "RY" | "BY")
                })
        })
}

/// The layers of one channel set (a part, or one view of a part): its bare channels at this
/// level, then one folder per channel group (`diffuse.R` → `diffuse`), in the file's channel
/// order. `channels` are `(index into the part's channels, name with any view taken out)`.
fn set_nodes(part: usize, channels: &[(usize, String)]) -> Vec<Node> {
    let mut prefixes: Vec<&str> = vec![""];
    for (_, name) in channels {
        let prefix = craft_exr::channels::split_channel(name).0;
        if !prefixes.contains(&prefix) {
            prefixes.push(prefix);
        }
    }
    let mut out = Vec::new();
    for prefix in prefixes {
        let members: Vec<(usize, &str)> =
            channels.iter().map(|(i, n)| (*i, craft_exr::channels::split_channel(n))).filter(|(_, (p, _))| *p == prefix).map(|(i, (_, b))| (i, b)).collect();
        let find = |base: &str| members.iter().find(|(_, b)| *b == base).map(|(i, _)| *i);
        let (rgb, y, a) = ([find("R"), find("G"), find("B")], find("Y"), find("A"));
        let has_rgb = rgb.iter().any(Option::is_some);
        // Each layer sits where its first channel is in the file's (name-sorted) channel order.
        let mut leaves: Vec<(usize, Leaf)> = Vec::new();
        if let Some(first) = rgb.iter().flatten().min() {
            leaves.push((*first, Leaf::Rgb { rgb, alpha: a }));
        }
        if let Some(y) = y {
            let alpha = if has_rgb { None } else { a };
            leaves.push((y, Leaf::Gray { name: if alpha.is_some() { "YA" } else { "Y" }.into(), value: y, alpha }));
        }
        for (i, base) in &members {
            let taken = matches!(*base, "R" | "G" | "B" | "Y") || (*base == "A" && (has_rgb || y.is_some()));
            if !taken {
                leaves.push((*i, Leaf::Gray { name: (*base).to_string(), value: *i, alpha: None }));
            }
        }
        leaves.sort_by_key(|(at, _)| *at);
        let nodes: Vec<Node> = leaves.into_iter().map(|(_, leaf)| Node::Leaf { part, leaf }).collect();
        if nodes.is_empty() {
            continue;
        }
        if prefix.is_empty() {
            out.extend(nodes);
        } else {
            out.push(Node::Folder { name: prefix.to_string(), children: nodes });
        }
    }
    out
}

/// One view's channels of a part: the view (`None` outside any view) and its `(index into the
/// part's channels, name with the view taken out)`.
type ViewSet = (Option<String>, Vec<(usize, String)>);

/// A part's channels by view, in view order: a single-part stereo file lists its views in
/// `multiView`, the default view first. As the OpenEXR multi-view convention names them, a channel
/// without a period is the default view's, `right.R` and `diffuse.right.R` are the `right` view's
/// `R` and `diffuse.R`, and any other dotted channel (`background.data`) is in no view: those
/// come last, outside the view folders. Any other part is one set, of its `view` attribute's
/// view when it has one.
fn views_of(part: &craft_exr::decode::PartInfo) -> Vec<ViewSet> {
    let all = || part.channels.iter().enumerate().map(|(i, c)| (i, c.name.clone())).collect::<Vec<_>>();
    let Some(default) = part.multi_view.first() else { return vec![(part.view.clone(), all())] };
    let mut out: Vec<ViewSet> = part.multi_view.iter().map(|v| (Some(v.clone()), Vec::new())).collect();
    let mut no_view = Vec::new();
    for (i, name) in all() {
        let pieces: Vec<&str> = name.split('.').collect();
        // The view name is the piece before the channel's own name.
        let named = pieces.len().checked_sub(2).and_then(|at| pieces.get(at).filter(|v| part.multi_view.iter().any(|m| m == *v)).map(|v| (at, *v)));
        let (view, stripped) = match named {
            Some((at, view)) => (view, pieces.iter().enumerate().filter(|(k, _)| *k != at).map(|(_, p)| *p).collect::<Vec<_>>().join(".")),
            None if pieces.len() > 1 => {
                no_view.push((i, name.clone()));
                continue;
            }
            None => (default.as_str(), name.clone()),
        };
        if let Some((_, set)) = out.iter_mut().find(|(v, _)| v.as_deref() == Some(view)) {
            set.push((i, stripped));
        }
    }
    out.push((None, no_view));
    out.retain(|(_, set)| !set.is_empty());
    out
}

/// The leaves of `nodes` with their parts, top of the panel first.
fn leaves<'a>(nodes: &'a [Node], out: &mut Vec<(usize, &'a Leaf)>) {
    for node in nodes {
        match node {
            Node::Folder { children, .. } => leaves(children, out),
            Node::Leaf { part, leaf } => out.push((*part, leaf)),
        }
    }
}

/// The tree File › Open makes of these parts, or `None` when the file keeps today's open: it is
/// not [`tree_eligible`], it gives a single layer (a Background, or a layer named `RGB` / `YA`),
/// or it has no `RGB` layer to show.
///
/// A part becomes a folder named after it when the file has more than one; a view becomes a
/// top-level folder holding its parts (or, in a single-part stereo file, its channels), in the
/// order the views first appear.
fn plan(parts: &[craft_exr::decode::PartInfo]) -> Option<Plan> {
    if !tree_eligible(parts) {
        return None;
    }
    let mut top: Vec<Node> = Vec::new();
    // Where each view's folder is in `top`.
    let mut view_at: Vec<(String, usize)> = Vec::new();
    for (k, part) in parts.iter().enumerate() {
        for (view, channels) in views_of(part) {
            let content = set_nodes(k, &channels);
            let nodes = if parts.len() > 1 {
                let name = part.name.clone().unwrap_or_else(|| format!("part {}", part.index));
                vec![Node::Folder { name, children: content }]
            } else {
                content
            };
            match view {
                Some(view) => {
                    let at = match view_at.iter().find(|(v, _)| *v == view) {
                        Some((_, at)) => *at,
                        None => {
                            top.push(Node::Folder { name: view.clone(), children: Vec::new() });
                            view_at.push((view, top.len() - 1));
                            top.len() - 1
                        }
                    };
                    if let Some(Node::Folder { children, .. }) = top.get_mut(at) {
                        children.extend(nodes);
                    }
                }
                None => top.extend(nodes),
            }
        }
    }
    let mut all = Vec::new();
    leaves(&top, &mut all);
    if all.len() < 2 || !all.iter().any(|(_, l)| matches!(l, Leaf::Rgb { .. })) {
        return None;
    }
    let x0 = parts.iter().map(|p| i64::from(p.data_window.x)).min()?;
    let y0 = parts.iter().map(|p| i64::from(p.data_window.y)).min()?;
    let x1 = parts.iter().map(|p| i64::from(p.data_window.x) + i64::from(p.data_window.width)).max()?;
    let y1 = parts.iter().map(|p| i64::from(p.data_window.y) + i64::from(p.data_window.height)).max()?;
    let (width, height) = (u32::try_from(x1 - x0).ok()?, u32::try_from(y1 - y0).ok()?);
    Some(Plan { nodes: top, canvas: (x0, y0, width, height) })
}

/// Tiles the decoded image would add must fit the RAM tile budget when no scratch disk can
/// take cold tiles. Shared with the native band decoder (`exr_native`), which only the desktop
/// builds have; the layer tree is built on every target, the browser's included.
pub(crate) fn ram_tile_capacity(resident: u64, budget: u64, growth: u64) -> Result<(), IoError> {
    if resident.checked_add(growth).is_none_or(|required| required > budget) {
        return Err(IoError::OutOfMemory("decoded image tiles exceed the available RAM budget; enable a scratch disk or close other documents".into()));
    }
    Ok(())
}

/// The header limits File › Open's decoder applies, as `craft-exr` limits.
fn exr_limits(opts: &crate::ImportOptions) -> craft_exr::decode::Limits {
    let codec = crate::flat::decode_limits(opts);
    let mut limits = craft_exr::decode::Limits::default();
    limits.max_width = codec.max_width;
    limits.max_height = codec.max_height;
    limits.max_pixels = codec.max_pixels;
    limits.max_bytes = codec.max_alloc;
    limits
}

/// A `craft-exr` failure as the codec error File › Open reports for an EXR.
fn exr_error(e: craft_exr::decode::Error) -> IoError {
    let format = codecs::Format::OpenExr;
    IoError::Codec(match e {
        craft_exr::decode::Error::LimitExceeded(m) => codecs::CodecError::LimitExceeded(m),
        craft_exr::decode::Error::Unsupported(reason) => codecs::CodecError::Unsupported { format, reason },
        e => codecs::CodecError::Malformed { format, message: e.to_string() },
    })
}

/// Whether File › Open makes a layer tree of the EXR whose first bytes are `prefix` (at most
/// [`HEADER_PREFIX_BYTES`]): the path open then reads the file whole instead of streaming one
/// layer from it. Headers it cannot read are left to the decoders, which report them.
#[cfg(any(unix, windows))]
pub(crate) fn opens_layered(prefix: &[u8], opts: &crate::ImportOptions) -> bool {
    craft_exr::decode::parts(prefix, &exr_limits(opts)).ok().as_deref().and_then(plan).is_some()
}

/// The samples of a decoded channel (32- or 16-bit float; read as `f32` per sample, no copy).
fn plane_of(part: &craft_exr::decode::Part, index: usize) -> Result<&craft_exr::decode::Samples, IoError> {
    part.channels.get(index).map(|c| &c.samples).ok_or_else(|| IoError::Unsupported("an EXR channel was not decoded".into()))
}

/// One leaf's RGBA 32-bit float layer, its pixels at `rect` (the part's data window on the
/// canvas). It is named `RGB`, `Y`, `YA` or after its channel, and starts visible.
fn leaf_layer(part: &craft_exr::decode::Part, leaf: &Leaf, rect: photocraft_geom::Rect, ctl: &photocraft_raster::Interrupt) -> Result<Layer, IoError> {
    let plane = |i: Option<usize>| i.map(|i| plane_of(part, i)).transpose();
    let (name, color, alpha) = match leaf {
        Leaf::Rgb { rgb, alpha } => ("RGB".to_string(), [plane(rgb[0])?, plane(rgb[1])?, plane(rgb[2])?], plane(*alpha)?),
        Leaf::Gray { name, value, alpha } => {
            let v = plane_of(part, *value)?;
            (name.clone(), [Some(v); 3], plane(*alpha)?)
        }
    };
    let width = rect.width() as usize;
    let pixels = width * rect.height() as usize;
    if color.iter().flatten().chain(alpha.iter()).any(|p| p.len() != pixels) {
        return Err(IoError::Unsupported("an EXR channel does not match its part's size".into()));
    }
    let mut surface = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Rgb, DocSample::F32, true));
    let (rx, ry) = (rect.x0, rect.y0);
    surface
        .try_write_rows_with_interrupt(rect, ctl, |y, x0, dst| {
            let start = (y - ry) as usize * width + (x0 - rx) as usize;
            let sample = |p: Option<&craft_exr::decode::Samples>, at: usize, missing: f32| p.map_or(missing, |p| p.get_f32(at));
            for (j, px) in dst.as_chunks_mut::<16>().0.iter_mut().enumerate() {
                let at = start + j;
                let values = [sample(color[0], at, 0.0), sample(color[1], at, 0.0), sample(color[2], at, 0.0), sample(alpha, at, 1.0)];
                for (out, v) in px.as_chunks_mut::<4>().0.iter_mut().zip(values) {
                    *out = v.to_ne_bytes();
                }
            }
        })
        .map_err(|e| match e {
            photocraft_raster::RowWriteError::Cancelled(_) => IoError::Cancelled,
            photocraft_raster::RowWriteError::Allocation(e) => IoError::OutOfMemory(format!("EXR layer {name}: {e}")),
        })?;
    if alpha.is_some() {
        surface.prune();
    }
    Ok(Layer::new(name, LayerContent::Raster(surface)))
}

/// Turns planned nodes (top first) into document layers (bottom first), taking each leaf's
/// built layer in panel order.
fn assemble(nodes: Vec<Node>, built: &mut std::vec::IntoIter<Layer>) -> Result<Vec<Layer>, IoError> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        out.push(match node {
            Node::Folder { name, children } => Layer::group(name, assemble(children, built)?),
            Node::Leaf { .. } => built.next().ok_or_else(|| IoError::Unsupported("an EXR layer was not built".into()))?,
        });
    }
    out.reverse();
    Ok(out)
}

/// File › Open of an EXR as the reference application opens it: every part, view and channel
/// group of the file as layers in folders, in one RGB 32-bit document (see [`plan`] for the
/// tree, and for the files that keep today's open, which give `Ok(None)`).
///
/// `R`, `G`, `B` become a layer `RGB` with the set's `A` as its transparency; `Y` a grey layer
/// `Y` / `YA`; every other channel a grey layer named after it, values above 1 kept. Only the
/// first `RGB` layer in panel order is visible. Folders are pass-through, layers normal at 100 %.
/// Each part is decoded once, admitted against the working-memory budget first, and its layers
/// are built before the next part is read.
pub(crate) fn open_layered(name: &str, bytes: &[u8], opts: &crate::ImportOptions, ctl: &photocraft_raster::Interrupt) -> Result<Option<ImportResult>, IoError> {
    if codecs::detect(bytes) != Some(codecs::Format::OpenExr) {
        return Ok(None);
    }
    let limits = exr_limits(opts);
    let Ok(parts) = craft_exr::decode::parts(bytes, &limits) else { return Ok(None) };
    let Some(plan) = plan(&parts) else { return Ok(None) };
    let (cx, cy, cw, ch) = plan.canvas;
    // The parts' own windows are within the limits; the canvas holding them all must be too.
    let canvas_pixels = u64::from(cw).checked_mul(u64::from(ch));
    if cw > limits.max_width || ch > limits.max_height || canvas_pixels.is_none_or(|n| n > limits.max_pixels) {
        return Err(IoError::Codec(codecs::CodecError::LimitExceeded(format!(
            "the EXR's parts span a {cw}x{ch} canvas, over the {} pixel limit",
            limits.max_pixels
        ))));
    }
    let mut order = Vec::new();
    leaves(&plan.nodes, &mut order);
    // Each part's data window, placed on the canvas.
    let place = |at: i32, origin: i64| i32::try_from(i64::from(at) - origin).map_err(|_| IoError::Unsupported("EXR data window out of range".into()));
    let rects = parts
        .iter()
        .map(|p| Ok(photocraft_geom::Rect::from_xywh(place(p.data_window.x, cx)?, place(p.data_window.y, cy)?, p.data_window.width, p.data_window.height)))
        .collect::<Result<Vec<_>, IoError>>()?;
    // A layer stores only the tiles its part's window touches (RGBA 32-bit: 16 bytes a pixel).
    let tile_bytes = |leaves: usize, rect: photocraft_geom::Rect| {
        let side = photocraft_geom::TILE_SIZE as u64;
        rect.tile_count().saturating_mul(side * side * 16).saturating_mul(leaves as u64)
    };
    // The layers' tiles must fit the RAM tile budget when no scratch disk can take cold ones.
    if !photocraft_raster::spill::enabled() {
        let growth = order.iter().fold(0u64, |sum, (k, _)| sum.saturating_add(rects.get(*k).map_or(0, |r| tile_bytes(1, *r))));
        ram_tile_capacity(photocraft_raster::spill::stats().resident_bytes as u64, photocraft_raster::memory::refresh().tile_bytes, growth)?;
    }
    // Leaves are built part by part, then placed in the tree in panel order.
    let mut built: Vec<Option<Layer>> = (0..order.len()).map(|_| None).collect();
    for (k, info) in parts.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let wanted: Vec<usize> = (0..order.len()).filter(|i| order[*i].0 == k).collect();
        let Some(rect) = rects.get(k).copied().filter(|_| !wanted.is_empty()) else { continue };
        // Admitted again for this part's layers: other opens' tiles, or a scratch disk that went
        // away since the open started, count against the budget now.
        if !photocraft_raster::spill::enabled() {
            ram_tile_capacity(
                photocraft_raster::spill::stats().resident_bytes as u64,
                photocraft_raster::memory::stats().1.tile_bytes,
                tile_bytes(wanted.len(), rect),
            )?;
        }
        let w = info.data_window;
        let samples = u64::from(w.width).saturating_mul(u64::from(w.height)).saturating_mul(info.channels.len() as u64);
        let _working = photocraft_raster::memory::try_reserve_operation(samples.saturating_mul(4)).map_err(IoError::OutOfMemory)?;
        let part = craft_exr::decode::decode_part(bytes, k, &|_| true, &limits).map_err(exr_error)?;
        for i in wanted {
            built[i] = Some(leaf_layer(&part, order[i].1, rect, ctl)?);
        }
        ctl.progress(0.05 + 0.9 * (k + 1) as f32 / parts.len() as f32);
    }
    let mut layers: Vec<Layer> = built.into_iter().collect::<Option<Vec<_>>>().ok_or_else(|| IoError::Unsupported("an EXR layer was not built".into()))?;
    // Only the first colour layer in panel order is visible.
    let first_rgb = order.iter().position(|(_, l)| matches!(l, Leaf::Rgb { .. }));
    for (i, layer) in layers.iter_mut().enumerate() {
        layer.visible = Some(i) == first_rgb;
    }
    let tree = assemble(plan.nodes, &mut layers.into_iter())?;
    // As today's open says for a half-float file.
    let half = parts.iter().any(|p| p.channels.iter().any(|c| c.sample == craft_exr::decode::SampleType::F16));
    let warnings = if half { vec!["16-bit float samples are stored as 32-bit float".to_string()] } else { Vec::new() };
    let mut doc = Document::new(name, Size::new(cw, ch), ColorMode::Rgb, DocSample::F32);
    doc.layers = tree.into();
    doc.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
    Ok(Some(ImportResult { document: doc, warnings, source_read_only: false, preview_only: false }))
}

/// The name of an EXR's only layer when it has transparency: `RGB`, or `YA` when the file's
/// channels are `Y` and `A` (`gray`), whatever mode the document opens in.
pub(crate) fn single_layer_name(gray: bool) -> &'static str {
    if gray { "YA" } else { "RGB" }
}
