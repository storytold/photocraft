//! Cryptomatte (specification 1.2): per-pixel object/material IDs with coverage in EXR
//! channels, as written by Arnold, V-Ray, Redshift, Mantra/Karma and Cycles. Layers are
//! announced by `cryptomatte/<key>/name|hash|conversion|manifest` header attributes; the
//! samples live in `<name>NN` RGBA channel sets, each holding two (ID, coverage) ranks:
//! `.red`/`.green` (rank 2·NN) and `.blue`/`.alpha` (rank 2·NN+1).
//!
//! IDs are the float *values*: MurmurHash3 x86-32 (seed 0) of the object name, its
//! exponent fixed away from 0/255 so no ID is denormal, infinite or NaN, read as an f32
//! bit pattern. Everything here compares IDs by value, never by bit pattern.

use std::io::Cursor;

use exr::meta::MetaData;
use exr::meta::attribute::AttributeValue;

use crate::Format;
use crate::error::CodecError;
use crate::image::{CryptomatteBuffer, CryptomatteLayer};
use crate::options::Limits;

const F: Format = Format::OpenExr;

pub(crate) use craft_exr::cryptomatte::split_stream;

/// The Cryptomatte ID of a name: the MurmurHash3 bits with the exponent fixed away from
/// 0 and 255, as an f32 value.
pub fn cryptomatte_id(name: &str) -> f32 {
    craft_exr::cryptomatte::id(name)
}

/// The 7-hex-digit metadata key of a layer name (the ID bits without their last digit).
pub fn cryptomatte_key(name: &str) -> String {
    craft_exr::cryptomatte::key(name)
}

/// The preview colour of an ID (the reference formula): green and blue channels shifted
/// out of the ID's bit pattern, red unused.
pub fn cryptomatte_preview_color(id: f32) -> [f32; 3] {
    craft_exr::cryptomatte::preview_color(id)
}

/// The `cryptomatte/<key>/<field>` text attribute of a header, if present.
fn attribute(other: &std::collections::HashMap<exr::meta::attribute::Text, AttributeValue>, key: &str, field: &str) -> Option<String> {
    let full = exr::meta::attribute::Text::new_or_none(format!("cryptomatte/{key}/{field}").as_str())?;
    match other.get(&full)? {
        AttributeValue::Text(t) => Some(t.to_string()),
        _ => None,
    }
}

/// A manifest JSON (`{"object name": "<8-hex ID bits>"}`) as (ID value, name) pairs, by name.
/// Entries that are not hex strings are skipped; a broken document yields none.
fn parse_manifest(json: &str) -> Vec<(f32, String)> {
    craft_exr::cryptomatte::parse_manifest(json).into_iter().map(|(name, bits)| (f32::from_bits(bits), name)).collect()
}

/// How a layer name compares with a stream's channel prefix: the channel prefix and the `name`
/// attribute differ in case or `_`/`-` between renderers (`CryptoAsset` vs `crypto_asset`).
fn normalize_layer(s: &str) -> String {
    s.to_ascii_lowercase().replace(['_', '-'], "")
}

/// The stream channel names of every layer of a header (`crypto_asset00`, `crypto_asset01`, …,
/// exactly as the file spells them), keyed by [`normalize_layer`] of their prefix. Built in one
/// pass over the channels, so looking up each `cryptomatte/<key>/name` costs no channel scan.
fn layer_streams(header: &exr::meta::header::Header) -> std::collections::HashMap<String, Vec<String>> {
    let mut found: std::collections::HashMap<String, Vec<(u32, String)>> = std::collections::HashMap::new();
    for c in &header.channels.list {
        let Some(s) = split_stream(&c.name.to_string()) else { continue };
        if s.coverage {
            continue;
        }
        let layer = normalize_layer(s.stream.trim_end_matches(|c: char| c.is_ascii_digit()));
        found.entry(layer).or_default().push((s.index, s.stream));
    }
    found
        .into_iter()
        .map(|(layer, mut streams)| {
            streams.sort();
            streams.dedup();
            (layer, streams.into_iter().map(|(_, stream)| stream).collect())
        })
        .collect()
}

/// Lists every Cryptomatte layer of the file from the headers alone: what a Cryptomatte
/// picker offers. The embedded manifest (capped, JSON) is parsed into (ID, name) pairs.
pub(crate) fn layers(bytes: &[u8], limits: &Limits) -> Result<Vec<CryptomatteLayer>, CodecError> {
    super::exr::check_previews(bytes)?;
    let meta = MetaData::read_from_buffered(Cursor::new(bytes), false).map_err(|e| CodecError::malformed(F, e))?;
    // Manifests can be large; bound the embedded text before parsing anything.
    const MANIFEST_CAP: usize = 16 << 20;
    let mut out = Vec::new();
    for (part, header) in meta.headers.iter().enumerate() {
        let size = header.layer_size;
        let (w, h) = (u32::try_from(size.0).unwrap_or(u32::MAX), u32::try_from(size.1).unwrap_or(u32::MAX));
        limits.check_bytes(w, h, 8)?;
        // Every channel's stream, indexed once by layer. Each layer's list is moved into the
        // first key that names it: file-controlled keys can all name the same layer (in any
        // case or `_`/`-` spelling), and copying its list for each of them would make the
        // result grow with keys * channels. A second key naming a listed layer is skipped;
        // `decode` resolves a name to its first layer anyway.
        let mut streams_for = layer_streams(header);
        // A key whose manifest is missing or unparseable only stands in for its layer when no later
        // key naming it has a usable one.
        let mut fallback: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
        let other = &header.own_attributes.other;
        let mut push = |name: String, key: String, manifest: Vec<_>, channels: Vec<String>| {
            out.push(CryptomatteLayer {
                part,
                name,
                conversion: attribute(other, &key, "conversion"),
                manifest,
                sidecar_manifest: attribute(other, &key, "manif_file"),
                key,
                channels,
            });
        };
        for key in cryptomatte_keys(other) {
            let Some(name) = attribute(other, &key, "name") else { continue };
            let norm = normalize_layer(&name);
            if streams_for.get(&norm).is_none_or(|c| c.is_empty()) {
                continue;
            }
            let manifest = attribute(other, &key, "manifest").filter(|m| m.len() <= MANIFEST_CAP).map(|m| parse_manifest(m.as_str())).unwrap_or_default();
            if manifest.is_empty() {
                fallback.entry(norm).or_insert((name, key));
            } else if let Some(channels) = streams_for.remove(&norm) {
                push(name, key, manifest, channels);
            }
        }
        for (norm, (name, key)) in fallback {
            if let Some(channels) = streams_for.remove(&norm) {
                push(name, key, Vec::new(), channels);
            }
        }
    }
    out.sort_by(|a, b| (&a.part, &a.name).cmp(&(&b.part, &b.name)));
    Ok(out)
}

/// Every `cryptomatte/<key>/` key of a header, sorted, without duplicates.
fn cryptomatte_keys(other: &std::collections::HashMap<exr::meta::attribute::Text, exr::meta::attribute::AttributeValue>) -> Vec<String> {
    let names: Vec<String> = other.keys().map(|name| name.to_string()).collect();
    let mut keys = craft_exr::cryptomatte::keys(names.iter().map(String::as_str));
    keys.sort(); // HashMap order varies per call; layer names must stay stable
    keys
}

/// Decodes the samples of the Cryptomatte layer called `layer_name` (a
/// [`CryptomatteLayer::name`]): per pixel its (ID, coverage) pairs, coverage above zero,
/// sorted by descending coverage. Refuses files that also contain deep parts, like the
/// other flat part decoders.
pub(crate) fn decode(bytes: &[u8], layer_name: &str, limits: &Limits) -> Result<CryptomatteBuffer, CodecError> {
    let found = layers(bytes, limits)?;
    // An exact name wins, so layers whose names only differ after normalising can each be asked for.
    let layer = found
        .iter()
        .find(|l| l.name == layer_name)
        .or_else(|| found.iter().find(|l| normalize_layer(&l.name) == normalize_layer(layer_name)))
        .ok_or_else(|| CodecError::unsupported(F, format!("no Cryptomatte layer named {layer_name}")))?;
    // Also caps all parts' declared sizes together, like every other flat decode.
    let meta = super::exr::read_meta(bytes, limits)?;
    if meta.requirements.has_deep_data {
        return Err(CodecError::unsupported(F, "this file has deep parts; Cryptomatte decoding stays with flat files"));
    }
    let header = meta.headers.get(layer.part).ok_or_else(|| CodecError::malformed(F, "the Cryptomatte part is missing"))?;
    let size = header.layer_size;
    let (w, h) = (size.0, size.1);
    let npx = w.checked_mul(h).ok_or_else(|| CodecError::malformed(F, "the layer is too large"))?;
    let too_large = || CodecError::malformed(F, "the layer is too large");
    let (w32, h32) = (u32::try_from(w).map_err(|_| too_large())?, u32::try_from(h).map_err(|_| too_large())?);
    // Each pixel also gets a pair list, so count its header with the channel samples (four
    // f32 channels per set: two ranks of ID and coverage). Include the heap capacity the pair
    // list can reach: up to one (id, coverage) pair per rank, 8 bytes each. Without this the
    // returned per-pixel vectors could exceed the caller's max_alloc budget.
    let ranks = layer.channels.len().saturating_mul(2);
    let pair_capacity = (ranks as u64).saturating_mul(std::mem::size_of::<(f32, f32)>() as u64);
    let per_pixel =
        (layer.channels.len().max(1) as u64).saturating_mul(16).saturating_add(std::mem::size_of::<Vec<(f32, f32)>>() as u64).saturating_add(pair_capacity);
    limits.check_bytes(w32, h32, per_pixel)?;

    let ex_layer = super::exr::read_layer(bytes, layer.part)?;

    // Two (ID, coverage) ranks per set: rank `slot * 2 + stream.rank`.
    let mut ids: Vec<Vec<f32>> = vec![Vec::new(); ranks];
    let mut coverages: Vec<Vec<f32>> = vec![Vec::new(); ranks];
    // Stream name -> its slot, built once: a linear search per decoded channel is quadratic in
    // the file's stream count.
    let slots: std::collections::HashMap<&str, usize> = layer.channels.iter().enumerate().rev().map(|(i, s)| (s.as_str(), i)).collect();
    for c in &ex_layer.channel_data.list {
        let Some(stream) = split_stream(&c.name.to_string()) else { continue };
        let Some(&slot) = slots.get(stream.stream.as_str()) else { continue };
        let values: Vec<f32> = c.sample_data.values_as_f32().collect();
        if values.len() != npx {
            return Err(CodecError::malformed(F, "a Cryptomatte channel is smaller than its layer"));
        }
        let bucket = if stream.coverage { &mut coverages } else { &mut ids };
        let Some(dst) = slot.checked_mul(2).and_then(|r| r.checked_add(stream.rank)).and_then(|r| bucket.get_mut(r)) else { continue };
        *dst = values;
    }

    let mut pixels = vec![Vec::new(); npx];
    for (p, out) in pixels.iter_mut().enumerate() {
        for (id, cov) in ids.iter().zip(&coverages) {
            let (id, cov) = (id.get(p).copied().unwrap_or(0.0), cov.get(p).copied().unwrap_or(0.0));
            if cov > 0.0 && id.is_finite() {
                out.push((id, cov));
            }
        }
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
    }
    Ok(CryptomatteBuffer { width: w32, height: h32, pixels })
}
