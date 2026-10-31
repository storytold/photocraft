use super::bar::released_on;
use super::items::{MenuItem, menu_tint};
use super::state::translated_menu_label;

pub(super) fn render_level(ui: &mut egui::Ui, items: &[&MenuItem], depth: usize, clicked: &mut Option<String>, nav: &mut crate::menu_nav::Nav) {
    render_level_with(ui, depth, nav, |ui, nav| render_level_rows(ui, items, depth, clicked, nav));
}

/// Menu popup bodies go through the shared [`crate::direction::popup_body`] direction wrapper;
/// re-exported here so `menus::popup_body` call sites keep working.
pub(crate) use crate::direction::popup_body;

/// One menu level, with `pre` drawing the level's content (its rows, or the Help search above
/// them). Menu popups can be taller than the window: each level stays on screen and scrolls
/// (wheel, scroll arrows, keyboard), like a native menu on a small display. The level's body
/// goes through [`popup_body`] like every menu popup body.
pub(super) fn render_level_with(ui: &mut egui::Ui, depth: usize, nav: &mut crate::menu_nav::Nav, pre: impl FnOnce(&mut egui::Ui, &mut crate::menu_nav::Nav)) {
    popup_body(ui, |ui| crate::menu_nav::level(ui, depth, nav, pre));
}

/// Height of a menu separator: a line with room above and below, as in native menus.
const MENU_SEPARATOR: f32 = 9.0;

pub(super) fn render_level_rows(ui: &mut egui::Ui, items: &[&MenuItem], depth: usize, clicked: &mut Option<String>, nav: &mut crate::menu_nav::Nav) {
    let lang = crate::i18n::current();
    // One source of truth with the fork's arrow-glyph rotation and popup sides (set from the
    // language by the theme): in a right-to-left menu the shortcut and the submenu arrow sit at
    // the row's far end, which is now its left.
    let rtl = ui.style().right_to_left;
    // Items never wrap: the menu widens to its longest label plus shortcut (translations can be
    // longer than the English).
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    // Rows touch, as in native menus: the dialog spacing between them made long menus a fifth
    // taller than they need to be (#402).
    ui.spacing_mut().item_spacing.y = 0.0;
    // Secondary menus use the exact same Spectrum hover contrast and spacing.
    crate::widgets::style_spectrum_popup_menu(ui);
    // Walk in Photoshop order: leaves and separators at this depth; a submenu appears at the position
    // of its first child.
    // Each row's mnemonic (#4988), unique within this level, in display order: the leaves' labels
    // and the submenus' names as shown.
    // A label with no Latin letter shows its English label's key after it: "新建(N)…" (#5863).
    let (translated, english): (Vec<String>, Vec<&str>) = {
        let mut subs: Vec<&str> = Vec::new();
        items
            .iter()
            .filter_map(|it| {
                if it.path.len() == depth {
                    (it.label != "---").then(|| (translated_menu_label(lang, it), it.label.as_str()))
                } else {
                    let name = it.path.get(depth).map(String::as_str)?;
                    (!subs.contains(&name)).then(|| {
                        subs.push(name);
                        (crate::i18n::tr(lang, name).to_string(), name)
                    })
                }
            })
            .unzip()
    };
    let (shown_labels, mnemonics) = crate::menu_keys::localized(&translated, &english, &[]);
    let mut next_row = 0usize;
    let mut shown_subs: Vec<&str> = Vec::new();
    let mut last_was_sep = true;
    for (i, it) in items.iter().enumerate() {
        if it.path.len() == depth {
            if it.label == "---" {
                if !last_was_sep && i + 1 < items.len() {
                    ui.add(egui::Separator::default().spacing(MENU_SEPARATOR));
                    last_was_sep = true;
                }
                continue;
            }
            let label = shown_labels.get(next_row).cloned().unwrap_or_else(|| translated_menu_label(lang, it));
            let mnemonic = mnemonics.get(next_row).copied().flatten();
            next_row += 1;
            // Checked, unchecked and plain rows share one checkmark column (#5705), at the row's
            // reading start; the key shows only while the mnemonics do.
            let text =
                crate::menu_keys::menu_row_text(ui, it.checked == Some(true), &label, mnemonic, nav.underline, crate::menu_keys::GutterSide::ReadingStart);
            let mut b = crate::menu_keys::menu_row_button(text, it.shortcut.clone(), rtl);
            if let Some(tint) = it.color.as_deref().and_then(menu_tint) {
                b = b.fill(tint.gamma_multiply(0.55));
            }
            let hit = nav.row(ui, depth - 1, it.enabled, Some(&it.id), mnemonic, |ui, _| {
                let r = ui.add_enabled(it.enabled, b);
                let r = match it.id.as_str() {
                    "image.mode.bits8" | "image.mode.bits16" => r.on_hover_text(crate::i18n::tr(lang, "Integer")),
                    "image.mode.bits32" => r.on_hover_text(crate::i18n::tr(lang, "Floating point")),
                    _ => r,
                };
                let hit = r.clicked() || released_on(ui, &r);
                (r, hit)
            });
            if hit {
                *clicked = Some(it.id.clone());
                ui.close();
            }
            last_was_sep = false;
        } else if it.path.len() > depth {
            let name = it.path[depth].as_str();
            if shown_subs.contains(&name) {
                continue;
            }
            shown_subs.push(name);
            let child: Vec<&MenuItem> = items.iter().copied().filter(|c| c.path.len() > depth && c.path[depth] == name).collect();
            let enabled = submenu_enabled(&child);
            let mnemonic = mnemonics.get(next_row).copied().flatten();
            let shown = shown_labels.get(next_row).cloned().unwrap_or_else(|| crate::i18n::tr(lang, name).to_string());
            next_row += 1;
            let title = crate::menu_keys::menu_row_text(ui, false, &shown, mnemonic, nav.underline, crate::menu_keys::GutterSide::ReadingStart);
            ui.add_enabled_ui(enabled, |ui| {
                nav.row(ui, depth - 1, enabled, None, mnemonic, |ui, nav| {
                    let (response, inner) = crate::menu_keys::submenu_row(ui, title, |ui| render_level(ui, &child, depth + 1, clicked, nav));
                    if inner.is_some() {
                        // Where the submenu hangs from, to keep it below the menu bar (#319).
                        nav.set_anchor(depth, response.rect);
                    }
                    (response, ())
                });
            });
            last_was_sep = false;
        }
    }
}

/// A submenu header is enabled only while some item under it (at any depth) can run, so a
/// submenu of greyed items is greyed too instead of opening onto them (#1976).
pub(super) fn submenu_enabled(children: &[&MenuItem]) -> bool {
    children.iter().any(|c| c.enabled && c.label != "---")
}

/// The command a menu click runs: ⌥ + Merge Down / Merge Layers / Merge Visible keep the
/// originals, running Stamp Down / Stamp Visible instead (#217).
pub fn alt_click(id: String, alt: bool) -> String {
    match photocraft_engine::stamp_cmds::alt_variant(&id) {
        Some(stamp) if alt => stamp.to_string(),
        _ => id,
    }
}

#[cfg(test)]
mod disabled_rtl_tests {
    use super::*;
    use crate::direction::{Dir, direction_scope};

    fn level_items() -> Vec<MenuItem> {
        vec![
            MenuItem {
                id: "edit.undo".into(),
                label: "Undo".into(),
                path: vec!["Edit".into()],
                shortcut: Some("Ctrl+Z".into()),
                enabled: true,
                checked: None,
                color: None,
            },
            // Greyed with no shortcut, like the welcome screen's Edit items: a single text atom.
            MenuItem {
                id: "edit.spellCheck".into(),
                label: "Spell Check".into(),
                path: vec!["Edit".into()],
                shortcut: None,
                enabled: false,
                checked: None,
                color: None,
            },
        ]
    }

    fn galley_rect(out: &egui::FullOutput, contains: &str) -> Option<egui::Rect> {
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text.contains(contains) => Some(t.galley.rect.translate(t.pos.to_vec2())),
                _ => None,
            })
            .next()
    }

    /// A greyed row without a shortcut still hugs the reading-start edge of its row: the right
    /// edge in a right-to-left menu, and the unchanged left edge in a left-to-right one.
    #[test]
    fn a_disabled_row_without_a_shortcut_hugs_the_reading_start_edge() {
        for (dir, rtl) in [(Dir::Ltr, false), (Dir::Rtl, true)] {
            let _scope = direction_scope(dir);
            let ctx = egui::Context::default();
            // The theme sets the flag from the language; the test sets it directly.
            ctx.all_styles_mut(|s| s.right_to_left = rtl);
            let items = level_items();
            let refs: Vec<&MenuItem> = items.iter().collect();
            let mut nav = crate::menu_nav::Nav::default();
            nav.rows.resize(1, Vec::new());
            let mut clicked = None;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                // Justified at a fixed size, like the popup body, so every row shares one
                // full-width rect and the gap/margin comparison is between rows of one width.
                ui.allocate_ui_with_layout(egui::vec2(320.0, 200.0), egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                    render_level_rows(ui, &refs, 1, &mut clicked, &mut nav);
                });
            });
            let row = nav.rows[0].iter().find(|r| r.command.as_deref() == Some("edit.spellCheck")).expect("the disabled row drew").rect;
            let label = galley_rect(&out, "Spell Check").expect("its label painted");
            // The button frame's horizontal margin, measured on the enabled row's shortcut
            // galley, which sits at its row's reading end in both directions.
            let shortcut = galley_rect(&out, "Z").expect("the shortcut painted");
            let margin = if rtl { shortcut.left() - row.left() } else { row.right() - shortcut.right() };
            let gap = if rtl { row.right() - label.right() } else { label.left() - row.left() };
            assert!((gap - margin).abs() < 2.0, "{dir:?}: the disabled label hugs the reading-start edge (gap {gap}, margin {margin}): {label:?} in {row:?}");
            let start_half = if rtl { label.right() > row.center().x } else { label.left() < row.center().x };
            assert!(start_half, "{dir:?}: the disabled label sits on the reading-start half of the row: {label:?} in {row:?}");
            out.textures_delta.clear();
        }
    }
}

#[cfg(test)]
mod rtl_tests {
    use super::*;
    use egui::AtomKind;

    fn atom_kinds(b: &egui::Button<'_>) -> Vec<String> {
        b.atoms()
            .iter()
            .map(|a| match &a.kind {
                AtomKind::Text(t) => format!("text:{}", t.text()),
                _ if a.grow => "grow".to_string(),
                _ => "other".to_string(),
            })
            .collect()
    }

    /// Right-to-left rows run in reading order — the label last, so it lands at the row's right,
    /// the shortcut or submenu arrow first, at the left. Left-to-right rows are unchanged.
    #[test]
    fn rtl_row_atoms_run_in_reading_order() {
        let b = crate::menu_keys::menu_row_button("Undo".into(), Some("Ctrl+Z".to_string()), true);
        assert_eq!(atom_kinds(&b), ["text:Ctrl+Z", "grow", "text:Undo"]);
        let b = crate::menu_keys::menu_row_button("Preferences".into(), None, true);
        assert_eq!(atom_kinds(&b), ["grow", "text:Preferences"]);
        let b = crate::menu_keys::submenu_row_button("Sub".into(), true);
        assert_eq!(atom_kinds(&b), ["other", "grow", "text:Sub"]);
        let b = crate::menu_keys::menu_row_button("Undo".into(), Some("Ctrl+Z".to_string()), false);
        assert_eq!(atom_kinds(&b), ["text:Undo", "grow", "text:Ctrl+Z"]);
        let b = crate::menu_keys::menu_row_button("Preferences".into(), None, false);
        assert_eq!(atom_kinds(&b), ["text:Preferences"]);
        let b = crate::menu_keys::submenu_row_button("Sub".into(), false);
        assert_eq!(atom_kinds(&b), ["text:Sub", "grow", "other"]);
    }

    fn menu_items() -> Vec<MenuItem> {
        let item = |id: &str, label: &str, path: &[&str], shortcut: Option<&str>| MenuItem {
            id: id.to_string(),
            label: label.to_string(),
            path: path.iter().map(|s| s.to_string()).collect(),
            shortcut: shortcut.map(str::to_string),
            enabled: true,
            checked: None,
            color: None,
        };
        vec![
            item("edit.undo", "Undo", &["Edit"], Some("Ctrl+Z")),
            item("edit.prefs", "Preferences", &["Edit"], None),
            item("edit.sub.item", "Item", &["Edit", "Sub"], None),
        ]
    }

    /// Render one menu level at a fixed width in `dir` and collect each painted text galley's
    /// on-screen rect with its text.
    fn level_text_rects(dir: crate::direction::Dir) -> Vec<(String, egui::Rect)> {
        let _scope = crate::direction::direction_scope(dir);
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| s.right_to_left = dir == crate::direction::Dir::Rtl);
        let mut clicked = None;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let items = menu_items();
            let refs: Vec<&MenuItem> = items.iter().collect();
            let mut nav = crate::menu_nav::Nav::default();
            ui.allocate_ui_with_layout(egui::vec2(300.0, 200.0), egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                render_level_rows(ui, &refs, 1, &mut clicked, &mut nav);
            });
        });
        let texts: Vec<(String, egui::Rect)> = out
            .shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(ts) => Some((ts.galley.text().to_string(), cs.shape.visual_bounding_rect())),
                _ => None,
            })
            .collect();
        out.textures_delta.clear();
        texts
    }

    /// In a right-to-left menu the label sits at the row's right (reading start) and the shortcut
    /// at its left, whatever the label's script; left-to-right renders the mirror image.
    #[test]
    fn rtl_menu_rows_mirror_label_and_shortcut() {
        let at = |v: &[(String, egui::Rect)], s: &str| v.iter().find(|(t, _)| t.contains(s)).map(|(_, r)| *r).unwrap_or(egui::Rect::NOTHING);
        let rtl = level_text_rects(crate::direction::Dir::Rtl);
        let ltr = level_text_rects(crate::direction::Dir::Ltr);
        let (undo_r, undo_l) = (at(&rtl, "Undo"), at(&ltr, "Undo"));
        let (sc_r, sc_l) = (at(&rtl, "Z"), at(&ltr, "Z"));
        let (pref_r, pref_l) = (at(&rtl, "Preferences"), at(&ltr, "Preferences"));
        assert!(undo_r.center().x > 150.0 && undo_l.center().x < 150.0, "the label swaps sides: rtl {undo_r:?} ltr {undo_l:?}");
        assert!(sc_r.center().x < 150.0 && sc_l.center().x > 150.0, "the shortcut swaps sides: rtl {sc_r:?} ltr {sc_l:?}");
        assert!(pref_r.center().x > 150.0 && pref_l.center().x < 150.0, "a row without a shortcut mirrors too: rtl {pref_r:?} ltr {pref_l:?}");
    }

    /// Render one level through `render_level` (or, when `via_render_level` is false, through a
    /// direct `menu_nav::level` call) inside a fixed-size popup rect; returns the painted text
    /// galley rects and the level ui's resulting `min_rect` width.
    fn level_via(dir: crate::direction::Dir, via_render_level: bool) -> (Vec<(String, egui::Rect)>, f32) {
        let _scope = crate::direction::direction_scope(dir);
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| s.right_to_left = dir == crate::direction::Dir::Rtl);
        let items = menu_items();
        let refs: Vec<&MenuItem> = items.iter().collect();
        let mut nav = crate::menu_nav::Nav::default();
        let mut clicked = None;
        let mut width = 0.0;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.allocate_ui_with_layout(egui::vec2(300.0, 200.0), egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                if via_render_level {
                    render_level(ui, &refs, 1, &mut clicked, &mut nav);
                } else {
                    crate::menu_nav::level(ui, 1, &mut nav, |ui, nav| render_level_rows(ui, &refs, 1, &mut clicked, nav));
                }
                width = ui.min_rect().width();
            });
        });
        let texts = out
            .shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(ts) => Some((ts.galley.text().to_string(), cs.shape.visual_bounding_rect())),
                _ => None,
            })
            .collect();
        out.textures_delta.clear();
        (texts, width)
    }

    /// Render one level through the same `Popup::menu` path the menu bar uses, run the sizing
    /// pass plus two steady frames, and return the popup area's stored width.
    fn popup_level_width(dir: crate::direction::Dir) -> f32 {
        let _scope = crate::direction::direction_scope(dir);
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| s.right_to_left = dir == crate::direction::Dir::Rtl);
        let items = menu_items();
        let refs: Vec<&MenuItem> = items.iter().collect();
        let mut popup_id = egui::Id::NULL;
        for _ in 0..3 {
            let mut clicked = None;
            let mut nav = crate::menu_nav::Nav::default();
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    let btn = ui.button("Layer");
                    popup_id = egui::Popup::default_response_id(&btn);
                    egui::Popup::menu(&btn)
                        .open_memory(Some(egui::SetOpenCommand::Bool(true)))
                        .show(|ui| {
                            ui.set_min_width(220.0);
                            render_level(ui, &refs, 1, &mut clicked, &mut nav);
                        });
                },
            );
            out.textures_delta.clear();
        }
        egui::AreaState::load(&ctx, popup_id).and_then(|s| s.size).map(|s| s.x).unwrap_or(0.0)
    }

    /// A menu popup measures the same width in both directions: the right-to-left level scope
    /// must not inflate the popup's measured width past the left-to-right one (#w7).
    #[test]
    fn menu_popup_width_matches_in_rtl_and_ltr() {
        let ltr = popup_level_width(crate::direction::Dir::Ltr);
        let rtl = popup_level_width(crate::direction::Dir::Rtl);
        assert!((ltr - rtl).abs() < 1.0, "popup width differs: ltr {ltr}, rtl {rtl}");
    }

    /// The RTL `render_level` scope (justified child pinned to the popup's
    /// `max_rect`) must not balloon its `min_rect` past the popup width, and the rows must still
    /// paint. Left-to-right, `render_level` stays the direct `menu_nav::level` call — the scope
    /// is not taken, so the painted geometry is identical to calling `level` directly.
    #[test]
    fn rtl_render_level_scope_stays_inside_the_popup_rect() {
        let (rtl_texts, rtl_w) = level_via(crate::direction::Dir::Rtl, true);
        assert!(rtl_w <= 300.0 + 0.5, "rtl: the level ballooned its min_rect to {rtl_w} inside a 300-wide popup");
        assert!(rtl_texts.len() >= 3, "rtl: the rows still paint: {rtl_texts:?}");
        let (ltr_texts, ltr_w) = level_via(crate::direction::Dir::Ltr, true);
        let (direct_texts, direct_w) = level_via(crate::direction::Dir::Ltr, false);
        assert!(ltr_w <= 300.0 + 0.5 && direct_w <= 300.0 + 0.5, "ltr: min_rect {ltr_w}/{direct_w} inside a 300-wide popup");
        assert_eq!(ltr_texts, direct_texts, "ltr: render_level took the RTL scope");
    }

    /// Render one level (an enabled row with a shortcut, a disabled row without one, a submenu
    /// row) through the same `Popup::menu` path the menu bar uses, run the sizing pass plus
    /// steady frames, and return the popup's rect and the painted text galley rects.
    fn popup_level_texts(dir: crate::direction::Dir) -> (egui::Rect, Vec<(String, egui::Rect)>) {
        let _scope = crate::direction::direction_scope(dir);
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| s.right_to_left = dir == crate::direction::Dir::Rtl);
        let item = |id: &str, label: &str, path: &[&str], shortcut: Option<&str>, enabled: bool| MenuItem {
            id: id.to_string(),
            label: label.to_string(),
            path: path.iter().map(|s| s.to_string()).collect(),
            shortcut: shortcut.map(str::to_string),
            enabled,
            checked: None,
            color: None,
        };
        let items = [
            item("edit.undo", "Undo", &["Edit"], Some("Ctrl+Z"), true),
            item("edit.spellCheck", "Spell Check", &["Edit"], None, false),
            item("edit.sub.item", "Item", &["Edit", "Sub"], None, true),
        ];
        let refs: Vec<&MenuItem> = items.iter().collect();
        let mut popup = egui::Rect::NOTHING;
        let mut texts = Vec::new();
        for _ in 0..3 {
            let mut clicked = None;
            let mut nav = crate::menu_nav::Nav::default();
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    let btn = ui.button("Edit");
                    if let Some(r) = egui::Popup::menu(&btn)
                        .open_memory(Some(egui::SetOpenCommand::Bool(true)))
                        .show(|ui| {
                            ui.set_min_width(220.0);
                            popup_body(ui, |ui| render_level(ui, &refs, 1, &mut clicked, &mut nav));
                        })
                    {
                        popup = r.response.rect;
                    }
                },
            );
            texts = out
                .shapes
                .iter()
                .filter_map(|cs| match &cs.shape {
                    egui::Shape::Text(ts) => Some((ts.galley.text().to_string(), cs.shape.visual_bounding_rect())),
                    _ => None,
                })
                .collect();
            out.textures_delta.clear();
        }
        (popup, texts)
    }

    /// In a right-to-left menu popup every row anchors at the popup's right edge (reading
    /// start) with the shortcut at its left — including a disabled row without one; the
    /// left-to-right popup is unchanged (labels at the left edge).
    #[test]
    fn rtl_popup_menu_body_mirrors_every_row() {
        let at = |v: &[(String, egui::Rect)], s: &str| v.iter().find(|(t, _)| t.contains(s)).map(|(_, r)| *r).unwrap_or(egui::Rect::NOTHING);
        let (rtl_popup, rtl) = popup_level_texts(crate::direction::Dir::Rtl);
        let (ltr_popup, ltr) = popup_level_texts(crate::direction::Dir::Ltr);
        assert!(rtl_popup.is_positive() && ltr_popup.is_positive(), "the popups opened: {rtl_popup:?} {ltr_popup:?}");
        // Frame margin plus button padding and checkmark gutter: the label may sit this far
        // from the popup edge.
        let slack = 32.0;
        let (undo, sc, dis) = (at(&rtl, "Undo"), at(&rtl, "Z"), at(&rtl, "Spell Check"));
        assert!(rtl_popup.right() - undo.right() < slack, "rtl: the label hugs the popup's right edge: {undo:?} in {rtl_popup:?}");
        assert!(rtl_popup.right() - dis.right() < slack, "rtl: the disabled row's label hugs the right edge too: {dis:?} in {rtl_popup:?}");
        assert!(sc.left() - rtl_popup.left() < slack, "rtl: the shortcut sits at the popup's left edge: {sc:?} in {rtl_popup:?}");
        let (undo, sc, dis) = (at(&ltr, "Undo"), at(&ltr, "Z"), at(&ltr, "Spell Check"));
        assert!(undo.left() - ltr_popup.left() < slack, "ltr: the label hugs the popup's left edge: {undo:?} in {ltr_popup:?}");
        assert!(dis.left() - ltr_popup.left() < slack, "ltr: the disabled row's label hugs the left edge too: {dis:?} in {ltr_popup:?}");
        assert!(ltr_popup.right() - sc.right() < slack, "ltr: the shortcut sits at the popup's right edge: {sc:?} in {ltr_popup:?}");
    }
}
