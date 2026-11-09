//! Shared layout for preset pickers, independent of the asset-specific browser and commands.

use crate::pickers::browser::asset_browser::{BrowserAction, BrowserAdapter, browser};
use crate::pickers::footer::FooterOptions;
use crate::pickers::state::PresetUi;

#[cfg(test)]
pub(crate) use crate::pickers::picker_panel_menu::picker_menu_popup;
pub(crate) use crate::pickers::picker_panel_menu::{MenuContentsCallback, PickerMenuOptions, picker_menu_contents, picker_menu_contents_ordered};

const FOOTER_GUESS: f32 = 32.0;
type UiCallback<'a> = &'a mut dyn FnMut(&mut egui::Ui);
/// Controls for the standard asset-browser shell. The standard footer belongs only to this
/// contract because this shell owns the browser viewport and translates footer actions.
#[derive(Default)]
pub(crate) struct ShellOptions<'a> {
    /// Optional consumer-owned header content above the shared picker controls.
    pub(crate) header: Option<UiCallback<'a>>,
    /// Optional search control; the consumer owns its query state and filtering.
    pub(crate) search: Option<UiCallback<'a>>,
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
    crate::pickers::search::show_search_bar(ui, st, &panel);

    let mut events = if st.recents_visible(&panel) {
        let recent_options = crate::pickers::recents_bar::Options::for_panel(&panel);
        crate::pickers::recents_bar::show(ui, st, adapter, recent_options)
    } else {
        Vec::new()
    };

    let footer_id = egui::Id::new(("preset-footer", panel.as_str()));
    let footer_h = if shell.footer.is_some() { ui.data(|d| d.get_temp::<f32>(footer_id)).unwrap_or(FOOTER_GUESS) } else { 0.0 };
    // Do not let a consumer's legacy thumbnail-grid cap leave unused space below a short control
    // row; the browser itself scrolls within the viewport reserved by the shell.
    let allocated_h = (ui.available_height() - footer_h).max(0.0);
    events.extend(browser(ui, st, adapter, allocated_h));
    for event in &events {
        if let BrowserAction::SelectItem { group, group_path, key } = event {
            st.record_recent_item(&panel, group, group_path, key);
        }
    }

    if let Some(footer) = shell.footer {
        let footer_top = ui.min_rect().bottom();
        ui.add_space(4.0);
        let footer_actions = crate::pickers::footer::draw(ui, st, &panel, footer);
        for action in footer_actions {
            match action {
                crate::pickers::footer::FooterAction::DeleteItem { group, group_path, key } => {
                    events.push(BrowserAction::DeleteItem { group, group_path, key })
                }
                crate::pickers::footer::FooterAction::DeleteSelection => {
                    let selected = st
                        .browser_selection_snapshot
                        .get(&panel)
                        .map(crate::pickers::browser::selection_operations::delete_selection)
                        .unwrap_or_default();
                    if selected.is_empty() {
                        if let (Some(key), Some(group), Some(group_path)) =
                            (st.selected.get(&panel).cloned(), st.selected_group.get(&panel).cloned(), st.selected_group_path.get(&panel).cloned())
                        {
                            events.push(BrowserAction::DeleteItem { group, group_path, key });
                        }
                    } else {
                        events.extend(selected);
                    }
                }
                crate::pickers::footer::FooterAction::New => events.push(BrowserAction::New),
                crate::pickers::footer::FooterAction::NewGroup => events.push(BrowserAction::NewGroup),
            }
        }
        let measured = ui.min_rect().bottom() - footer_top;
        if measured.is_finite() && (measured - footer_h).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(footer_id, measured));
            ui.ctx().request_repaint();
        }
    }
    let mut events = events;
    crate::pickers::browser::popup_dialogs::show_new_group_dialog(ui, &panel, adapter.groups, &mut events);
    crate::pickers::browser::popup_dialogs::show_rename_selection_dialog(ui, &panel, adapter.groups, &mut events, adapter.paint_item);
    crate::pickers::browser::popup_dialogs::confirm_deletions(ui, &panel, adapter.groups, events)
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
}

fn shell_controls_custom(ui: &mut egui::Ui, shell: &mut CustomShellOptions<'_>) {
    if let Some(header) = shell.header.as_deref_mut() {
        ui.horizontal(|ui| header(ui));
    }
    if let Some(search) = shell.search.as_deref_mut() {
        search(ui);
    }
}

#[cfg(test)]
mod display_menu_tests {
    use super::{PickerMenuOptions, picker_menu_contents, picker_menu_popup};
    use crate::pickers::state::PresetUi;
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
                    picker_menu_contents(
                        ui,
                        state,
                        "test-picker",
                        PickerMenuOptions { display: true, wrap_list: true, search_visibility: true, recents_visibility: false },
                        None,
                    );
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
        let search_rect = harness.get_by_label("Show Search Bar").rect();
        assert!(search_rect.top() > wrap_rect.bottom() + 4.0, "search visibility must follow Wrap List in its own section");
    }

    #[test]
    fn panel_menu_popup_aligns_its_left_edge_to_the_buttons_top_right_or_bottom_right() {
        for (height, spacer, aligns_top) in [(500.0, 0.0, true), (260.0, 220.0, false)] {
            let mut harness = Harness::builder().with_size(vec2(420.0, height)).build_ui_state(
                |ui, state: &mut PresetUi| {
                    ui.add_space(spacer);
                    let button = picker_menu_popup(ui, "Panel menu", egui::Vec2::ZERO, |ui| {
                        picker_menu_contents(ui, state, "gradients", PickerMenuOptions::for_panel("gradients"), None);
                    });
                    ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("panel-menu-popup-test"), egui::Popup::default_response_id(&button)));
                },
                PresetUi::default(),
            );
            crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
            harness.run_steps(2);
            let button = harness.get_by_label("Panel menu").rect();
            harness.get_by_label("Panel menu").click();
            harness.run_steps(2);
            let popup_id = harness.ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::new("panel-menu-popup-test"))).unwrap();
            let popup = harness.ctx.memory(|memory| memory.area_rect(popup_id)).expect("open panel menu has an area rect");
            assert!(
                (popup.left() - button.right()).abs() < 1.0,
                "popup left edge must align with the trigger's right edge: button={button:?}, popup={popup:?}"
            );
            if aligns_top {
                assert!((popup.top() - button.top()).abs() < 1.0, "popup top-left must align with the trigger's top-right: button={button:?}, popup={popup:?}");
            } else {
                assert!(
                    (popup.bottom() - button.bottom()).abs() < 1.0,
                    "popup bottom-left must align with the trigger's bottom-right when it cannot fit from the top: button={button:?}, popup={popup:?}"
                );
            }
        }
    }

    #[test]
    fn panel_menu_popup_flips_when_below_space_is_insufficient_even_if_neither_alignment_fits() {
        let mut harness = Harness::builder().with_size(vec2(420.0, 500.0)).build_ui_state(
            |ui, _state: &mut PresetUi| {
                let spacer = ui.ctx().data(|data| data.get_temp::<f32>(egui::Id::new("panel-menu-spacer"))).unwrap_or(0.0);
                ui.add_space(spacer);
                let button = ui.button("Panel menu");
                let popup = super::super::picker_panel_menu::panel_menu_popup(&button).open(true);
                let best_align = popup.get_best_align();
                ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("panel-menu-best-align"), best_align));
                popup.show(|ui| {
                    for index in 0..20 {
                        let _ = ui.button(format!("Menu action {index}"));
                    }
                });
            },
            PresetUi::default(),
        );
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);
        harness.ctx.data_mut(|data| data.insert_temp(egui::Id::new("panel-menu-spacer"), 240.0));
        harness.run_steps(2);
        let best_align = harness.ctx.data(|data| data.get_temp::<egui::RectAlign>(egui::Id::new("panel-menu-best-align")));
        assert_eq!(best_align, Some(egui::RectAlign::RIGHT_END), "the menu should flip rather than retain a top anchor that egui then shifts");
    }

    #[test]
    fn selecting_any_display_mode_closes_the_panel_menu() {
        use crate::pickers::state::PickerDisplayMode as Mode;

        for mode in Mode::ALL {
            let mut state = PresetUi::default();
            state.display_modes.insert("test-picker".into(), Mode::SmallThumbnail);
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
            let label = match mode {
                Mode::TextOnly => "Text Only",
                Mode::SmallThumbnail => "Small Thumbnail",
                Mode::LargeThumbnail => "Large Thumbnail",
                Mode::SmallList => "Small List",
                Mode::LargeList => "Large List",
            };
            harness.get_by_label(label).click();
            harness.run_steps(2);

            assert_eq!(harness.state().display_mode("test-picker"), mode, "the selected display mode is applied");
            assert!(harness.query_by_label("Wrap List").is_none(), "selecting {label} closes the panel menu");
        }
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
        assert!(harness.query_by_label("Text Only").is_none(), "toggling search visibility closes the panel menu");
    }

    #[test]
    fn shared_picker_menu_exposes_and_persists_recent_strip_visibility() {
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
        harness.get_by_label("Show Recents").click();
        harness.run_steps(2);
        assert!(!harness.state().recents_visible("gradients"));
        assert!(harness.query_by_label("Show Recents").is_none(), "toggling recents visibility closes the menu");
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
        use crate::pickers::state::PickerDisplayMode as Mode;

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
        use crate::pickers::state::PickerDisplayMode as Mode;

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
            assert!(harness.query_by_label("Wrap List").is_none(), "toggling Wrap List closes the panel menu for {mode:?}");
        }
    }
}
