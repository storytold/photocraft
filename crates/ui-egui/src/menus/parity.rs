//! Photoshop menu parity: how much of [`crate::menu_catalog::CATALOG`] is live.
//!
//! `cargo xtask parity` prints this report and writes `docs/parity-checklist.md`. A test keeps the live
//! count from regressing below [`FLOOR`].

use crate::menu_catalog::CATALOG;
use crate::menus::is_live;

/// RFC-0052 hand marks for catalog ids that aren't engine commands: `(id, headless, reach, why)`.
///
/// `headless` is `no` or `partial`; `reach` is `window` (needs a desktop window, dialog or pointer),
/// `os` (needs an OS service the engine doesn't wrap) or `engine-gap` (could be an engine command,
/// isn't one yet). `yes` is never written here: an item is `yes` only when
/// `photocraft_engine::commands::find` has its id, the registry `photocraft-cli` and the MCP server
/// load. [`is_live`] also counts a dozen desktop-only sources; those make an item live in the menu,
/// not headless. Without a mark, an id the desktop serves as a panel, workspace or pointer tool is
/// `no` with `reach: window` ([`desktop_window`]); any other is `unknown`.
pub const HEADLESS: &[(&str, &str, &str, &str)] = &[];

/// Why a non-engine id needs a desktop window, when the desktop serves it as a panel, a workspace
/// or a pointer tool.
fn desktop_window(id: &str) -> Option<&'static str> {
    if super::invoke::panel_alias(id).is_some() || crate::panel_docking::toggle_panel(id).is_some() {
        Some("shows or hides a desktop panel")
    } else if super::invoke::workspace_id(id).is_some() || crate::workspace_ui::handles(id) {
        Some("a desktop workspace")
    } else if crate::tool_shortcuts::tool(id).is_some() {
        Some("picks a pointer tool")
    } else {
        None
    }
}

/// An item's RFC-0052 `(headless, reach, why)`: `yes` for an engine command, else its mark in
/// `marks`, else `no`/`window` for a desktop panel, workspace or tool, else `unknown`.
pub fn headless_of<'a>(id: &str, in_engine: bool, marks: &[(&'a str, &'a str, &'a str, &'a str)]) -> (&'a str, Option<&'a str>, &'a str) {
    if in_engine {
        return ("yes", None, "");
    }
    if let Some(m) = marks.iter().find(|m| m.0 == id) {
        return (m.1, Some(m.2), m.3);
    }
    match desktop_window(id) {
        Some(why) => ("no", Some("window"), why),
        None => ("unknown", None, ""),
    }
}

/// Problems with `marks` against `ids` (`(id, is an engine command)`): bad values, repeats, ids
/// not in the catalog, and marks on engine commands (they would hide a `yes`).
pub fn headless_errors(marks: &[(&str, &str, &str, &str)], ids: &[(&str, bool)]) -> Vec<String> {
    let mut errs = Vec::new();
    for (i, (id, value, reach, _)) in marks.iter().enumerate() {
        if !matches!(*value, "no" | "partial") {
            errs.push(format!("headless mark `{id}`: `{value}` is not `no` or `partial` (`yes` comes from the registry)"));
        }
        if !matches!(*reach, "window" | "os" | "engine-gap") {
            errs.push(format!("headless mark `{id}`: reach `{reach}` is not window, os or engine-gap"));
        }
        if marks[..i].iter().any(|m| m.0 == *id) {
            errs.push(format!("headless mark `{id}` is written twice"));
        }
        match ids.iter().find(|x| x.0 == *id) {
            None => errs.push(format!("headless mark `{id}` is not a catalog id")),
            Some((_, true)) => errs.push(format!("headless mark `{id}` is stale: it is an engine command, so it is `yes`")),
            Some(_) => {}
        }
    }
    errs
}

/// RFC-0052 counts over the catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct HeadlessCounts {
    pub yes: usize,
    pub no_window: usize,
    pub no_os: usize,
    pub no_engine_gap: usize,
    pub partial: usize,
    pub unknown: usize,
}

impl HeadlessCounts {
    fn add(&mut self, value: &str, reach: Option<&str>) {
        match (value, reach) {
            ("yes", _) => self.yes += 1,
            ("no", Some("window")) => self.no_window += 1,
            ("no", Some("os")) => self.no_os += 1,
            ("no", _) => self.no_engine_gap += 1,
            ("partial", _) => self.partial += 1,
            _ => self.unknown += 1,
        }
    }
}

/// Minimum number of live catalog items. Raise it when parity grows; never lower it.
pub const FLOOR: usize = 663;

/// One top-level menu's coverage.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MenuParity {
    pub menu: String,
    pub live: usize,
    pub total: usize,
    /// Labels (with submenu path) of items not implemented yet.
    pub missing: Vec<String>,
    /// Items that aren't `headless: yes`, with their value (RFC-0052).
    pub not_headless: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Parity {
    pub live: usize,
    pub total: usize,
    pub menus: Vec<MenuParity>,
    pub headless: HeadlessCounts,
}

impl Parity {
    pub fn percent(&self) -> f64 {
        if self.total == 0 { 0.0 } else { 100.0 * self.live as f64 / self.total as f64 }
    }

    /// The report as Markdown (the body of `docs/parity-checklist.md`).
    pub fn to_markdown(&self) -> String {
        let mut s = String::new();
        s.push_str("# Photoshop menu parity\n\n");
        s.push_str("Generated by `cargo xtask parity` from `crates/ui-egui/src/menu_catalog.rs` (Photoshop's menu tree) and the live command registry. Do not edit by hand.\n\n");
        s.push_str(&format!("**{} of {} menu items live ({:.1}%).**\n\n", self.live, self.total, self.percent()));
        s.push_str("| Menu | Live | Total | % |\n|---|---:|---:|---:|\n");
        for m in &self.menus {
            let pct = if m.total == 0 { 0.0 } else { 100.0 * m.live as f64 / m.total as f64 };
            s.push_str(&format!("| {} | {} | {} | {:.0}% |\n", m.menu, m.live, m.total, pct));
        }
        let h = &self.headless;
        s.push_str(&format!(
            "\n**Headless ([RFC-0052](https://github.com/storytold/craft-contrib/blob/main/rfcs/0052-headless-reach.md)): {} yes, {} no (window {}, os {}, engine-gap {}), {} partial, {} unknown.**\n\n",
            h.yes,
            h.no_window + h.no_os + h.no_engine_gap,
            h.no_window,
            h.no_os,
            h.no_engine_gap,
            h.partial,
            h.unknown
        ));
        s.push_str("Live counts the desktop app's own panels, workspaces, tools and view handlers. Headless counts only the engine registry that `photocraft-cli` and the MCP server load: an item is `yes` when its id is an engine command. The rest are listed under [Not headless](#not-headless), with the hand mark from `HEADLESS` in `crates/ui-egui/src/menus/parity.rs`, `no` (`window`) for a desktop panel, workspace or tool, or `unknown`.\n");
        s.push_str("\n## Not yet implemented\n");
        for m in self.menus.iter().filter(|m| !m.missing.is_empty()) {
            s.push_str(&format!("\n### {} ({})\n\n", m.menu, m.missing.len()));
            for item in &m.missing {
                s.push_str(&format!("- {item}\n"));
            }
        }
        s.push_str("\n## Not headless\n");
        for m in self.menus.iter().filter(|m| !m.not_headless.is_empty()) {
            s.push_str(&format!("\n### {} ({})\n\n", m.menu, m.not_headless.len()));
            for item in &m.not_headless {
                s.push_str(&format!("* {item}\n"));
            }
        }
        s
    }
}

/// Compute coverage of the Photoshop menu catalog (separators excluded; duplicate ids counted once).
pub fn compute() -> Parity {
    let mut menus: Vec<MenuParity> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut headless = HeadlessCounts::default();
    for &(path, label, _, id) in CATALOG {
        if id == "---" || !seen.insert(id) {
            continue;
        }
        let top = path.first().copied().unwrap_or("?");
        let i = match menus.iter().position(|m| m.menu == top) {
            Some(i) => i,
            None => {
                menus.push(MenuParity { menu: top.to_string(), live: 0, total: 0, missing: Vec::new(), not_headless: Vec::new() });
                menus.len() - 1
            }
        };
        let Some(m) = menus.get_mut(i) else { continue };
        m.total += 1;
        let mut full: Vec<&str> = path[1..].to_vec();
        full.push(label);
        let name = format!("{} (`{id}`)", full.join(" › "));
        let (value, reach, why) = headless_of(id, photocraft_engine::commands::find(id).is_some(), HEADLESS);
        headless.add(value, reach);
        if value != "yes" {
            let reach = reach.map(|r| format!(", reach: {r} ({why})")).unwrap_or_default();
            m.not_headless.push(format!("{name}: headless: {value}{reach}"));
        }
        if is_live(id) {
            m.live += 1;
        } else {
            m.missing.push(name);
        }
    }
    Parity { live: menus.iter().map(|m| m.live).sum(), total: menus.iter().map(|m| m.total).sum(), menus, headless }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parity_does_not_regress() {
        let p = compute();
        assert!(p.total > 500, "catalog shrank: {}", p.total);
        assert!(p.live >= FLOOR, "menu parity regressed: {} live < floor {FLOOR}", p.live);
    }

    #[test]
    fn headless_marks_are_current() {
        let ids: Vec<(&str, bool)> = CATALOG.iter().filter(|c| c.3 != "---").map(|c| (c.3, photocraft_engine::commands::find(c.3).is_some())).collect();
        let errs = headless_errors(HEADLESS, &ids);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn headless_yes_is_the_engine_registry_only() {
        let p = compute();
        let h = &p.headless;
        assert_eq!(h.yes + h.no_window + h.no_os + h.no_engine_gap + h.partial + h.unknown, p.total);
        assert!(h.yes < p.live, "desktop-only sources are live, not headless");
        let listed: usize = p.menus.iter().map(|m| m.not_headless.len()).sum();
        assert_eq!(listed, p.total - h.yes);
        // A panel toggle is live in the desktop app and never headless.
        let window = p.menus.iter().find(|m| m.menu == "Window").map(|m| m.not_headless.len()).unwrap_or(0);
        assert!(window > 0, "Window panels read as not headless");
        assert!(p.to_markdown().contains(&format!("{} yes,", h.yes)));
    }

    #[test]
    fn headless_values_and_bad_marks() {
        let marks = [("c.d", "no", "engine-gap", "could be a command")];
        assert_eq!(headless_of("c.d", true, &marks).0, "yes", "an engine id is yes whatever the mark says");
        assert_eq!(headless_of("c.d", false, &marks), ("no", Some("engine-gap"), "could be a command"));
        assert_eq!(headless_of("q.never", false, &marks).0, "unknown");
        let ids = [("a.b", true), ("c.d", false)];
        let bad = [("c.d", "yes", "os", ""), ("c.d", "no", "desk", ""), ("a.b", "no", "os", ""), ("z.z", "no", "os", "")];
        let errs = headless_errors(&bad, &ids);
        for want in ["is not `no` or `partial`", "reach `desk`", "written twice", "`a.b` is stale", "`z.z` is not a catalog id"] {
            assert!(errs.iter().any(|e| e.contains(want)), "{want}: {errs:?}");
        }
        let mut c = HeadlessCounts::default();
        for (v, r) in [("yes", None), ("no", Some("window")), ("no", Some("os")), ("no", Some("engine-gap")), ("partial", Some("os")), ("unknown", None)] {
            c.add(v, r);
        }
        assert_eq!(c, HeadlessCounts { yes: 1, no_window: 1, no_os: 1, no_engine_gap: 1, partial: 1, unknown: 1 });
    }

    #[test]
    fn markdown_lists_every_missing_item() {
        let p = compute();
        let md = p.to_markdown();
        // `- ` lines are the missing items; the Not headless list uses `* `.
        let listed = md.lines().filter(|l| l.starts_with("- ")).count();
        assert_eq!(listed, p.total - p.live);
    }
}
