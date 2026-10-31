//! ⌘F command palette: fuzzy search over every menu command and tool.

use egui::{Align2, Color32, CornerRadius, RichText, Sense, Stroke, vec2};

use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons};

/// Case-insensitive subsequence match score (higher is better); None = no match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut ti = 0;
    let mut last = None;
    for qc in query.to_lowercase().chars() {
        let mut found = false;
        while ti < t.len() {
            if t[ti] == qc {
                score += if last == Some(ti.wrapping_sub(1)) { 8 } else { 1 };
                if ti == 0 || t[ti - 1] == ' ' {
                    score += 5;
                }
                last = Some(ti);
                ti += 1;
                found = true;
                break;
            }
            ti += 1;
        }
        if !found {
            return None;
        }
    }
    Some(score - (t.len() as i32 / 8))
}

/// Bonus that puts a command or tool whose name is exactly the query ahead of every fuzzy match
/// ("settings" finds Settings before Color Settings).
fn exact_bonus(query: &str, names: &[&str]) -> i32 {
    let q = query.trim();
    if !q.is_empty() && names.iter().any(|n| n.trim_end_matches('…').trim().to_lowercase() == q.to_lowercase()) { 1000 } else { 0 }
}

/// How many recently run commands the palette remembers.
const RECENT_CAP: usize = 8;

/// The id of the palette's recently-run list (egui temp data: session-scoped, like the query).
fn recent_id() -> egui::Id {
    egui::Id::unique("palette-recent")
}

/// Remember `id` as the most recently run command: deduplicated, newest first, capped.
pub(crate) fn note_recent(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| {
        let mut list: Vec<String> = d.get_temp(recent_id()).unwrap_or_default();
        push_recent(&mut list, id, RECENT_CAP);
        d.insert_temp(recent_id(), list);
    });
}

/// The palette's recently run commands, newest first (for tests).
#[cfg(test)]
pub(crate) fn recent_ids(ctx: &egui::Context) -> Vec<String> {
    ctx.data(|d| d.get_temp(recent_id()).unwrap_or_default())
}

/// [`note_recent`]'s core: move `id` to the front, dropping the oldest past `cap`.
fn push_recent(list: &mut Vec<String>, id: &str, cap: usize) {
    list.retain(|x| x != id);
    list.insert(0, id.to_string());
    list.truncate(cap);
}

/// The empty-query hits: the recent commands as rows, newest first (UI-217-7). Ids that are
/// no longer menu commands are skipped.
fn recent_hits(recents: &[String], menus: &[crate::menus::MenuItem], lang: crate::i18n::Lang) -> Vec<(i32, String, String, Option<String>, bool)> {
    recents
        .iter()
        .enumerate()
        .filter_map(|(i, id)| {
            let m = menus.iter().find(|m| &m.id == id)?;
            let path = m.path.iter().map(|p| crate::i18n::tr(lang, p)).collect::<Vec<_>>().join(" › ");
            let label = crate::i18n::tr_id(lang, &m.id, &m.label);
            Some((
                1000 - i as i32,
                m.id.clone(),
                label.trim_end_matches('…').to_string(),
                Some(path + &m.shortcut.as_ref().map(|s| format!("   {}", crate::shortcuts::pretty(s))).unwrap_or_default()),
                m.enabled,
            ))
        })
        .collect()
}

/// The menu commands matching `q`, as palette rows. Separators are not commands: they would
/// match on their menu path and list as blank "---" rows (#2479).
fn menu_hits(menus: Vec<crate::menus::MenuItem>, q: &str, lang: crate::i18n::Lang) -> Vec<(i32, String, String, Option<String>, bool)> {
    menus
        .into_iter()
        .filter(|m| m.label != "---")
        .filter_map(|m| {
            let path_en = m.path.join(" › ");
            let path = m.path.iter().map(|p| crate::i18n::tr(lang, p)).collect::<Vec<_>>().join(" › ");
            let label = crate::i18n::tr_id(lang, &m.id, &m.label);
            // Match what is shown and the English name (commands are documented in English).
            let s = fuzzy_score(q, &format!("{label} {path} {} {path_en}", m.label))? + exact_bonus(q, &[label, &m.label]);
            Some((
                s,
                m.id,
                label.trim_end_matches('…').to_string(),
                Some(path + &m.shortcut.map(|s| format!("   {}", crate::shortcuts::pretty(&s))).unwrap_or_default()),
                m.enabled,
            ))
        })
        .collect()
}

/// The palette's search field: in a right-to-left UI the hint and the typed text read from the
/// right (right-aligned galley on a right-to-left paragraph base); LTR keeps egui's default
/// left-aligned, auto-direction edit, unchanged.
fn search_edit(ui: &mut egui::Ui, q: &mut String, width: f32) -> egui::Response {
    let font = egui::FontId::proportional(15.0);
    let mut layouter = crate::widgets::rtl_singleline_layouter(font.clone());
    let mut edit = crate::widgets::rtl_singleline(q, &mut layouter)
        .hint_text(tl!("Search commands, tools and panels…"))
        .frame(egui::Frame::NONE)
        .font(font.clone())
        .desired_width(width - 40.0);
    if crate::direction::is_rtl() {
        // The hint is laid out from its own job, not the shared layouter, so pin its paragraph
        // base direction too; the hint color is the style's weak text color, as for a plain hint.
        let mut hint_job = egui::text::LayoutJob::simple_singleline(tl!("Search commands, tools and panels…").to_owned(), font, ui.visuals().weak_text_color());
        hint_job.direction = egui::epaint::text::TextDirection::RightToLeft;
        edit = edit.hint_text(egui::WidgetText::from(hint_job));
    }
    ui.add(edit)
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.palette_open {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // Scrim.
    egui::Area::new(egui::Id::unique("palette-scrim")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let r = ui.allocate_exact_size(screen.size(), Sense::click()).1;
        ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(110));
        if r.clicked() {
            app.ui.palette_open = false;
        }
    });
    let width = 560.0;
    let mut run: Option<String> = None;
    egui::Area::new(egui::Id::unique("palette"))
        .order(egui::Order::Tooltip)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(screen.center().x, screen.top() + 110.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.card)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(CornerRadius::same(t.radius_lg as u8))
                .shadow(egui::Shadow { offset: [0, 18], blur: 48, spread: 0, color: t.shadow })
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(width);
                    let qid = egui::Id::unique("palette-query");
                    let mut q: String = ui.data_mut(|d| d.get_temp(qid).unwrap_or_default());
                    // The search row mirrors in a right-to-left UI. The size is the one
                    // `ui.horizontal` would allocate, so the left-to-right geometry is unchanged.
                    let row_size = vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
                    ui.allocate_ui_with_layout(row_size, crate::direction::row(), |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                        icons::paint(ui, r, "search", 16.0, t.text_dim);
                        let resp = search_edit(ui, &mut q, width);
                        // Only when focus is missing: every request_focus interrupts the IME, and
                        // doing that each frame makes Wayland input methods drop key releases (#585).
                        if !resp.has_focus() {
                            resp.request_focus();
                        }
                    });
                    ui.add_space(6.0);
                    crate::widgets::hairline(ui);
                    ui.add_space(6.0);
                    let lang = crate::i18n::Lang::from_pref(&app.session.prefs().interface.language);
                    let menus = crate::menus::menu_items(app);
                    let recents: Vec<String> = ui.data(|d| d.get_temp(recent_id()).unwrap_or_default());
                    let empty = q.trim().is_empty();
                    // An empty query lists the recently run commands (UI-217-7); with none (or a
                    // fresh session) it falls back to the full fuzzy list, as before.
                    let mut hits: Vec<(i32, String, String, Option<String>, bool)> = if empty { recent_hits(&recents, &menus, lang) } else { Vec::new() };
                    if !empty || hits.is_empty() {
                        hits.extend(menu_hits(menus, &q, lang));
                        for tool in crate::state::Tool::ALL {
                            if let Some(s) = fuzzy_score(&q, &format!("{} {}", tl!(tool.label()), tool.label())) {
                                let s = s + exact_bonus(&q, &[tl!(tool.label()).as_ref(), tool.label()]);
                                hits.push((
                                    s + 2,
                                    format!("tool:{tool:?}"),
                                    tl!(tool.label()).into(),
                                    Some(format!("{}   {}", crate::i18n::tr(lang, "Tool"), tool.key())),
                                    true,
                                ));
                            }
                        }
                    }
                    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.cmp(&b.2)));
                    hits.truncate(12);
                    let sel_id = egui::Id::unique("palette-sel");
                    let mut sel: usize = ui.data_mut(|d| d.get_temp(sel_id).unwrap_or(0));
                    let (down, up, enter, esc) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::ArrowDown),
                            i.key_pressed(egui::Key::ArrowUp),
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if down {
                        sel = (sel + 1).min(hits.len().saturating_sub(1));
                    }
                    if up {
                        sel = sel.saturating_sub(1);
                    }
                    sel = sel.min(hits.len().saturating_sub(1));
                    if hits.is_empty() {
                        crate::direction::dialog_row(ui, |ui| {
                            ui.label(RichText::new(tl!("No matching commands")).color(t.text_faint));
                        });
                    }
                    for (i, (_, id, label, detail, enabled)) in hits.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
                        if i == sel || resp.hovered() {
                            ui.painter().rect_filled(rect, t.radius_sm, if i == sel { t.accent_soft } else { t.hover });
                        }
                        let color = if *enabled { t.text } else { t.text_faint };
                        let (l_pos, l_align, d_pos, d_align) = hit_row_anchors(rect);
                        ui.painter().text(l_pos, l_align, label, theme::medium(13.0), color);
                        if let Some(d) = detail {
                            ui.painter().text(d_pos, d_align, d, egui::FontId::proportional(11.5), t.text_faint);
                        }
                        if resp.clicked() && *enabled {
                            run = Some(id.clone());
                        }
                    }
                    if enter && let Some(h) = hits.get(sel).filter(|h| h.4) {
                        run = Some(h.1.clone());
                        // dialogs::show runs later in the same frame and would read this Enter as
                        // its OK button, instantly closing the dialog we are about to open (#536).
                        ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                    }
                    if esc {
                        app.ui.palette_open = false;
                    }
                    ui.add_space(4.0);
                    crate::direction::dialog_row(ui, |ui| {
                        ui.label(
                            RichText::new(crate::i18n::fmt(tl!("↑↓ navigate   {key} run   esc close"), &[("key", &crate::shortcuts::pretty("Enter"))]))
                                .small()
                                .color(t.text_faint),
                        );
                    });
                    ui.data_mut(|d| {
                        d.insert_temp(qid, q);
                        d.insert_temp(sel_id, sel);
                    });
                });
        });
    if let Some(id) = run {
        app.ui.palette_open = false;
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("palette-query"), String::new()));
        if let Some(tool) = id.strip_prefix("tool:") {
            if let Some(t) = crate::state::Tool::from_name(tool) {
                app.ui.tool = t;
            }
        } else if let Err(e) = crate::menus::invoke(app, ctx, &id, serde_json::json!({})) {
            app.ui.status = e;
        }
    }
}

/// The anchors of a hit row's label and detail: the label sits at the reading-start edge and the
/// detail at the reading-end edge, each inset 12 px from the row's edge.
fn hit_row_anchors(rect: egui::Rect) -> (egui::Pos2, Align2, egui::Pos2, Align2) {
    let (l_pos, l_align) = crate::direction::start_text_anchor(rect, 12.0);
    let (d_pos, d_align) = crate::direction::end_text_anchor(rect, 12.0);
    (l_pos, l_align, d_pos, d_align)
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    use super::{exact_bonus, fuzzy_score, push_recent};
    use crate::PhotocraftApp;

    #[test]
    fn open_palette_interrupts_the_ime_only_when_it_takes_focus() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // Fonts bind on the next pass; this one opens nothing.
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        app.ui.palette_open = true;
        // The field takes focus on the first pass and owns the IME from the second on.
        let interrupts: Vec<Option<bool>> = (0..4)
            .map(|_| {
                let mut out = ctx.run_ui(Default::default(), |ui| super::show(&mut app, ui.ctx()));
                out.textures_delta.clear();
                out.platform_output.ime.map(|ime| ime.should_interrupt_composition)
            })
            .collect();
        assert_eq!(interrupts, [None, Some(false), Some(false), Some(false)]);
    }

    #[test]
    fn push_recent_moves_to_the_front_and_caps() {
        let mut l = Vec::new();
        push_recent(&mut l, "b", 3);
        push_recent(&mut l, "a", 3);
        push_recent(&mut l, "c", 3);
        assert_eq!(l, ["c", "a", "b"]);
        push_recent(&mut l, "a", 3);
        assert_eq!(l, ["a", "c", "b"], "re-running a command moves it to the front");
        push_recent(&mut l, "d", 3);
        assert_eq!(l, ["d", "a", "c"], "the oldest falls off the cap");
    }

    #[test]
    fn the_palette_records_and_replays_recent_commands_on_an_empty_query() {
        let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
        });
        h.state_mut().run("file.new", json!({"width": 64, "height": 64})).unwrap();
        h.state_mut().sync_views();
        let ctx = h.ctx.clone();
        // Run "Keyboard Shortcuts" through the palette: it becomes the recent command.
        crate::menus::invoke(h.state_mut(), &ctx, "edit.search", json!({})).unwrap();
        h.run_steps(2);
        h.event(Event::Text("keyboard shortcuts".into()));
        h.run_steps(2);
        h.event(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
        assert_eq!(super::recent_ids(&ctx).len(), 1, "the command the palette ran is remembered");
        // Close its dialog, reopen the palette with an empty query and press Enter: the recent
        // command is the first hit, so the same dialog opens again.
        let dialog = h
            .state()
            .ui
            .dialogs
            .iter()
            .find(|d| d.fields.get("__prefsui").and_then(|v| v.as_str()) == Some("shortcuts"))
            .map(|d| d.id)
            .expect("the shortcuts dialog");
        h.state_mut().ui.close_dialog(dialog);
        h.run_steps(2);
        crate::menus::invoke(h.state_mut(), &ctx, "edit.search", json!({})).unwrap();
        h.run_steps(2);
        h.event(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
        assert!(
            h.state().ui.dialogs.iter().any(|d| d.fields.get("__prefsui").and_then(|v| v.as_str()) == Some("shortcuts")),
            "the empty-query palette runs the recent command"
        );
        assert_eq!(super::recent_ids(&ctx).len(), 1, "re-running it moves it to the front, no duplicate");
    }

    /// #2479: menu separators matched on their menu path ("f" → File, Filter) and filled the
    /// palette with blank "---" rows.
    #[test]
    fn separators_are_not_palette_hits() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let menus = crate::menus::menu_items(&app);
        assert!(menus.iter().any(|m| m.label == "---"), "the menus have separators to filter");
        for q in ["f", "file", "filter", "", "-"] {
            let hits = super::menu_hits(menus.clone(), q, crate::i18n::Lang::EN);
            assert!(!hits.is_empty(), "{q:?} still matches commands");
            assert!(hits.iter().all(|h| h.1 != "---" && h.2 != "---"), "{q:?} lists a separator");
        }
    }

    #[test]
    fn fuzzy_ranks_prefix_and_contiguous_higher() {
        assert!(fuzzy_score("hue", "Hue/Saturation…").unwrap() > fuzzy_score("hue", "Channel Mixer Hue").unwrap_or(-99));
        assert!(fuzzy_score("gblur", "Gaussian Blur").is_some());
        assert!(fuzzy_score("xyz", "Gaussian Blur").is_none());
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    /// Typing a command's exact name ranks it first, even when a longer name scores the same.
    #[test]
    fn exact_name_ranks_first() {
        let settings = fuzzy_score("settings", "Settings Edit › Preferences Settings Edit › Preferences").unwrap() + exact_bonus("settings", &["Settings"]);
        let color = fuzzy_score("settings", "Color Settings… Edit Color Settings… Edit").unwrap() + exact_bonus("settings", &["Color Settings…"]);
        assert!(settings > color);
        assert_eq!(exact_bonus("Color settings", &["Color Settings…"]), 1000);
        assert_eq!(exact_bonus(" ", &[""]), 0);
        assert_eq!(exact_bonus("set", &["Settings"]), 0);
    }

    /// #536: Edit › Search → "Keyboard Shortcuts" must open the dialog, with and without a document.
    #[test]
    fn palette_opens_the_keyboard_shortcuts_dialog() {
        for with_doc in [false, true] {
            let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
            });
            if with_doc {
                h.state_mut().run("file.new", json!({"width": 64, "height": 64})).unwrap();
                h.state_mut().sync_views();
            }
            let ctx = h.ctx.clone();
            crate::menus::invoke(h.state_mut(), &ctx, "edit.search", json!({})).unwrap();
            h.run_steps(2);
            assert!(h.state().ui.palette_open, "Edit > Search opens the palette (with_doc={with_doc})");
            h.event(Event::Text("keyboard shortcuts".into()));
            h.run_steps(2);
            h.event(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            h.run_steps(3);
            let app = h.state();
            assert!(!app.ui.palette_open, "Enter closes the palette (with_doc={with_doc})");
            let dialog = app
                .ui
                .dialogs
                .iter()
                .find(|d| d.fields.get("__prefsui").and_then(|v| v.as_str()) == Some("shortcuts"))
                .map(|d| d.id)
                .unwrap_or_else(|| panic!("#536: the Keyboard Shortcuts dialog did not open (with_doc={with_doc})"));
            // The dialog keeps its own Enter: a second press is its OK and closes it.
            h.event(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            h.run_steps(3);
            assert!(h.state().ui.dialogs.iter().all(|d| d.id != dialog), "a second Enter closes the dialog as its OK (with_doc={with_doc})");
        }
    }

    /// RTL: the hit row's hand-painted composition mirrors — the command label anchors to the
    /// row's reading-start edge and the shortcut detail to the reading-end edge.
    #[test]
    fn hit_row_anchors_mirror_under_rtl() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 200.0), egui::vec2(560.0, 32.0));
        let _ltr = crate::direction::direction_scope(crate::direction::Dir::Ltr);
        let (label, label_align, detail, detail_align) = super::hit_row_anchors(rect);
        assert_eq!(label, egui::pos2(rect.left() + 12.0, rect.center().y));
        assert_eq!(label_align, egui::Align2::LEFT_CENTER);
        assert_eq!(detail, egui::pos2(rect.right() - 12.0, rect.center().y));
        assert_eq!(detail_align, egui::Align2::RIGHT_CENTER);
        drop(_ltr);
        let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
        let (label, label_align, detail, detail_align) = super::hit_row_anchors(rect);
        assert_eq!(label, egui::pos2(rect.right() - 12.0, rect.center().y));
        assert_eq!(label_align, egui::Align2::RIGHT_CENTER);
        assert_eq!(detail, egui::pos2(rect.left() + 12.0, rect.center().y));
        assert_eq!(detail_align, egui::Align2::LEFT_CENTER);
    }

    /// RTL: the painted hit rows mirror — each row's command label sits at the row's right edge
    /// and its shortcut detail at the left (LTR: label left, detail right), on the same rows.
    #[test]
    fn hit_rows_mirror_under_rtl() {
        /// The label and detail text rects of the painted palette rows, paired row by row:
        /// labels are 13-pt medium, details 11.5-pt, and a row's two share their center y.
        fn rows(rtl: bool) -> Vec<(egui::Rect, egui::Rect)> {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            // Fonts bind on the next pass (as in open_palette_interrupts_the_ime_only_when_it_takes_focus).
            let mut warmup = ctx.run_ui(Default::default(), |_| {});
            warmup.textures_delta.clear();
            app.ui.palette_open = true;
            let dir = if rtl { crate::direction::Dir::Rtl } else { crate::direction::Dir::Ltr };
            let _scope = crate::direction::direction_scope(dir);
            // The area's first frame is a sizing pass that paints nothing; classify the second.
            let mut out = ctx.run_ui(Default::default(), |ui| super::show(&mut app, ui.ctx()));
            out.textures_delta.clear();
            let mut out = ctx.run_ui(Default::default(), |ui| super::show(&mut app, ui.ctx()));
            out.textures_delta.clear();
            // The galley's layout rect (not the glyph mesh bounds) is what the anchor aligns,
            // so its center is exactly the anchor point for *_CENTER aligns.
            let layout_rect = |t: &egui::epaint::TextShape| t.galley.rect.translate(t.pos.to_vec2());
            let mut texts: Vec<(egui::Rect, f32)> = Vec::new();
            for shape in &out.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    let size = text.galley.job.sections.first().map_or(0.0, |s| s.format.font_id.size);
                    if size == 13.0 || size == 11.5 {
                        texts.push((layout_rect(text), size));
                    }
                }
            }
            // Pair each detail (11.5-pt) with the label (13-pt) on the same row (same center y);
            // other 13-pt text in the card (the hint, the warning icon) has no detail beside it.
            let mut rows = Vec::new();
            for (rect, size) in &texts {
                if let Some((label, _)) = texts.iter().find(|(r, s)| *s == 13.0 && (r.center().y - rect.center().y).abs() < 1.0)
                    && *size == 11.5
                {
                    rows.push((*label, *rect));
                }
            }
            rows
        }
        let ltr = rows(false);
        let rtl = rows(true);
        assert!(ltr.len() >= 8, "the palette paints its default hit list: {} rows", ltr.len());
        assert_eq!(rtl.len(), ltr.len(), "the palette paints the same hit list in RTL");
        for (i, ((l_label, l_detail), (r_label, r_detail))) in ltr.iter().zip(&rtl).enumerate() {
            assert!((l_label.center().y - r_label.center().y).abs() < 0.5, "row {i} keeps its place in both directions");
            assert!((l_label.center().y - l_detail.center().y).abs() < 1.0, "row {i}'s label and detail share the row: label={l_label:?} detail={l_detail:?}");
            assert!(l_label.right() < l_detail.left(), "LTR: row {i}'s label sits left of its detail");
            assert!(r_label.left() > r_detail.right(), "RTL: row {i}'s label sits right of its detail");
        }
    }

    /// The palette keeps its place and its commands in both directions.
    #[test]
    fn palette_keeps_its_place_under_rtl() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // Fonts bind on the next pass (as in open_palette_interrupts_the_ime_only_when_it_takes_focus).
        let mut warmup = ctx.run_ui(Default::default(), |_| {});
        warmup.textures_delta.clear();
        app.ui.palette_open = true;
        let mut area = |rtl: bool| {
            let dir = if rtl { crate::direction::Dir::Rtl } else { crate::direction::Dir::Ltr };
            let _scope = crate::direction::direction_scope(dir);
            // Two frames per direction so both are measured in the steady state.
            for _ in 0..2 {
                let mut out = ctx.run_ui(Default::default(), |ui| super::show(&mut app, ui.ctx()));
                out.textures_delta.clear();
            }
            ctx.memory(|m| m.area_rect(egui::Id::unique("palette")))
        };
        let rtl = area(true).expect("the palette is shown in RTL");
        let ltr = area(false).expect("the palette is shown in LTR");
        assert!(rtl.intersects(ctx.content_rect()), "the RTL palette stays on screen");
        assert_eq!(rtl, ltr, "the palette keeps its place in both directions");
    }

    /// RTL: the search field runs right-to-left in a right-to-left UI — its hint galley has a
    /// right-to-left paragraph base and sits at the field's right edge; LTR keeps egui's
    /// default auto-direction, left-aligned edit.
    #[test]
    fn the_search_field_galley_is_rtl_in_rtl_and_ltr_in_ltr() {
        let hint = tl!("Search commands, tools and panels…");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // Fonts bind on the next pass (as in open_palette_interrupts_the_ime_only_when_it_takes_focus).
        let mut warmup = ctx.run_ui(Default::default(), |_| {});
        warmup.textures_delta.clear();
        for (dir, rtl) in [(crate::direction::Dir::Ltr, false), (crate::direction::Dir::Rtl, true)] {
            let _scope = crate::direction::direction_scope(dir);
            let mut q = String::new();
            let mut field = egui::Rect::NOTHING;
            let mut shapes = Vec::new();
            // Two frames per direction so both are measured in the steady state.
            for _ in 0..2 {
                let mut out = ctx.run_ui(Default::default(), |ui| {
                    ui.set_width(560.0);
                    field = super::search_edit(ui, &mut q, 560.0).rect;
                });
                out.textures_delta.clear();
                shapes = out.shapes;
            }
            fn hint_galley<'a>(shape: &'a egui::Shape, hint: &str) -> Option<(&'a egui::Galley, egui::Pos2)> {
                match shape {
                    egui::Shape::Text(t) if t.galley.job.text == hint => Some((&t.galley, t.pos)),
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| hint_galley(s, hint)),
                    _ => None,
                }
            }
            let (g, pos) = shapes
                .iter()
                .find_map(|s| hint_galley(&s.shape, hint))
                .expect("the empty field paints its hint");
            let want_dir = if rtl { egui::epaint::text::TextDirection::RightToLeft } else { egui::epaint::text::TextDirection::Auto };
            assert_eq!(g.job.direction, want_dir, "{dir:?}: paragraph base direction");
            let rect = egui::Rect::from_min_size(pos, g.size());
            if rtl {
                assert!(rect.right() > field.center().x, "{dir:?}: hint right-aligned in {field:?}, got {rect:?}");
            } else {
                assert!(rect.left() < field.center().x, "{dir:?}: hint left-aligned in {field:?}, got {rect:?}");
            }
        }
    }
}
