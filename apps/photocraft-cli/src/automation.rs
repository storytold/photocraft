use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};

use serde_json::Value;

use photocraft_automation::{Headless, PhotocraftMcp, security};

use crate::{
    Result,
    args::{CommandsArgs, McpArgs, ServeArgs},
    output::{JsonStyle, print_json},
};

pub(crate) fn commands(a: &CommandsArgs, out: &mut dyn Write) -> Result {
    let h = Headless::new();
    let list = h.command_list();
    let needle = a.filter.as_deref().map(str::to_lowercase);
    let items: Vec<&Value> = list
        .as_array()
        .map(|v| v.iter().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|c| needle.as_ref().is_none_or(|n| photocraft_engine::commands::list_entry_matches(c, n)))
        .collect();
    if a.json {
        return print_json(out, &Value::Array(items.into_iter().cloned().collect()), JsonStyle::Pretty);
    }
    for c in items {
        let menu = c["menu"].as_array().map(|m| m.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" > ")).unwrap_or_default();
        writeln!(out, "{:<44} {:<32} {}", c["id"].as_str().unwrap_or(""), c["label"].as_str().unwrap_or(""), menu)?;
    }
    Ok(())
}

/// The JSON type an `edit.colorSettings` key takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingType {
    /// A string (a profile name or path, a policy, an intent).
    Text,
    /// `true` or `false`.
    Flag,
    /// `blendTextGamma`: a gamma, or a bool for the default or none.
    FlagOrNumber,
}

/// The `edit.colorSettings` keys `--color-settings` takes, with their types, as
/// `color_cmds::color_settings` reads them. The command skips any other key and any value of
/// another type, so a misspelt key or a quoted bool would start the server on settings the
/// operator didn't ask for.
const COLOR_SETTINGS_KEYS: &[(&str, SettingType)] = &[
    ("reset", SettingType::Flag),
    ("workingRgb", SettingType::Text),
    ("workingCmyk", SettingType::Text),
    ("workingGray", SettingType::Text),
    ("monitorProfile", SettingType::Text),
    ("policyRgb", SettingType::Text),
    ("policyCmyk", SettingType::Text),
    ("policyGray", SettingType::Text),
    ("askOnMismatch", SettingType::Flag),
    ("askOnPaste", SettingType::Flag),
    ("askOnMissing", SettingType::Flag),
    ("bpc", SettingType::Flag),
    ("dither", SettingType::Flag),
    ("intent", SettingType::Text),
    ("blendTextGamma", SettingType::FlagOrNumber),
];

/// Err naming the problem when `v` isn't an object of [`COLOR_SETTINGS_KEYS`] with values of
/// their types. The values themselves (a profile, a policy name) are the command's to check.
fn check_color_settings(v: &serde_json::Value) -> std::result::Result<(), String> {
    let map = v.as_object().ok_or_else(|| format!("expected a JSON object of edit.colorSettings params, not {v}"))?;
    for (k, val) in map {
        let Some(&(_, ty)) = COLOR_SETTINGS_KEYS.iter().find(|(n, _)| n == k) else {
            let names: Vec<&str> = COLOR_SETTINGS_KEYS.iter().map(|(n, _)| *n).collect();
            return Err(format!("unknown key `{k}` (expected: {})", names.join(", ")));
        };
        let (ok, want) = match ty {
            SettingType::Text => (val.is_string(), "a string"),
            SettingType::Flag => (val.is_boolean(), "true or false"),
            SettingType::FlagOrNumber => (val.is_boolean() || val.is_number(), "a number or a bool"),
        };
        if !ok {
            return Err(format!("`{k}` must be {want}, not {val}"));
        }
    }
    Ok(())
}

/// `photocraft-cli serve`: one headless session answering JSON lines on stdio or a local port,
/// with the operator's `--color-settings` applied before the first request.
pub(crate) fn serve(a: &ServeArgs, err: &mut dyn Write) -> Result {
    use std::sync::{Arc, Mutex};
    let mut h = Headless::with_workspace(a.automation.workspace()?);
    let policy = crate::gpu::GpuPolicy::from_args(&a.gpu).map_err(crate::CliError::Other)?;
    policy.apply_to_headless(&mut h);
    // Operator settings, applied before any request. Clients stay gated: they may not run
    // `edit.colorSettings`, so they cannot point the working spaces at another file.
    if let Some(json) = &a.color_settings {
        let params: serde_json::Value = serde_json::from_str(json).map_err(|e| crate::CliError::Other(format!("bad --color-settings: {e}")))?;
        check_color_settings(&params).map_err(|e| crate::CliError::Other(format!("bad --color-settings: {e}")))?;
        let gate = h.session.authorize.take();
        let applied = h.session.execute("edit.colorSettings", params);
        h.session.authorize = gate;
        applied.map_err(|e| crate::CliError::Other(format!("--color-settings: {e}")))?;
    }
    for d in h.drain_diagnostics() {
        let _ = writeln!(err, "warning: {d}");
    }
    let h = Arc::new(Mutex::new(h));
    match a.port {
        Some(port) => {
            if a.port_from_env {
                // Name the mode switch: an exported variable changes what this command does, and
                // unsetting it (or setting it empty) is the way back to stdio.
                let _ = writeln!(err, "photocraft-cli: PHOTOCRAFT_CONTROL_PORT={port} is set; serving on TCP instead of stdio (unset it to serve stdio)");
            }
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port)).to_string();
            let (supplied, token_file) = (a.control.control_token.as_deref(), a.control.control_token_file.as_deref());
            let token = security::server_token(supplied, token_file)?;
            if let Some(path) = token_file {
                let _ = writeln!(err, "photocraft-cli control token file: {}", path.display());
            } else if supplied.is_none() {
                let _ = writeln!(err, "photocraft-cli control token: {token}");
            }
            photocraft_automation::rpc::serve_tcp(&addr, h, token, |local| {
                let _ = writeln!(err, "photocraft-cli serving on {local}");
            })?;
            Ok(())
        }
        None => {
            let stdin = std::io::stdin();
            Ok(photocraft_automation::rpc::serve_lines(&h, stdin.lock(), std::io::stdout())?)
        }
    }
}

pub(crate) fn mcp(a: &McpArgs, err: &mut dyn Write) -> Result {
    let server = match &a.bridge {
        Some(addr) => {
            if a.bridge_from_env {
                // Name the mode switch, as `serve` does: the way back to headless is to unset it.
                let _ = writeln!(
                    err,
                    "photocraft-cli: PHOTOCRAFT_CONTROL_PORT is set; bridging to the app on {addr} instead of a headless engine (unset it to run headless)"
                );
            }
            let token = security::client_token(a.control.control_token.as_deref(), a.control.control_token_file.as_deref())?;
            PhotocraftMcp::bridge(&addr.to_string(), &token)?
        }
        None => {
            let mut h = Headless::with_workspace(a.automation.workspace()?);
            let policy = crate::gpu::GpuPolicy::from_args(&a.gpu).map_err(crate::CliError::Other)?;
            policy.apply_to_headless(&mut h);
            for d in h.drain_diagnostics() {
                let _ = writeln!(err, "warning: {d}");
            }
            PhotocraftMcp::with_headless(h)
        }
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    Ok(rt.block_on(server.serve_stdio())?)
}

#[cfg(test)]
mod tests {
    use super::{COLOR_SETTINGS_KEYS, Headless, SettingType};

    /// `--color-settings` takes exactly the keys `edit.colorSettings` documents in its params,
    /// each with the type documented there (a quoted value is a string, `bool` a flag, a range or
    /// `bool` a number or flag). A key added to the command without being added here fails this
    /// test, instead of being refused only when an operator first passes it.
    #[test]
    fn color_settings_keys_match_the_command_params_doc() {
        let list = Headless::new().command_list();
        let spec = list.as_array().into_iter().flatten().find(|c| c["id"] == "edit.colorSettings").expect("edit.colorSettings is listed");
        let doc = spec["params"].as_str().expect("a params doc");
        let inner = doc.strip_prefix('{').and_then(|d| d.split_once('}')).map(|(i, _)| i).expect("a params object first");
        let mut documented: Vec<(String, SettingType)> = inner
            .split(',')
            .map(|kv| {
                let (k, v) = kv.split_once(':').unwrap_or_else(|| panic!("`{kv}` is not key:value"));
                let ty = if v.starts_with('"') {
                    SettingType::Text
                } else if v.starts_with("bool") {
                    SettingType::Flag
                } else if v.contains("bool") {
                    SettingType::FlagOrNumber
                } else {
                    panic!("{k}: no rule for `{v}`")
                };
                (k.trim_matches('"').to_string(), ty)
            })
            .collect();
        let mut ours: Vec<(String, SettingType)> = COLOR_SETTINGS_KEYS.iter().map(|&(k, t)| (k.to_string(), t)).collect();
        documented.sort_by(|a, b| a.0.cmp(&b.0));
        ours.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(ours, documented);
    }
}
