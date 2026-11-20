//! Three-way colour preference merge for browser storage, which has no SettingsStore yet.
use serde_json::Value;

fn merge_cmyk_role_slots(base: &Value, ours: &Value, theirs: &mut Value) -> bool {
    let (Some(old), Some(new), Some(stored)) = (base.as_array(), ours.as_array(), theirs.as_array_mut()) else {
        return false;
    };

    if old.len() != 2 || new.len() != 2 || stored.len() != 2 {
        return false;
    }
    if !(0..2).all(|pair| {
        old[pair].as_array().is_some_and(|v| v.len() == 2)
            && new[pair].as_array().is_some_and(|v| v.len() == 2)
            && stored[pair].as_array().is_some_and(|v| v.len() == 2)
    }) {
        return false;
    }

    for pair in 0..2 {
        let Some(old_pair) = old[pair].as_array() else { return false };
        let Some(new_pair) = new[pair].as_array() else { return false };
        let Some(stored_pair) = stored[pair].as_array_mut() else { return false };

        for role in 0..2 {
            if new_pair[role] != old_pair[role] {
                stored_pair[role] = new_pair[role].clone();
            }
        }
    }
    true
}

/// Read one native CMYK association from the stored role matrix.
fn cmyk_role(value: &Value, pair: usize, role: usize) -> Option<&Value> {
    value.get("cmykRoles")?.get(pair)?.get(role)
}

/// Merge a swatch and its native CMYK association as one selection.
/// Independent pixel and mask roles can still be changed by separate windows.
fn merge_tool_colors(base: &Value, ours: &Value, theirs: &mut Value) -> bool {
    if !base.is_object() || !ours.is_object() || !theirs.is_object() {
        return false;
    }

    let remote = theirs.clone();

    for pair in 0..2 {
        for role in 0..2 {
            if cmyk_role(base, pair, role).is_none() || cmyk_role(ours, pair, role).is_none() || cmyk_role(&remote, pair, role).is_none() {
                return false;
            }
        }
    }

    // Merge unrelated fields, including the bounded CMYK pick history.
    merge_changes(base, ours, theirs);

    for (key, pair, role) in [("foreground", 0, 0), ("background", 0, 1), ("maskForeground", 1, 0), ("maskBackground", 1, 1)] {
        let (Some(old_rgb), Some(local_rgb), Some(remote_rgb), Some(old_inks), Some(local_inks), Some(remote_inks)) =
            (base.get(key), ours.get(key), remote.get(key), cmyk_role(base, pair, role), cmyk_role(ours, pair, role), cmyk_role(&remote, pair, role))
        else {
            continue;
        };

        // Local RGB changes carry their corresponding CMYK association.
        // If the remote RGB changed but only local inks changed,
        // retain the complete remote selection.
        let (rgb, inks) = if local_rgb != old_rgb {
            (local_rgb, local_inks)
        } else if remote_rgb != old_rgb {
            (remote_rgb, remote_inks)
        } else if local_inks != old_inks {
            (local_rgb, local_inks)
        } else {
            (remote_rgb, remote_inks)
        };

        if let Some(colors) = theirs.as_object_mut() {
            colors.insert(key.to_owned(), rgb.clone());
        }

        if let Some(slot) = theirs.get_mut("cmykRoles").and_then(|v| v.get_mut(pair)).and_then(|v| v.get_mut(role)) {
            *slot = inks.clone();
        }
    }

    true
}
/// Merge recently picked CMYK colors without discarding picks from another window.
/// Each RGB key retains its most recently supplied native inks.
fn merge_cmyk_picks(base: &Value, ours: &Value, theirs: &mut Value) -> bool {
    type Pick = ([f32; 3], [f32; 4]);

    let parse = |v: &Value| -> Option<Vec<Pick>> {
        let entries = v.as_array()?;
        let picks: Vec<Pick> = entries.iter().map(|entry| serde_json::from_value::<Pick>(entry.clone())).collect::<Result<_, _>>().ok()?;

        picks.iter().all(|(rgb, inks)| rgb.iter().all(|v| v.is_finite()) && inks.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))).then_some(picks)
    };

    let (Some(old), Some(local), Some(mut stored)) = (parse(base), parse(ours), parse(theirs)) else {
        return false;
    };

    if local == old {
        return true;
    }

    // Unchanged picks form a prefix in their original order. Everything after the
    // first moved/new pick was selected locally and must become recent in storage.
    let mut old_position = 0;
    let mut changed_suffix = false;
    for &(rgb, inks) in &local {
        if !changed_suffix && let Some(offset) = old.iter().skip(old_position).position(|pick| *pick == (rgb, inks)) {
            old_position += offset + 1;
            continue;
        }
        changed_suffix = true;
        stored.retain(|(existing_rgb, _)| *existing_rgb != rgb);
        stored.push((rgb, inks));
    }

    let extra = stored.len().saturating_sub(crate::ToolState::CMYK_PICKS);
    stored.drain(..extra);

    if let Ok(value) = serde_json::to_value(stored) {
        *theirs = value;
        true
    } else {
        false
    }
}
fn merge_changes(base: &Value, ours: &Value, theirs: &mut Value) {
    if ours == base {
        return;
    }
    match (base, ours, theirs) {
        (Value::Object(b), Value::Object(o), Value::Object(t)) => {
            for (k, ov) in o {
                match (b.get(k), t.get_mut(k)) {
                    (Some(bv), Some(tv)) if k == "toolColors" => {
                        if !merge_tool_colors(bv, ov, tv) {
                            merge_changes(bv, ov, tv);
                        }
                    }
                    (Some(bv), Some(tv)) if k == "cmykPicks" => {
                        if !merge_cmyk_picks(bv, ov, tv) {
                            merge_changes(bv, ov, tv);
                        }
                    }
                    (Some(bv), Some(tv)) if k == "cmykRoles" => {
                        if !merge_cmyk_role_slots(bv, ov, tv) {
                            merge_changes(bv, ov, tv);
                        }
                    }
                    (Some(bv), Some(tv)) => merge_changes(bv, ov, tv),
                    // Removed there, unchanged here: it stays removed.
                    (Some(bv), None) if bv == ov => {}
                    _ => {
                        t.insert(k.clone(), ov.clone());
                    }
                }
            }
            for k in b.keys().filter(|k| !o.contains_key(*k)) {
                t.remove(k);
            }
        }
        (_, _, theirs) => *theirs = ours.clone(),
    }
}

/// Normalize legacy slots before merging so an unchanged missing role cannot erase a remote association.
pub(super) fn merge(base: &Value, ours: &Value, theirs: &Value) -> Value {
    let normalized = |value: &Value| {
        let mut value = super::tool_colors::decode(super::tool_colors::encode(&super::tool_colors::decode(value.clone())));
        if let Some(fields) = value.as_object_mut() {
            fields.remove("savedRoleInks");
            fields.remove("savedPickEntries");
            fields.remove("cmykPickOrder");
        }
        value
    };
    let base = serde_json::json!({"toolColors":normalized(base)});
    let ours = serde_json::json!({"toolColors":normalized(ours)});
    let mut theirs = serde_json::json!({"toolColors":normalized(theirs)});
    merge_changes(&base, &ours, &mut theirs);
    let mut colors = theirs["toolColors"].take();
    // The merged list determines recency. Independent ranks could reorder it on the next load.
    let order: serde_json::Map<String, Value> = colors["cmykPicks"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(rank, entry)| {
            let rgb = serde_json::from_value::<[f32; 3]>(entry.get(0)?.clone()).ok()?;
            Some((super::tool_colors::rgb_key(rgb), serde_json::json!(rank)))
        })
        .collect();
    colors["cmykPickOrder"] = Value::Object(order);
    colors
}

#[cfg(test)]
mod tests {
    use super::merge;
    use serde_json::json;

    #[test]
    fn merged_pick_recency_survives_an_unchanged_merge() {
        let a = json!([[1.0, 0.0, 0.0], [0.0, 1.0, 1.0, 0.0]]);
        let b = json!([[0.0, 1.0, 0.0], [1.0, 0.0, 1.0, 0.0]]);
        let c = json!([[0.0, 0.0, 1.0], [1.0, 1.0, 0.0, 0.0]]);
        let base = json!({"cmykPicks": [a, b]});
        let local = json!({"cmykPicks": [b, a]});
        let remote = json!({"cmykPicks": [a, b, c]});
        let merged = merge(&base, &local, &remote);

        assert_eq!(merged["cmykPicks"], json!([b, c, a]));
        assert_eq!(merge(&merged, &merged, &merged), merged);
    }
}
