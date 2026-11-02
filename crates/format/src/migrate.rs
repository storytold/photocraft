//! Manifest migrations. Each step upgrades the raw JSON from version `n` to
//! `n + 1`, so old bundles keep loading as the schema evolves.

use serde_json::Value;

use crate::manifest::FORMAT_VERSION;
use crate::{FormatError, Result};

pub(crate) type Step = fn(&mut Value) -> Result<()>;

/// Version 2 adds optional sprite state; version 1 needs no structural migration.
pub(crate) const STEPS: &[Step] = &[|_| Ok(())];

/// Upgrade `v` in place to `target` using `steps`.
pub(crate) fn migrate_with(v: &mut Value, steps: &[Step], target: u32) -> Result<u32> {
    let found = v.get("format_version").and_then(Value::as_u64).ok_or_else(|| FormatError::corrupt("manifest has no format_version"))?;
    let found = u32::try_from(found).map_err(|_| FormatError::corrupt("format_version out of range"))?;
    if found == 0 {
        return Err(FormatError::corrupt("format_version 0 is invalid"));
    }
    if found > target {
        return Err(FormatError::TooNew { found, supported: target });
    }
    for ver in found..target {
        let step = steps.get(ver as usize - 1).ok_or_else(|| FormatError::Unsupported(format!("no migration from format {ver}")))?;
        step(v)?;
        v["format_version"] = Value::from(ver + 1);
    }
    Ok(found)
}

pub(crate) fn migrate(v: &mut Value) -> Result<u32> {
    migrate_with(v, STEPS, FORMAT_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rename_title(v: &mut Value) -> Result<()> {
        let t = v["document"]["title"].take();
        v["document"]["name"] = t;
        Ok(())
    }
    fn add_field(v: &mut Value) -> Result<()> {
        v["document"]["added"] = json!(true);
        Ok(())
    }

    #[test]
    fn chained_steps_run_in_order() {
        let mut v = json!({"format_version": 1, "document": {"title": "old"}});
        let found = migrate_with(&mut v, &[rename_title, add_field], 3).unwrap();
        assert_eq!(found, 1);
        assert_eq!(v["format_version"], 3);
        assert_eq!(v["document"]["name"], "old");
        assert_eq!(v["document"]["added"], true);
    }

    #[test]
    fn partial_chain_from_middle() {
        let mut v = json!({"format_version": 2, "document": {}});
        migrate_with(&mut v, &[rename_title, add_field], 3).unwrap();
        assert_eq!(v["document"]["added"], true);
        assert!(v["document"].get("name").is_none());
    }

    #[test]
    fn too_new_and_invalid_versions() {
        let mut v = json!({"format_version": 99});
        assert!(matches!(migrate(&mut v), Err(FormatError::TooNew { found: 99, .. })));
        assert!(migrate(&mut json!({"format_version": 0})).is_err());
        assert!(migrate(&mut json!({})).is_err());
        assert!(migrate_with(&mut json!({"format_version": 1}), &[], 2).is_err());
    }

    #[test]
    fn current_version_is_noop() {
        let mut v = json!({"format_version": FORMAT_VERSION});
        assert_eq!(migrate(&mut v).unwrap(), FORMAT_VERSION);
    }
}

#[cfg(test)]
mod sprite_version_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn version_one_reader_refuses_sprite_version_and_current_reader_keeps_old_manifests() {
        let mut sprite = json!({"format_version": 2, "document": {"sprite": {"current": 0}}});
        assert!(matches!(migrate_with(&mut sprite, &[], 1), Err(FormatError::TooNew { found: 2, supported: 1 })));
        assert_eq!(migrate(&mut sprite).unwrap(), 2);
        let mut old = json!({"format_version": 1, "document": {"name": "Static"}});
        assert_eq!(migrate(&mut old).unwrap(), 1);
        assert_eq!(old["document"]["name"], "Static");
        assert_eq!(old["format_version"], 2);
    }
}
