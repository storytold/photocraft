//! Admission for XCF's borrowed XML DOM. The node cap alone does not bound
//! the parser's duplicate-attribute checks and namespace-table work.

use photocraft_raster::Interrupt;

use super::{Result, invalid, work_budget::WorkBudget};
use crate::IoError;

const MAX_ATTRIBUTES: usize = 256;
const PARSE_WORK: usize = 1 << 20;
// roxmltree 0.21 sizes its initial node and attribute vectors from raw
// '<' and '=' counts, including occurrences in text, comments and values.
const MAX_INITIAL_XML_SLOTS: usize = 64 * 1024;

pub(super) fn admit(xml: &str, label: &str, max_input: usize, max_depth: usize, ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if xml.len() > max_input {
        return Err(IoError::Unsupported(format!("{label} exceeds the bounded XML byte allowance")));
    }
    let mut slots_left = MAX_INITIAL_XML_SLOTS;
    for chunk in xml.as_bytes().chunks(4096) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let slots = chunk.iter().filter(|&&byte| matches!(byte, b'<' | b'=')).count();
        slots_left = slots_left.checked_sub(slots).ok_or_else(|| IoError::Unsupported(format!("{label} exceeds the bounded XML parser reservation")))?;
    }
    if craft_svg::guard::too_wide(xml, MAX_ATTRIBUTES, PARSE_WORK) {
        return Err(IoError::Unsupported(format!("{label} exceeds the bounded XML attribute-parse allowance")));
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if craft_svg::guard::deeper_than(xml, max_depth) {
        return Err(IoError::Unsupported(format!("{label} exceeds the bounded XML nesting allowance")));
    }
    let limits = craft_xml::Limits { max_input, max_depth, max_attrs: MAX_ATTRIBUTES, ..Default::default() };
    check_namespace_work(xml, label, limits, PARSE_WORK, ctl)
}

/// Namespace changes can copy a parent's scope through repeated prefix
/// searches and insert new bindings into the DOM's global sorted table.
pub(super) fn check_namespace_work(xml: &str, label: &str, limits: craft_xml::Limits, mut work_left: usize, ctl: &Interrupt<'_>) -> Result<()> {
    let options = craft_xml::Options {
        names: craft_xml::Names::AsWritten,
        ns_decls: true,
        strict: true,
        doctype: craft_xml::Doctype::Reject,
        unknown_entity: craft_xml::UnknownEntity::Error,
        limits,
        ..Default::default()
    };
    let mut events = options.events(xml.as_bytes());
    let mut scopes: Vec<usize> = Vec::new();
    let mut declarations_seen = 1usize; // Includes the predefined XML binding.
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let Some(event) = events.next().map_err(|error| invalid(format!("invalid {label}: {error}")))? else { break };
        match event {
            craft_xml::Event::Start { attrs, .. } => {
                let declarations = attrs.iter().filter(|(name, _)| name == "xmlns" || name.starts_with("xmlns:")).count();
                // Overcount shadowed or repeated prefixes instead of resolving them.
                let scope = scopes.last().copied().unwrap_or(0).saturating_add(declarations);
                let copying = if declarations == 0 { 0 } else { scope.saturating_mul(scope) };
                let inserted = declarations_seen.saturating_add(declarations);
                let insertion = declarations.saturating_mul(inserted);
                let cost = copying.saturating_add(scope.saturating_mul(attrs.len().saturating_add(1))).saturating_add(insertion);
                if !WorkBudget::new(&mut work_left).keep_remaining(cost) {
                    return Err(IoError::Unsupported(format!("{label} exceeds the bounded XML namespace-parse allowance")));
                }
                declarations_seen = inserted;
                scopes.push(scope);
            }
            craft_xml::Event::End { .. } => {
                scopes.pop();
            }
            _ => {}
        }
    }
    Ok(())
}
