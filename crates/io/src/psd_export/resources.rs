use super::{Document, ImageResource, TaggedBlock};

pub(crate) fn legacy_name(s: &str) -> Vec<u8> {
    craft_psd::resources::encode_legacy_name(s)
}

pub(crate) fn unicode_names_resource(names: &[&str]) -> Vec<u8> {
    craft_psd::unicode_names::write(names.iter().copied())
}

pub(crate) fn pascal_names_resource(names: &[&str]) -> Vec<u8> {
    craft_psd::channel_resources::pascal_names(names.iter().copied())
}

pub(crate) fn guides_resource(doc: &Document) -> Vec<u8> {
    let n = doc.guides.horizontal.len() + doc.guides.vertical.len();
    let mut v = Vec::with_capacity(16 + n * 5);
    v.extend_from_slice(&craft_psd::guides::header_v1(576, 576, n as u32));
    for (list, direction) in [(&doc.guides.vertical, 0u8), (&doc.guides.horizontal, 1u8)] {
        for p in list {
            v.extend_from_slice(&craft_psd::guides::GuideV1 { location: (p * 32.0).round() as i32, direction }.to_bytes());
        }
    }
    v
}

/// Path resources: saved paths (2000+), the work path (1025) and the clipping path name (2999),
/// the last only when the imported one doesn't already name the document's clipping path.
pub(crate) fn path_resources(doc: &Document, warnings: &mut Vec<String>) -> Vec<ImageResource> {
    use crate::vector_map::{CLIPPING_PATH, WORK_PATH, path_from_resource, path_to_records};
    let mut out = Vec::new();
    let (w, h) = (doc.size.width, doc.size.height);
    let max = (*crate::vector_map::SAVED_PATHS.end() - *crate::vector_map::SAVED_PATHS.start() + 1) as usize;
    if doc.paths.len() > max {
        warnings.push(format!("only {max} saved paths fit in PSD; the rest were dropped"));
    }
    for (i, p) in doc.paths.iter().take(max).enumerate() {
        let data = match &p.psd_raw {
            Some(r) if path_from_resource(r, w, h).as_ref() == Some(&p.path) => r.to_vec(),
            _ => path_to_records(&p.path, w, h).to_bytes(),
        };
        let mut r = ImageResource::new(crate::vector_map::SAVED_PATHS.start() + i as u16, data);
        r.name = legacy_name(&p.name);
        out.push(r);
    }
    if let Some(wp) = &doc.work_path {
        out.push(ImageResource::new(WORK_PATH, path_to_records(wp, w, h).to_bytes()));
    }
    let raw_clip = doc.metadata.psd_resources.iter().find(|(id, _, _)| *id == CLIPPING_PATH);
    let raw_name = raw_clip.map(|(_, _, d)| String::from_utf8_lossy(craft_psd::path::clipping_path_name(d).unwrap_or_default()).into_owned());
    if let Some(c) = &doc.clipping_path
        && raw_name.as_deref() != Some(c.name.as_str())
    {
        // Pascal name, then flatness as 16.16 fixed (best effort; see vector_map docs).
        out.push(ImageResource::new(CLIPPING_PATH, craft_psd::path::clipping_path_resource(&c.name, c.flatness)));
    }
    out
}

/// Whether an imported (unmodelled) resource still describes the document and is written back.
pub(crate) fn keep_imported_resource(doc: &Document, id: u16, data: &[u8]) -> bool {
    // Measurement scale / count resources: only while they still describe the document.
    if !crate::annotations_map::keep_resource(doc, id) {
        return false;
    }
    // Guides v2 and guide GUIDs: only while they still describe the document's guides.
    if !crate::guides_map::keep_resource(doc, id, data) {
        return false;
    }
    // A preserved clipping-path resource is only valid while it names the current one.
    if id == crate::vector_map::CLIPPING_PATH {
        let raw_name = String::from_utf8_lossy(craft_psd::path::clipping_path_name(data).unwrap_or_default());
        return doc.clipping_path.as_ref().is_some_and(|c| c.name == raw_name);
    }
    true
}

/// Find an explicit flatness edit relative to the last matching imported resource, once per
/// export. Import applies resources in order, so earlier duplicate resources cannot establish
/// that the model changed.
pub(crate) fn imported_clipping_flatness_edit(doc: &Document) -> Option<f32> {
    let c = doc.clipping_path.as_ref()?;
    let (_, _, data) = doc.metadata.psd_resources.iter().rfind(|(id, _, data)| {
        *id == crate::vector_map::CLIPPING_PATH && craft_psd::path::clipping_path_name(data).is_some_and(|name| c.name == String::from_utf8_lossy(name))
    })?;
    (c.flatness != crate::vector_map::clipping_flatness(data)).then_some(c.flatness)
}

/// Copy a kept resource for export. An edit to clipping flatness must not discard the original
/// Pascal name, fill-rule bytes or any other trailing bytes in the resource.
pub(crate) fn imported_resource_data(id: u16, data: &[u8], clipping_flatness: Option<f32>) -> Vec<u8> {
    let mut out = data.to_vec();
    if id == crate::vector_map::CLIPPING_PATH
        && let Some(flatness) = clipping_flatness
        && let Some(start) = crate::vector_map::clipping_flatness_offset(data)
    {
        // Use the shared writer's representation. An empty name leaves one Pascal length byte.
        let encoded = craft_psd::path::clipping_path_resource("", flatness);
        if let Some(field) = encoded.get(1..5) {
            if let Some(target) = out.get_mut(start..start + 4) {
                target.copy_from_slice(field);
            } else {
                // On editing a legacy name-only resource, supply its missing field.
                out.truncate(start);
                out.extend_from_slice(field);
            }
        }
    }
    out
}

/// A document-level (global) tagged block. Photoshop pads these to a multiple of 4, and readers
/// such as psd-tools step to the next block that way: an even pad after `CAI ` (77 bytes)
/// misaligned every block after it (#200).
pub(crate) fn global_block(signature: [u8; 4], key: [u8; 4], data: &[u8]) -> TaggedBlock {
    let mut tb = TaggedBlock::new(key, data.to_vec());
    tb.signature = signature;
    tb.padding = Some(vec![0; (4 - data.len() % 4) % 4]);
    tb
}
