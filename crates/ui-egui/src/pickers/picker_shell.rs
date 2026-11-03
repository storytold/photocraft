//! Shared layout for preset pickers, independent of the asset-specific browser and commands.

use crate::pickers::asset_browser::{BrowserAction, BrowserAdapter, browser};
use crate::pickers::picker_footer::FooterOptions;
use crate::pickers::picker_state::{PickerDisplayMode, PresetUi};

const FOOTER_GUESS: f32 = 32.0;
type UiCallback<'a> = &'a mut dyn FnMut(&mut egui::Ui);
pub(crate) type MenuContentsCallback<'a> = &'a mut dyn FnMut(&mut egui::Ui, &mut PresetUi, &str);
/// Shared menu points offered by an asset-picker shell. Consumers choose capabilities, never
/// ordering or separators; domain actions are appended through the final extension point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PickerMenuOptions {
    pub(crate) display: bool,
    pub(crate) search_visibility: bool,
}

impl PickerMenuOptions {
    pub(crate) fn for_panel(panel: &str) -> Self {
        Self { display: true, search_visibility: matches!(panel, "gradients" | "patterns" | "swatches" | "brush-picker") }
    }
}

#[cfg(test)]
fn picker_menu_popup(ui: &mut egui::Ui, label: &str, anchor_offset: egui::Vec2, contents: impl FnOnce(&mut egui::Ui)) {
    let button = crate::widgets::panel_menu_button(ui, label);
    let anchor = match egui::PopupAnchor::from(&button) {
        egui::PopupAnchor::ParentRect(rect) if anchor_offset != egui::Vec2::ZERO => egui::PopupAnchor::ParentRect(rect.translate(anchor_offset)),
        other => other,
    };
    egui::Popup::menu(&button).anchor(anchor).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        crate::widgets::style_spectrum_popup_menu(ui);
        contents(ui);
    });
}

/// Render the shared display controls without owning section boundaries; the composer owns them.
fn display_mode_items(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str) {
    let selected = st.display_mode(panel);
    for mode in PickerDisplayMode::ALL {
        let label = match mode {
            PickerDisplayMode::TextOnly => tl!("Text Only"),
            PickerDisplayMode::SmallThumbnail => tl!("Small Thumbnail"),
            PickerDisplayMode::LargeThumbnail => tl!("Large Thumbnail"),
            PickerDisplayMode::SmallList => tl!("Small List"),
            PickerDisplayMode::LargeList => tl!("Large List"),
        };
        if menu_check_item(ui, selected == mode, label, true).clicked() {
            st.display_modes.insert(panel.to_owned(), mode);
        }
    }
}

fn wrap_list_item(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str) {
    let selected = st.display_mode(panel);
    let list_mode = selected.is_list();
    let wrap = st.wrap_list.get(panel).copied().unwrap_or(true);
    if menu_check_item(ui, wrap, tl!("Wrap List"), list_mode).clicked() && list_mode {
        st.wrap_list.insert(panel.to_owned(), !wrap);
    }
}

/// Render a checked menu row with the same fixed checkmark gutter as the application menu bar.
fn menu_check_item(ui: &mut egui::Ui, checked: bool, label: &str, enabled: bool) -> egui::Response {
    let text = crate::menu_keys::menu_row_text(ui, checked, label, None);
    let enabled = enabled && ui.is_enabled();
    let response = ui.add_enabled(enabled, egui::Button::new(text));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
}

/// Common visibility control for the standard pickers' search row.
fn search_toggle(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str) {
    let visible = st.search_visible.entry(panel.to_string()).or_insert(true);
    if ui.checkbox(visible, tl!("Show Search Bar")).changed() && !*visible {
        st.search.remove(panel);
    }
}

/// Compose a preset picker's menu. The shell owns shared points, their order, and section boundary;
/// consumers contribute only their optional domain-specific entries.
pub(crate) fn picker_menu_contents(ui: &mut egui::Ui, state: &mut PresetUi, panel: &str, options: PickerMenuOptions, extra: Option<MenuContentsCallback<'_>>) {
    let mut has_shared_points = false;
    if options.display {
        display_mode_items(ui, state, panel);
        ui.separator();
        wrap_list_item(ui, state, panel);
        has_shared_points = true;
    }
    if options.search_visibility {
        if has_shared_points {
            ui.separator();
        }
        search_toggle(ui, state, panel);
        has_shared_points = true;
    }
    // One shell-owned boundary separates shared controls from domain extensions. The presence of
    // an extension is the source of truth; callers never coordinate a second separator flag.
    if let Some(extra) = extra {
        if has_shared_points {
            ui.separator();
        }
        extra(ui, state, panel);
    }
}

/// Controls for the standard asset-browser shell. The standard footer belongs only to this
/// contract because this shell owns the browser viewport and translates footer actions.
#[derive(Default)]
pub(crate) struct ShellOptions<'a> {
    /// Optional consumer-owned header content above the shared picker controls.
    pub(crate) header: Option<UiCallback<'a>>,
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
    shell_controls(ui, &mut shell);
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

fn shell_controls(ui: &mut egui::Ui, shell: &mut ShellOptions<'_>) {
    if let Some(header) = shell.header.as_deref_mut() {
        ui.horizontal(|ui| header(ui));
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
    if let Some(search) = shell.search.as_deref_mut() {
        search(ui);
    }
    if let Some(recents) = shell.recents.as_deref_mut() {
        recents(ui);
    }
}

#[cfg(test)]
mod display_menu_tests {
    use super::{PickerMenuOptions, picker_menu_contents, picker_menu_popup};
    use crate::pickers::picker_state::PresetUi;
    use egui::vec2;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    #[test]
    fn panel_menu_trigger_exposes_all_five_display_modes_and_wrap_list() {
        assert!(crate::icons::exists("align-justify") && crate::icons::exists("ellipsis"), "the shared trigger uses each theme's panel-menu glyph");
        let mut harness = Harness::builder().with_size(vec2(420.0, 260.0)).build_ui_state(
            |ui, state: &mut PresetUi| {
                picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    picker_menu_contents(ui, state, "test-picker", PickerMenuOptions::for_panel("test-picker"), None);
                });
            },
            PresetUi::default(),
        );
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);
        assert_eq!(harness.get_by_label("Panel menu").rect().size(), vec2(20.0, 20.0), "the picker reuses the standard panel-menu hit area");
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);

        let modes = ["Text Only", "Small Thumbnail", "Large Thumbnail", "Small List", "Large List"];
        let mut previous_bottom = None;
        for label in modes {
            let rect = harness.get_by_label(label).rect();
            if let Some(bottom) = previous_bottom {
                assert!(rect.top() >= bottom, "display modes must appear together in the requested order");
            }
            previous_bottom = Some(rect.bottom());
        }
        let wrap_rect = harness.get_by_label("Wrap List").rect();
        assert!(wrap_rect.top() > previous_bottom.unwrap() + 4.0, "Wrap List must follow a menu separator after the display modes");
    }

    #[test]
    fn standard_picker_shell_does_not_render_a_picker_local_menu_trigger() {
        use super::{ShellOptions, shell_controls};

        let mut harness = Harness::builder()
            .with_size(vec2(420.0, 260.0))
            .build_ui_state(|ui, _state: &mut PresetUi| shell_controls(ui, &mut ShellOptions::default()), PresetUi::default());
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        assert!(harness.query_by_label("Panel menu").is_none(), "the containing panel owns the only menu trigger");
    }

    #[test]
    fn shared_picker_menu_exposes_search_visibility_for_searchable_panels() {
        let mut harness = Harness::builder().with_size(vec2(420.0, 260.0)).build_ui_state(
            |ui, state: &mut PresetUi| {
                picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    picker_menu_contents(ui, state, "gradients", PickerMenuOptions::for_panel("gradients"), None);
                });
            },
            PresetUi::default(),
        );
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);
        harness.get_by_label("Show Search Bar").click();
        harness.run_steps(2);
        assert_eq!(harness.state().search_visible.get("gradients"), Some(&false));
    }

    #[test]
    fn shared_menu_orders_shell_points_before_adapter_extension() {
        let mut harness = Harness::builder().with_size(vec2(420.0, 320.0)).build_ui_state(
            |ui, state: &mut PresetUi| {
                picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    let mut extension = |ui: &mut egui::Ui, _state: &mut PresetUi, _panel: &str| {
                        let _ = ui.button("Adapter action");
                    };
                    picker_menu_contents(ui, state, "gradients", PickerMenuOptions::for_panel("gradients"), Some(&mut extension));
                });
            },
            PresetUi::default(),
        );
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);
        let display = harness.get_by_label("Text Only").rect();
        let search = harness.get_by_label("Show Search Bar").rect();
        let adapter = harness.get_by_label("Adapter action").rect();
        assert!(display.bottom() <= search.top(), "display controls precede search visibility");
        assert!(search.bottom() <= adapter.top(), "the shell's extension point follows shared controls");
    }

    #[test]
    fn panels_without_a_search_row_do_not_receive_a_search_menu_point() {
        let mut harness = Harness::builder().with_size(vec2(420.0, 260.0)).build_ui_state(
            |ui, state: &mut PresetUi| {
                picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                    picker_menu_contents(ui, state, "styles", PickerMenuOptions::for_panel("styles"), None);
                });
            },
            PresetUi::default(),
        );
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Panel menu").click();
        harness.run_steps(2);
        assert!(harness.query_by_label("Show Search Bar").is_none());
        assert!(harness.query_by_label("Text Only").is_some());
    }

    #[test]
    fn wrap_list_is_ineffective_and_not_mutated_outside_list_modes() {
        use crate::pickers::picker_state::PickerDisplayMode as Mode;

        for mode in [Mode::SmallThumbnail, Mode::LargeThumbnail] {
            let mut state = PresetUi::default();
            state.display_modes.insert("test-picker".into(), mode);
            state.wrap_list.insert("test-picker".into(), true);
            let mut harness = Harness::builder().with_size(vec2(420.0, 260.0)).build_ui_state(
                |ui, state: &mut PresetUi| {
                    picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                        picker_menu_contents(ui, state, "test-picker", PickerMenuOptions::for_panel("test-picker"), None);
                    });
                },
                state,
            );
            crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
            harness.run_steps(2);
            harness.get_by_label("Panel menu").click();
            harness.run_steps(2);
            assert_eq!(harness.state().wrap_list.get("test-picker"), Some(&true));
            assert!(!harness.state().wrap_list_enabled("test-picker"));
            assert!(harness.get_by_label("Wrap List").accesskit_node().is_disabled(), "Wrap List must be disabled for {mode:?}");
            let _ = harness.get_by_label("Wrap List").rect();
        }
    }

    #[test]
    fn wrap_list_menu_checkbox_toggles_in_all_three_list_modes() {
        use crate::pickers::picker_state::PickerDisplayMode as Mode;

        for mode in [Mode::TextOnly, Mode::SmallList, Mode::LargeList] {
            let mut state = PresetUi::default();
            state.display_modes.insert("test-picker".into(), mode);
            let mut harness = Harness::builder().with_size(vec2(420.0, 260.0)).build_ui_state(
                |ui, state: &mut PresetUi| {
                    picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                        picker_menu_contents(ui, state, "test-picker", PickerMenuOptions::for_panel("test-picker"), None);
                    });
                },
                state,
            );
            crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
            harness.run_steps(2);
            harness.get_by_label("Panel menu").click();
            harness.run_steps(2);
            assert!(!harness.get_by_label("Wrap List").accesskit_node().is_disabled(), "Wrap List must be enabled for {mode:?}");
            assert!(harness.state().wrap_list_enabled("test-picker"), "Wrap List is enabled by default for {mode:?}");
            harness.get_by_label("Wrap List").click();
            harness.run_steps(2);
            assert!(!harness.state().wrap_list_enabled("test-picker"), "Wrap List can be turned off for {mode:?}");
        }
    }
}
