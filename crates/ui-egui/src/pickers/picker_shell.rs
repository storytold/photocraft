//! Shared layout for preset pickers, independent of the asset-specific browser and commands.

pub(crate) use crate::pickers::asset_browser::MIN_THUMB;
use crate::pickers::asset_browser::{BrowserAction, BrowserAdapter, browser};
use crate::pickers::picker_footer::FooterOptions;
use crate::pickers::picker_state::PresetUi;

const FOOTER_GUESS: f32 = 32.0;
type UiCallback<'a> = &'a mut dyn FnMut(&mut egui::Ui);
type PanelOptionsCallback<'a> = &'a mut dyn FnMut(&mut egui::Ui, &mut PresetUi, &str);

/// The shared preset-picker menu button. Keep the glyph consistent across asset pickers.
pub(crate) fn picker_menu_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    crate::icons::button(ui, "settings", 24.0, false, tl!(label))
}

/// Common visibility control for the standard pickers' search row.
pub(crate) fn search_toggle(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str) {
    let visible = st.search_visible.entry(panel.to_string()).or_insert(true);
    if ui.checkbox(visible, tl!("Show Search Bar")).changed() && !*visible {
        st.search.remove(panel);
    }
}

/// Controls for the standard asset-browser shell. The standard footer belongs only to this
/// contract because this shell owns the browser viewport and translates footer actions.
#[derive(Default)]
pub(crate) struct ShellOptions<'a> {
    /// Optional consumer-owned header content above the shared picker controls.
    pub(crate) header: Option<UiCallback<'a>>,
    /// Optional consumer-owned menu button or settings controls.
    pub(crate) panel_menu: Option<UiCallback<'a>>,
    /// Standard-panel menu that can update the panel's persisted search visibility.
    pub(crate) panel_options: Option<PanelOptionsCallback<'a>>,
    /// Optional search control; the consumer owns its query state and filtering.
    pub(crate) search: Option<UiCallback<'a>>,
    /// Optional recent-items strip rendered above the browser.
    pub(crate) recents: Option<UiCallback<'a>>,
    /// Standard shared footer controls, when the picker needs them.
    pub(crate) footer: Option<FooterOptions<'a>>,
}

pub(crate) type CustomFooter<'a> = (UiCallback<'a>, f32);

/// Controls for a consumer-owned browser renderer. Its footer is a renderer plus an explicit
/// height reservation; it does not expose standard-browser options that this shell cannot honor.
#[derive(Default)]
pub(crate) struct CustomShellOptions<'a> {
    /// Optional consumer-owned header content above the shared picker controls.
    pub(crate) header: Option<UiCallback<'a>>,
    /// Optional consumer-owned menu button or settings controls.
    pub(crate) panel_menu: Option<UiCallback<'a>>,
    /// Optional search control; the consumer owns its query state and filtering.
    pub(crate) search: Option<UiCallback<'a>>,
    /// Optional recent-items strip rendered above the browser.
    pub(crate) recents: Option<UiCallback<'a>>,
    /// Consumer-owned footer renderer plus the height it reserves in the browser viewport.
    pub(crate) footer: Option<CustomFooter<'a>>,
    /// Total height budget for controls, browser and footer. When set, the shell subtracts its
    /// measured controls and footer before allocating the browser viewport.
    pub(crate) max_total_height: Option<f32>,
}

/// Compose the standard controls, generic browser viewport, and optional standard footer.
pub(crate) fn picker_shell(ui: &mut egui::Ui, st: &mut PresetUi, adapter: &mut BrowserAdapter<'_>, mut shell: ShellOptions<'_>) -> Vec<BrowserAction> {
    let panel = adapter.panel.to_string();
    shell_controls(ui, &mut shell, st, &panel);
    if matches!(panel.as_str(), "gradients" | "swatches" | "patterns") && st.search_visible.get(&panel).copied().unwrap_or(true) {
        let query = st.search.entry(panel.clone()).or_default();
        ui.add(egui::TextEdit::singleline(query).desired_width(ui.available_width()).hint_text(tl!("Search")));
    }

    let footer_id = egui::Id::new(("preset-footer", panel.as_str()));
    let footer_h = if shell.footer.is_some() { ui.data(|d| d.get_temp::<f32>(footer_id)).unwrap_or(FOOTER_GUESS) } else { 0.0 };
    // Do not let a consumer's legacy thumbnail-grid cap leave unused space below a short control
    // row; the browser itself scrolls within the viewport reserved by the shell.
    let allocated_h = (ui.available_height() - footer_h).max(0.0);
    let mut events = browser(ui, st, adapter, allocated_h);

    if let Some(footer) = shell.footer {
        let footer_top = ui.min_rect().bottom();
        ui.add_space(4.0);
        let footer_actions = crate::pickers::picker_footer::draw(ui, st, &panel, footer);
        events.extend(footer_actions.into_iter().map(|action| match action {
            crate::pickers::picker_footer::FooterAction::DeleteItem { group, group_path, key } => BrowserAction::DeleteItem { group, group_path, key },
            crate::pickers::picker_footer::FooterAction::New => BrowserAction::New,
            crate::pickers::picker_footer::FooterAction::NewGroup => BrowserAction::NewGroup,
        }));
        let measured = ui.min_rect().bottom() - footer_top;
        if measured.is_finite() && (measured - footer_h).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(footer_id, measured));
            ui.ctx().request_repaint();
        }
    }
    events
}

/// Shared shell layout for consumers with a specialised browser renderer.
pub(crate) fn picker_shell_custom<R>(
    ui: &mut egui::Ui,
    max_h: f32,
    mut shell: CustomShellOptions<'_>,
    draw_browser: impl FnOnce(&mut egui::Ui, f32) -> R,
) -> R {
    let controls_top = ui.cursor().min.y;
    shell_controls_custom(ui, &mut shell);
    let controls_h = (ui.cursor().min.y - controls_top).max(0.0);
    let footer_h = shell.footer.as_ref().map_or(0.0, |(_, h)| *h + 4.0);
    let browser_budget = shell.max_total_height.map_or(max_h, |total_h| (total_h - controls_h - footer_h).max(0.0));
    let allocated_h = browser_budget.min((ui.available_height() - footer_h).max(0.0));
    // Give the renderer a child whose allocated rectangle is the browser viewport. Passing a
    // height alone is advisory: a renderer that asks for more can otherwise grow the picker past
    // its window and push the footer out of the shell.
    let browser_size = egui::vec2(ui.max_rect().width(), allocated_h);
    let (browser_rect, _) = ui.allocate_exact_size(browser_size, egui::Sense::hover());
    let mut browser_ui = ui.new_child(egui::UiBuilder::new().max_rect(browser_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    browser_ui.set_min_size(browser_size);
    browser_ui.set_max_size(browser_size);
    browser_ui.set_clip_rect(browser_ui.clip_rect().intersect(browser_rect));
    let result = draw_browser(&mut browser_ui, allocated_h);
    if let Some((footer, _)) = shell.footer {
        ui.add_space(4.0);
        footer(ui);
    }
    result
}

fn shell_controls(ui: &mut egui::Ui, shell: &mut ShellOptions<'_>, st: &mut PresetUi, panel: &str) {
    if let Some(header) = shell.header.as_deref_mut() {
        ui.horizontal(|ui| header(ui));
    }
    if let Some(menu) = shell.panel_options.as_deref_mut() {
        ui.horizontal(|ui| menu(ui, st, panel));
    } else if let Some(menu) = shell.panel_menu.as_deref_mut() {
        ui.horizontal(|ui| menu(ui));
    }
    if let Some(search) = shell.search.as_deref_mut() {
        search(ui);
    }
    if let Some(recents) = shell.recents.as_deref_mut() {
        recents(ui);
    }
}

fn shell_controls_custom(ui: &mut egui::Ui, shell: &mut CustomShellOptions<'_>) {
    if let Some(header) = shell.header.as_deref_mut() {
        ui.horizontal(|ui| header(ui));
    }
    if let Some(menu) = shell.panel_menu.as_deref_mut() {
        ui.horizontal(|ui| menu(ui));
    }
    if let Some(search) = shell.search.as_deref_mut() {
        search(ui);
    }
    if let Some(recents) = shell.recents.as_deref_mut() {
        recents(ui);
    }
}
