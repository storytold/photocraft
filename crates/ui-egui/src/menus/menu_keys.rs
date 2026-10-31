//! The menu bar from the keyboard (#4988), as Photoshop's on Windows: a tap on Alt (or F10)
//! puts the keyboard on the bar (File lit; ← / → move along it, ↓ / ↩ open the lit menu,
//! Esc, Space or a click leaves), Alt+letter opens the menu with that mnemonic straight away, and in an
//! open menu a letter runs the row with that mnemonic (or opens its submenu).
//!
//! Mnemonics are underlined while Alt is held, the bar is focused or a menu was opened from the
//! keyboard; a menu opened with the mouse shows none until Alt is pressed, as Windows does.
//! They are assigned per menu level from the labels as shown (so translated), unique within
//! the level: Photoshop's letters for the top menus where the label has them (F E I L Y S T V
//! W H), else the first unused letter, word starts first. Only ASCII letters and digits are keys
//! the keyboard reports as such, so a label without one shows its English label's letter after it,
//! as Windows menus do in Chinese, Japanese and Korean: "文件(F)" ([`localized`]). A right-to-left
//! language keeps its label instead and shows the key only while the mnemonics show, as a
//! transient mark after the label ([`mnemonic_text`], [`menu_row_text`]).
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
    /// The title the keyboard is on with no menu open, in the menus' logical order (Alt tap, F10).
    pub focus: Option<usize>,
    /// Underline the mnemonics: the bar is focused or a menu was opened from the keyboard.
    pub underline: bool,
    /// Alt is down (from its key events, whatever the platform reports as modifiers).
    alt_down: bool,
    /// Alt went down and nothing else was pressed since: releasing it is a tap.
    alt_tap: bool,
    /// The mouse buttons that are down, one bit each, from the button events in order (the
    /// frame's pointer state is its end state, which a whole click within one frame doesn't show).
    buttons_held: u8,
    /// A menu was just opened from the keyboard: light its first row once it is drawn.
    first_row: bool,
}

fn keys_id() -> Id {
    Id::unique("pc-menu-bar-keys")
}

fn focus_id() -> Id {
    Id::unique("pc-menu-bar-focus")
}

/// Is the keyboard on the menu bar (no menu open yet)? The shortcuts leave the keys to it then,
/// as they do for an open menu: ← / → / ↓ / ↩ / Esc and the letters drive the bar.
pub fn is_focused(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(focus_id())).is_some_and(|seen| seen + 2 >= pass)
}

/// A command shortcut pressed while the keyboard is on the bar with no menu open (⌘D, ⌘⇧I, F7…)
/// takes it off the bar, so the shortcut runs in the same press, as a native menu bar does
/// (#6573). Keys the bar uses (arrows, ↩, Esc, letters, Alt+letter, F10) keep driving it, and
/// an open menu still swallows shortcuts. True when the bar was left.
pub fn leave_for_shortcut(ctx: &egui::Context) -> bool {
    if !is_focused(ctx) || crate::menu_nav::is_open(ctx) {
        return false;
    }
    let shortcut = ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::Key { key, pressed: true, modifiers, .. } if is_command_key(*key, *modifiers))));
    if !shortcut {
        return false;
    }
    let mut keys = Keys::load(ctx);
    keys.focus = None;
    keys.underline = false;
    keys.store(ctx);
    ctx.request_repaint();
    true
}

/// A key press that is a command shortcut rather than a key of the bar: ⌘ / Ctrl with a key
/// (not the modifier keys themselves), or a function key other than F10 (which toggles the bar).
fn is_command_key(key: Key, mods: Modifiers) -> bool {
    let modifier_key = matches!(
        key,
        Key::ShiftLeft | Key::ShiftRight | Key::ControlLeft | Key::ControlRight | Key::AltLeft | Key::AltRight | Key::SuperLeft | Key::SuperRight
    );
    if modifier_key {
        return false;
    }
    let function_key = key != Key::F10 && key.name().strip_prefix('F').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    mods.command || mods.ctrl || mods.mac_cmd || function_key
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
/// the others. Compared case-insensitively; the returned letter is upper case. `reserved` letters
/// are taken already (keys shown by other rows).
pub fn assign(labels: &[&str], preferred: &[Option<char>], reserved: &[char]) -> Vec<Option<char>> {
    let mut used: Vec<char> = reserved.to_vec();
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

/// The labels to show and their mnemonics for translated `labels` whose English originals are
/// `english` (same order). A label with an ASCII letter or digit keeps its text and is assigned
/// as [`assign`] does. One without (Chinese, Japanese, Korean…) can't be typed on its own, so it
/// shows its English label's letter after it in parentheses and takes that letter, as Windows menus
/// do in those languages ("文件(F)", #5863). In a right-to-left language the label is shown
/// unchanged (the key comes only while the mnemonics show, in the row-text helpers) and the
/// English letter is still its key, so Alt+letter keeps firing. Mnemonics stay unique within the
/// group, and a shown "(F)" is always the row's key.
pub fn localized(labels: &[String], english: &[&str], preferred: &[Option<char>]) -> (Vec<String>, Vec<Option<char>>) {
    let rtl = crate::direction::is_rtl();
    let typed = |l: &str| l.chars().any(|c| c.is_ascii_alphanumeric());
    // The key goes before a trailing ellipsis, as Windows writes it: "新建(N)…".
    let with_key = |l: &str, k: char| {
        let (base, tail) = split_trailing_ellipsis(l);
        format!("{base}({k}){tail}")
    };
    let english_keys = assign(english, preferred, &[]);
    let mut shown = Vec::with_capacity(labels.len());
    let mut pref = Vec::with_capacity(labels.len());
    for (i, label) in labels.iter().enumerate() {
        match english_keys.get(i).copied().flatten().filter(|_| !typed(label)) {
            Some(k) if !rtl => {
                shown.push(with_key(label, k));
                pref.push(Some(k));
            }
            // The label renders unchanged; the English key stays the row's mnemonic, just never
            // written into the label.
            Some(k) => {
                shown.push(label.clone());
                pref.push(Some(k));
            }
            None => {
                shown.push(label.clone());
                pref.push(preferred.get(i).copied().flatten());
            }
        }
    }
    // Letter-less rows take their English key before the lettered rows assign (the old
    // suffixed-first priority): in a right-to-left UI nothing is suffixed, so `assign` alone
    // would wipe their keys, and in Chinese a label with letters of its own must not take the key
    // a suffix shows ("Camera Raw 滤镜…" before "转换为智能滤镜(C)" leaves C to the latter). The
    // test is on the original labels: a shown suffix carries the key's letter itself.
    let mut keys: Vec<Option<char>> =
        (0..shown.len()).map(|i| (!typed(labels.get(i).map_or("", String::as_str))).then(|| english_keys.get(i).copied().flatten()).flatten()).collect();
    let reserved: Vec<char> = keys.iter().flatten().copied().collect();
    let rest: Vec<usize> = (0..shown.len()).filter(|&i| keys.get(i).copied().flatten().is_none()).collect();
    let ordered: Vec<&str> = rest.iter().map(|&i| shown.get(i).map_or("", String::as_str)).collect();
    let ordered_pref: Vec<Option<char>> = rest.iter().map(|&i| pref.get(i).copied().flatten()).collect();
    for (&i, key) in rest.iter().zip(assign(&ordered, &ordered_pref, &reserved)) {
        if let Some(slot) = keys.get_mut(i) {
            *slot = key;
        }
    }
    (shown, keys)
}

/// Splits a trailing ellipsis off a label, where a key or transient mark goes before it:
/// `("فتح", "…")`, `("Open", "...")`, or `("Open", "")` when there is none.
fn split_trailing_ellipsis(label: &str) -> (&str, &str) {
    label.strip_suffix('…').map(|base| (base, "…")).or_else(|| label.strip_suffix("...").map(|base| (base, "..."))).unwrap_or((label, ""))
}

/// A label split into its base and the transient `" (M)"` mark that goes before a trailing
/// ellipsis while the mnemonics show: `("فتح", " (M)…")`.
fn transient_mark(label: &str, m: char) -> (String, String) {
    let (base, tail) = split_trailing_ellipsis(label);
    (base.to_owned(), format!(" ({m}){tail}"))
}

/// `prefix + label` as menu text with the mnemonic's first occurrence in `label` underlined
/// (none when `mnemonic` is None). `color` is the text colour; None takes the widget's. While the
/// mnemonics show (`visible`), a label without its letter gains a transient `" (M)"` before a
/// trailing ellipsis (an Arabic label keeps its key discoverable); without them the text is plain.
pub fn mnemonic_text(ui: &Ui, prefix: &str, label: &str, mnemonic: Option<char>, visible: bool, color: Option<Color32>) -> egui::WidgetText {
    let color = color.unwrap_or(Color32::PLACEHOLDER);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let plain = egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
    let at = mnemonic.and_then(|m| label.char_indices().find(|(_, c)| c.to_ascii_uppercase() == m).map(|(i, c)| (i, c.len_utf8())));
    let Some((i, len)) = at.filter(|_| visible) else {
        let marked = match mnemonic.filter(|_| visible) {
            Some(m) => {
                let (base, mark) = transient_mark(label, m);
                format!("{prefix}{base}{mark}")
            }
            None => format!("{prefix}{label}"),
        };
        return egui::RichText::new(marked).color(color).into();
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

/// The checkmark menu rows draw in their gutter.
pub const CHECK_MARK: &str = "✔ ";

/// Width of the checkmark column every menu row reserves (#5705): the width of [`CHECK_MARK`] in
/// the button font, so checked, unchecked and plain rows (and submenu headers) all start their
/// label at the same x, whatever the script of the label.
pub fn check_gutter(ui: &Ui) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    ui.painter().layout_no_wrap(CHECK_MARK.to_owned(), font, Color32::PLACEHOLDER).size().x
}

/// Which end of a menu row the checkmark column sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterSide {
    /// The row's reading start: its left edge in a left-to-right row, its right edge in a
    /// right-to-left one, where native menus keep the column.
    ReadingStart,
    /// The row's reading end.
    ReadingEnd,
}

/// A menu row's text (#5705): the label behind a fixed-width checkmark column ([`check_gutter`])
/// on `side`, with ✔ in the column when `checked`, and the mnemonic underlined while
/// `mnemonic_visible` — a label without its letter then shows a transient `" (M)"`, as
/// [`mnemonic_text`] does.
pub fn menu_row_text(ui: &Ui, checked: bool, label: &str, mnemonic: Option<char>, mnemonic_visible: bool, side: GutterSide) -> egui::WidgetText {
    let rtl = ui.style().right_to_left;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let plain = egui::TextFormat { font_id: font, color: Color32::PLACEHOLDER, ..Default::default() };
    let underlined = egui::TextFormat { underline: Stroke::new(1.0, Color32::PLACEHOLDER), ..plain.clone() };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    // A right-to-left row reads from the right: the base direction is pinned so the column stays
    // on the reading-start edge whatever script the label starts with (a Latin or digit-first
    // label would infer a left-to-right paragraph and move it back).
    if rtl {
        job.direction = egui::epaint::text::TextDirection::RightToLeft;
    }
    let at = mnemonic.and_then(|m| label.char_indices().find(|(_, c)| c.to_ascii_uppercase() == m).map(|(i, c)| (i, c.len_utf8())));
    // The label's pieces in reading order: the underlined letter, or the transient mark when the
    // label can't hold it and the mnemonics show.
    let mut pieces: Vec<(String, egui::TextFormat)> = match at.filter(|_| mnemonic_visible) {
        Some((i, len)) => vec![
            (label.get(..i).unwrap_or_default().to_owned(), plain.clone()),
            (label.get(i..i + len).unwrap_or_default().to_owned(), underlined),
            (label.get(i + len..).unwrap_or_default().to_owned(), plain.clone()),
        ],
        None => match mnemonic.filter(|_| mnemonic_visible).map(|m| transient_mark(label, m)) {
            Some((base, mark)) => vec![(base, plain.clone()), (mark, plain.clone())],
            None => vec![(label.to_owned(), plain.clone())],
        },
    };
    pieces.retain(|(text, _)| !text.is_empty());
    // The label starts after the column: a checked row draws the mark in it, the others leave it
    // empty (a leading space of the same width, not padding characters whose width varies).
    let mut lead = check_gutter(ui);
    let append_part = |job: &mut egui::text::LayoutJob, text: &str, format: &egui::TextFormat, lead: &mut f32| {
        if text.is_empty() {
            return;
        }
        job.append(text, *lead, format.clone());
        *lead = 0.0;
    };
    let gutter_first = side == GutterSide::ReadingStart;
    if gutter_first && checked {
        job.append(CHECK_MARK, 0.0, plain.clone());
        lead = 0.0;
    }
    for (text, format) in &pieces {
        append_part(&mut job, text, format, &mut lead);
    }
    if !gutter_first && checked {
        job.append(CHECK_MARK, 0.0, plain.clone());
        lead = 0.0;
    }
    if job.sections.is_empty() {
        // An empty label still keeps the row as wide as its column.
        job.append("", lead, plain);
    }
    job.into()
}

/// A menu row's button atoms in reading order, whatever the direction: plain `Button` lays its
/// atoms out left-to-right whatever the direction flag, so `trailing` (a shortcut, a submenu
/// arrow) moves to the physical left via [`egui::Button::left_text`] in a right-to-left row,
/// and a row without one gets a leading grow so its label hugs the row's reading start.
pub fn menu_row_button_atom<'a>(text: egui::WidgetText, trailing: Option<egui::Atom<'a>>, rtl: bool) -> egui::Button<'a> {
    match (rtl, trailing) {
        (true, Some(t)) => egui::Button::new(text).left_text(t),
        (true, None) => egui::Button::new((egui::Atom::grow(), text)),
        (false, Some(t)) => egui::Button::new(text).right_text(t),
        (false, None) => egui::Button::new(text),
    }
}

/// A menu row's button: `text` with its `shortcut` at the row's reading end, weak-tinted as
/// [`egui::Button::shortcut_text`] does. The tint is applied to the atom here, so the LTR pixels
/// are unchanged and the RTL atom order comes from [`menu_row_button_atom`].
pub fn menu_row_button<'a>(text: egui::WidgetText, shortcut: Option<String>, rtl: bool) -> egui::Button<'a> {
    let trailing = shortcut.map(|sc| egui::Atom::from(egui::RichText::new(crate::shortcuts::pretty(&sc)).weak()));
    menu_row_button_atom(text, trailing, rtl)
}

/// A submenu row's button: the arrow at the row's reading end, where the submenu opens (the
/// glyph itself rotates with the direction flag in the fork's `arrow_atom`).
pub fn submenu_row_button(title: egui::WidgetText, rtl: bool) -> egui::Button<'static> {
    menu_row_button_atom(title, Some(egui::containers::menu::SubMenuButton::arrow_atom(None)), rtl)
}

/// A hand-built multi-atom row's atoms in reading order: as laid out left-to-right, reversed in
/// a right-to-left one (grow included). Callers need `egui::IntoAtoms` for `.into_atoms()`.
pub fn mirror_atoms<'a>(mut atoms: egui::Atoms<'a>, rtl: bool) -> egui::Atoms<'a> {
    if rtl {
        atoms.reverse();
    }
    atoms
}

/// One row of any popup menu (a context menu, a flyout, the ruler's unit menu):
/// `label` behind the fixed-width checkmark column at the row's reading start
/// ([`menu_row_text`]), `shortcut` at the reading end, in a button that mirrors under a
/// right-to-left direction — plain `ui.button` lays its atoms left-to-right whatever the
/// direction flag, so popup bodies use this instead. A row that is neither checked nor in a
/// right-to-left UI keeps a plain label, pixel-identical to a plain button.
pub fn menu_row(ui: &mut egui::Ui, label: &str, shortcut: Option<&str>, checked: bool, enabled: bool) -> egui::Response {
    let rtl = ui.style().right_to_left;
    let text = if checked || rtl { menu_row_text(ui, checked, label, None, false, GutterSide::ReadingStart) } else { egui::RichText::new(label).into() };
    ui.add_enabled(enabled, menu_row_button(text, shortcut.map(str::to_string), rtl))
}

/// A submenu row, drawn as `ui.menu_button` draws it, except that a row scrolled out of view
/// doesn't open its submenu. egui opens a submenu whenever the pointer is over its row, clipped
/// or not: a menu reopened scrolled down (after Edit › Preferences or Window › Theme) has rows
/// hidden above its view, over the menu bar, and the hidden one under the title opened on the
/// press; the release on the title then counted as a click outside that submenu and closed the
/// whole menu (#5896, #6225). An open submenu (e.g. opened from the keyboard) is drawn as usual.
pub fn submenu_row<R>(ui: &mut egui::Ui, title: egui::WidgetText, content: impl FnOnce(&mut egui::Ui) -> R) -> (egui::Response, Option<R>) {
    use egui::containers::menu::{MenuState, SubMenu};
    let sub = SubMenu::id_from_widget_id(ui.next_auto_id());
    let open = MenuState::from_ui(ui, |state, _| state.open_item == Some(sub));
    let inactive = ui.style().visuals.widgets.inactive;
    if open {
        ui.style_mut().visuals.widgets.inactive = ui.style().visuals.widgets.open;
    }
    let response = ui.add(submenu_row_button(title, ui.style().right_to_left));
    ui.style_mut().visuals.widgets.inactive = inactive;
    // The rect egui tests the pointer against (`SubMenu::show`), against the part that can be seen.
    let reach = response.rect.expand2(ui.spacing().item_spacing / 2.0);
    let hidden = ui.ctx().pointer_hover_pos().is_some_and(|p| reach.contains(p) && !ui.clip_rect().contains(p));
    if hidden && !open {
        return (response, None);
    }
    let inner = SubMenu::new().show(ui, &response, |ui| crate::direction::popup_body(ui, content));
    (response, inner.map(|r| r.inner))
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
    /// and `mnemonics` their letters, both in logical menu order (File first); `nav` is the open
    /// menu's state. A command a mnemonic ran lands in `clicked`.
    pub fn keys(&mut self, ctx: &egui::Context, tops: &[Id], mnemonics: &[Option<char>], nav: &mut Nav, clicked: &mut Option<String>) {
        if tops.is_empty() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        let mut tap = false;
        // The tap is judged on every event of the frame, also those another handler consumed
        // before the bar's turn: the shortcuts run first, so Ctrl+Alt+I (Image Size) left no
        // trace in `events`, and releasing Alt counted as a tap that put the keyboard on the bar;
        // Enter then opened File instead of confirming the dialog (#7481).
        let raw = ctx.input(|i| i.raw.events.clone());
        for e in &raw {
            match e {
                // A held Alt auto-repeats (after about half a second on Windows): only its first
                // press starts a tap. A repeat must neither re-arm a tap that Alt+wheel or a key
                // already spent, nor spend one itself (holding Alt alone is still a tap).
                Event::Key { key: Key::AltLeft | Key::AltRight, pressed: true, repeat: true, .. } => self.alt_down = true,
                Event::Key { key: Key::AltLeft | Key::AltRight, pressed: true, .. } => {
                    self.alt_down = true;
                    // Alt pressed while a mouse button is down is a modifier for that drag
                    // (Free Transform's scale-from-centre), never a tap (#8113).
                    self.alt_tap = self.buttons_held == 0;
                }
                Event::Key { key: Key::AltLeft | Key::AltRight, pressed: false, .. } => {
                    self.alt_down = false;
                    tap |= std::mem::take(&mut self.alt_tap);
                }
                // A click or the end of a drag while Alt is held is no tap. It also cancels a tap
                // taken earlier in this frame: a whole drag can arrive in one frame (press, Alt
                // down, Alt up, release).
                Event::PointerButton { button, pressed, .. } => {
                    let bit = 1u8 << (*button as u8);
                    if *pressed {
                        self.buttons_held |= bit;
                    } else {
                        self.buttons_held &= !bit;
                    }
                    self.alt_tap = false;
                    tap = false;
                }
                // Alt going to the desktop is no tap, and a button let go out there is up.
                Event::WindowFocused(false) => {
                    self.alt_tap = false;
                    self.buttons_held = 0;
                }
                // Alt+wheel (zoom, brush size…) is no tap either.
                Event::Key { pressed: true, .. } | Event::Text(_) | Event::MouseWheel { .. } => self.alt_tap = false,
                _ => {}
            }
        }
        // Typing a zoom: Alt only answers with the system alert. The menu bar stays off and the
        // field keeps its focus and its text.
        if tap && crate::zoom_field::is_focused(ctx) {
            crate::zoom_field::request_alert(ctx);
            tap = false;
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
            // A letter runs the row with that mnemonic in the level the arrows are in, unless it is
            // typed into a field in the menu: Help › Search took "S" for System Info (#5996). A zoom
            // field that kept the keyboard through the click on the menu title is not in the menu,
            // but while Help › Search is open the letters are still typing.
            let typing = ctx.text_edit_focused() && !(crate::zoom_field::is_focused(ctx) && !super::search::shown(ctx));
            let letter = events.iter().filter(|_| !typing).find_map(|e| match e {
                Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.ctrl && !modifiers.alt => {
                    key_char(*key).map(|c| (*key, c))
                }
                _ => None,
            });
            if let Some((key, c)) = letter
                && let Some(row) = nav.row_with_mnemonic(c)
            {
                consume(Modifiers::NONE, key);
                crate::zoom_field::drop_text(ctx, c);
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
        // In a right-to-left UI the bar is drawn right-to-left: the physical arrows swap, so the
        // forward key (→, or ← there) moves to the next menu.
        let (fwd, back) = crate::direction::horizontal_nav_keys();
        if press(Key::Escape) {
            self.focus = None;
        } else if press(fwd) {
            self.focus = Some((focus + 1) % n);
        } else if press(back) {
            self.focus = Some((focus + n - 1) % n);
        } else if press(Key::ArrowDown) || press(Key::Enter) {
            open_top(focus, self);
        } else if events.iter().any(|e| matches!(e, Event::Key { key: Key::Space, pressed: true, .. })) {
            // Space is the Hand tool, and on Windows it opens the window menu there, never File:
            // it takes the keyboard off the bar, so a stray Alt tap can't have held Space run
            // File › New….
            self.focus = None;
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
        assert_eq!(assign(&labels, &preferred, &[]), "FEILYSTVWH".chars().map(Some).collect::<Vec<_>>());
        // Items: first letters first, then the next free letter of the word starts, then any.
        assert_eq!(
            assign(&["New…", "Open…", "Open Recent", "Close", "Close All", "Save", "Save As…"], &[], &[]),
            [Some('N'), Some('O'), Some('R'), Some('C'), Some('A'), Some('S'), Some('V'),]
        );
        // A preferred letter the label lacks (a translation) falls back; a taken one too.
        assert_eq!(assign(&["Datei", "Bearbeiten"], &[Some('F'), Some('E')], &[]), [Some('D'), Some('E')]);
        assert_eq!(assign(&["Aa", "Ab", "Ba"], &[Some('A'), Some('A'), Some('A')], &[]), [Some('A'), Some('B'), None]);
        // A reserved letter (a key another row shows) is not taken again.
        assert_eq!(assign(&["关闭(C)-like", "Copy"], &[], &['C']), [Some('L'), Some('O')]);
        // Only ASCII letters and digits are keys: no mnemonic without one.
        assert_eq!(assign(&["ファイル", "Zoom 100%", "✔ Rulers"], &[], &[]), [None, Some('Z'), Some('R')]);
    }

    #[test]
    fn key_chars_are_plain_letters_and_digits() {
        assert_eq!(key_char(Key::A), Some('A'));
        assert_eq!(key_char(Key::Num7), Some('7'));
        assert_eq!(key_char(Key::F10), None);
        assert_eq!(key_char(Key::ArrowDown), None);
        assert_eq!(key_char(Key::Space), None);
    }

    #[test]
    fn command_keys_are_command_combos_and_function_keys_but_f10() {
        assert!(is_command_key(Key::D, Modifiers::COMMAND));
        assert!(is_command_key(Key::I, Modifiers::COMMAND | Modifiers::SHIFT));
        assert!(is_command_key(Key::F7, Modifiers::NONE));
        assert!(is_command_key(Key::F12, Modifiers::SHIFT));
        assert!(!is_command_key(Key::F10, Modifiers::NONE), "F10 toggles the bar");
        assert!(!is_command_key(Key::ControlLeft, Modifiers::CTRL), "Ctrl going down is no shortcut yet");
        assert!(!is_command_key(Key::E, Modifiers::NONE), "a letter is a mnemonic");
        assert!(!is_command_key(Key::E, Modifiers::ALT), "Alt+letter opens a menu");
        assert!(!is_command_key(Key::ArrowRight, Modifiers::NONE));
        assert!(!is_command_key(Key::Enter, Modifiers::NONE));
        assert!(!is_command_key(Key::Escape, Modifiers::NONE));
    }

    /// A popup row built with [`menu_row`]: in a right-to-left body the label hugs the row's
    /// right edge, the shortcut sits at its left and the checkmark column at the reading start;
    /// left-to-right keeps a plain button's look (no checkmark column on an unchecked row).
    #[test]
    fn menu_row_mirrors_label_shortcut_and_gutter() {
        use crate::direction::{Dir, direction_scope};
        // (galley text, galley rect, x of the first glyph of `needle` in that galley)
        let render = |dir: Dir| -> Vec<(String, egui::Rect, Vec<(char, f32)>)> {
            let _scope = direction_scope(dir);
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|s| s.right_to_left = dir == Dir::Rtl);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.allocate_ui_with_layout(egui::vec2(240.0, 100.0), egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                    menu_row(ui, "Inches", None, false, true);
                    menu_row(ui, "Pixels", Some("Ctrl+P"), true, true);
                });
            });
            let texts = out
                .shapes
                .iter()
                .filter_map(|cs| match &cs.shape {
                    egui::Shape::Text(ts) => {
                        let glyphs = ts.galley.rows.first().map(|r| r.glyphs.iter().map(|g| (g.chr, g.pos.x)).collect()).unwrap_or_default();
                        Some((ts.galley.text().to_string(), ts.visual_bounding_rect(), glyphs))
                    }
                    _ => None,
                })
                .collect();
            out.textures_delta.clear();
            texts
        };
        let at = |v: &[(String, egui::Rect, Vec<(char, f32)>)], s: &str| v.iter().find(|(t, _, _)| t.contains(s)).cloned();
        let glyph = |v: &[(String, egui::Rect, Vec<(char, f32)>)], s: &str, c: char| at(v, s).and_then(|(_, _, g)| g.iter().find(|(ch, _)| *ch == c).map(|(_, x)| *x));
        let rtl = render(Dir::Rtl);
        let (inch, pix, sc) = (at(&rtl, "Inches").unwrap().1, at(&rtl, "Pixels").unwrap().1, at(&rtl, "Ctrl").unwrap().1);
        assert!(inch.right() > 240.0 - 32.0 && pix.right() > 240.0 - 32.0, "rtl: the labels hug the right edge: {inch:?} {pix:?}");
        assert!(sc.left() < 32.0, "rtl: the shortcut sits at the left edge: {sc:?}");
        let (mark, label) = (glyph(&rtl, "Pixels", '✔').unwrap(), glyph(&rtl, "Pixels", 'P').unwrap());
        assert!(mark > label, "rtl: the checkmark column is at the reading start (right) of the label: {mark} {label}");
        let ltr = render(Dir::Ltr);
        let (inch, pix, sc) = (at(&ltr, "Inches").unwrap().1, at(&ltr, "Pixels").unwrap().1, at(&ltr, "Ctrl").unwrap().1);
        assert!(inch.left() < 32.0, "ltr: an unchecked row keeps its plain label at the left edge (no gutter): {inch:?}");
        assert!(pix.left() > inch.left(), "ltr: a checked row starts its label after the column: {pix:?} {inch:?}");
        let (mark, label) = (glyph(&ltr, "Pixels", '✔').unwrap(), glyph(&ltr, "Pixels", 'P').unwrap());
        assert!(mark < label, "ltr: the checkmark is at the left of the label: {mark} {label}");
        assert!(sc.right() > 240.0 - 32.0, "ltr: the shortcut sits at the right edge: {sc:?}");
    }
}

#[cfg(test)]
mod harness_tests;
