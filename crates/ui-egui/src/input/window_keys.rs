//! Escape for PhotoCraft's own code beside the craft runtime.
//!
//! The runtime takes a plain Escape (Alt and Shift may be held) out of egui's input. While a
//! framework stop has focus it goes there; otherwise it reaches the window as `Intent::Cancel`
//! (craft-framework.md 2.2). While a PhotoCraft widget holds egui focus, the press stays in
//! egui's input instead. A pass carries one or the other, never both, so every reader here takes
//! whichever is there, once.
//!
//! The window intent carries no modifiers: they come from `ctx.input`, which still holds the
//! modifier state of the pass.

use craft::ui::runtime::keys::{Intent, Intents, with_window_intents};
use egui::{Context, Key, Modifiers};

fn rtl() -> bool {
    crate::i18n::current().dir() == crate::i18n::Dir::Rtl
}

/// Offers the window's Cancel to `take`. With `mods`, only when the held modifiers match them as
/// egui's `consume_key` would (extra Shift and Alt ignored); without, whatever is held, as egui's
/// `key_pressed`.
fn window_cancel(ctx: &Context, mods: Option<Modifiers>, take: bool) -> bool {
    if mods.is_some_and(|mods| !ctx.input(|i| i.modifiers.matches_logically(mods))) {
        return false;
    }
    let mut seen = false;
    let taken = with_window_intents(ctx, rtl(), |intents: &mut Intents| {
        seen = intents.take(Intent::Cancel);
        seen && take
    });
    if seen {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| d.insert_temp(seen_id(), pass));
    }
    taken || seen
}

fn seen_id() -> egui::Id {
    egui::Id::unique("photocraft.window_keys.escape_seen")
}

/// Takes this pass's Escape with no Command or Ctrl held (Shift and Alt may be), as
/// `consume_key(Modifiers::NONE, Key::Escape)` did.
pub fn take_escape(ctx: &Context) -> bool {
    take_escape_with(ctx, Modifiers::NONE)
}

/// Takes this pass's Escape when the held modifiers match `mods` logically.
pub fn take_escape_with(ctx: &Context, mods: Modifiers) -> bool {
    ctx.input_mut(|i| i.consume_key(mods, Key::Escape)) || window_cancel(ctx, Some(mods), true)
}

/// Takes the window's Cancel only, for a reader that walks egui's events itself and already sees
/// an Escape left there.
pub fn take_window_escape(ctx: &Context) -> bool {
    window_cancel(ctx, Some(Modifiers::NONE), true)
}

/// Whether this pass had an Escape, taken by an earlier reader or not.
pub fn escape_this_pass(ctx: &Context) -> bool {
    escape_pressed(ctx) || ctx.data(|d| d.get_temp::<u64>(seen_id())) == Some(ctx.cumulative_pass_nr())
}

/// Whether this pass has an Escape no reader has taken yet, whatever modifiers are held, without
/// taking it.
pub fn escape_pressed(ctx: &Context) -> bool {
    ctx.input(|i| i.key_pressed(Key::Escape)) || window_cancel(ctx, None, false)
}
