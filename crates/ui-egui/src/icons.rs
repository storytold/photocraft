//! PhotoCraft's icon names. The glyphs come from the framework's catalog and from the app's own set in
//! `icon_data.rs`, registered once by [`register`]; drawing is the framework's `Icon` / `IconSpec::svg`.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

use craft::behavior::IconId;
use craft::ui::contract::CommandSet;
use craft::ui::primitives::register_app_icons;

use crate::icon_data::PHOTOCRAFT_ICONS;
use crate::panels::tool_commands::{ToolCommand, command_of};
use crate::state::Tool;

/// Registers the app's own icon set with the framework. The registry is process-wide and ignores a set
/// registered twice, so a second window or a test may call it again.
pub fn register() {
    register_app_icons(&PHOTOCRAFT_ICONS);
}

/// Panel footer buttons share one glyph per action and one size, so the Layers footer and the
/// Swatches, Gradients and Patterns footers read the same, as in Photoshop (#10656).
pub const FOOTER_SIZE: f32 = 26.0;
/// The footer's "new" glyph.
pub const FOOTER_NEW: &str = "square_plus";
/// The footer's "new group" glyph.
pub const FOOTER_NEW_GROUP: &str = "folder";
/// The footer's "delete" glyph.
pub const FOOTER_DELETE: &str = "trash";

/// The icon of tool `t`: the one its command names (`ToolCommand`'s `icon` attribute), which the tool rail also draws.
pub fn tool_icon(t: Tool) -> IconId {
    static ICONS: OnceLock<HashMap<String, Cow<'static, str>>> = OnceLock::new();
    let icons =
        ICONS.get_or_init(|| ToolCommand::defs().unwrap_or_default().into_iter().filter_map(|def| def.icon.map(|icon| (def.command.id, icon))).collect());
    icons.get(command_of(t).id()).map_or_else(|| IconId::new("more"), |name| IconId::from(name.clone()))
}

#[cfg(test)]
mod tests {
    use craft::ui::primitives::has_icon;

    use super::*;
    use crate::icon_data::ICONS;

    /// A missing glyph paints nothing and only logs, so a call site left on a renamed (hyphenated)
    /// name is caught here: every name PhotoCraft asks for resolves once the app set is registered.
    #[test]
    fn every_name_photocraft_asks_for_resolves() {
        register();
        register();
        for t in Tool::ALL {
            assert!(has_icon(&tool_icon(t)), "{t:?}: {}", tool_icon(t).name());
        }
        // The rail draws each command's own icon, so every command's attribute resolves too.
        for def in ToolCommand::defs().expect("tool commands define") {
            let name = def.icon.unwrap_or_else(|| panic!("{} names no icon", def.command.id));
            assert!(has_icon(&IconId::from(name.clone())), "{}: {name}", def.command.id);
        }
        for name in [FOOTER_NEW, FOOTER_NEW_GROUP, FOOTER_DELETE, "symmetry_vertical", "symmetry_horizontal", "symmetry_diagonal", "symmetry_dual"] {
            assert!(has_icon(&IconId::new(name)), "{name}");
        }
        for &(name, _) in ICONS {
            assert!(has_icon(&IconId::new(name)), "{name}");
        }
    }

    /// Every panel's iconic form draws the registry's icon, so each registry name resolves.
    #[test]
    fn every_panel_icon_resolves() {
        register();
        for panel in crate::panels::registry::PANELS {
            assert!(has_icon(&IconId::new(panel.icon)), "panel {}: icon {:?} is not in the catalog", panel.id, panel.icon);
        }
    }

    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("source dir reads").flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    /// An unknown id paints nothing and only logs, so every id written as a string literal in the sources must resolve.
    /// Ids built in match arms are not scanned.
    #[test]
    fn every_icon_id_literal_in_the_sources_resolves() {
        register();
        let form = concat!("IconId", "::new(\"");
        let mut files = Vec::new();
        sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        let (mut seen, mut unknown) = (0, Vec::new());
        for file in files.iter().filter(|file| !file.ends_with("icons.rs")) {
            let text = std::fs::read_to_string(file).expect("source reads");
            for (number, line) in text.lines().enumerate().filter(|(_, line)| !line.trim_start().starts_with("//")) {
                for (at, _) in line.match_indices(form) {
                    let rest = &line[at + form.len()..];
                    let Some(end) = rest.find('"') else { continue };
                    seen += 1;
                    if !has_icon(&IconId::new(String::leak(rest[..end].to_owned()))) {
                        unknown.push(format!("{}:{}: {:?}", file.display(), number + 1, &rest[..end]));
                    }
                }
            }
        }
        assert!(seen > 20, "the scan found {seen} icon ids: its pattern no longer matches the sources");
        assert!(unknown.is_empty(), "icon ids that are not in the catalog:\n{}", unknown.join("\n"));
    }
}
