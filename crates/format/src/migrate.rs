//! Manifest migrations. Each step upgrades the raw JSON from version `n` to
//! `n + 1`, so old bundles keep loading as the schema evolves.

use serde_json::Value;

use crate::manifest::FORMAT_VERSION;
use crate::{FormatError, Result};

pub(crate) type Step = craft_pcraft_format::Step<FormatError>;

/// `STEPS[i]` upgrades version `i + 1` to `i + 2`. Empty while version 1 is
/// the only version.
pub(crate) const STEPS: &[Step] = &[];

/// Upgrade `v` in place to `target` using `steps`, with craft-pcraft-format's runner.
pub(crate) fn migrate_with(v: &mut Value, steps: &[Step], target: u32) -> Result<u32> {
    craft_pcraft_format::migrate(v, steps, target)
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
