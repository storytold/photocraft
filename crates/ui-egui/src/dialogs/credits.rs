//! The About window's credits: every contributor (GitHub login always; display and real names only
//! with recorded consent) and the AI models that helped. The tables are baked into the binary by
//! build.rs from contributors/contributors.json (craftrules standards/contributors.md); nothing is
//! read at run time. UI strings go through `tl!` (column abbreviations like `+LOC` stay as they are).

use std::cmp::Ordering;

use egui::RichText;

use crate::PhotocraftApp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contributor {
    pub login: &'static str,
    pub display_name: Option<&'static str>,
    pub real_name: Option<&'static str>,
    pub prs: u64,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
    pub binary_added: u64,
    pub binary_deleted: u64,
    /// ISO 8601 UTC; empty for someone credited only through a merged PR.
    pub first_commit: &'static str,
    pub last_commit: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub company: &'static str,
    pub model: &'static str,
    pub version: &'static str,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
}

include!(concat!(env!("OUT_DIR"), "/credits.rs"));

/// Which name to show. Display and real names fall back to the `@username` when not given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NameMode {
    #[default]
    Username,
    DisplayName,
    RealName,
}

impl NameMode {
    pub const ALL: [NameMode; 3] = [NameMode::Username, NameMode::DisplayName, NameMode::RealName];

    pub fn label(self) -> &'static str {
        match self {
            NameMode::Username => tl!("Username"),
            NameMode::DisplayName => tl!("Display name"),
            NameMode::RealName => tl!("Real name"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Prs,
    Commits,
    LinesAdded,
    LinesDeleted,
    LinesDelta,
    BinaryAdded,
    BinaryDeleted,
    #[default]
    FirstCommit,
    LastCommit,
}

impl SortKey {
    pub const ALL: [SortKey; 10] = [
        SortKey::Name,
        SortKey::Prs,
        SortKey::Commits,
        SortKey::LinesAdded,
        SortKey::LinesDeleted,
        SortKey::LinesDelta,
        SortKey::BinaryAdded,
        SortKey::BinaryDeleted,
        SortKey::FirstCommit,
        SortKey::LastCommit,
    ];

    /// Menu label and table header, in the UI language.
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            SortKey::Name => (tl!("Name (A–Z)"), tl!("Name")),
            SortKey::Prs => (tl!("Merged PRs"), "PRs"),
            SortKey::Commits => (tl!("Commits"), tl!("Commits")),
            SortKey::LinesAdded => (tl!("Lines added"), "+LOC"),
            SortKey::LinesDeleted => (tl!("Lines deleted"), "−LOC"),
            SortKey::LinesDelta => (tl!("Line delta"), "ΔLOC"),
            SortKey::BinaryAdded => (tl!("Binary assets added"), "+Bin"),
            SortKey::BinaryDeleted => (tl!("Binary assets removed"), "−Bin"),
            SortKey::FirstCommit => (tl!("First commit"), tl!("First")),
            SortKey::LastCommit => (tl!("Last commit"), tl!("Last")),
        }
    }

    /// Names and dates read naturally oldest/A first; counts read biggest first.
    pub fn default_ascending(self) -> bool {
        matches!(self, SortKey::Name | SortKey::FirstCommit | SortKey::LastCommit)
    }

    /// Counts line up on their column's reading end; names and dates read from the reading start.
    fn numeric(self) -> bool {
        !self.default_ascending()
    }

    /// The table cell for `c` (the name column shows `c.name(..)` as a link instead).
    fn cell(self, c: &Contributor) -> String {
        match self {
            SortKey::Name => c.login.to_string(),
            SortKey::Prs => group(c.prs),
            SortKey::Commits => group(c.commits),
            SortKey::LinesAdded => group(c.lines_added),
            SortKey::LinesDeleted => group(c.lines_deleted),
            SortKey::LinesDelta => signed(c.lines_delta()),
            SortKey::BinaryAdded => group(c.binary_added),
            SortKey::BinaryDeleted => group(c.binary_deleted),
            SortKey::FirstCommit => day(c.first_commit).to_string(),
            SortKey::LastCommit => day(c.last_commit).to_string(),
        }
    }
}

impl Contributor {
    pub fn name(&self, mode: NameMode) -> String {
        let chosen = match mode {
            NameMode::Username => None,
            NameMode::DisplayName => self.display_name,
            NameMode::RealName => self.real_name,
        };
        chosen.map_or_else(|| format!("@{}", self.login), str::to_string)
    }

    pub fn lines_delta(&self) -> i128 {
        i128::from(self.lines_added) - i128::from(self.lines_deleted)
    }

    /// One line with everything we know, for tooltips.
    pub fn summary(&self) -> String {
        crate::i18n::fmt(
            tl!("@{login}: {prs} PRs, {commits} commits, +{added} / −{deleted} lines (Δ {delta}), +{binAdded} / −{binDeleted} binary assets, {first} – {last}"),
            &[
                ("login", self.login),
                ("prs", &group(self.prs)),
                ("commits", &group(self.commits)),
                ("added", &group(self.lines_added)),
                ("deleted", &group(self.lines_deleted)),
                ("delta", &signed(self.lines_delta())),
                ("binAdded", &group(self.binary_added)),
                ("binDeleted", &group(self.binary_deleted)),
                ("first", day(self.first_commit)),
                ("last", day(self.last_commit)),
            ],
        )
    }
}

/// Alphabetical key: case-insensitive, ignoring the `@` of usernames.
fn name_key(name: &str) -> String {
    name.trim_start_matches('@').to_lowercase()
}

fn compare(a: &Contributor, b: &Contributor, mode: NameMode, key: SortKey) -> Ordering {
    match key {
        SortKey::Name => name_key(&a.name(mode)).cmp(&name_key(&b.name(mode))),
        SortKey::Prs => a.prs.cmp(&b.prs),
        SortKey::Commits => a.commits.cmp(&b.commits),
        SortKey::LinesAdded => a.lines_added.cmp(&b.lines_added),
        SortKey::LinesDeleted => a.lines_deleted.cmp(&b.lines_deleted),
        SortKey::LinesDelta => a.lines_delta().cmp(&b.lines_delta()),
        SortKey::BinaryAdded => a.binary_added.cmp(&b.binary_added),
        SortKey::BinaryDeleted => a.binary_deleted.cmp(&b.binary_deleted),
        // A PR-only contributor (no commit date) sorts after everyone with a date.
        SortKey::FirstCommit => dated(a.first_commit).cmp(&dated(b.first_commit)),
        SortKey::LastCommit => dated(a.last_commit).cmp(&dated(b.last_commit)),
    }
}

fn dated(d: &str) -> (bool, &str) {
    (d.is_empty(), d)
}

/// Contributors in display order. Ties fall back to the name (A–Z) so the order is stable.
pub fn sorted(list: &[Contributor], mode: NameMode, key: SortKey, ascending: bool) -> Vec<&Contributor> {
    let mut v: Vec<&Contributor> = list.iter().collect();
    v.sort_by(|a, b| {
        let o = compare(a, b, mode, key);
        let o = if ascending { o } else { o.reverse() };
        o.then_with(|| compare(a, b, mode, SortKey::Name)).then_with(|| a.login.cmp(b.login))
    });
    v
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn signed(n: i128) -> String {
    let g = group(u64::try_from(n.unsigned_abs()).unwrap_or(u64::MAX));
    if n < 0 { format!("−{g}") } else { format!("+{g}") }
}

fn day(iso: &str) -> &str {
    if iso.is_empty() { "—" } else { iso.get(..10).unwrap_or(iso) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    names: NameMode,
    key: SortKey,
    ascending: bool,
    table: bool,
}

impl Default for View {
    fn default() -> Self {
        View { names: NameMode::Username, key: SortKey::FirstCommit, ascending: true, table: false }
    }
}

/// Height of the Contributors list (grab bag or table, header included), the same on both views so
/// switching between them doesn't resize the window. The Models table is as tall as its rows, up
/// to this.
const LIST_HEIGHT: f32 = 340.0;
const TABLE_HEADER_H: f32 = 24.0;
const TABLE_ROW_H: f32 = 22.0;
/// Room for the sort arrow in every header, so the columns don't move when the sort does.
const SORT_ARROW_W: f32 = 12.0;
/// Padding inside each cell, on each side.
const CELL_PAD: f32 = 6.0;
const NAME_MIN_W: f32 = 110.0;

/// About ▸ Contributors: a name toggle, a sort, and the list as a grab bag or a table.
pub fn contributors_ui(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    contributors_in(app, ui, CONTRIBUTORS, TOTAL_COMMITS);
}

/// [`contributors_ui`] over a given list (tests pass their own).
fn contributors_in(app: &mut PhotocraftApp, ui: &mut egui::Ui, contributors: &[Contributor], total_commits: u64) {
    // The dialog body mirrors its blocks one by one; this one mirrors as a whole, so its rows read
    // from the right.
    crate::text::direction::window_body(ui, |ui| {
        contributors_body(app, ui, contributors, total_commits);
    });
}

/// The contributors list: the controls, the counts and the list itself.
fn contributors_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, contributors: &[Contributor], total_commits: u64) {
    let id = egui::Id::unique("credits_view");
    let mut v = ui.data_mut(|d| d.get_temp::<View>(id)).unwrap_or_default();
    ui.with_layout(crate::direction::row().with_main_wrap(true), |ui| {
        ui.label(tl!("Show"));
        for m in NameMode::ALL {
            if ui.selectable_label(v.names == m, m.label()).clicked() {
                v.names = m;
            }
        }
        ui.separator();
        ui.label(tl!("Sort"));
        let (press, close) = crate::press_menu::PressCombo::before(ui, "credits_sort");
        let sort = craft_ui::ComboBox::from_id_salt("credits_sort").selected_text(v.key.label().0).close_behavior(close).show_ui(ui, |ui| {
            for k in SortKey::ALL {
                let item = ui.selectable_label(v.key == k, k.label().0);
                if press.chosen(ui, &item) {
                    v.key = k;
                    v.ascending = k.default_ascending();
                }
            }
        });
        press.after(&sort.response);
        let _ = sort.response.accessible_name(tl!("Sort"));
        if ui.button(if v.ascending { "▲" } else { "▼" }).on_hover_text(tl!("Reverse the order")).clicked() {
            v.ascending = !v.ascending;
        }
        ui.separator();
        if ui.selectable_label(!v.table, tl!("Grab bag")).clicked() {
            v.table = false;
        }
        if ui.selectable_label(v.table, tl!("Table")).clicked() {
            v.table = true;
        }
    });
    let list = sorted(contributors, v.names, v.key, v.ascending);
    let counts = crate::i18n::fmt(
        tl!("Contributors: {contributors} · Commits: {commits}"),
        &[("contributors", &group(list.len() as u64)), ("commits", &group(total_commits))],
    );
    ui.label(RichText::new(counts).small().weak());
    ui.separator();
    let height = LIST_HEIGHT;
    if list.is_empty() {
        ui.label(tl!("No contributor data was built into this copy."));
    } else if v.table {
        table(app, ui, &list, &mut v, height);
    } else {
        egui::ScrollArea::vertical().id_salt("credits_bag").auto_shrink([false, false]).max_height(height).show(ui, |ui| {
            // Spacing alone separates the names: a "·" between them started some lines.
            ui.spacing_mut().item_spacing = egui::vec2(14.0, 8.0);
            // Each name is one unit: otherwise egui flows a link that doesn't fit onto the next
            // line as wrapped text, at the font's line height instead of the row spacing.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            // horizontal_wrapped gives its first row the interact height; text rows are shorter,
            // so the first gap came out wider than the rest.
            ui.spacing_mut().interact_size.y = 0.0;
            ui.with_layout(crate::direction::row().with_main_wrap(true), |ui| {
                for c in &list {
                    link(app, ui, c, v.names);
                }
            });
        });
    }
    ui.data_mut(|d| d.insert_temp(id, v));
}

/// Height of a table showing all `rows` (header included), up to [`LIST_HEIGHT`].
fn table_height(ui: &egui::Ui, rows: usize) -> f32 {
    // Each row, and the header, is followed by the row gap.
    let gap = ui.spacing().item_spacing.y;
    let body = rows as f32 * (TABLE_ROW_H + gap) + gap;
    (TABLE_HEADER_H + body).min(LIST_HEIGHT)
}

/// A contributor's name, linking to their GitHub profile. Routes through [`crate::links::open`]
/// (the platform browser service) rather than `ui.hyperlink_to`'s `ctx.open_url`, which the
/// desktop build can't rely on (egui_winit needs its "links" feature, which we don't enable).
fn link(app: &mut PhotocraftApp, ui: &mut egui::Ui, c: &Contributor, names: NameMode) -> egui::Response {
    let url = format!("https://github.com/{}", c.login);
    let r = ui.add(egui::Link::new(c.name(names))).on_hover_text(c.summary());
    if r.clicked() {
        crate::links::open(app, ui.ctx(), &url);
    }
    r
}

fn text_width(ui: &egui::Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), egui::Color32::WHITE).size().x)
}

/// A column's width: its widest cell, or its header plus the sort arrow's room, so no column
/// changes width when the sort or the rows change.
fn column_width<'a>(ui: &egui::Ui, header: &str, cells: impl Iterator<Item = &'a str>) -> f32 {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let widest = cells.map(|c| text_width(ui, c, &body)).fold(0.0, f32::max);
    let head = text_width(ui, header, &crate::theme::semibold(body.size)) + SORT_ARROW_W;
    (widest.max(head) + 2.0 * CELL_PAD).ceil()
}

/// The columns in the order they are drawn: in a right-to-left UI the reading start is the
/// right, so the columns come from the end. `egui_extras::Table` lays its cells out from the
/// left whatever the direction, so the order is ours to set.
fn visual_order<T: Copy, const N: usize>(columns: &[T; N]) -> [T; N] {
    let mut out = *columns;
    if crate::text::direction::is_rtl() {
        out.reverse();
    }
    out
}

/// Does the content of a cell hug its right edge? Counts line up on the reading end of their
/// column and names and dates read from the reading start, so in a right-to-left UI both swap
/// sides.
fn hugs_the_right(numeric: bool) -> bool {
    if numeric { !crate::text::direction::is_rtl() } else { crate::text::direction::is_rtl() }
}

/// A credits table: columns of `widths`, the `None` one taking what the others leave (at least
/// `flex_min`). The header stays put while the rows scroll, within `height` (header included). Too
/// narrow for every column (a long translation), it scrolls sideways rather than clip one.
fn credits_table(
    ui: &mut egui::Ui,
    id: &str,
    widths: &[Option<f32>],
    flex_min: f32,
    height: f32,
    header: impl FnOnce(egui_extras::TableRow<'_, '_>),
    body: impl FnOnce(egui_extras::TableBody<'_>),
) {
    // The rows' scroll bar sits beside the columns: leave it room, or it pushes the table past the
    // window and a sideways scroll bar appears under it.
    let fixed: f32 = widths.iter().flatten().sum::<f32>() + ui.spacing().scroll.allocated_width();
    // The table puts a row gap under its header.
    let body_h = (height - TABLE_HEADER_H - ui.spacing().item_spacing.y).max(TABLE_ROW_H);
    egui::ScrollArea::horizontal().id_salt((id, "h")).auto_shrink([false, true]).show(ui, |ui| {
        let flex = (ui.available_width() - fixed).max(flex_min);
        // No gaps between cells: their padding spaces them, and the header rule runs unbroken.
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut b = egui_extras::TableBuilder::new(ui)
            .id_salt(id)
            .striped(true)
            .resizable(false)
            .auto_shrink([false, false])
            .min_scrolled_height(body_h)
            .max_scroll_height(body_h)
            .cell_layout(crate::text::direction::row());
        for w in widths {
            b = b.column(match w {
                Some(w) => egui_extras::Column::exact(*w),
                None => egui_extras::Column::exact(flex).clip(true),
            });
        }
        b.header(TABLE_HEADER_H, header).body(body);
    });
}

/// The Contributors table; clicking a header sorts by it (again reverses it).
fn table(app: &mut PhotocraftApp, ui: &mut egui::Ui, list: &[&Contributor], v: &mut View, height: f32) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let names = v.names;
    let keys = visual_order(&SortKey::ALL);
    let widths: Vec<Option<f32>> = keys
        .iter()
        .map(|&k| {
            let cells: Vec<String> = list.iter().map(|c| k.cell(c)).collect();
            (k != SortKey::Name).then(|| column_width(ui, k.label().1, cells.iter().map(String::as_str)))
        })
        .collect();
    credits_table(
        ui,
        "credits_table",
        &widths,
        NAME_MIN_W,
        height,
        |mut header| {
            for &k in &keys {
                header.col(|ui| {
                    let sort = (v.key == k).then_some(v.ascending);
                    let (long, short) = k.label();
                    if header_cell(ui, &t, short, k.numeric(), Some(sort)).on_hover_text(long).clicked() {
                        if v.key == k {
                            v.ascending = !v.ascending;
                        } else {
                            v.key = k;
                            v.ascending = k.default_ascending();
                        }
                    }
                });
            }
        },
        |body| {
            body.rows(TABLE_ROW_H, list.len(), |mut row| {
                let Some(c) = list.get(row.index()) else { return };
                for &k in &keys {
                    row.col(|ui| {
                        if k == SortKey::Name {
                            ui.add_space(CELL_PAD);
                            link(app, ui, c, names);
                        } else {
                            cell(ui, k.numeric(), &k.cell(c));
                        }
                    });
                }
            });
        },
    );
}

fn cell(ui: &mut egui::Ui, numeric: bool, text: &str) {
    // Counts line up on the reading end of their column, names and dates read from the reading
    // start; the numbers themselves stay left-to-right.
    let layout = if numeric { crate::text::direction::trailing() } else { crate::text::direction::row() };
    ui.with_layout(layout, |ui| {
        ui.add_space(CELL_PAD);
        ui.add(egui::Label::new(text).selectable(false));
    });
}

/// One header: its label (over the counts, on their reading end) under a rule. A sortable
/// one (`sort` is `Some`) is a button, with the arrow on the label's inner side when it's the sort
/// (`Some(Some(ascending))`), in room every header keeps.
fn header_cell(ui: &mut egui::Ui, t: &crate::theme::Tokens, label: &str, numeric: bool, sort: Option<Option<bool>>) -> egui::Response {
    let rect = ui.max_rect();
    let resp = ui.allocate_rect(rect, if sort.is_some() { egui::Sense::click() } else { egui::Sense::hover() });
    let kind = if sort.is_some() { egui::Role::Button } else { egui::Role::Label };
    resp.widget_info(|| egui::WidgetInfo::labeled(kind, true, label));
    let resp = if sort.is_some() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    let current = sort.flatten();
    let color = if sort.is_none() || current.is_some() || resp.hovered() { t.text } else { t.text_dim };
    let size = egui::TextStyle::Body.resolve(ui.style()).size;
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), crate::theme::semibold(size), color);
    job.direction = crate::direction::text_direction();
    let galley = ui.painter().layout_job(job);
    let inner = rect.shrink2(egui::vec2(CELL_PAD, 0.0));
    let y = rect.center().y - galley.size().y / 2.0;
    let (text_x, arrow_x) = if hugs_the_right(numeric) {
        let x = inner.right() - galley.size().x;
        (x, x - SORT_ARROW_W / 2.0)
    } else {
        (inner.left(), inner.left() + galley.size().x + SORT_ARROW_W / 2.0)
    };
    ui.painter().galley(egui::pos2(text_x, y), galley, color);
    if let Some(ascending) = current {
        let (c, h) = (egui::pos2(arrow_x, rect.center().y), 3.0);
        let pts = if ascending {
            vec![c + egui::vec2(-h, h * 0.6), c + egui::vec2(h, h * 0.6), c + egui::vec2(0.0, -h * 0.9)]
        } else {
            vec![c + egui::vec2(-h, -h * 0.6), c + egui::vec2(h, -h * 0.6), c + egui::vec2(0.0, h * 0.9)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, t.accent, egui::Stroke::NONE));
    }
    let line = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 1.0), rect.max);
    ui.painter().rect_filled(line, 0.0, t.separator);
    resp
}

/// About ▸ Models: AI models credited in Co-Authored-By trailers.
pub fn models_ui(ui: &mut egui::Ui) {
    models_in(ui, MODELS, TOTAL_COMMITS);
}

/// Share of all commits, whole percent; a model with any commits never reads "0%".
fn percent(commits: u64, total: u64) -> String {
    let p = 100.0 * commits as f64 / total.max(1) as f64;
    if commits > 0 && p < 0.5 { "<1%".into() } else { format!("{:.0}%", p.min(100.0)) }
}

/// The Models table's cells for `m`, column by column; counts are the last three.
fn model_cells(m: &Model, total: u64) -> [String; 6] {
    [
        m.company.to_string(),
        m.model.to_string(),
        m.version.to_string(),
        group(m.commits),
        percent(m.commits, total),
        format!("+{} / −{}", group(m.lines_added), group(m.lines_deleted)),
    ]
}

/// The Models table's headers, one per column (`model_cells` fills the same order).
const MODEL_COLUMNS: usize = 6;

/// [`models_ui`] over a given list (tests pass their own).
fn models_in(ui: &mut egui::Ui, models: &[Model], total_commits: u64) {
    crate::text::direction::window_body(ui, |ui| {
        models_body(ui, models, total_commits);
    });
}

/// The models table: the counts and the list itself.
fn models_body(ui: &mut egui::Ui, models: &[Model], total_commits: u64) {
    if models.is_empty() {
        ui.label(tl!("No model credits were built into this copy."));
        return;
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    // Counted against every commit, or the busiest model's if the total somehow trails it.
    let total = total_commits.max(models.iter().map(|m| m.commits).max().unwrap_or(0));
    let heads: [&str; MODEL_COLUMNS] = [tl!("Company"), tl!("Model"), tl!("Version"), tl!("Commits"), tl!("% of all commits"), tl!("Lines +/−")];
    let rows: Vec<[String; 6]> = models.iter().map(|m| model_cells(m, total)).collect();
    let order = visual_order(&core::array::from_fn::<usize, MODEL_COLUMNS, _>(|i| i));
    // The model name takes the spare width.
    let widths: Vec<Option<f32>> =
        order.iter().map(|&i| (i != 1).then(|| column_width(ui, heads[i], rows.iter().filter_map(|r| r.get(i)).map(String::as_str)))).collect();
    let numeric = |i: usize| i >= 3;
    credits_table(
        ui,
        "credits_models",
        &widths,
        NAME_MIN_W,
        table_height(ui, rows.len()),
        |mut header| {
            for &i in &order {
                header.col(|ui| {
                    header_cell(ui, &t, heads[i], numeric(i), None);
                });
            }
        },
        |body| {
            body.rows(TABLE_ROW_H, rows.len(), |mut row| {
                let Some(cells) = rows.get(row.index()) else { return };
                for &i in &order {
                    row.col(|ui| cell(ui, numeric(i), &cells[i]));
                }
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::{NodeT, Queryable};

    /// The toolbar row reads in reading order: the "Show" label hugs the row's reading-start
    /// edge, left in LTR and right in RTL, and the item gaps stay equal in both directions.
    #[test]
    fn toolbar_row_mirrors_item_order_in_rtl() {
        use crate::direction::{Dir, direction_scope};
        fn items(dir: Dir) -> (egui::Rect, egui::Rect) {
            let _scope = direction_scope(dir);
            let ctx = egui::Context::default();
            let mut out = None;
            let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
                out = Some(
                    ui.allocate_ui_with_layout(egui::vec2(400.0, 200.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.with_layout(crate::direction::row().with_main_wrap(true), |ui| {
                            let a = ui.label("Show");
                            let b = ui.label("Sort");
                            (a.rect, b.rect)
                        })
                        .inner
                    })
                    .inner,
                );
            });
            full.textures_delta.clear();
            out.unwrap()
        }
        let (ltr_a, ltr_b) = items(Dir::Ltr);
        let (rtl_a, rtl_b) = items(Dir::Rtl);
        assert!(ltr_a.left() < ltr_b.left(), "LTR: Show leads on the left: {ltr_a:?} then {ltr_b:?}");
        assert!(rtl_b.left() < rtl_a.left(), "RTL: Show leads on the right: {rtl_a:?} then {rtl_b:?}");
        let ltr_gap = ltr_b.left() - ltr_a.right();
        let rtl_gap = rtl_a.left() - rtl_b.right();
        assert!((ltr_gap - rtl_gap).abs() < 1.0, "same item gap in both directions: {ltr_gap} vs {rtl_gap}");
    }

    fn c(login: &'static str, display: Option<&'static str>, real: Option<&'static str>, added: u64, deleted: u64, first: &'static str) -> Contributor {
        Contributor {
            login,
            display_name: display,
            real_name: real,
            prs: 0,
            commits: 1,
            lines_added: added,
            lines_deleted: deleted,
            binary_added: 0,
            binary_deleted: 0,
            first_commit: first,
            last_commit: first,
        }
    }

    fn people() -> [Contributor; 4] {
        [
            c("zed", None, None, 5, 50, "2026-01-02T00:00:00Z"),
            c("Bob", Some("Ann"), None, 10, 0, "2026-01-01T00:00:00Z"),
            c("alice", None, None, 7, 1, ""),
            c("carol", None, Some("Dana"), 1, 0, "2026-01-03T00:00:00Z"),
        ]
    }

    fn logins(v: &[&Contributor]) -> Vec<&'static str> {
        v.iter().map(|c| c.login).collect()
    }

    #[test]
    fn names_fall_back_to_the_username() {
        let p = c("bob", Some("Bobby"), None, 0, 0, "");
        assert_eq!(p.name(NameMode::Username), "@bob");
        assert_eq!(p.name(NameMode::DisplayName), "Bobby");
        assert_eq!(p.name(NameMode::RealName), "@bob");
    }

    #[test]
    fn alphabetical_ignores_at_and_case() {
        let all = people();
        let v = sorted(&all, NameMode::Username, SortKey::Name, true);
        assert_eq!(logins(&v), ["alice", "Bob", "carol", "zed"]);
        let v = sorted(&all, NameMode::DisplayName, SortKey::Name, true);
        assert_eq!(logins(&v), ["alice", "Bob", "carol", "zed"]); // "Ann" sorts with the @usernames
        let v = sorted(&all, NameMode::RealName, SortKey::Name, false);
        assert_eq!(logins(&v), ["zed", "carol", "Bob", "alice"]); // "Dana" between bob and zed
    }

    #[test]
    fn delta_and_dates() {
        let all = people();
        let v = sorted(&all, NameMode::Username, SortKey::LinesDelta, false);
        assert_eq!(logins(&v), ["Bob", "alice", "carol", "zed"]);
        let v = sorted(&all, NameMode::Username, SortKey::FirstCommit, true);
        assert_eq!(logins(&v), ["Bob", "zed", "carol", "alice"]); // undated last
    }

    #[test]
    fn formatting() {
        assert_eq!(group(1234567), "1,234,567");
        assert_eq!(group(12), "12");
        assert_eq!(signed(-1000), "−1,000");
        assert_eq!(day("2026-10-07T18:45:23Z"), "2026-10-07");
        assert_eq!(day(""), "—");
    }

    /// The Contributors table as drawn, for the right-to-left twin.
    fn table_harness(contributors: Vec<Contributor>) -> egui_kittest::Harness<'static> {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(720.0, 520.0)).build_ui(move |ui| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            let id = egui::Id::unique("credits_view");
            if ui.data(|d| d.get_temp::<View>(id)).is_none() {
                ui.data_mut(|d| d.insert_temp(id, View { table: true, ..View::default() }));
            }
            ui.set_width(700.0);
            contributors_in(&mut app, ui, &contributors, 1);
        });
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        h
    }

    /// A column's header: the button that sorts by it.
    fn header_rect<S>(h: &egui_kittest::Harness<'_, S>, k: SortKey) -> egui::Rect {
        let label = k.label().1;
        h.query_all_by_role(egui::accesskit::Role::Button)
            .find(|n| n.accesskit_node().label().is_some_and(|l| l == label || l.starts_with(label)))
            .map(|n| n.rect())
            .unwrap_or_else(|| panic!("no {label} header"))
    }

    /// The cells under a column's header.
    fn column_cells<S>(h: &egui_kittest::Harness<'_, S>, col: egui::Rect) -> Vec<egui::Rect> {
        h.query_all_by_role(egui::accesskit::Role::Label)
            .map(|n| n.rect())
            .filter(|r| r.top() > col.bottom() && r.center().x > col.left() && r.center().x < col.right())
            .collect()
    }

    /// The Models table as drawn, for the right-to-left twin.
    fn models_harness(models: Vec<Model>) -> egui_kittest::Harness<'static> {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(720.0, 520.0)).build_ui(move |ui| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            ui.set_width(700.0);
            models_in(ui, &models, 482);
        });
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        h
    }

    /// A header of the Models table: it sorts nothing, so it is a label.
    fn model_header(h: &egui_kittest::Harness<'_>, label: &str) -> egui::Rect {
        h.query_all_by_label(label).next().map(|n| n.rect()).unwrap_or_else(|| panic!("no {label} header"))
    }

    #[test]
    fn the_models_table_mirrors_in_a_right_to_left_ui() {
        let _rtl = crate::text::direction::direction_scope(crate::text::direction::Dir::Rtl);
        let models: Vec<Model> = ["A", "B", "C"]
            .iter()
            .enumerate()
            .map(|(i, name)| Model {
                company: "Anthropic",
                model: name,
                version: "1.0",
                commits: 228 - i as u64 * 100,
                lines_added: 150_000,
                lines_deleted: 20_000,
            })
            .collect();
        let h = models_harness(models);
        // The company is the reading start: the rightmost column.
        let company = model_header(&h, "Company");
        let lines = model_header(&h, "Lines +/−");
        assert!(company.left() > lines.left(), "the columns didn't mirror: {company:?} vs {lines:?}");
        // The counts line up on the reading end (the left).
        let counts = column_cells(&h, model_header(&h, "Commits"));
        assert_eq!(counts.len(), 3, "{} commit cells", counts.len());
        assert!(counts.iter().all(|r| (r.left() - counts[0].left()).abs() < 0.5), "the counts don't line up: {counts:?}");
    }

    #[test]
    fn the_contributors_table_mirrors_in_a_right_to_left_ui() {
        let _rtl = crate::text::direction::direction_scope(crate::text::direction::Dir::Rtl);
        let h = table_harness(people().to_vec());
        let headers: Vec<egui::Rect> = SortKey::ALL.iter().map(|&k| header_rect(&h, k)).collect();
        // The reading start (the name) is the rightmost column, the reading end the leftmost.
        assert!(headers.windows(2).all(|w| w[0].left() > w[1].left()), "the columns didn't mirror: {headers:?}");
        // The counts line up on the reading end (the left), the dates read from the reading start.
        let counts = column_cells(&h, header_rect(&h, SortKey::LinesAdded));
        assert!(counts.len() >= 4, "{} line-count cells", counts.len());
        assert!(counts.iter().all(|r| (r.left() - counts[0].left()).abs() < 0.5), "the counts don't line up: {counts:?}");
        assert!(counts.iter().any(|r| (r.right() - counts[0].right()).abs() > 0.5), "the counts aren't left-aligned: {counts:?}");
        let dates = column_cells(&h, header_rect(&h, SortKey::FirstCommit));
        assert!(dates.len() >= 4, "{} date cells", dates.len());
        assert!(dates.iter().all(|r| (r.right() - dates[0].right()).abs() < 0.5), "the dates don't line up: {dates:?}");
        assert!(dates.iter().any(|r| (r.left() - dates[0].left()).abs() > 0.5), "the dates aren't right-aligned: {dates:?}");
    }

    /// The About dialog on its Contributors tab, table view, as drawn in the window.
    fn about_contributors_harness() -> egui_kittest::Harness<'static, crate::PhotocraftApp> {
        let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let mut fields = serde_json::Map::new();
        fields.insert("tab".into(), serde_json::json!("contributors"));
        h.state_mut().ui.open_dialog(crate::state::DialogKind::About, fields);
        h.ctx.data_mut(|d| d.insert_temp(egui::Id::unique("credits_view"), View { table: true, ..View::default() }));
        h.run_steps(6);
        h
    }

    /// RTL: the About dialog opens as wide as its left-to-right twin — the mirrored body scope
    /// must not balloon the frame (the dialog host's one unverified fork behaviour).
    #[test]
    fn the_about_dialog_keeps_its_width_in_a_right_to_left_ui() {
        let width = |dir: crate::text::direction::Dir| -> f32 {
            let _dir = crate::text::direction::direction_scope(dir);
            let h = about_contributors_harness();
            crate::dialogs::rects(&h.ctx)[0].width()
        };
        let ltr = width(crate::text::direction::Dir::Ltr);
        let rtl = width(crate::text::direction::Dir::Rtl);
        assert!((ltr - rtl).abs() <= 1.0, "the About frame is {ltr} pt wide in LTR and {rtl} pt in RTL");
    }

    /// RTL twin of `every_column_fits_the_about_window` (`ui_tests`), written order-independently
    /// (the smallest left, the largest right) so it holds whichever way the columns are drawn: a
    /// long translation that pushes the table past the window fails here.
    #[test]
    fn every_column_fits_the_about_window_in_a_right_to_left_ui() {
        let _rtl = crate::text::direction::direction_scope(crate::text::direction::Dir::Rtl);
        let h = about_contributors_harness();
        let frame = crate::dialogs::rects(&h.ctx)[0];
        let hs: Vec<egui::Rect> = SortKey::ALL.iter().map(|&k| header_rect(&h, k)).collect();
        let left = hs.iter().map(|r| r.left()).fold(f32::INFINITY, f32::min);
        let right = hs.iter().map(|r| r.right()).fold(f32::NEG_INFINITY, f32::max);
        assert!(right - left <= frame.width() + 0.5, "the table is {} pt wide in a {} pt window", right - left, frame.width());
        // Every column sits inside the window, whichever side it is drawn on.
        for (k, col) in SortKey::ALL.iter().zip(&hs) {
            assert!(col.left() >= frame.left() - 1.0 && col.right() <= frame.right() + 1.0, "{k:?}: {col:?} spills out of {frame:?}");
        }
    }
}

#[cfg(test)]
mod ui_tests;
