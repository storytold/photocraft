//! TOML storage for swatches: each role's inks are keyed by their RGB stand-in, so a stale window
//! cannot attach its inks to another window's swatch. Independent roles and remembered RGB keys
//! remain independently mergeable by SettingsStore. The public prefs JSON
//! keeps the legacy shape; deserialization accepts both that shape and this storage representation.

use serde_json::{Value, json};

const ROLES: [(&str, usize, usize); 4] = [("foreground", 0, 0), ("background", 0, 1), ("maskForeground", 1, 0), ("maskBackground", 1, 1)];

pub(super) fn rgb_key(rgb: [f32; 3]) -> String {
    rgb.map(|v| format!("{:08x}", if v == 0.0 { 0 } else { v.to_bits() })).join("-")
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn stamp_picks(value: &mut Value, previous: &Value) {
    let old = previous.get("cmykPicks").and_then(Value::as_array);
    let ranks = previous.get("cmykPickOrder").and_then(Value::as_object);
    let max_order = i64::MAX as u64 - crate::ToolState::CMYK_PICKS as u64;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| chrono::DateTime::<chrono::Utc>::from_timestamp(i64::try_from(d.as_secs()).ok()?, d.subsec_nanos()))
        .map(|d| d.timestamp_micros().max(0) as u64)
        .unwrap_or(0);
    let latest = ranks.into_iter().flatten().filter_map(|(_, v)| v.as_u64()).map(|v| v.min(max_order)).max().unwrap_or(0);
    let now = now.max(latest.saturating_add(1)).min(max_order);
    let (mut position, mut changed) = (0, false);
    let mut order = serde_json::Map::new();
    for (i, pick) in value.get("cmykPicks").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let Some(rgb) = pick.get(0).and_then(|v| serde_json::from_value::<[f32; 3]>(v.clone()).ok()) else { continue };
        if !changed {
            if let Some(offset) = old.and_then(|picks| picks.iter().skip(position).position(|p| p == pick)) {
                position += offset + 1;
            } else {
                changed = true;
            }
        }
        let key = rgb_key(rgb);
        let rank =
            if changed { now.saturating_add(i as u64) } else { ranks.and_then(|v| v.get(&key)).and_then(Value::as_u64).unwrap_or(i as u64).min(max_order) };
        order.insert(key, json!(rank));
    }
    value["cmykPickOrder"] = Value::Object(order);
}

pub(super) fn encode(value: &Value) -> Value {
    let defaults = crate::ToolState::saved_colors([crate::ToolState::PIXEL_COLORS, crate::ToolState::MASK_COLORS], &[]);
    let mut selections = serde_json::Map::new();
    for (key, pair, role) in ROLES {
        let rgba = serde_json::from_value::<[f32; 4]>(value.get(key).unwrap_or(&defaults[key]).clone()).ok().filter(|v| v.iter().all(|n| n.is_finite()));
        let rgb = rgba.map(|v| [v[0], v[1], v[2]]);
        let association = value.get("cmykRoles").and_then(|v| v.get(pair)).and_then(|v| v.get(role));
        let explicit = association
            .and_then(|v| v.get(0).and_then(|v| serde_json::from_value::<[f32; 3]>(v.clone()).ok()).filter(|v| Some(*v) == rgb).and_then(|_| v.get(1)))
            .cloned();
        let legacy = || {
            value.get("cmykPicks").and_then(Value::as_array).into_iter().flatten().rev().find_map(|v| {
                let key = serde_json::from_value::<[f32; 3]>(v.get(0)?.clone()).ok()?;
                (Some(key) == rgb).then(|| v.get(1).cloned()).flatten()
            })
        };
        let inks = explicit
            .or_else(|| (!value.get("cmykRoles").is_some_and(Value::is_array)).then(legacy).flatten())
            .and_then(|v| serde_json::from_value::<[f32; 4]>(v).ok())
            .filter(|v| v.iter().all(|n| (0.0..=1.0).contains(n)))
            .map(|v| json!(v))
            .unwrap_or_else(|| json!([]));
        let association_key = rgb_key(rgb.unwrap_or_default());
        let mut links = value.get("savedRoleInks").and_then(|v| v.get(key)).and_then(Value::as_object).cloned().unwrap_or_default();
        links.insert(association_key, inks);
        selections.insert(key.into(), json!({"color":rgba.map(|v| json!(v)).unwrap_or_else(|| defaults[key].clone()), "inks":links}));
    }
    // Retain the keys read from disk for SettingsStore's known-key deletion bookkeeping.
    // A fresh tool choice writes a bounded map, so older keys are removed rather than treated as unknown.
    let mut picks = value.get("savedPickEntries").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(entries) = value.get("cmykPicks").and_then(Value::as_array) {
        for (order, entry) in entries.iter().enumerate() {
            let Ok((rgb, inks)) = serde_json::from_value::<([f32; 3], [f32; 4])>(entry.clone()) else { continue };
            if rgb.iter().any(|v| !v.is_finite()) || inks.iter().any(|v| !(0.0..=1.0).contains(v)) {
                continue;
            }
            let key = rgb_key(rgb);
            let rank = value.get("cmykPickOrder").and_then(|v| v.get(&key)).and_then(Value::as_u64).filter(|v| *v <= i64::MAX as u64).unwrap_or(order as u64);
            picks.insert(key, json!({"value":[json!(rgb), json!(inks)], "order":rank}));
        }
    }
    json!({"selections":selections, "picks":picks})
}

pub(super) fn decode(value: Value) -> Value {
    let Some(selections) = value.get("selections").and_then(Value::as_object) else { return value };
    let mut decoded = crate::ToolState::saved_colors([crate::ToolState::PIXEL_COLORS, crate::ToolState::MASK_COLORS], &[]);
    let mut roles = [[Value::Null, Value::Null], [Value::Null, Value::Null]];
    let mut saved_links = serde_json::Map::new();
    for (key, pair, role) in ROLES {
        let Some(selection) = selections.get(key) else { continue };
        let Some(rgba) = selection.get("color").and_then(|v| serde_json::from_value::<[f32; 4]>(v.clone()).ok()).filter(|v| v.iter().all(|n| n.is_finite()))
        else {
            continue;
        };
        decoded[key] = json!(rgba);
        if let Some(links) = selection.get("inks") {
            saved_links.insert(key.into(), links.clone());
        }
        if let Some(inks) = selection
            .get("inks")
            .and_then(|v| v.get(rgb_key([rgba[0], rgba[1], rgba[2]])))
            .and_then(|v| serde_json::from_value::<[f32; 4]>(v.clone()).ok())
            .filter(|v| v.iter().all(|n| n.is_finite() && (0.0..=1.0).contains(n)))
        {
            roles[pair][role] = json!([[rgba[0], rgba[1], rgba[2]], inks]);
        }
    }
    decoded["cmykRoles"] = json!(roles);
    decoded["savedRoleInks"] = Value::Object(saved_links);
    decoded["savedPickEntries"] = value.get("picks").cloned().unwrap_or_else(|| json!({}));
    let mut picks: Vec<_> = value
        .get("picks")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, entry)| {
            let (rgb, inks) = serde_json::from_value::<([f32; 3], [f32; 4])>(entry.get("value")?.clone()).ok()?;
            (rgb.iter().all(|v| v.is_finite()) && inks.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)))
                .then(|| (entry.get("order").and_then(Value::as_u64).unwrap_or(0), key, json!([rgb, inks])))
        })
        .collect();
    picks.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    let extra = picks.len().saturating_sub(crate::ToolState::CMYK_PICKS);
    let mut order = serde_json::Map::new();
    let entries = picks
        .into_iter()
        .skip(extra)
        .map(|(rank, key, pick)| {
            order.insert(key.clone(), json!(rank));
            pick
        })
        .collect();
    decoded["cmykPicks"] = Value::Array(entries);
    decoded["cmykPickOrder"] = Value::Object(order);
    decoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::{SavedPrefs, ToolColors};
    use craft::settings::document;

    fn document(colors: Value) -> craft::settings::DocumentMut {
        let mut prefs = SavedPrefs::default();
        prefs.prefs.tool_colors = ToolColors(colors);
        let mut doc = document::new_document(1);
        document::merge_into(&mut doc, &prefs).unwrap();
        doc
    }

    #[test]
    fn settings_merge_keeps_remote_rgb_and_inks_when_only_local_inks_changed() {
        let mut base = crate::ToolState::saved_colors([crate::ToolState::PIXEL_COLORS, crate::ToolState::MASK_COLORS], &[]);
        base["cmykRoles"] = json!([[[[0., 0., 0.], [0., 0., 0., 1.]], null], [null, null]]);
        let mut remote = base.clone();
        remote["foreground"] = json!([0.2, 0.4, 0.6, 1.]);
        remote["cmykRoles"][0][0] = json!([[0.2, 0.4, 0.6], [0., 1., 0., 0.]]);
        let mut local = base.clone();
        local["cmykRoles"][0][0] = json!([[0., 0., 0.], [1., 0., 0., 0.]]);
        let mut disk = document(remote);
        document::merge_changes::<SavedPrefs>(&document(base), &document(local), &mut disk);
        let (merged, errors) = document::deserialize_sections::<SavedPrefs>(&disk);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(serde_json::from_value::<[f32; 4]>(merged.prefs.tool_colors["foreground"].clone()).unwrap(), [0.2, 0.4, 0.6, 1.]);
        assert_eq!(merged.prefs.tool_colors["cmykRoles"][0][0][1], json!([0., 1., 0., 0.]));
    }

    #[test]
    fn toml_merge_preserves_independent_roles_and_pick_keys_without_nulls() {
        let base = crate::ToolState::saved_colors([crate::ToolState::PIXEL_COLORS, crate::ToolState::MASK_COLORS], &[]);
        let mut a = base.clone();
        a["foreground"] = json!([0.2, 0.4, 0.6, 1.]);
        a["cmykPicks"] = json!([[[0.2, 0.4, 0.6], [1., 0., 0., 0.]]]);
        let mut b = base.clone();
        b["background"] = json!([0.6, 0.4, 0.2, 1.]);
        b["cmykPicks"] = json!([[[0.6, 0.4, 0.2], [0., 1., 0., 0.]]]);
        let mut disk = document(a);
        document::merge_changes::<SavedPrefs>(&document(base), &document(b), &mut disk);
        let (merged, errors) = document::deserialize_sections::<SavedPrefs>(&disk);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(serde_json::from_value::<[f32; 4]>(merged.prefs.tool_colors["foreground"].clone()).unwrap(), [0.2, 0.4, 0.6, 1.]);
        assert_eq!(serde_json::from_value::<[f32; 4]>(merged.prefs.tool_colors["background"].clone()).unwrap(), [0.6, 0.4, 0.2, 1.]);
        assert_eq!(merged.prefs.tool_colors["cmykPicks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn browser_merge_keeps_remote_rgb_with_its_own_inks() {
        let mut base = crate::ToolState::saved_colors([crate::ToolState::PIXEL_COLORS, crate::ToolState::MASK_COLORS], &[]);
        base["cmykRoles"] = json!([[[[0., 0., 0.], [0., 0., 0., 1.]], null], [null, null]]);
        let mut remote = base.clone();
        remote["foreground"] = json!([0.2, 0.4, 0.6, 1.]);
        remote["cmykRoles"][0][0] = json!([[0.2, 0.4, 0.6], [0., 1., 0., 0.]]);
        let mut local = base.clone();
        local["cmykRoles"][0][0] = json!([[0., 0., 0.], [1., 0., 0., 0.]]);
        let merged = crate::prefs::merge_tool_colors(&base, &local, &remote);
        assert_eq!(serde_json::from_value::<[f32; 4]>(merged["foreground"].clone()).unwrap(), [0.2, 0.4, 0.6, 1.]);
        assert_eq!(merged["cmykRoles"][0][0][1], json!([0., 1., 0., 0.]));
    }
}
