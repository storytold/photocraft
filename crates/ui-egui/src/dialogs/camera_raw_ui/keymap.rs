//! The develop view's keyboard shortcuts as data: a table of action ids and key chords
//! ([`CAMERA_RAW`], Adobe Camera Raw's keys), read into a [`Keymap`] the dialog keeps. What each
//! action does is elsewhere ([`super::keys`]), so another host could hand the dialog a table of its
//! own (LightCraft's Q for Heal, a user's overrides) without touching that code.
//!
//! Chords use PhotoCraft's shortcut notation (`Cmd+Shift+N`, see [`crate::shortcuts::parse`]):
//! `Cmd` is ⌘ on the Mac and Ctrl elsewhere, and a press matches the way the app's own shortcuts
//! do ([`crate::shortcuts::key_matches`]: exact modifiers, Delete also Backspace, ⇧`[` also `{`).
//! [`Keymap::label`] shows a chord with the platform's symbols for tooltips.

use egui::KeyboardShortcut;

use super::rail::Tool;

/// Something a develop-view shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Picks a tool on the rail (its key again goes back to Edit).
    Tool(Tool),
    /// The active brush (mask brush, heal spot, red eye pupil) smaller / larger.
    BrushSmaller,
    BrushLarger,
    /// The active brush's feather down / up, where it has one.
    FeatherLess,
    FeatherMore,
    /// The mask brush's Auto Mask on / off.
    AutoMask,
    /// The selected mask's overlay shown / hidden (Masking only).
    MaskOverlay,
    /// Edit › Geometry › Upright › Guided and its tool (a second press puts the tool down).
    GuidedUpright,
    /// Opens (or folds) the Edit panel's nth section, 1-based, in panel order.
    Section(u8),
    /// Removes the selected guide, mask, spot or red eye.
    DeleteSelected,
}

/// The most sections the keymap can number (⌘1 … ⌘9).
pub(crate) const MAX_SECTIONS: u8 = 9;

impl Action {
    /// The action's id in a keymap table.
    pub(crate) fn id(self) -> String {
        match self {
            Action::Tool(t) => format!("tool.{}", t.id()),
            Action::BrushSmaller => "brush.smaller".into(),
            Action::BrushLarger => "brush.larger".into(),
            Action::FeatherLess => "brush.featherLess".into(),
            Action::FeatherMore => "brush.featherMore".into(),
            Action::AutoMask => "brush.autoMask".into(),
            Action::MaskOverlay => "mask.overlay".into(),
            Action::GuidedUpright => "geometry.guidedUpright".into(),
            Action::Section(n) => format!("edit.section.{n}"),
            Action::DeleteSelected => "selection.delete".into(),
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Action> {
        if let Some(t) = id.strip_prefix("tool.") {
            return Tool::from_id(t).map(Action::Tool);
        }
        if let Some(n) = id.strip_prefix("edit.section.") {
            return n.parse::<u8>().ok().filter(|n| (1..=MAX_SECTIONS).contains(n)).map(Action::Section);
        }
        Some(match id {
            "brush.smaller" => Action::BrushSmaller,
            "brush.larger" => Action::BrushLarger,
            "brush.featherLess" => Action::FeatherLess,
            "brush.featherMore" => Action::FeatherMore,
            "brush.autoMask" => Action::AutoMask,
            "mask.overlay" => Action::MaskOverlay,
            "geometry.guidedUpright" => Action::GuidedUpright,
            "selection.delete" => Action::DeleteSelected,
            _ => return None,
        })
    }
}

/// Adobe Camera Raw's keys for what the develop view has: E Edit, B Heal (Camera Raw's Remove),
/// K or M Masking (K is listed first, so it is the one tooltips show), ⇧E Red Eye; `[` `]` brush size and ⇧`[` ⇧`]` feather; A
/// the mask brush's Auto Mask (Lightroom's key for it; free in Camera Raw's table); O the mask overlay while masking
/// (elsewhere O stays the highlight clipping warning's); ⇧T Guided Upright; ⌘1 … ⌘9
/// the Edit sections; Delete the selection.
pub(crate) const CAMERA_RAW: &[(&str, &str)] = &[
    ("tool.edit", "E"),
    ("tool.heal", "B"),
    ("tool.masking", "K"),
    ("tool.masking", "M"),
    ("tool.redEye", "Shift+E"),
    ("brush.smaller", "["),
    ("brush.larger", "]"),
    ("brush.featherLess", "Shift+["),
    ("brush.featherMore", "Shift+]"),
    ("brush.autoMask", "A"),
    ("mask.overlay", "O"),
    ("geometry.guidedUpright", "Shift+T"),
    ("edit.section.1", "Cmd+1"),
    ("edit.section.2", "Cmd+2"),
    ("edit.section.3", "Cmd+3"),
    ("edit.section.4", "Cmd+4"),
    ("edit.section.5", "Cmd+5"),
    ("edit.section.6", "Cmd+6"),
    ("edit.section.7", "Cmd+7"),
    ("edit.section.8", "Cmd+8"),
    ("edit.section.9", "Cmd+9"),
    ("selection.delete", "Delete"),
];

/// A table of shortcuts read for matching: each action's chord, as written and parsed.
#[derive(Clone, Debug)]
pub(crate) struct Keymap {
    bindings: Vec<(Action, String, KeyboardShortcut)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::from_table(CAMERA_RAW)
    }
}

impl Keymap {
    /// `table`'s rows of (action id, chord). A row with an unknown action or a chord that doesn't
    /// parse is left out rather than turned into another shortcut.
    pub(crate) fn from_table(table: &[(&str, &str)]) -> Keymap {
        let bindings = table.iter().filter_map(|(id, chord)| Some((Action::from_id(id)?, chord.to_string(), crate::shortcuts::parse(chord)?))).collect();
        Keymap { bindings }
    }

    /// The chord bound to `action`, as written in the table.
    pub(crate) fn chord(&self, action: Action) -> Option<&str> {
        self.bindings.iter().find(|b| b.0 == action).map(|b| b.1.as_str())
    }

    /// `action`'s chord with the platform's symbols (`⌘1` on the Mac, `Ctrl+1` elsewhere).
    pub(crate) fn label(&self, action: Action) -> Option<String> {
        self.chord(action).map(crate::shortcuts::pretty)
    }

    /// `name` followed by `action`'s chord in parentheses (`Masking (K)`); bare `name` unbound.
    pub(crate) fn tip(&self, name: &str, action: Action) -> String {
        self.label(action).map_or_else(|| name.to_string(), |k| format!("{name} ({k})"))
    }

    /// The table as the control channel describes it: action id → chord.
    pub(crate) fn describe(&self) -> serde_json::Value {
        serde_json::Value::Object(self.bindings.iter().map(|(a, chord, _)| (a.id(), serde_json::Value::from(chord.as_str()))).collect())
    }

    /// The actions this frame's key presses ask for, in the order pressed, of those `wanted` takes
    /// now; their presses are consumed, so nothing behind the dialog sees them too. A press of an
    /// action not wanted (Delete with nothing of the keymap's selected) is left for the widgets.
    pub(crate) fn take(&self, ctx: &egui::Context, wanted: impl Fn(Action) -> bool) -> Vec<Action> {
        ctx.input_mut(|i| {
            let mut out = Vec::new();
            i.events.retain(|e| {
                let egui::Event::Key { key, pressed: true, modifiers, .. } = e else { return true };
                let hit = self.bindings.iter().find(|b| wanted(b.0) && crate::shortcuts::key_matches(&b.2, *key, *modifiers));
                let Some(b) = hit else { return true };
                out.push(b.0);
                false
            });
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers};

    fn press(ctx: &egui::Context, key: Key, modifiers: Modifiers, map: &Keymap) -> Vec<Action> {
        let raw = egui::RawInput { events: vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() };
        ctx.begin_pass(raw);
        let got = map.take(ctx, |_| true);
        ctx.end_pass().textures_delta.clear();
        got
    }

    /// Every row of Camera Raw's table is read, every action id round-trips, and a press of each
    /// chord is that action and nothing else.
    #[test]
    fn camera_raws_table_reads_whole_and_each_chord_is_its_action() {
        let map = Keymap::default();
        assert_eq!(map.bindings.len(), CAMERA_RAW.len(), "no row was dropped");
        for (id, _) in CAMERA_RAW {
            let a = Action::from_id(id).unwrap();
            assert_eq!(a.id(), *id);
        }
        let ctx = egui::Context::default();
        let cases = [
            (Key::E, Modifiers::NONE, Action::Tool(Tool::Edit)),
            (Key::B, Modifiers::NONE, Action::Tool(Tool::Heal)),
            (Key::K, Modifiers::NONE, Action::Tool(Tool::Masking)),
            (Key::M, Modifiers::NONE, Action::Tool(Tool::Masking)),
            (Key::E, Modifiers::SHIFT, Action::Tool(Tool::RedEye)),
            (Key::OpenBracket, Modifiers::NONE, Action::BrushSmaller),
            (Key::CloseBracket, Modifiers::NONE, Action::BrushLarger),
            (Key::OpenBracket, Modifiers::SHIFT, Action::FeatherLess),
            // a US layout reports ⇧] as `}`
            (Key::CloseCurlyBracket, Modifiers::SHIFT, Action::FeatherMore),
            (Key::A, Modifiers::NONE, Action::AutoMask),
            (Key::O, Modifiers::NONE, Action::MaskOverlay),
            (Key::T, Modifiers::SHIFT, Action::GuidedUpright),
            (Key::Num1, Modifiers::COMMAND, Action::Section(1)),
            (Key::Num9, Modifiers::COMMAND, Action::Section(9)),
            (Key::Delete, Modifiers::NONE, Action::DeleteSelected),
            (Key::Backspace, Modifiers::NONE, Action::DeleteSelected),
        ];
        for (key, m, want) in cases {
            assert_eq!(press(&ctx, key, m, &map), vec![want], "{key:?} {m:?}");
        }
        // modifiers match exactly: ⌥T, ⌘E, a bare 1 and ⌘⇧1 are no shortcuts here
        for (key, m) in [
            (Key::T, Modifiers::ALT | Modifiers::SHIFT),
            (Key::E, Modifiers::COMMAND),
            (Key::Num1, Modifiers::NONE),
            (Key::Num1, Modifiers::COMMAND | Modifiers::SHIFT),
            (Key::Q, Modifiers::NONE),
        ] {
            assert!(press(&ctx, key, m, &map).is_empty(), "{key:?} {m:?}");
        }
    }

    /// Another host's table: its own chords, unknown actions and malformed chords left out.
    #[test]
    fn another_table_replaces_the_keys() {
        let map = Keymap::from_table(&[("tool.heal", "Q"), ("tool.crop", "C"), ("brush.larger", "Cmd+Nope"), ("no.such", "X")]);
        assert_eq!(map.chord(Action::Tool(Tool::Heal)), Some("Q"));
        assert_eq!(map.chord(Action::BrushLarger), None);
        assert_eq!(map.bindings.len(), 1);
        let ctx = egui::Context::default();
        assert_eq!(press(&ctx, Key::Q, Modifiers::NONE, &map), vec![Action::Tool(Tool::Heal)]);
        assert!(press(&ctx, Key::B, Modifiers::NONE, &map).is_empty());
        assert_eq!(map.tip("Edit", Action::Tool(Tool::Edit)), "Edit", "unbound: the bare name");
    }

    /// Tooltips show the chord with the platform's modifier symbols.
    #[test]
    fn labels_use_the_platforms_symbols() {
        let map = Keymap::default();
        let mac = cfg!(target_os = "macos");
        assert_eq!(map.label(Action::Section(2)).as_deref(), Some(if mac { "⌘2" } else { "Ctrl+2" }));
        assert_eq!(map.label(Action::GuidedUpright).as_deref(), Some(if mac { "⇧T" } else { "Shift+T" }));
        assert_eq!(map.tip("Masking", Action::Tool(Tool::Masking)), "Masking (K)");
        assert!(Action::from_id("edit.section.0").is_none() && Action::from_id("edit.section.10").is_none());
    }
}
