//! Temporary edge reveal never changes saved panel visibility.
use crate::PhotocraftApp;

#[derive(Clone, Copy, Default)]
pub(crate) struct Reveal {
    pub toolbar: bool,
    pub options: bool,
    pub dock: bool,
}
#[derive(Clone, Default)]
struct Hover {
    open: bool,
    left_at: Option<f64>,
    generation: u64,
}

pub(crate) fn reveal(app: &PhotocraftApp, ctx: &egui::Context) -> Reveal {
    let key = egui::Id::unique("hidden-panels-hover");
    if !app.session.prefs().workspace.auto_show_hidden_panels {
        ctx.data_mut(|d| d.remove::<Hover>(key));
        return Reveal::default();
    }
    let saved = app.ui.panels.hidden_by_tab.unwrap_or([false, false, true]);
    let hidden = Reveal { toolbar: !app.ui.panels.toolbar && saved[0], options: !app.ui.panels.options_bar && saved[1], dock: !app.ui.panels.dock && saved[2] };
    if !hidden.toolbar && !hidden.options && !hidden.dock {
        ctx.data_mut(|d| d.remove::<Hover>(key));
        return Reveal::default();
    }
    let screen = ctx.content_rect();
    let (pos, down, time, focused) = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.any_down(), i.time, i.focused));
    let mut hover = ctx.data_mut(|d| d.get_temp::<Hover>(key)).unwrap_or_default();
    if hover.generation != app.ui.docking_generation || !focused {
        hover = Hover::default();
        hover.generation = app.ui.docking_generation;
    }
    let edge = pos.is_some_and(|p| screen.contains(p) && (p.x <= screen.left() + 8.0 || p.x >= screen.right() - 8.0));
    if edge && !down && focused {
        hover.open = true;
        hover.left_at = None;
    }
    if hover.open {
        let mut areas = ctx.data_mut(|d| d.get_temp::<Vec<egui::Rect>>(key.with("areas"))).unwrap_or_default();
        for name in ["tools", "options"] {
            if let Some(rect) = ctx.data(|d| d.get_temp::<egui::Rect>(key.with(name))) {
                areas.push(rect);
            }
        }
        let inside = edge || pos.is_some_and(|p| areas.iter().any(|r| r.expand(6.0).contains(p))) || down || egui::Popup::is_any_open(ctx);
        if inside {
            hover.left_at = None;
        } else {
            let left = hover.left_at.get_or_insert(time);
            if time - *left >= 0.25 {
                hover.open = false;
                hover.left_at = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
    }
    let open = hover.open;
    ctx.data_mut(|d| d.insert_temp(key, hover));
    if open { hidden } else { Reveal::default() }
}

/// Remember actual chrome bounds so floating Tools and the options bar retain a temporary reveal.
pub(crate) fn chrome(ctx: &egui::Context, name: &str, rect: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("hidden-panels-hover").with(name), rect));
}

pub(crate) fn areas(ctx: &egui::Context, mut areas: Vec<egui::Rect>) {
    let screen = ctx.content_rect();
    areas.push(egui::Rect::from_min_size(screen.min, egui::vec2(crate::panel_docking::COMPACT_COLUMN_WIDTH, screen.height())));
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("hidden-panels-hover").with("areas"), areas));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(app: &PhotocraftApp, ctx: &egui::Context, p: egui::Pos2, time: f64, focused: bool) -> Reveal {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            time: Some(time),
            focused,
            events: vec![egui::Event::PointerMoved(p)],
            ..Default::default()
        };
        let mut result = Reveal::default();
        let mut output = ctx.run_ui(input, |_| result = reveal(app, ctx));
        output.textures_delta.clear();
        result
    }
    #[test]
    fn edge_hover_retains_floating_tools_then_closes_without_saving_visibility() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.panels.hidden_by_tab = Some([true, true, true]);
        app.ui.panels.toolbar = false;
        app.ui.panels.options_bar = false;
        app.ui.panels.dock = false;
        let saved = crate::dock::snapshot(&app);
        let ctx = egui::Context::default();
        chrome(&ctx, "tools", egui::Rect::from_min_size(egui::pos2(250.0, 150.0), egui::vec2(44.0, 400.0)));
        assert!(step(&app, &ctx, egui::pos2(1.0, 350.0), 0.0, true).toolbar);
        assert!(step(&app, &ctx, egui::pos2(270.0, 200.0), 0.5, true).toolbar);
        assert!(step(&app, &ctx, egui::pos2(600.0, 600.0), 1.0, true).dock);
        assert!(!step(&app, &ctx, egui::pos2(600.0, 600.0), 1.4, true).dock);
        assert_eq!(crate::dock::snapshot(&app), saved);
    }
    #[test]
    fn reveal_respects_preference_and_focus_and_does_not_reveal_explicitly_hidden_tools() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.panels.hidden_by_tab = Some([false, false, true]);
        app.ui.panels.toolbar = false;
        app.ui.panels.dock = false;
        let ctx = egui::Context::default();
        let at = egui::pos2(1.0, 350.0);
        let r = step(&app, &ctx, at, 0.0, true);
        assert!(r.dock && !r.toolbar);
        assert!(!step(&app, &ctx, at, 0.1, false).dock);
        app.session.prefs.edit(|p| p.workspace.auto_show_hidden_panels = false);
        assert!(!step(&app, &ctx, at, 0.2, true).dock);
    }
}
