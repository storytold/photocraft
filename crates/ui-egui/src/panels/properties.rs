use super::{CornerRadius, Layer, LayerContent, LayerId, PhotocraftApp, Rect, RichText, Sense, Stroke, Tokens, empty, icons, json, pos2, theme, vec2, widgets};

// ----------------------------------------------------------------------------- properties

/// Floating Properties card anchored to the canvas' top-right corner. It has the docked group's
/// tabs: Properties (`properties_body`: document, layer, adjustment and mask properties) and
/// Adjustments (`adjustments_grid`).
pub fn properties_window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.docking.is_some() {
        return;
    }
    // Pro (Photoshop) docks Properties; Studio floats it over the canvas.
    if !app.ui.panels.properties || Tokens::get(ctx).pro {
        return;
    }
    if app.session.active().is_none() {
        return;
    }
    let t = Tokens::get(ctx);
    let canvas = app.last_canvas_rect;
    let width = 320.0;
    // Anchored to the canvas' reading-end corner: top-right in LTR, top-left in RTL.
    let x = crate::direction::float_window_x(canvas, width, 12.0, 0.0);
    let pos = pos2(x, canvas.top() + 44.0);
    let frame = egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .shadow(egui::Shadow { offset: [0, 12], blur: 36, spread: 0, color: t.shadow })
        .inner_margin(egui::Margin::same(14));
    // A floating panel (Order::Middle like other panels, so menus, popups and dialogs stay above
    // it), anchored to the canvas corner and dragged by its title off the part being worked on.
    let card_id = egui::Id::unique("properties-card");
    let offset: egui::Vec2 = ctx.data(|m| m.get_temp(card_id)).unwrap_or_default();
    let mut drag = egui::Vec2::ZERO;
    let shown = egui::Area::new(card_id).order(egui::Order::Middle).fixed_pos(pos + offset).show(ctx, |ui| {
        frame.show(ui, |ui| {
            ui.set_width(width - 28.0);
            // Properties | Adjustments, as on the docked group (#7265).
            let group = crate::dock::Group::Properties;
            let visible = app.ui.dock.visible_tabs(group, false);
            let indices: Vec<usize> = visible.iter().map(|(i, _)| *i).collect();
            let tabs: Vec<&str> = visible.iter().map(|(_, name)| *name).collect();
            let mut sel = indices.iter().position(|i| *i == app.ui.dock_tabs.properties).unwrap_or(0);
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            let hide = if crate::direction::is_rtl() {
                Rect::from_min_max(pos2(row.left(), row.top() + 1.0), pos2(row.left() + 22.0, row.bottom() - 1.0))
            } else {
                Rect::from_min_max(pos2(row.right() - 22.0, row.top() + 1.0), pos2(row.right(), row.bottom() - 1.0))
            };
            if icons::button(&mut ui.new_child(egui::UiBuilder::new().max_rect(hide)), "minus", 22.0, false, tl!("Hide Properties")).clicked() {
                app.ui.panels.properties = false;
            }
            // The title bar drags the card; registered before the tabs so they keep their clicks.
            let bar = if crate::direction::is_rtl() {
                Rect::from_min_max(pos2((hide.right() + 6.0).min(row.right()), row.top()), pos2(row.right(), row.bottom()))
            } else {
                Rect::from_min_max(row.min, pos2((hide.left() - 6.0).max(row.left()), row.bottom()))
            };
            let grip = ui.interact(bar, card_id.with("title"), Sense::drag());
            if grip.hovered() || grip.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            drag = grip.drag_delta();
            let context = crate::tab_strip::pill_tabs(ui, card_id.with("tabs"), bar, &tabs, &mut sel);
            if let Some(&picked) = indices.get(sel) {
                app.ui.dock_tabs.properties = picked;
            }
            match context {
                Some(crate::tab_strip::TabContextAction::Close(i)) => {
                    if let Some(&tab) = indices.get(i) {
                        crate::dock::close_tab(app, group, tab, false);
                    }
                }
                Some(crate::tab_strip::TabContextAction::CloseGroup) => app.ui.panels.properties = false,
                None => {}
            }
            ui.add_space(6.0);
            widgets::hairline(ui);
            ui.add_space(8.0);
            // Each Properties body scopes its own layout (see `properties_body`).
            match indices.get(sel) {
                Some(0) | None => properties_body(app, ui),
                Some(_) => adjustments_grid(app, ui),
            }
        });
    });
    if drag != egui::Vec2::ZERO && canvas.is_positive() {
        // Keep the title bar inside the canvas.
        let r = shown.response.rect.translate(drag);
        let dx = (canvas.left() - r.left()).max(0.0) - (r.right() - canvas.right()).max(0.0);
        let dy = (canvas.top() - r.top()).max(0.0) - (r.top() + 40.0 - canvas.bottom()).max(0.0);
        ctx.data_mut(|m| m.insert_temp(card_id, offset + drag + egui::vec2(dx, dy)));
    }
}

fn adjustment_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: LayerId, adj: &photocraft_doc::Adjustment) {
    // Keep the scope reachable before tall editors and within a compact dock (#5231).
    // Adjustment layers affect the stack by default; clipping limits them to their base.
    if let Some(mut clip) = app.session.active().and_then(|s| s.doc.layer(id).map(|l| l.clipped))
        && widgets::toggle(ui, &mut clip, tl!("Clip to layer below")).changed()
    {
        let _ = app.run("layer.setProps", json!({"layer": id.0, "clipped": clip}));
    }
    ui.add_space(6.0);
    crate::adjust_editors::layer_editor(app, ui, id, adj);
    ui.add_space(6.0);
    if let Some(mut vis) = app.session.active().and_then(|s| s.doc.layer(id).map(|l| l.visible))
        && widgets::toggle(ui, &mut vis, tl!("Adjustment visible")).changed()
    {
        let _ = app.run("layer.setProps", json!({"layer": id.0, "visible": vis}));
    }
    ui.add_space(6.0);
    if widgets::secondary_button(ui, tl!("Reset to defaults"), ui.available_width()).clicked() {
        let doc = app.session.active().map_or(0, |s| s.doc.id.0);
        // Photoshop's two-step Reset where measured (Color Balance), else the defaults.
        let params = crate::adjust_editors::properties_reset(ui.ctx(), doc, id, adj).unwrap_or_else(|| json!({"layer": id.0}));
        let _ = app.run("layer.setAdjustment", params);
        app.live_adjust = None;
    }
}

fn layer_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let t = Tokens::get(ui.ctx());
    if let Some(s) = layer.surface() {
        let b = app.layer_frame_bounds(layer, s);
        let rows = [("X", b.x0.to_string()), ("Y", b.y0.to_string()), ("W", b.width().to_string()), ("H", b.height().to_string())];
        // The fork's `Grid` cannot run under a right-to-left cursor, so it always runs in an
        // LTR scope; `grid_cells` emits the label/value pair in reading order.
        crate::direction::ltr_grid(ui, "props-grid", 2, vec2(12.0, 6.0), |ui| {
            for (k, v) in rows {
                crate::direction::grid_cells(
                    ui,
                    |ui| {
                        ui.label(RichText::new(k).color(t.text_dim));
                    },
                    |ui| {
                        ui.label(RichText::new(format!("{v} px")).font(theme::mono(12.0)).color(t.text));
                    },
                );
                ui.end_row();
            }
        });
        ui.add_space(6.0);
    }
    let mut o = layer.opacity * 100.0;
    let r = widgets::slider_row(ui, tl!("Opacity"), &mut o, 0.0..=100.0, "%", None);
    if r.drag_stopped() || (r.changed() && !r.dragged()) {
        let _ = app.run("layer.setProps", json!({"layer": layer.id.0, "opacity": o / 100.0}));
    }
    let mut f = layer.fill_opacity * 100.0;
    let r = widgets::slider_row(ui, crate::i18n::tr_ctx(crate::i18n::current(), "layers", "Fill"), &mut f, 0.0..=100.0, "%", None);
    if r.drag_stopped() || (r.changed() && !r.dragged()) {
        let _ = app.run("layer.setProps", json!({"layer": layer.id.0, "fill": f / 100.0}));
    }
}

/// Docked Properties body (Pro theme).
pub(super) fn properties_body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let identity = app.session.active().map(|st| (st.doc.id, st.active_layer, app.ui.mask_target, app.ui.vector_mask_target, app.ui.properties_layer));
    ui.push_id(("properties-target", identity), |ui| {
        // The dock shell draws its panels with `top_down(Align::Min)`, where nested rows stay
        // left-to-right even under the direction flag; this scope mirrors them.
        ui.with_layout(crate::direction::body_layout(), |ui| properties_body_inner(app, ui));
    });
}

fn properties_body_inner(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No properties"));
        return;
    };
    let doc = st.doc.clone();
    if let Some(layer) = st.active_layer.and_then(|id| doc.layer(id))
        && crate::mask_props_ui::targeted(app, layer)
    {
        crate::props_layout::header(ui, layer);
        crate::mask_props_ui::properties(app, ui, layer);
        return;
    }
    // The vector mask's own controls, when its thumbnail is targeted.
    if let Some(layer) = st.active_layer.and_then(|id| doc.layer(id))
        && crate::mask_props_ui::vector_targeted(app, layer)
    {
        crate::props_layout::header(ui, layer);
        crate::mask_props_ui::vector_properties(app, ui, layer);
        return;
    }
    // Photoshop shows the Document properties when nothing or the Background layer is selected,
    // with Align and Distribute above them when the Background is one of several (#4608).
    if crate::doc_props_ui::shows_document(&st.doc, st.active_layer) {
        if t.pro && st.selected_layers().len() > 1 {
            crate::layer_props_ui::selection_align(app, ui);
        }
        crate::doc_props_ui::properties(app, ui);
        return;
    }
    let Some(id) = st.active_layer else { return };
    // Borrowed from the document snapshot: cloning the layer every frame copied whole groups.
    let Some(layer) = doc.layer(id) else { return };
    // Header: kind icon, layer name and kind (#155); sections below draw their own separators.
    crate::props_layout::header(ui, layer);
    let is_adjustment = matches!(layer.content, LayerContent::Adjustment(_));
    if is_adjustment || layer.artboard().is_some() || !t.pro {
        ui.add_space(4.0);
        widgets::hairline(ui);
        ui.add_space(6.0);
    }
    if let LayerContent::Adjustment(adj) = &layer.content {
        adjustment_controls(app, ui, id, adj);
    } else if layer.artboard().is_some() {
        crate::artboard_ui::properties(app, ui, layer);
    } else if t.pro {
        // Transform, Align, the kind's sections and Quick Actions.
        crate::layer_props_ui::properties(app, ui, layer);
    } else {
        if matches!(layer.content, LayerContent::Fill(photocraft_doc::Fill::Gradient { .. })) {
            crate::gradient_ui::properties(app, ui, layer);
            ui.add_space(6.0);
        }
        layer_controls(app, ui, layer);
        if matches!(layer.content, LayerContent::Text(_)) {
            crate::type_tool::type_properties(app, ui);
        }
        if matches!(layer.content, LayerContent::Shape(_)) {
            crate::vector_ui::shape_properties(app, ui, id);
            crate::mask_props_ui::shape_properties(app, ui, layer);
        }
    }
}

/// Photoshop's Adjustments panel: a grid of one-click adjustment layers.
pub(super) fn adjustments_grid(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // The same scope as the Properties body, so the docked tab mirrors too.
    ui.with_layout(crate::direction::body_layout(), |ui| adjustments_grid_inner(app, ui));
}

fn adjustments_grid_inner(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Add an adjustment")).color(t.text_dim));
    ui.add_space(4.0);
    let items: [(&str, &str, &str); 16] = [
        ("brightnessContrast", "sun", tl!("Brightness/Contrast")),
        ("levels", "gauge", tl!("Levels")),
        ("curves", "pen-tool", tl!("Curves")),
        ("exposure", "scan", tl!("Exposure")),
        ("vibrance", "sparkles", tl!("Vibrance")),
        ("hueSaturation", "droplet", tl!("Hue/Saturation")),
        ("colorBalance", "blend", tl!("Color Balance")),
        ("blackWhite", "contrast", tl!("Black & White")),
        ("photoFilter", "circle-dot", tl!("Photo Filter")),
        ("channelMixer", "sliders-horizontal", tl!("Channel Mixer")),
        ("invert", "squares-subtract", tl!("Invert")),
        ("posterize", "layers", tl!("Posterize")),
        ("threshold", "square", tl!("Threshold")),
        ("gradientMap", "palette", tl!("Gradient Map")),
        ("selectiveColor", "swatch-book", tl!("Selective Color")),
        ("colorLookup", "grid-3x3", tl!("Color Lookup")),
    ];
    let mut run = None;
    // The column count matches the old wrapped flow of 30 pt buttons with 2 pt spacing.
    // row_width reads max_rect in RTL, where the fork's available-width read balloons, so the
    // count is capped by the visible width: a scroll area's content max_rect grows past the
    // viewport once the grid itself is wide, which would otherwise unwrap every button.
    let width = crate::direction::row_width(ui).min(ui.clip_rect().width());
    let cols = ((width + 2.0) / 32.0).floor().max(1.0) as usize;
    if !crate::direction::is_rtl() {
        // The fork's Grid never auto-wraps at num_columns; each row ends explicitly.
        crate::direction::ltr_grid(ui, "adjustments-grid", cols, vec2(2.0, 2.0), |ui| {
            for chunk in items.chunks(cols) {
                for (kind, icon, tip) in chunk {
                    if icons::button(ui, icon, 30.0, false, tip).clicked() {
                        run = Some(format!("layer.newAdjustmentLayer.{kind}"));
                    }
                }
                ui.end_row();
            }
        });
    } else {
        // The fork's Grid mis-wraps under the RTL style flag (later rows overflow the
        // max_rect), so under RTL each row is a `direction::row()` scope: it starts from the
        // reading start, so the first adjustment of each row lands rightmost.
        for chunk in items.chunks(cols) {
            ui.with_layout(crate::direction::row(), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (kind, icon, tip) in chunk {
                    if icons::button(ui, icon, 30.0, false, tip).clicked() {
                        run = Some(format!("layer.newAdjustmentLayer.{kind}"));
                    }
                }
            });
            ui.add_space(2.0);
        }
    }
    if let Some(id) = run {
        let _ = app.run(&id, json!({}));
        app.ui.dock_tabs.properties = 0;
    }
}

/// The options bar's Select and Mask… button: greyed out, as the menu item is, until there is a
/// selection (or a layer mask) to refine (#7520).
pub(super) fn select_and_mask_button(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let enabled = crate::menus::is_enabled(app, "select.selectAndMask");
    let clicked = ui.add_enabled_ui(enabled, |ui| widgets::secondary_button(ui, tl!("Select and Mask…"), 0.0).clicked()).inner;
    if clicked {
        let _ = crate::menus::invoke(app, ui.ctx(), "select.selectAndMask", json!({}));
    }
}

/// New / add / subtract / intersect selection buttons (shared by selection tools).
pub(super) fn selection_mode_buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.x = 2.0;
    for (i, (icon, tip)) in [
        ("square", tl!("New selection").to_string()),
        ("plus", crate::i18n::fmt(tl!("Add to selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift"))])),
        ("minus", crate::i18n::fmt(tl!("Subtract from selection  ({key})"), &[("key", &crate::shortcuts::pretty("Alt"))])),
        ("squares-subtract", crate::i18n::fmt(tl!("Intersect with selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift+Alt"))])),
    ]
    .iter()
    .enumerate()
    {
        if icons::button(ui, icon, 24.0, app.ui.selection_mode == i as u8, tip).clicked() {
            app.ui.selection_mode = i as u8;
        }
    }
    ui.spacing_mut().item_spacing.x = 8.0;
}

#[cfg(test)]
mod properties_rtl_tests {
    use super::*;
    use crate::direction::{Dir, direction_scope};
    use egui_kittest::{Harness, kittest::Queryable};

    fn harness(adjustments_tab: bool) -> Harness<'static, PhotocraftApp> {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        session.execute("layer.new.layer", json!({})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.panels.properties = true;
        if adjustments_tab {
            app.ui.dock_tabs.properties = 1;
        }
        app.last_canvas_rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0));
        let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                    properties_window(app, ui.ctx());
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(4);
        h
    }

    fn card_rect(h: &Harness<'_, PhotocraftApp>) -> Rect {
        h.ctx.memory(|m| m.area_rect(egui::Id::unique("properties-card"))).expect("the card is shown")
    }

    fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
        h.hover_at(from);
        h.run_steps(1);
        h.drag_at(from);
        h.run_steps(1);
        for i in 1..=6 {
            h.hover_at(from + (to - from) * (i as f32 / 6.0));
            h.run_steps(1);
        }
        h.drop_at(to);
        h.run_steps(3);
    }

    /// The docked body drawn straight into a plain panel, as craft-ui's dock draws it.
    fn docked_harness() -> Harness<'static, PhotocraftApp> {
        const WIDTH: f32 = 400.0;
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(WIDTH, 600.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                    adjustments_grid(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(4);
        h
    }

    /// The X-row's value cell: the "… px" label sharing the X label's row.
    fn x_value_rect(h: &mut Harness<'_, PhotocraftApp>) -> (Rect, Rect) {
        let x = h.get_by_label("X").rect();
        let want = {
            let layer = {
                let st = h.state().session.active().unwrap();
                st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
            };
            let app = h.state_mut();
            let b = app.layer_frame_bounds(&layer, layer.surface().unwrap());
            format!("{} px", b.x0)
        };
        let val =
            h.query_all_by_label(&want).find(|n| (n.rect().center().y - x.center().y).abs() < 3.0).unwrap_or_else(|| panic!("no {want:?} on the X row")).rect();
        (x, val)
    }

    /// The floating card anchors to the canvas' reading-end corner.
    #[test]
    fn card_anchors_to_the_canvas_reading_end() {
        let canvas = Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0));
        {
            let _rtl = direction_scope(Dir::Rtl);
            let card = card_rect(&harness(false));
            assert!((card.left() - (canvas.left() + 12.0)).abs() < 3.0, "RTL: {card:?}");
        }
        {
            let _ltr = direction_scope(Dir::Ltr);
            let card = card_rect(&harness(false));
            assert!((card.right() - (canvas.right() - 12.0)).abs() < 3.0, "LTR: {card:?}");
        }
    }

    /// The hide button sits at the title row's reading end and still hides the card.
    #[test]
    fn hide_button_mirrors_and_hides_the_card() {
        {
            let _rtl = direction_scope(Dir::Rtl);
            let mut h = harness(false);
            let card = card_rect(&h);
            let hide = h.get_by_label("Hide Properties").rect();
            assert!((hide.left() - (card.left() + 14.0)).abs() < 3.0, "RTL: {hide:?} in {card:?}");
            h.get_by_label("Hide Properties").click();
            h.run_steps(2);
            assert!(!h.state().ui.panels.properties, "the mirrored button hides the card");
        }
        {
            let _ltr = direction_scope(Dir::Ltr);
            let h = harness(false);
            let card = card_rect(&h);
            let hide = h.get_by_label("Hide Properties").rect();
            assert!((hide.right() - (card.right() - 14.0)).abs() < 3.0, "LTR: {hide:?} in {card:?}");
        }
    }

    /// egui::Grid ignores the direction flag, so the label/value cells are emitted in reading
    /// order: the X label sits right of its value in RTL, left of it in LTR.
    #[test]
    fn layer_grid_cells_follow_reading_order() {
        {
            let _rtl = direction_scope(Dir::Rtl);
            let (x, val) = x_value_rect(&mut harness(false));
            assert!(x.center().x > val.center().x, "RTL: {x:?} vs {val:?}");
        }
        {
            let _ltr = direction_scope(Dir::Ltr);
            let (x, val) = x_value_rect(&mut harness(false));
            assert!(x.center().x < val.center().x, "LTR: {x:?} vs {val:?}");
        }
    }

    /// The card body is a mirrored `body_layout()`: its blocks hug the reading end and the
    /// adjustments palette flows from it (first icon rightmost in RTL).
    #[test]
    fn card_body_and_palette_mirror() {
        {
            let _rtl = direction_scope(Dir::Rtl);
            let h = harness(true);
            let card = card_rect(&h);
            let header = h.get_by_label("Add an adjustment").rect();
            assert!((header.right() - (card.right() - 14.0)).abs() < 3.0, "RTL header: {header:?} in {card:?}");
            let first = h.get_by_label("Brightness/Contrast").rect();
            let last = h.get_by_label("Color Lookup").rect();
            assert!(first.right() > last.right(), "RTL palette: {first:?} vs {last:?}");
        }
        {
            let _ltr = direction_scope(Dir::Ltr);
            let h = harness(true);
            let card = card_rect(&h);
            let header = h.get_by_label("Add an adjustment").rect();
            assert!((header.left() - (card.left() + 14.0)).abs() < 3.0, "LTR header: {header:?} in {card:?}");
            let first = h.get_by_label("Brightness/Contrast").rect();
            let last = h.get_by_label("Color Lookup").rect();
            assert!(first.left() < last.left(), "LTR palette: {first:?} vs {last:?}");
        }
    }

    /// The docked Properties body draws inside craft-ui's `top_down(Align::Min)` panel, so the
    /// body scopes its own layout: its blocks hug the reading end and its rows mirror.
    #[test]
    fn docked_body_mirrors_inside_the_dock_panel() {
        {
            let _rtl = direction_scope(Dir::Rtl);
            let h = docked_harness();
            let mid = 200.0; // the docked harness' 400-point panel
            let header = h.get_by_label("Add an adjustment").rect();
            assert!(header.right() > mid, "RTL header hugs the reading end: {header:?}");
            let first = h.get_by_label("Brightness/Contrast").rect();
            let last = h.get_by_label("Color Lookup").rect();
            assert!(first.right() > last.right(), "RTL palette: {first:?} vs {last:?}");
        }
        {
            let _ltr = direction_scope(Dir::Ltr);
            let h = docked_harness();
            let mid = 200.0; // the docked harness' 400-point panel
            let header = h.get_by_label("Add an adjustment").rect();
            assert!(header.left() < mid, "LTR header hugs the reading start: {header:?}");
            let first = h.get_by_label("Brightness/Contrast").rect();
            let last = h.get_by_label("Color Lookup").rect();
            assert!(first.left() < last.left(), "LTR palette: {first:?} vs {last:?}");
        }
    }

    /// The mirrored title strip drags the floating card; a drag that starts on the hide
    /// button's rect must not move it.
    #[test]
    fn drag_bar_mirrors_and_moves_the_card() {
        {
            let _rtl = direction_scope(Dir::Rtl);
            let mut h = harness(false);
            let card = card_rect(&h);
            // The strip runs from the hide button's right edge to the title row's reading end.
            let grab = pos2(card.right() - 30.0, card.top() + 26.0);
            drag(&mut h, grab, grab + vec2(40.0, 0.0));
            let after = card_rect(&h);
            assert!(after.left() > card.left(), "the mirrored strip drags the card: {card:?} -> {after:?}");

            let mut h = harness(false);
            let before = card_rect(&h);
            let hide = h.get_by_label("Hide Properties").rect();
            drag(&mut h, hide.center(), hide.center() + vec2(40.0, 0.0));
            assert!(h.state().ui.panels.properties, "the hide button keeps the card");
            let after = card_rect(&h);
            assert!((after.left() - before.left()).abs() < 0.5, "the hide button does not drag the card: {before:?} -> {after:?}");
        }
        {
            // The same strip drags the card in left-to-right too.
            let _ltr = direction_scope(Dir::Ltr);
            let mut h = harness(false);
            let card = card_rect(&h);
            let grab = pos2(card.left() + 30.0, card.top() + 26.0);
            drag(&mut h, grab, grab + vec2(40.0, 0.0));
            let after = card_rect(&h);
            assert!(after.left() > card.left(), "the strip drags the card: {card:?} -> {after:?}");
        }
    }

    /// Twin of the migrated `ltr_grid` adjustments strip: cell order stays left-to-right in
    /// both directions (the strip is physical), with equal gaps.
    #[test]
    fn migrated_ltr_grid_keeps_cell_order_in_both_directions() {
        let rects = |dir: Dir| {
            let _scope = direction_scope(dir);
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|s| s.right_to_left = dir == Dir::Rtl);
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
                ..Default::default()
            };
            let mut out = Vec::new();
            let mut full = ctx.run_ui(input, |ui| {
                crate::direction::ltr_grid(ui, "twin-adjustments-grid", 3, vec2(2.0, 2.0), |ui| {
                    for _ in 0..3 {
                        out.push(ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::hover()).0);
                    }
                });
            });
            full.textures_delta.clear();
            out
        };
        for dir in [Dir::Ltr, Dir::Rtl] {
            let r = rects(dir);
            assert!(r[0].left() < r[1].left() && r[1].left() < r[2].left(), "{dir:?}: cells stay LTR: {r:?}");
            let gap01 = r[1].left() - r[0].right();
            let gap12 = r[2].left() - r[1].right();
            assert!((gap01 - gap12).abs() < 0.5, "{dir:?}: equal gaps: {gap01} {gap12}");
        }
    }
}
