//! Toolkit-independent menu model. Plain data, serializable to the dbusmenu properties on
//! the wire; the exporter assigns item ids and diffs updates against the previous model.

use std::fmt;

/// A modifier in a keyboard shortcut, in canonical naming (`Ctrl`, `Alt`, `Shift`, `Cmd` —
/// `Cmd` being the platform's command modifier, Ctrl on Linux/Windows, ⌘ on macOS).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShortcutMod {
    Ctrl,
    Alt,
    Shift,
    Cmd,
}

/// A menu keyboard shortcut (modifiers plus the key, e.g. `N`, `F1`, `0`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub modifiers: Vec<ShortcutMod>,
    pub key: String,
}

impl Shortcut {
    /// Parses a display shortcut string like `"Ctrl+Shift+N"` or `"Cmd+Alt+W"`; `None` when
    /// the string is empty or malformed (never panics).
    pub fn parse(s: &str) -> Option<Shortcut> {
        let mut modifiers: Vec<ShortcutMod> = Vec::new();
        let mut key = String::new();
        for part in s.split('+') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match part.to_ascii_lowercase().as_str() {
                "cmd" => modifiers.push(ShortcutMod::Cmd),
                "ctrl" => modifiers.push(ShortcutMod::Ctrl),
                "alt" | "option" => modifiers.push(ShortcutMod::Alt),
                "shift" => modifiers.push(ShortcutMod::Shift),
                _ if key.is_empty() => key.push_str(part),
                // A second key (or unknown junk) is not a shortcut we can express.
                _ => return None,
            }
        }
        // Sort modifiers into the canonical Ctrl, Alt, Shift, Cmd order and drop duplicates.
        modifiers.sort();
        modifiers.dedup();
        if key.is_empty() { None } else { Some(Shortcut { modifiers, key }) }
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for m in &self.modifiers {
            write!(f, "{m:?}+")?;
        }
        write!(f, "{}", self.key)
    }
}

/// One application menu entry. Submenus nest through [`MenuEntry::command`]'s builder; the
/// registry maps every leaf back to its `action` token, which the application defines.
#[derive(Clone, Debug)]
pub enum MenuEntry {
    /// An item (leaf or submenu) carrying a label and an action token.
    Command(MenuCommand),
    /// A horizontal rule.
    Separator,
}

impl MenuEntry {
    /// A menu leaf: `label` (shown, e.g. "New…"), `action` (returned on click, e.g.
    /// `"file.new"`) and `enabled`.
    pub fn command(label: impl Into<String>, action: impl Into<String>, enabled: bool) -> MenuEntry {
        MenuEntry::Command(MenuCommand { label: label.into(), action: action.into(), enabled, checked: None, shortcut: None, children: None })
    }

    /// Adds a submenu to this item (`label`/`action` stay; it becomes a submenu title).
    /// Children are anything convertible into entries, so lists of `MenuCommand` work too.
    pub fn submenu<I, T>(self, children: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<MenuEntry>,
    {
        match self {
            MenuEntry::Command(mut c) => {
                c.children = Some(children.into_iter().map(Into::into).collect());
                MenuEntry::Command(c)
            }
            MenuEntry::Separator => self,
        }
    }

    /// Marks the item with a checkbox state (`Separator` ignores it).
    pub fn checked(self, checked: bool) -> Self {
        match self {
            MenuEntry::Command(mut c) => {
                c.checked = Some(checked);
                MenuEntry::Command(c)
            }
            MenuEntry::Separator => self,
        }
    }

    /// Adds a keyboard shortcut, parsed from display form (`"Ctrl+Shift+N"`); ignored when
    /// unparseable.
    pub fn shortcut(self, shortcut: impl AsRef<str>) -> Self {
        match self {
            MenuEntry::Command(mut c) => {
                c.shortcut = Shortcut::parse(shortcut.as_ref());
                MenuEntry::Command(c)
            }
            MenuEntry::Separator => self,
        }
    }
}

impl From<MenuCommand> for MenuEntry {
    fn from(c: MenuCommand) -> Self {
        MenuEntry::Command(c)
    }
}

impl From<bool> for MenuEntry {
    /// `true` for a separator, `false` for nothing (builders accept `Option`).
    fn from(_: bool) -> Self {
        MenuEntry::Separator
    }
}

/// A menu item: label, action token, state, optional shortcut, optional submenu.
#[derive(Clone, Debug)]
pub struct MenuCommand {
    pub label: String,
    pub action: String,
    pub enabled: bool,
    /// Checkbox/radio state: `None` for plain items.
    pub checked: Option<bool>,
    pub shortcut: Option<Shortcut>,
    /// `Some` when the item opens a submenu; `action` is ignored for such items.
    pub children: Option<Vec<MenuEntry>>,
}

impl MenuCommand {
    /// Adds a submenu (`label`/`action` still apply to the item itself). Children are
    /// anything convertible into entries, so lists of `MenuCommand` work directly.
    pub fn submenu<I, T>(mut self, children: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<MenuEntry>,
    {
        self.children = Some(children.into_iter().map(Into::into).collect());
        self
    }

    /// Marks the item with a checkbox state.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// Adds a keyboard shortcut, parsed from display form (`"Ctrl+Shift+N"`); ignored when
    /// unparseable.
    pub fn shortcut(mut self, shortcut: impl AsRef<str>) -> Self {
        self.shortcut = Shortcut::parse(shortcut.as_ref());
        self
    }
}

/// A published menu: the (invisible) root item's children are the top-level menus.
#[derive(Clone, Debug, Default)]
pub struct MenuModel {
    pub children: Vec<MenuEntry>,
}

impl MenuModel {
    pub fn top<I, T>(children: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<MenuEntry>,
    {
        MenuModel { children: children.into_iter().map(Into::into).collect() }
    }
}
