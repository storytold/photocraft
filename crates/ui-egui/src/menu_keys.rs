//! The menu bar from the keyboard (#4988), as Photoshop's on Windows: a tap on Alt (or F10)
//! puts the keyboard on the bar (File lit; ← / → move along it, ↓ / ↩ / Space open the lit menu,
//! Esc or a click leaves), Alt+letter opens the menu with that mnemonic straight away, and in an
//! open menu a letter runs the row with that mnemonic (or opens its submenu).
//!
//! Mnemonics are underlined while Alt is held, the bar is focused or a menu was opened from the
//! keyboard; a menu opened with the mouse shows none until Alt is pressed, as Windows does.
//! They are assigned per menu level from the labels as shown (so translated), unique within
//! the level: Photoshop's letters for the top menus where the label has them (F E I L Y S T V
//! W H), else the first unused letter, word starts first. Only ASCII letters and digits are keys
//! the keyboard reports as such, so a label without one (Japanese, Chinese…) has no mnemonic.
//!
//! The state is view state in egui memory, like [`crate::menu_nav::Nav`]. On macOS the native
//! menu bar replaces the in-window one and this never runs.

use egui::{Color32, Event, Id, Key, Modifiers, Stroke, Ui};

use crate::menu_nav::Nav;

/// Photoshop's mnemonics for the top menus (Windows), used where the shown label has the letter.
pub const TOP_PREFERRED: [(&str, char); 10] = [
    ("File", 'F'),
    ("Edit", 'E'),
    ("Image", 'I'),
    ("Layer", 'L'),
    ("Type", 'Y'),
    ("Select", 'S'),
    ("Filter", 'T'),
    ("View", 'V'),
    ("Window", 'W'),
    ("Help", 'H'),
];

/// Keyboard state of the menu bar between frames.
#[derive(Clone, Debug, Default)]
pub struct Keys {
    /// The title the keyboard is on with no menu open, in screen order (Alt tap, F10).
    pub focus: Option<usize>,
    /// Underline the mnemonics: the bar is focused or a menu was opened from the keyboard.
    pub underline: bool,
    /// Alt is down (from its key events, whatever the platform reports as modifiers).
    alt_down: bool,
    /// Alt went down and nothing else was pressed since: releasing it is a tap.
    alt_tap: bool,
    /// A menu was just opened from the keyboard: light its first row once it is drawn.
    first_row: bool,
}

fn keys_id() -> Id {
    Id::new("pc-menu-bar-keys")
}

fn focus_id() -> Id {
    Id::new("pc-menu-bar-focus")
}

/// Is the keyboard on the menu bar (no menu open yet)? The shortcuts leave the keys to it then,
/// as they do for an open menu: ← / → / ↓ / ↩ / Esc and the letters drive the bar.
pub fn is_focused(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(focus_id())).is_some_and(|seen| seen + 2 >= pass)
}

/// The mnemonic a key press stands for: the letter or digit of a plain key.
fn key_char(key: Key) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

/// Mnemonics for `labels`, unique within the group: the preferred letter when the label has it,
/// else the first unused ASCII letter or digit, trying the first character of each word before
/// the others. Compared case-insensitively; the returned letter is upper case.
pub fn assign(labels: &[&str], preferred: &[Option<char>]) -> Vec<Option<char>> {
    let mut used: Vec<char> = Vec::new();
    let mut out = Vec::with_capacity(labels.len());
    for (i, label) in labels.iter().enumerate() {
        let upper: Vec<char> = label.chars().map(|c| c.to_ascii_uppercase()).collect();
        let has = |c: char| upper.contains(&c.to_ascii_uppercase());
        let mut pick = preferred.get(i).copied().flatten().map(|c| c.to_ascii_uppercase()).filter(|&c| has(c) && !used.contains(&c));
        if pick.is_none() {
            let starts = label.split(|c: char| !c.is_alphanumeric()).filter_map(|w| w.chars().next());
            let rest = label.chars();
            pick = starts.chain(rest).filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).find(|c| !used.contains(c));
        }
        if let Some(c) = pick {
            used.push(c);
        }
        out.push(pick);
    }
    out
}

/// `prefix + label` as menu text with the mnemonic's first occurrence in `label` underlined
/// (none when `mnemonic` is None). `color` is the text colour; None takes the widget's.
pub fn mnemonic_text(ui: &Ui, prefix: &str, label: &str, mnemonic: Option<char>, color: Option<Color32>) -> egui::WidgetText {
    let color = color.unwrap_or(Color32::PLACEHOLDER);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let plain = egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
    let at = mnemonic.and_then(|m| label.char_indices().find(|(_, c)| c.to_ascii_uppercase() == m).map(|(i, c)| (i, c.len_utf8())));
    let Some((i, len)) = at else {
        return egui::RichText::new(format!("{prefix}{label}")).color(color).into();
    };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let underlined = egui::TextFormat { underline: Stroke::new(1.0, color), ..plain.clone() };
    let before = format!("{prefix}{}", &label[..i]);
    if !before.is_empty() {
        job.append(&before, 0.0, plain.clone());
    }
    job.append(&label[i..i + len], 0.0, underlined);
    if i + len < label.len() {
        job.append(&label[i + len..], 0.0, plain);
    }
    job.into()
}

impl Keys {
    /// Last frame's state.
    pub fn load(ctx: &egui::Context) -> Keys {
        ctx.data(|d| d.get_temp(keys_id())).unwrap_or_default()
    }

    /// The state as the last frame left it (tests, inspection).
    pub fn current(ctx: &egui::Context) -> Keys {
        Self::load(ctx)
    }

    pub fn store(self, ctx: &egui::Context) {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| {
            if self.focus.is_some() {
                d.insert_temp(focus_id(), pass);
            } else {
                d.remove::<u64>(focus_id());
            }
            d.insert_temp(keys_id(), self);
        });
    }

    /// Underline the mnemonics this frame?
    pub fn show_underlines(&self, ctx: &egui::Context) -> bool {
        self.underline || self.focus.is_some() || self.alt_down || ctx.input(|i| i.modifiers.alt)
    }

    /// A menu just opened from the keyboard: light its first row (once, when it is drawn).
    pub fn take_first_row(&mut self, nav: &mut Nav) {
        if self.first_row && nav.top.is_some() {
            nav.highlight_first(0);
            self.first_row = false;
        }
    }

    /// Handle this frame's keys, after the menus were drawn. `tops` are the titles' popup ids
    /// and `mnemonics` their letters, both in screen order; `nav` is the open menu's state. A
    /// command a mnemonic ran lands in `clicked`.
    pub fn keys(&mut self, ctx: &egui::Context, tops: &[Id], mnemonics: &[Option<char>], nav: &mut Nav, clicked: &mut Option<String>) {
        if tops.is_empty() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        let mut tap = false;
        for e in &events {
            match e {
                Event::Key { key: Key::AltLeft | Key::AltRight, pressed: true, .. } => {
                    self.alt_down = true;
                    self.alt_tap = true;
                }
                Event::Key { key: Key::AltLeft | Key::AltRight, pressed: false, .. } => {
                    self.alt_down = false;
                    tap |= std::mem::take(&mut self.alt_tap);
                }
                // Alt+wheel (zoom, brush size…) or Alt going to the desktop is no tap either.
                Event::Key { pressed: true, .. }
                | Event::Text(_)
                | Event::PointerButton { pressed: true, .. }
                | Event::MouseWheel { .. }
                | Event::WindowFocused(false) => self.alt_tap = false,
                _ => {}
            }
        }
        if events.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, .. })) {
            self.focus = None;
            self.underline = false;
        }
        let open = nav.top.is_some();
        let n = tops.len();
        let open_top = |i: usize, keys: &mut Keys| {
            if let Some(&id) = tops.get(i) {
                egui::Popup::open_id(ctx, id);
                keys.focus = None;
                keys.underline = true;
                keys.first_row = true;
                ctx.request_repaint();
            }
        };
        let consume = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        // Alt+letter opens that menu, open or not (an open one switches), as on Windows.
        let alt_letter = events.iter().find_map(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } if modifiers.alt && !modifiers.command && !modifiers.ctrl => key_char(*key).map(|c| (*key, c)),
            _ => None,
        });
        if let Some((key, c)) = alt_letter
            && let Some(i) = mnemonics.iter().position(|m| *m == Some(c))
        {
            consume(Modifiers::ALT, key);
            self.alt_tap = false;
            open_top(i, self);
            return;
        }
        if open {
            // A letter runs the row with that mnemonic in the level the arrows are in.
            let letter = events.iter().find_map(|e| match e {
                Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.ctrl && !modifiers.alt => {
                    key_char(*key).map(|c| (*key, c))
                }
                _ => None,
            });
            if let Some((key, c)) = letter
                && let Some(row) = nav.row_with_mnemonic(c)
            {
                consume(Modifiers::NONE, key);
                self.underline = true;
                if let Some(cmd) = row.command.clone() {
                    *clicked = Some(cmd);
                    egui::Popup::close_all(ctx);
                } else {
                    nav.open_sub_row(ctx, row.widget);
                }
                ctx.request_repaint();
            }
            if tap {
                egui::Popup::close_all(ctx);
                self.underline = false;
                ctx.request_repaint();
            }
            return;
        }
        // No menu open: Alt (tapped) or F10 puts the keyboard on the bar or takes it off again.
        let f10 = consume(Modifiers::NONE, Key::F10);
        if tap || f10 {
            self.focus = if self.focus.is_some() { None } else { Some(0) };
            self.underline = false;
            ctx.request_repaint();
            return;
        }
        let Some(focus) = self.focus else { return };
        let press = |k: Key| consume(Modifiers::NONE, k);
        if press(Key::Escape) {
            self.focus = None;
        } else if press(Key::ArrowRight) {
            self.focus = Some((focus + 1) % n);
        } else if press(Key::ArrowLeft) {
            self.focus = Some((focus + n - 1) % n);
        } else if press(Key::ArrowDown) || press(Key::Enter) || press(Key::Space) {
            open_top(focus, self);
        } else if let Some((key, c)) = events.iter().find_map(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } if modifiers.is_none() => key_char(*key).map(|c| (*key, c)),
            _ => None,
        }) {
            if let Some(i) = mnemonics.iter().position(|m| *m == Some(c)) {
                consume(Modifiers::NONE, key);
                open_top(i, self);
            }
        } else {
            return;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_unique_and_prefer_photoshop_s() {
        let labels = ["File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"];
        let preferred: Vec<Option<char>> = TOP_PREFERRED.iter().map(|(_, c)| Some(*c)).collect();
        assert_eq!(assign(&labels, &preferred), "FEILYSTVWH".chars().map(Some).collect::<Vec<_>>());
        // Items: first letters first, then the next free letter of the word starts, then any.
        assert_eq!(
            assign(&["New…", "Open…", "Open Recent", "Close", "Close All", "Save", "Save As…"], &[]),
            [Some('N'), Some('O'), Some('R'), Some('C'), Some('A'), Some('S'), Some('V'),]
        );
        // A preferred letter the label lacks (a translation) falls back; a taken one too.
        assert_eq!(assign(&["Datei", "Bearbeiten"], &[Some('F'), Some('E')]), [Some('D'), Some('E')]);
        assert_eq!(assign(&["Aa", "Ab", "Ba"], &[Some('A'), Some('A'), Some('A')]), [Some('A'), Some('B'), None]);
        // Only ASCII letters and digits are keys: no mnemonic without one.
        assert_eq!(assign(&["ファイル", "Zoom 100%", "✔ Rulers"], &[]), [None, Some('Z'), Some('R')]);
    }

    #[test]
    fn key_chars_are_plain_letters_and_digits() {
        assert_eq!(key_char(Key::A), Some('A'));
        assert_eq!(key_char(Key::Num7), Some('7'));
        assert_eq!(key_char(Key::F10), None);
        assert_eq!(key_char(Key::ArrowDown), None);
        assert_eq!(key_char(Key::Space), None);
    }
}

#[cfg(test)]
#[path = "menu_keys_tests.rs"]
mod harness_tests;
