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
    let rt = mcp_runtime()?;
    Ok(rt.block_on(server.serve_stdio())?)
}

fn mcp_runtime() -> Result<tokio::runtime::Runtime> {
    let mut rt_builder = tokio::runtime::Builder::new_multi_thread();
    // A stdio MCP server is usually idle. Keep one async worker by default; image work runs on
    // the blocking pool, and Tokio still honors an explicitly configured worker count.
    if std::env::var_os("TOKIO_WORKER_THREADS").is_none() {
        rt_builder.worker_threads(1);
    }
    Ok(rt_builder.enable_all().build()?)
}

#[cfg(test)]
mod tests {
    use super::mcp_runtime;

    #[test]
    fn mcp_stdio_defaults_to_one_worker_and_honors_override() {
        if let Ok(expected) = std::env::var("PHOTOCRAFT_MCP_TEST_EXPECTED_WORKERS") {
            let expected = expected.parse::<usize>().unwrap();
            let workers = mcp_runtime().unwrap().metrics().num_workers();
            assert_eq!(workers, expected, "TOKIO_WORKER_THREADS={:?}", std::env::var_os("TOKIO_WORKER_THREADS"));
            return;
        }

        // Spawn the test in a fresh process for each environment: Rust 2024 cannot safely mutate
        // another test's process environment, and Tokio reads its override at runtime startup.
        for (worker_override, expected) in [(None, 1), (Some("2"), 2)] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "automation::tests::mcp_stdio_defaults_to_one_worker_and_honors_override"]);
            command.env_remove("TOKIO_WORKER_THREADS").env("PHOTOCRAFT_MCP_TEST_EXPECTED_WORKERS", expected.to_string());
            if let Some(workers) = worker_override {
                command.env("TOKIO_WORKER_THREADS", workers);
            }
            let output = command.output().unwrap();
            assert!(output.status.success(), "TOKIO_WORKER_THREADS={worker_override:?}: {}", String::from_utf8_lossy(&output.stdout));
        }
    }
}
