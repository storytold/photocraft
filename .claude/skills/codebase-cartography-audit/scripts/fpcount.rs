//! Cartography-audit IFPUG transaction counts (EI / EQ / EO) from PhotoCraft's command registry.
//!
//! Single-file, std-only Rust: build with `rustc -O --edition 2021 fpcount.rs -o <bin>`.
//!
//! Usage: fpcount <commands.json>
//!   commands.json  output of `cargo run -q -p photocraft-cli -- commands --json`
//!
//! Rule (printed with the result so the basis is auditable):
//!   1. Families that differ only by a parameter collapse to one elementary process:
//!      a trailing `More` (blur/blurMore), `slotN` (channel.target.slot3..9) and
//!      `bits8|16|32` (image.mode.bitsN) are stripped before de-duplicating.
//!   2. EQ (retrieval, no derivation): last id segment is info/list/get/inspect/printers/
//!      browse, ends in `Info`, or starts with `list`.
//!   3. EO (derived output to a file/printer): id contains export, print (not printers),
//!      saveACopy, saveContents, presets.save, actions.save, package, contactSheet,
//!      renderVideo or generate.imageAssets.
//!   4. EI: every other command (state-changing elementary process).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: fpcount <commands.json>");
        std::process::exit(2);
    };
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("fpcount: read {path}: {e}");
            std::process::exit(1);
        }
    };
    let ids = extract_ids(&text);
    if ids.is_empty() {
        eprintln!("fpcount: no \"id\" fields found in {path}");
        std::process::exit(1);
    }

    let mut sets: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut by_prefix: BTreeMap<String, [usize; 3]> = BTreeMap::new();
    for id in &ids {
        let norm = normalize(id);
        let kind = classify(id);
        if sets.entry(kind).or_default().insert(norm) {
            let p = id.split('.').next().unwrap_or("").to_string();
            let slot = match kind { "EI" => 0, "EQ" => 1, _ => 2 };
            if let Some(v) = by_prefix.entry(p).or_default().get_mut(slot) {
                *v += 1;
            }
        }
    }
    let n = |k: &str| sets.get(k).map_or(0, |s| s.len());
    let unique = n("EI") + n("EQ") + n("EO");

    println!("### Transaction functions from the command registry\n");
    println!("Registry commands: {}; after collapsing parameter-only families: {}.\n", ids.len(), unique);
    println!("| Type | Count | Rule |\n|---|--:|---|");
    println!("| EI | {} | every state-changing command not matched below |", n("EI"));
    println!("| EQ | {} | last segment info/list/get/inspect/printers/browse, `*Info`, `list*` |", n("EQ"));
    println!("| EO | {} | export, print, saveACopy, saveContents, presets/actions save, package, contactSheet, renderVideo, imageAssets |", n("EO"));
    println!("\nCollapsed: trailing `More`, `slotN`, `bits8/16/32` variants count once.\n");
    println!("| Command prefix | EI | EQ | EO |\n|---|--:|--:|--:|");
    for (p, v) in &by_prefix {
        println!("| {p} | {} | {} | {} |", v[0], v[1], v[2]);
    }
    for k in ["EQ", "EO"] {
        if let Some(s) = sets.get(k) {
            println!("\n{k} commands: {}", s.iter().cloned().collect::<Vec<_>>().join(", "));
        }
    }
}

/// Pull every `"id": "<value>"` string out of the JSON without a JSON parser.
fn extract_ids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(p) = rest.find("\"id\"") {
        rest = rest.get(p + 4..).unwrap_or("");
        let after = rest.trim_start();
        let Some(after) = after.strip_prefix(':') else { continue };
        let after = after.trim_start();
        let Some(after) = after.strip_prefix('"') else { continue };
        if let Some(end) = after.find('"') {
            out.push(after.get(..end).unwrap_or("").to_string());
        }
    }
    out
}

fn normalize(id: &str) -> String {
    let mut s = id.to_string();
    if let Some(stripped) = s.strip_suffix("More") {
        s = stripped.to_string();
    }
    for pre in ["slot", "bits"] {
        if let Some(pos) = s.rfind(pre) {
            let tail = s.get(pos + pre.len()..).unwrap_or("");
            if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
                s.truncate(pos + pre.len());
            }
        }
    }
    s
}

fn classify(id: &str) -> &'static str {
    let last = id.rsplit('.').next().unwrap_or(id);
    if matches!(last, "info" | "list" | "get" | "inspect" | "printers" | "browse")
        || last.ends_with("Info")
        || last.starts_with("list")
    {
        return "EQ";
    }
    let eo = ["export", "saveACopy", "saveContents", "presets.save", "actions.save", "contactSheet", "renderVideo", "generate.imageAssets"];
    if eo.iter().any(|k| id.contains(k)) || (id.contains("print") && !id.ends_with("printers")) || last == "package" {
        return "EO";
    }
    "EI"
}
