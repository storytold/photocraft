//! Build-time check for a local shared UI override. No runtime files or network access.
use std::path::Path;

fn text_at<'a>(value: &'a toml::Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(value, |node, key| node.get(*key)).and_then(toml::Value::as_str)
}
pub fn local_override(workspace: &str) -> Result<Option<String>, String> {
    let value: toml::Value = toml::from_str(workspace).map_err(|e| format!("panel contract: invalid workspace manifest: {e}"))?;
    Ok(text_at(&value, &["patch", "https://github.com/storytold/craft-libs", "craft-ui", "path"]).map(str::to_owned))
}
pub fn compatible(workspace: &str, local: &str) -> Result<(), String> {
    if local_override(workspace)?.is_none() {
        return Ok(());
    }
    let workspace: toml::Value = toml::from_str(workspace).map_err(|e| format!("panel contract: {e}"))?;
    let local: toml::Value = toml::from_str(local).map_err(|e| format!("panel contract: {e}"))?;
    for (name, actual, expected) in [
        (
            "craft-ui",
            text_at(&workspace, &["workspace", "dependencies", "craft-ui", "rev"]),
            text_at(&local, &["package", "metadata", "photocraft-panel-contract", "source-revision"]),
        ),
        (
            "egui",
            text_at(&workspace, &["patch", "crates-io", "egui", "rev"]),
            text_at(&local, &["package", "metadata", "photocraft-panel-contract", "egui-revision"]),
        ),
    ] {
        let (Some(actual), Some(expected)) = (actual, expected) else {
            return Err(format!("panel contract: missing {name} revision; reconcile the local docking adapter before building"));
        };
        if actual != expected {
            return Err(format!(
                "panel contract: workspace {name} requests {actual}, local docking adapter was reviewed against {expected}. Reconcile vendor/craft-ui and run panel compatibility/visual tests before updating its contract."
            ));
        }
    }
    Ok(())
}
pub fn check(root: &Path) -> Result<(), String> {
    let path = root.join("Cargo.toml");
    // Published/standalone UI consumers have no local workspace override to reconcile.
    if !path.is_file() {
        return Ok(());
    }
    println!("cargo::rerun-if-changed={}", path.display());
    let workspace = std::fs::read_to_string(&path).map_err(|e| format!("panel contract: {}: {e}", path.display()))?;
    if let Some(relative) = local_override(&workspace)? {
        let path = root.join(relative).join("Cargo.toml");
        println!("cargo::rerun-if-changed={}", path.display());
        let local = std::fs::read_to_string(&path).map_err(|e| format!("panel contract: {}: {e}", path.display()))?;
        compatible(&workspace, &local)?;
    }
    Ok(())
}
