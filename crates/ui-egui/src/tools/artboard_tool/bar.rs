//! The Artboard tool's options bar (#11385): the framework's `ArtboardBar` for the selected board, or for the
//! tool's own options when no board is selected, and the gear menu. The options live in
//! [`ArtboardToolUi`](super::ArtboardToolUi).

// PLAN: the `craft::ui::widgets::{Button, Menu}` and `craft::ui::contract::{Entry, Trigger}` facade paths are assumed.

use craft::behavior::kurbo::Size;
use craft::behavior::{EditEvent, EditEvents};
use craft::draw::ui::artboard::{ArtboardBackground as Shown, ArtboardBar, ArtboardBarTarget, ArtboardPreset};
use craft::theme::Srgba;
use craft::ui::buttons::Button;
use craft::ui::contract::{Entry, Show as _, Trigger};
use craft::ui::menu::Menu;
use craft::ui::runtime::requests::ColorRequest;
use photocraft_doc::{ArtboardBackground, LayerId};
use photocraft_engine::artboard_cmds::PRESETS;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The size preset a fresh bar (and Reset Tool) shows, with its width and height.
pub const DEFAULT_SIZE: (&str, u32, u32) = ("iPhone 8/7/6", 750, 1334);

/// The largest board side the bar accepts (the engine's limit).
const MAX_SIDE: f32 = 300_000.0;

/// The gear menu's entries, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Gear {
    AutoNest,
    AutoSizeCanvas,
    KeepRelativePosition,
    ShrinkwrapOnSave,
}

/// Each gear entry and its label.
pub const GEAR: [(Gear, &str); 4] = [
    (Gear::AutoNest, "Auto-nest Layers"),
    (Gear::AutoSizeCanvas, "Auto-size Canvas"),
    (Gear::KeepRelativePosition, "Keep Relative Position During Layer Reordering"),
    (Gear::ShrinkwrapOnSave, "Shrinkwrap Canvas on Save"),
];

impl Gear {
    /// Is the entry checked?
    pub fn checked(self, o: &super::ArtboardToolUi) -> bool {
        match self {
            Gear::AutoNest => o.auto_nest,
            Gear::AutoSizeCanvas => o.auto_size_canvas,
            Gear::KeepRelativePosition => o.keep_relative_position,
            Gear::ShrinkwrapOnSave => o.shrinkwrap_on_save,
        }
    }

    /// Only Auto-nest Layers does something yet (the Move tool's auto-nesting, #10512); the others
    /// are drawn checked and disabled until their behaviour is measured.
    pub fn enabled(self) -> bool {
        self == Gear::AutoNest
    }
}

/// "The bar's size": the selected board's size, else the tool option's. `[width, height]` in pixels.
pub fn bar_size(app: &PhotocraftApp) -> [i64; 2] {
    match super::active_board(app) {
        Some((_, r)) => [r[2].saturating_sub(r[0]), r[3].saturating_sub(r[1])],
        None => [i64::from(app.ui.artboard_tool.width), i64::from(app.ui.artboard_tool.height)],
    }
}

/// The name of the preset of size `w` x `h`, "" (Custom) for none. No two presets share a size.
pub fn preset_name(w: i64, h: i64) -> &'static str {
    PRESETS.iter().find(|(_, pw, ph)| i64::from(*pw) == w && i64::from(*ph) == h).map_or("", |(n, _, _)| *n)
}

/// `params` for `layer.new.artboard` plus the bar's background: `background`, and `color` for Other….
pub fn with_background(app: &PhotocraftApp, mut params: Value) -> Value {
    let o = &app.ui.artboard_tool;
    params["background"] = json!(o.background);
    if o.background == "custom" {
        params["color"] = json!(o.background_color);
    }
    params
}

/// Set the tool option size to `w` x `h`, clamped to 1..=300000.
pub fn set_size(app: &mut PhotocraftApp, w: f32, h: f32) {
    let clamp = |v: f32| if v.is_finite() { v.round().clamp(1.0, MAX_SIDE) as u32 } else { 1 };
    let o = &mut app.ui.artboard_tool;
    (o.width, o.height) = (clamp(w), clamp(h));
}

/// The selected boards: each selected layer's artboard, once, in document order.
fn selected_boards(app: &PhotocraftApp) -> Vec<LayerId> {
    let Some(st) = app.session.active() else { return Vec::new() };
    let mut ids: Vec<LayerId> = Vec::new();
    for l in st.selected_layers() {
        if let Some(b) = st.doc.artboard_of(l)
            && !ids.contains(&b)
        {
            ids.push(b);
        }
    }
    ids
}

/// The background the bar shows as `(name, #rrggbb)`: the first selected board's, else the tool
/// option's. The colour is the custom one (or the option's remembered one for the named ones).
pub fn shown_background(app: &PhotocraftApp) -> (String, String) {
    let o = &app.ui.artboard_tool;
    let first = selected_boards(app).first().and_then(|id| app.session.active()?.doc.layer(*id)?.artboard().map(|a| a.background));
    match first {
        Some(ArtboardBackground::Custom(c)) => {
            let [r, g, b, _] = c.to_rgba8();
            ("custom".to_string(), format!("#{r:02x}{g:02x}{b:02x}"))
        }
        Some(b) => (b.name().to_string(), o.background_color.clone()),
        None => (o.background.clone(), o.background_color.clone()),
    }
}

/// Choose background `name` (`white`, `black`, `transparent` or `custom` with `color`, `#rrggbb`).
/// It is the tool option for the next drawn board, and with boards selected also the background of
/// every selected board in one `layer.artboard.set` step ("Edit Artboard" / "Edit Artboards").
pub fn set_background(app: &mut PhotocraftApp, name: &str, color: &str) -> Result<Value, String> {
    let custom = name == "custom";
    if custom {
        crate::color_picker_ui::parse_hex(color).ok_or("not a colour")?;
    }
    let mut boards = selected_boards(app);
    // A board whose custom colour already has this RGB keeps its own value: the swatch shows it
    // as 8-bit RGB, so OK in the picker without a change must not flatten its alpha or precision.
    if custom && let Some(want) = crate::color_picker_ui::parse_hex(color) {
        let hex = |c: [f32; 3]| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let st = app.session.active();
        boards.retain(|id| match st.and_then(|st| st.doc.layer(*id)?.artboard().map(|a| a.background)) {
            Some(ArtboardBackground::Custom(c)) => {
                let [r, g, b, _] = c.to_rgba8();
                [r, g, b] != hex(want)
            }
            _ => true,
        });
    }
    let mut out = json!({});
    if !boards.is_empty() {
        let mut params = json!({"layers": boards.iter().map(|b| b.0).collect::<Vec<_>>(), "background": name});
        if custom {
            params["color"] = json!(color);
        }
        out = app.run("layer.artboard.set", params)?;
    }
    let o = &mut app.ui.artboard_tool;
    o.background = name.to_string();
    if custom {
        o.background_color = color.to_string();
    }
    Ok(out)
}

/// OK in the Color Picker opened by Other…: the custom background is `color` (`#rrggbb`), for the
/// selected boards (if any) and the next drawn one.
pub fn set_custom_background(app: &mut PhotocraftApp, color: &str) -> Result<Value, String> {
    set_background(app, "custom", color)?;
    Ok(json!({"color": color}))
}

/// [`set_custom_background`] for OK in the picker. `unchanged` (the result is the colour the picker
/// opened with, after any Only Web Colors snap) leaves every selected board alone, with no command
/// and no history step: only the tool option for the next board follows. Otherwise boards that
/// already have the picked RGB are skipped one by one ([`set_background`]).
pub fn set_custom_background_from_picker(app: &mut PhotocraftApp, color: &str, unchanged: bool) -> Result<Value, String> {
    if !unchanged {
        return set_custom_background(app, color);
    }
    crate::color_picker_ui::parse_hex(color).ok_or("not a colour")?;
    let o = &mut app.ui.artboard_tool;
    o.background = "custom".to_string();
    o.background_color = color.to_string();
    Ok(json!({"color": color}))
}

/// Opens the Color Picker for the custom background.
fn pick_background(app: &mut PhotocraftApp) {
    let rgb = crate::color_picker_ui::parse_hex(&shown_background(app).1).unwrap_or([1.0; 3]);
    let title = crate::color_picker_ui::title_for(tl!("Color"));
    if let Err(message) = crate::color_picker_ui::open_for_target(app, "artboardBackground", &title, rgb) {
        crate::notices::error(app, message);
    }
}

/// The framework's background for a bar background `name` (with `hex` for custom).
fn shown_of_name(name: &str, hex: &str) -> Shown {
    match name {
        "black" => Shown::Black,
        "transparent" => Shown::Transparent,
        "custom" => crate::color_picker_ui::parse_hex(hex).map_or(Shown::White, |[r, g, b]| Shown::Custom(Srgba::rgb(r, g, b))),
        _ => Shown::White,
    }
}

/// `(name, #rrggbb)` of a framework background, in the names `set_background` takes.
fn name_of(background: Shown) -> (&'static str, Option<String>) {
    match background {
        Shown::Black => ("black", None),
        Shown::Transparent => ("transparent", None),
        Shown::Custom(c) => {
            let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            ("custom", Some(format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))))
        }
        _ => ("white", None),
    }
}

/// The presets the bar lists.
fn presets() -> Vec<ArtboardPreset> {
    PRESETS.iter().map(|(name, w, h)| ArtboardPreset::new(*name, Size::new(f64::from(*w), f64::from(*h)))).collect()
}

/// The value a size edit proposes. A gesture's `Update`s apply as they come and its `Commit` ends it; a `Set` is one step.
fn proposed(event: &EditEvent<Size>) -> Option<Size> {
    match event {
        EditEvent::Update { value, .. } => Some(*value),
        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(*after),
        _ => None,
    }
}

/// Applies what the bar reported: sizes edit the board (`board`) or the tool's option, a background goes to
/// [`set_background`], and a colour request opens the picker.
pub(crate) fn route(app: &mut PhotocraftApp, board: Option<LayerId>, size: EditEvents<Size>, background: Option<Shown>, pick_color: Option<ColorRequest>) {
    for event in &size {
        let Some(value) = proposed(event) else { continue };
        let (w, h) = (value.width.round(), value.height.round());
        match board {
            Some(id) => {
                let mut params = json!({"layer": id.0, "width": w, "height": h});
                // PLAN: assumed `crate::history_keys::coalesce` (tl.P12); one key per gesture and board.
                if let Some(key) = crate::history_keys::coalesce(&format!("artboard-tool-wh-{}", id.0), event) {
                    params["coalesce"] = json!(key);
                }
                let r = app.run("layer.artboard.set", params);
                super::report(app, r);
            }
            None => set_size(app, w as f32, h as f32),
        }
    }
    if let Some(background) = background {
        let (name, hex) = name_of(background);
        let hex = hex.unwrap_or_else(|| shown_background(app).1);
        let r = set_background(app, name, &hex);
        super::report(app, r);
    }
    if pick_color.is_some() {
        pick_background(app);
    }
}

/// Shows the bar's members for the selected board, or for the next board, and applies what they report.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let boards = super::items(app);
    let selected = boards.selected.first().copied();
    let presets = presets();
    let next = {
        let [w, h] = bar_size(app);
        let (name, hex) = shown_background(app);
        ArtboardBarTarget::next(Size::new(w as f64, h as f64), shown_of_name(&name, &hex))
    };
    let target = match selected.and_then(|i| boards.items.get(i)) {
        Some(item) => ArtboardBarTarget::Board(item),
        None => next,
    };
    let out = ArtboardBar::new(target, &presets).show(ui);
    let id = selected.and_then(|i| boards.ids.get(i).copied());
    route(app, id, out.size, out.background, out.pick_color);
}

/// The gear button and its menu: the four entries as check rows, Auto-nest Layers live, the others disabled.
pub fn gear(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let opener = Button::new(tl!("Options")).show(ui).response;
    let o = &app.ui.artboard_tool;
    let entries: Vec<Entry<Gear>> = GEAR.iter().map(|(g, label)| Entry::new(*g, label.to_string()).checked(g.checked(o)).enabled(g.enabled())).collect();
    let pick = Menu::new(tl!("Options")).opened_by(&opener, Trigger::Press).pick(ui, &entries);
    if pick.picked == Some(Gear::AutoNest) {
        app.ui.artboard_tool.auto_nest = !app.ui.artboard_tool.auto_nest;
    }
}
