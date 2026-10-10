//! Dock modules: the panels that live in the right dock (Layers, History, Color, …).
//!
//! Each module is one file in this directory defining `pub const MODULE: Module`. `build.rs`
//! finds the files and lists them in [`ALL`], so adding a panel is adding a file: the dock,
//! the Window menu, the icon rail, the panel picker and saved layouts pick it up by its id.
//!
//! A module only describes itself and draws its body; where it sits (which pane, which tab,
//! how tall) is [`crate::dock::DockLayout`]'s business.

use crate::PhotocraftApp;

include!(concat!(env!("OUT_DIR"), "/modules.rs"));

/// How tall a module's pane wants to be (points, tab strip included).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    /// Height until the user resizes it, when the column has room.
    pub default: f32,
    /// What it gives way down to so the filling pane keeps its preferred height.
    pub compact: f32,
    /// The smallest height it can be dragged or squeezed to.
    pub min: f32,
    /// The height it asks for when it is the pane filling the column's rest.
    pub fill: f32,
}

impl Size {
    pub const fn new(default: f32, compact: f32, min: f32) -> Self {
        Self { default, compact, min, fill: min }
    }
}

/// One dock module.
pub struct Module {
    /// Stable id, used in saved layouts, `ui.set` and the control channel (`layers`, `layerComps`).
    pub id: &'static str,
    /// English title (tab label, picker, rail tooltip); translated when drawn.
    pub title: &'static str,
    /// Icon name (`icons::image`) for the rail and the picker.
    pub icon: &'static str,
    /// Menu items that show or hide it (Window › Layers is `window.panel.layers`).
    pub menu_ids: &'static [&'static str],
    /// Modules it joins when reopened and none of its pane is left (Channels joins Layers).
    pub siblings: &'static [&'static str],
    pub size: Size,
    /// The body lays out its own scrolling list and footer, so it gets the full pane height
    /// instead of a scroll area (Layers, History).
    pub fills: bool,
    /// Draw the body.
    pub body: fn(&mut PhotocraftApp, &mut egui::Ui),
    /// Extra items at the top of the pane's panel menu (≡) while this module is the front tab.
    pub menu: Option<fn(&mut PhotocraftApp, &mut egui::Ui)>,
}

/// The module with this id.
pub fn get(id: &str) -> Option<&'static Module> {
    ALL.iter().copied().find(|m| m.id == id)
}

/// The module a menu item shows or hides.
pub fn by_menu_id(menu_id: &str) -> Option<&'static Module> {
    ALL.iter().copied().find(|m| m.menu_ids.contains(&menu_id))
}

/// A module's title, translated.
pub fn title(id: &str) -> &'static str {
    get(id).map_or("", |m| tl!(m.title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_menu_ids_are_unique() {
        let mut ids = std::collections::HashSet::new();
        let mut menus = std::collections::HashSet::new();
        for m in ALL {
            assert!(ids.insert(m.id), "duplicate module id {}", m.id);
            for id in m.menu_ids {
                assert!(menus.insert(*id), "{id} opens two modules");
            }
        }
        assert!(ALL.len() >= 17, "build.rs found only {} modules", ALL.len());
    }

    #[test]
    fn modules_point_at_real_icons_siblings_and_menu_items() {
        for m in ALL {
            assert!(crate::icons::exists(m.icon), "{}: no icon {}", m.id, m.icon);
            for s in m.siblings {
                assert!(get(s).is_some(), "{}: unknown sibling {s}", m.id);
            }
            for id in m.menu_ids {
                assert!(crate::menu_catalog::CATALOG.iter().any(|(_, _, _, cid)| cid == id), "{}: {id} is not a menu item", m.id);
            }
            assert!(m.size.min > 0.0 && m.size.min <= m.size.compact && m.size.compact <= m.size.default, "{}: {:?}", m.id, m.size);
        }
    }

    /// Titles are dynamic labels the `tl!` literal scanner cannot see.
    #[test]
    fn titles_are_translated() {
        for l in crate::i18n::LANGUAGES.iter().filter(|l| l.complete_menus) {
            let missing: Vec<_> = ALL.iter().map(|m| m.title).filter(|t| l.catalog().plain(t).is_none()).collect();
            assert!(missing.is_empty(), "{}: untranslated module titles {missing:?}", l.code);
        }
    }
}
