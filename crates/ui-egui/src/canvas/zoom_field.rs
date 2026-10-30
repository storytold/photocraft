//! Photoshop's rules for a zoom field (status bar, Navigator) while a number is typed into it.
//!
//! A typed zoom is held until Enter (or a click on something other than the pasteboard). Until
//! then the image keeps its zoom, and these keys never apply what was typed:
//!
//! * Esc drops it: the field shows the current zoom again.
//! * Tab and ⇧Tab keep the focus in the field, select its text so typing replaces it, and play
//!   the system alert (Windows: the Asterisk sound).
//! * Alt (tapped) does only that alert: it neither puts the keyboard on the menu bar nor leaves
//!   the field.
//! * A click, with either button, on the pasteboard (the canvas around the image), anywhere on the
//!   title bar (menu titles, the empty drag area, the workspace switcher, search, the theme
//!   button, Minimise / Maximise / Close), or on an open menu or dropdown (Help's "Search menus"
//!   field included), keeps the typed number unapplied and selects the text again, as Tab does.
//! * Whatever the key or click, a typed number is never applied while the window is not focused.
//! * Only digits, `.`, `,` and `/` can be typed or pasted; any other character is dropped and asks for
//!   the same alert, without touching the text.
//! * `,` is a decimal point like `.` (`100,90` is `100.90`). The text holds at most one of them: a
//!   second `.` or `,`, a `,` next to a `.` (`1,000.5`) and a `.` next to a `/` (`100/5.0`; the ratio
//!   takes `100/5,0`) are typed away like any other character: dropped, with the alert. A paste is
//!   not cut up: `100/5.0` stays in the field, asks for the alert, and Enter puts the current zoom back.
//!
//! The field itself is `widgets::number_edit` (with `hold_typed`), which calls [`before`] and
//! [`after`] around the egui `DragValue`. The rest is bookkeeping other parts of the app read:
//! [`editing`] (the shortcuts must not take Esc or Tab in the frame egui drops the focus),
//! [`is_focused`] (the menu bar must not take Alt) and [`tick`] (plays the alert, and tells the
//! field about a press on the pasteboard, which only the app can recognise).

use egui::{Context, FocusDirection, Id, Key, Modifiers, SurrenderFocusOn, Ui};
use photocraft_doc::DocId;

fn editing_id() -> Id {
    Id::unique("pc-zoom-field-editing")
}

fn pasteboard_id() -> Id {
    Id::unique("pc-zoom-field-pasteboard-press")
}

fn menu_bar_rect_id() -> Id {
    Id::unique("pc-zoom-field-menu-bar-rect")
}

fn alert_id() -> Id {
    Id::unique("pc-zoom-field-alert")
}

fn keep_pass_id() -> Id {
    Id::unique("pc-zoom-field-keep-pass")
}

fn select_until_id() -> Id {
    Id::unique("pc-zoom-field-select-until")
}

fn title_bar_rect_id() -> Id {
    Id::unique("pc-zoom-field-title-bar-rect")
}

fn open_docs_id() -> Id {
    Id::unique("pc-zoom-field-open-docs")
}

/// What [`before`] decided for this frame, for [`after`] and the field.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Guard {
    /// The setting to put back: egui's "surrender focus on" was switched off for this frame.
    restore: Option<SurrenderFocusOn>,
    /// Esc was pressed in the field: whatever was typed is dropped, the field leaves the keyboard.
    esc: bool,
    /// Whatever egui does with the typed text this frame, it must not reach the zoom: Esc, Tab, a
    /// click that keeps the typed number, a window that has lost the keyboard.
    veto: bool,
}

impl Guard {
    /// The field must not apply a typed number this frame (a scrub is not typed and is exempt).
    pub(crate) fn vetoes_typed(self) -> bool {
        self.veto
    }
}

/// Makes the `TextEdit` of the field being drawn select all of its text, the way Ctrl+A does:
/// the key is put in this frame's input, where the `TextEdit` reads it as typed.
fn select_all_by_key(ui: &Ui) {
    let ctrl_a = egui::Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    ui.input_mut(|i| i.events.push(ctrl_a));
}

/// A zoom takes digits, `.`, `,` and `/` only (`/` for a ratio: `1/2` is 50 %, `5/2` is 250 %;
/// the `%` is drawn beside the field, never typed). Anything else typed or pasted is taken out of this
/// frame's input before the field sees it; true when something was.
///
/// A typed `.`, `,` or `/` that would make the text [`violates`] the rules (a second separator, `,`
/// with `.`, `.` with `/`) is taken out too. The text it would land in is the one the `DragValue`
/// keeps (`shown` before the first key), minus the selection. A paste is only filtered by
/// character, so what is invalid stays in the field, for the alert in [`after`] and for Enter.
///
/// egui keeps the text being edited in a private `EditState`, out of reach, so this module keeps
/// its own copy under [`mirror_id`]: what the field showed when it got the keyboard, with the keys
/// applied that are seen here (typed, pasted, cut, Backspace, Delete). A key that changes the text
/// in a way not followed (undo, a word deletion) makes the copy empty: the rules then only look at
/// what is typed from there on, and Enter still refuses text that [`violates`] them.
fn drop_unwanted_chars(ui: &Ui, id: Id, shown: &str) -> bool {
    let mirror = mirror_id(id);
    let stored = ui.data(|d| d.get_temp::<String>(mirror));
    let mut text: Vec<char> = stored.clone().unwrap_or_else(|| shown.to_owned()).chars().collect();
    let len = text.len();
    let range = egui::TextEdit::load_state(ui.ctx(), id).and_then(|s| s.cursor.char_range());
    // Before the first key the whole number is selected, as the field is reached.
    let mut sel = match (&stored, range) {
        (Some(_), Some(r)) => {
            let (a, b) = (r.primary.index.0, r.secondary.index.0);
            (a.min(b).min(len), a.max(b).min(len))
        }
        (Some(_), None) => (len, len),
        (None, _) => (0, len),
    };
    let allowed = |c: char| c.is_ascii_digit() || matches!(c, '.' | ',' | '/');
    let mut stale = false;
    let dropped = ui.input_mut(|i| {
        let mut dropped = false;
        i.events.retain_mut(|e| match e {
            egui::Event::Text(t) => {
                let mut kept = String::new();
                for c in t.chars() {
                    let mut next = text.clone();
                    next.drain(sel.0..sel.1);
                    next.insert(sel.0, c);
                    if c.is_ascii_digit() || (allowed(c) && !violates(&next.iter().collect::<String>())) {
                        text = next;
                        sel = (sel.0 + 1, sel.0 + 1);
                        kept.push(c);
                    }
                }
                dropped |= kept.chars().count() != t.chars().count();
                *t = kept;
                !t.is_empty()
            }
            egui::Event::Paste(t) => {
                let kept: String = t.chars().filter(|&c| allowed(c)).collect();
                dropped |= kept.chars().count() != t.chars().count();
                text.drain(sel.0..sel.1);
                for (n, c) in kept.chars().enumerate() {
                    text.insert(sel.0 + n, c);
                }
                let at = sel.0 + kept.chars().count();
                sel = (at, at);
                *t = kept;
                !t.is_empty()
            }
            egui::Event::Cut => {
                text.drain(sel.0..sel.1);
                sel = (sel.0, sel.0);
                true
            }
            egui::Event::Key { key: key @ (Key::Backspace | Key::Delete), pressed: true, modifiers, .. } => {
                if modifiers.is_none() || (*key == Key::Backspace && modifiers.shift_only()) {
                    if sel.0 != sel.1 {
                        text.drain(sel.0..sel.1);
                        sel = (sel.0, sel.0);
                    } else if *key == Key::Backspace && sel.0 > 0 {
                        text.remove(sel.0 - 1);
                        sel = (sel.0 - 1, sel.0 - 1);
                    } else if *key == Key::Delete && sel.0 < text.len() {
                        text.remove(sel.0);
                    }
                } else {
                    stale = true;
                }
                true
            }
            egui::Event::Key { key: Key::Z | Key::Y, pressed: true, modifiers, .. } => {
                stale |= modifiers.command;
                true
            }
            _ => true,
        });
        dropped
    });
    if stale {
        text.clear();
    }
    ui.data_mut(|d| d.insert_temp(mirror, text.iter().collect::<String>()));
    dropped
}

/// Where this module keeps its copy of the text being edited in the field with `id`.
fn mirror_id(id: Id) -> Id {
    id.with("pc-zoom-field-text")
}

/// Call right before the field's `DragValue` is added; `id` is that `DragValue`'s id and `shown` the
/// text it shows when nothing was typed yet.
pub(crate) fn before(ui: &Ui, id: Id, shown: &str) -> Guard {
    let ctx = ui.ctx();
    let pass = ctx.cumulative_pass_nr();
    let focused = ui.memory(|m| m.has_focus(id));
    // Esc: take it (no shortcut, no job cancel, no canvas acts on it), and drop what was typed.
    // Whether egui dropped the focus already (the key's own frame) or not, the field applies
    // nothing this frame (`Guard::vetoes_typed`) and leaves the keyboard (`after`).
    let owner = focused || editing_field(ctx) == Some(id);
    // Esc is also looked for in the frame's raw input: something may have consumed it before the
    // field was drawn (a shortcut, a job), and egui has dropped the focus on it already.
    let raw_esc = |ui: &Ui| {
        ui.input(|i| i.raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Escape, pressed: true, modifiers, .. } if modifiers.is_none())))
    };
    // Only the field that owns the keyboard takes Esc: any other must leave it to whoever wants it.
    if owner && (ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) | raw_esc(ui)) {
        ui.data_mut(|d| d.remove_temp::<String>(mirror_id(id)));
        return Guard { esc: true, veto: true, ..Default::default() };
    }
    // The focus left before the field was drawn, and no click took it (egui drops the focus on
    // some keys and a shortcut may then eat them, so the `DragValue` cannot rely on seeing them):
    // whatever was typed is dropped, never applied. A click away still applies.
    if !focused {
        ui.data_mut(|d| d.remove_temp::<String>(mirror_id(id)));
    }
    // A click that keeps what was typed (pasteboard, title bar, open menu), or a window that has
    // lost the keyboard: whatever egui does with the typed text, it does not reach the zoom. This
    // holds without the focus too (the click may have given it to another field earlier in this
    // frame, and egui hands a field its lost focus, with the text, when it is next drawn), and for
    // two frames after the click (the focus can go in one frame and be noticed in the next).
    let gesture = pasteboard_press(ctx);
    if gesture {
        ctx.data_mut(|d| d.insert_temp(keep_pass_id(), pass));
    }
    let recent = ctx.data(|d| d.get_temp::<u64>(keep_pass_id())).is_some_and(|p| pass <= p + 2);
    let veto = recent || ui.input(|i| i.viewport().focused) == Some(false);
    if !focused {
        return Guard { veto, ..Default::default() };
    }
    if drop_unwanted_chars(ui, id, shown) {
        request_alert(ctx);
    }
    // Tab: egui has already planned the move to the next widget; cancel it. The text is selected
    // now, and again in the next frames until something is typed: whatever moves the text cursor
    // meanwhile does not undo it.
    let tab = ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab) | i.consume_key(Modifiers::NONE, Key::Tab));
    if tab {
        ctx.memory_mut(|m| m.move_focus(FocusDirection::None));
        request_alert(ctx);
        select_all_by_key(ui);
        ctx.data_mut(|d| d.insert_temp(select_until_id(), pass + 3));
        ctx.request_repaint();
        return Guard { veto: true, ..Default::default() };
    }
    if let Some(until) = ctx.data(|d| d.get_temp::<u64>(select_until_id())) {
        let acted = ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: true, .. }
                )
            })
        });
        if acted || pass > until {
            ctx.data_mut(|d| d.remove_temp::<u64>(select_until_id()));
        } else {
            select_all_by_key(ui);
            ctx.request_repaint();
        }
    }
    if gesture {
        // egui drops the focus of the focused widget when a click lands elsewhere, and the
        // `DragValue` then applies the text; for this frame, neither.
        let prev = ctx.options(|o| o.input_options.surrender_focus_on);
        ctx.options_mut(|o| o.input_options.surrender_focus_on = SurrenderFocusOn::Never);
        // Selected when the button goes down and when it comes up, not on every frame it is held.
        if ui.input(|i| i.pointer.any_pressed() || i.pointer.any_released()) {
            select_all_by_key(ui);
        }
        return Guard { restore: Some(prev), veto: true, ..Default::default() };
    }
    Guard { veto, ..Default::default() }
}

/// Call right after the field's `DragValue` was added.
pub(crate) fn after(ui: &Ui, id: Id, guard: Guard) {
    let ctx = ui.ctx();
    if let Some(prev) = guard.restore {
        ctx.options_mut(|o| o.input_options.surrender_focus_on = prev);
    }
    if guard.esc {
        // Esc: the typed text goes, and so does the keyboard, so the field shows the zoom again.
        ui.data_mut(|d| d.remove_temp::<String>(mirror_id(id)));
        ui.memory_mut(|m| m.surrender_focus(id));
        ctx.request_repaint();
        return;
    }
    if !ui.memory(|m| m.has_focus(id)) {
        return;
    }
    // Text the rules refuse, which only a paste can bring in: it stays (Enter then restores the zoom),
    // but every key or paste into it asks for the alert.
    let edited = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Paste(_))));
    if edited && ui.data(|d| d.get_temp::<String>(mirror_id(id))).is_some_and(|t| violates(&t)) {
        request_alert(ctx);
    }
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(editing_id(), (id, pass)));
}

/// What a zoom field's text must not hold: more than one `.`, more than one `,`, a `,` with a `.`
/// (`1,000.5`), or a `.` with a `/` (`100/5.0`: the decimal separator of a ratio is the comma,
/// `100/5,0`).
fn violates(text: &str) -> bool {
    let (points, commas) = (text.matches('.').count(), text.matches(',').count());
    points + commas > 1 || (points > 0 && text.contains('/'))
}

/// The text of a zoom field as the number parser must read it: `,` is a decimal point, and text that
/// [`violates`] the rules is no number at all (`None`).
pub(crate) fn parse_ready(text: &str) -> Option<String> {
    (!violates(text)).then(|| text.replace(',', "."))
}

fn editing_field(ctx: &Context) -> Option<Id> {
    let (id, pass) = ctx.data(|d| d.get_temp::<(Id, u64)>(editing_id()))?;
    // Drawn with the focus in the last frame (the one before this): in the frame egui drops
    // the focus for Esc or Tab, nothing is focused any more, but the field is still the owner.
    (pass + 1 >= ctx.cumulative_pass_nr()).then_some(id)
}

/// A zoom field had the keyboard in the last frame. Like a layer rename, it keeps its keys even in
/// the frame egui drops its focus (Esc, Tab), so the shortcuts must treat it as text.
pub fn editing(ctx: &Context) -> bool {
    editing_field(ctx).is_some()
}

/// A zoom field has the keyboard right now.
pub fn is_focused(ctx: &Context) -> bool {
    editing_field(ctx).is_some_and(|id| ctx.memory(|m| m.focused()) == Some(id))
}

/// The system alert (Windows: Asterisk) is wanted; [`tick`] plays it in the next frame.
pub fn request_alert(ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(alert_id(), true));
    ctx.request_repaint();
}

/// Whether the alert is waiting to be played (tests).
#[cfg(test)]
pub(crate) fn alert_pending(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(alert_id())).unwrap_or(false)
}

/// Remembers the rectangle of the whole title bar (menus, the empty drag area, workspace switcher,
/// search and theme buttons, the caption buttons) for [`on_title_bar`].
pub(crate) fn set_title_bar_rect(ctx: &Context, rect: egui::Rect) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(title_bar_rect_id(), (rect, pass)));
}

/// Remembers the rectangle of the menu bar titles (`File` … `Help`) for [`on_menu_bar`].
pub(crate) fn set_menu_bar_rect(ctx: &Context, rect: egui::Rect) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(menu_bar_rect_id(), (rect, pass)));
}

/// Does the rectangle kept under `id` (with the pass it was drawn in) contain `pos`, and was it drawn
/// recently enough to still be there? Not drawn for a while (hidden by Tab or Full Screen Mode), its
/// rect is stale and must not catch a click. The one freshness rule of the title bar and the menu bar.
fn fresh_rect_contains(ctx: &Context, id: Id, pos: egui::Pos2) -> bool {
    ctx.data(|d| d.get_temp::<(egui::Rect, u64)>(id)).is_some_and(|(r, pass)| pass + 2 >= ctx.cumulative_pass_nr() && r.contains(pos))
}

/// A button went down on the pasteboard while a zoom field was being edited, and is still down or
/// came up in this very frame. (egui drops the focus of a text field on the click, which is the
/// release, so the press alone is not the whole gesture.)
fn pasteboard_press(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(pasteboard_id())).unwrap_or(false)
}

/// Once per frame ([`tick`], tests): `pressed` says a button went down on the pasteboard in this
/// frame. The gesture lasts until the frame after the button came up.
pub(crate) fn track_pasteboard(ctx: &Context, pressed: bool) {
    let (down, released) = ctx.input(|i| (i.pointer.any_down(), i.pointer.any_released()));
    ctx.data_mut(|d| {
        if pressed {
            d.insert_temp(pasteboard_id(), true);
        } else if !down && !released {
            d.remove::<bool>(pasteboard_id());
        }
    });
}

// ---- What needs the application: services and the document's geometry. ----

/// A document that was open in the last frame and is not now has closed: the glide a typed zoom left
/// for it is dropped ([`crate::zoom_levels::forget_pending`]), so nothing is left that another document
/// could take. `open` is every open document, as of this frame.
pub(crate) fn forget_closed(ctx: &Context, open: &[DocId]) {
    let before = ctx.data(|d| d.get_temp::<Vec<DocId>>(open_docs_id())).unwrap_or_default();
    if before == open {
        return;
    }
    for gone in before.iter().filter(|doc| !open.contains(doc)) {
        crate::zoom_levels::forget_pending(ctx, *gone);
    }
    ctx.data_mut(|d| d.insert_temp(open_docs_id(), open.to_vec()));
}

/// Once per frame, before the panels: plays a requested alert, follows a press on the pasteboard, and
/// drops the glide of a document that closed.
pub fn tick(app: &mut crate::PhotocraftApp, ctx: &Context) {
    let open: Vec<DocId> = app.session.documents().iter().map(|d| d.doc.id).collect();
    forget_closed(ctx, &open);
    if ctx.data_mut(|d| d.remove_temp::<bool>(alert_id())).unwrap_or(false)
        // The platform's alert, else its plain beep: an app that only sets `beep` still answers.
        && let Some(play) = app.services.alert.or(app.services.beep)
    {
        play();
    }
    let press =
        if editing(ctx) { ctx.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin().or(i.pointer.interact_pos()) } else { None }) } else { None };
    track_pasteboard(ctx, press.is_some_and(|pos| on_pasteboard(app, ctx, pos) || on_menu_bar(ctx, pos) || on_title_bar(ctx, pos) || on_open_popup(ctx, pos)));
}

/// Is screen point `pos` anywhere on the title bar: the menu titles and the gaps between them, the
/// workspace switcher (Essentials), search, the theme button, Minimise / Maximise / Close?
fn on_title_bar(ctx: &Context, pos: egui::Pos2) -> bool {
    fresh_rect_contains(ctx, title_bar_rect_id(), pos)
}

/// Is screen point `pos` on an open menu or dropdown (Help's "Search menus" field, a menu item,
/// the workspace list)? A menu or dropdown is open and `pos` is on a layer above the canvas.
fn on_open_popup(ctx: &Context, pos: egui::Pos2) -> bool {
    (egui::Popup::is_any_open(ctx) || crate::menu_nav::is_open(ctx)) && ctx.layer_id_at(pos).is_some_and(|l| l.order != egui::Order::Background)
}

/// Is screen point `pos` on one of the menu bar's top-level menu titles (`File`, `Edit`, …)?
fn on_menu_bar(ctx: &Context, pos: egui::Pos2) -> bool {
    fresh_rect_contains(ctx, menu_bar_rect_id(), pos)
}

/// Is screen point `pos` on the main canvas, outside the image, and not under a window or menu?
fn on_pasteboard(app: &crate::PhotocraftApp, ctx: &Context, pos: egui::Pos2) -> bool {
    let (Some(xf), Some(st)) = (crate::canvas::ViewXform::active(app), app.session.active()) else { return false };
    let doc = xf.to_doc(pos);
    let (w, h) = (f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    xf.rect.contains(pos)
        && doc.iter().all(|v| v.is_finite())
        && (doc[0] < 0.0 || doc[1] < 0.0 || doc[0] >= w || doc[1] >= h)
        && ctx.layer_id_at(pos).is_some_and(|l| l.order == egui::Order::Background)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comma_is_a_decimal_point_and_a_point_with_a_ratio_is_invalid() {
        assert_eq!(parse_ready("100,90").as_deref(), Some("100.90"));
        assert_eq!(parse_ready("100.90").as_deref(), Some("100.90"));
        assert_eq!(parse_ready("100/5,0").as_deref(), Some("100/5.0"));
        assert_eq!(parse_ready("100/5.0"), None);
        assert_eq!(parse_ready("1.5/2"), None);
        assert_eq!(parse_ready("1,5/2").as_deref(), Some("1.5/2"));
        for refused in ["1,000.5", "1.000,5", "1.5.2", "1,5,2", "1,5/2,5"] {
            assert_eq!(parse_ready(refused), None, "{refused}");
        }
    }

    #[test]
    fn the_title_bar_keeps_a_click_wherever_it_is_drawn() {
        let ctx = Context::default();
        let bar = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 32.0));
        assert!(!on_title_bar(&ctx, egui::pos2(10.0, 10.0)), "nothing drawn yet");
        set_title_bar_rect(&ctx, bar);
        assert!(on_title_bar(&ctx, egui::pos2(790.0, 10.0)), "the far end: Essentials, Minimise, Close");
        assert!(on_title_bar(&ctx, egui::pos2(400.0, 10.0)), "the empty drag area between the menus and them");
        assert!(!on_title_bar(&ctx, egui::pos2(10.0, 100.0)), "below the bar is the app");
    }

    /// The glide a typed zoom leaves belongs to its document and goes when the document closes, so a
    /// document that takes over its position in the views never inherits it.
    #[test]
    fn a_document_that_closed_leaves_no_zoom_glide() {
        let ctx = Context::default();
        let (a, b) = (DocId(10), DocId(20));
        let pending = |doc| ctx.data(|d| d.get_temp::<f32>(crate::zoom_levels::pending_id(doc)));
        let mut view = crate::state::View { zoom: 1.0, doc_size: [2000, 1500], ..Default::default() };
        forget_closed(&ctx, &[a, b]);
        crate::zoom_levels::set_from_field(&ctx, a, &mut view, false, 200.0);
        crate::zoom_levels::set_from_field(&ctx, b, &mut view, false, 50.0);
        // Nothing closed: both keep their glide.
        forget_closed(&ctx, &[a, b]);
        assert!(pending(a).is_some() && pending(b).is_some());
        // `a` closes, `b` moves to `a`'s position: only `b` keeps its glide.
        forget_closed(&ctx, &[b]);
        assert_eq!(pending(a), None, "the closed document's glide is gone");
        assert!(pending(b).is_some(), "the other document keeps its own");
    }

    /// The menu bar rect goes stale like the title bar's: not drawn for a while, it catches nothing.
    #[test]
    fn a_menu_bar_that_is_no_longer_drawn_keeps_no_click() {
        let ctx = Context::default();
        let bar = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 32.0));
        let inside = egui::pos2(50.0, 10.0);
        assert!(!on_menu_bar(&ctx, inside), "nothing drawn yet");
        // Drawn in the pass the clock is at now.
        set_menu_bar_rect(&ctx, bar);
        assert!(on_menu_bar(&ctx, inside), "just drawn");
        assert!(!on_menu_bar(&ctx, egui::pos2(500.0, 10.0)), "outside the titles");
        // A few passes later, with the menu bar not drawn again (hidden): the rect is stale.
        for _ in 0..3 {
            // The output is dropped: its texture deltas must be cleared first, or egui panics.
            ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        }
        assert!(!on_menu_bar(&ctx, inside), "stale: the bar is no longer drawn");
        // Drawn again: fresh again.
        set_menu_bar_rect(&ctx, bar);
        assert!(on_menu_bar(&ctx, inside), "drawn again");
    }
}
