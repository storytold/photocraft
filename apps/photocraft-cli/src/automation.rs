use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};

use serde_json::Value;

use photocraft_automation::{Headless, PhotocraftMcp, security};

use crate::{
    Result,
    args::{CommandsArgs, McpArgs, McpCommand, ServeArgs},
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

pub(crate) fn serve(a: &ServeArgs, err: &mut dyn Write) -> Result {
    use std::sync::{Arc, Mutex};
    let mut h = Headless::with_workspace(a.automation.workspace()?);
    let policy = crate::gpu::GpuPolicy::from_args(&a.gpu).map_err(crate::CliError::Other)?;
    policy.apply_to_headless(&mut h);
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

pub(crate) fn mcp(a: &McpArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result {
    if let Some(McpCommand::Install(i)) = &a.action {
        return mcp_install(&i.client, out);
    }
    let server = match &a.bridge {
        _ if a.live => PhotocraftMcp::live(live_config(a)?),
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

/// `mcp --live`: the app's settings folder, the closed-app override and the app launcher.
fn live_config(a: &McpArgs) -> Result<photocraft_automation::live::LiveConfig> {
    let other = |m: &str| crate::CliError::Other(m.to_string());
    let dir = photocraft_automation::app_dirs::from_env()
        .dir
        .ok_or_else(|| other("can't find PhotoCraft's settings folder (no home folder; set PHOTOCRAFT_CONFIG_DIR)"))?;
    let mut config = photocraft_automation::live::LiveConfig::new(dir);
    config.when_closed = a.when_closed;
    let reads = &a.automation.automation_read_root;
    if reads.len() > 1 {
        return Err(other("--live takes one --automation-read-root"));
    }
    config.read_root = reads.first().cloned();
    config.write_root = a.automation.automation_write_root.clone();
    config.launch = std::env::current_exe().ok().and_then(|exe| photocraft_automation::live::launcher(&exe));
    Ok(config)
}

/// `mcp install --client <id>`: add this CLI (`mcp --live`) to an MCP client.
fn mcp_install(id: &str, out: &mut dyn Write) -> Result {
    use photocraft_automation::agent_link::{Client, ClientEnv, McpLaunch, install};
    use std::path::Path;
    let other = |m: String| crate::CliError::Other(m);
    let ids = Client::ALL.map(Client::id).join(", ");
    let client = Client::parse(id).ok_or_else(|| other(format!("unknown --client `{id}`: expected one of {ids}")))?;
    let exe = std::env::current_exe().map_err(|e| other(format!("can't find this program's path: {e}")))?;
    let launch = match std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
        Some(image) => McpLaunch::beside(Path::new(""), Some(Path::new(&image))),
        // Not canonicalized: Windows would turn it into a `\\?\` path some clients can't start.
        None => McpLaunch::cli(&exe),
    };
    let done = install(client, &launch, &ClientEnv::from_env())?;
    writeln!(out, "{}", done.message(client))?;
    Ok(())
}
