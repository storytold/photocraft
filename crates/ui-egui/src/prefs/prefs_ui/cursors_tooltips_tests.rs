//! Preferences › Cursors: the reference application's tooltips (#16833). The window draws a rule's help line
//! under its control, and that same text joins the control's accessible description, so a trial reads the open
//! window's painted frame: the whole measured sentence, in the row it belongs to.
//!
//! The measured table names a sentence for every row of the reference page. PhotoCraft's page has five rows
//! today, and only the three whose reference row is one control have a sentence to show: the Painting and
//! Other selects stand for several reference radios each, and the Brush Tip Outline, No Cursor While Painting
//! and Brush Leash rows have no setting yet. Those rows keep the state they have today.

use super::fields::rules;
use super::tests::app_with_store;
use super::*;
use craft::settings::testing::MemoryFiles;
use craft::settings::{Location, OpenOptions, SettingsStore};
use craft::ui::patterns::preferences::RuleKind;
use egui::vec2;
use photocraft_engine::prefs;
use std::sync::Arc;
use std::time::Duration;

/// The folder of these trials' scratch settings store; its file lives in memory.
const DIR: &str = "/memory/photocraft-cursors-tooltips";

/// Services whose preferences store reads and writes `files`. A zero debounce keeps the store idle.
fn services_on(files: &Arc<MemoryFiles>) -> crate::Services {
    let options = OpenOptions::new(Location::Dir(DIR.into())).debounce(Duration::ZERO).files(files.clone());
    crate::Services { settings: Some(SettingsStore::<prefs::SavedPrefs>::open_with(options)), ..Default::default() }
}

/// A kittest harness on an app built inside it, with Preferences open on `section`.
fn prefs_open(files: &Arc<MemoryFiles>, section: &'static str) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let files = files.clone();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), services_on(&files));
        open_preferences(&mut app, section);
        assert!(window_open(&app), "Preferences did not open");
        app
    });
    h.run_steps(4);
    h
}

/// The texts the last frame painted, without a trailing colon, with each one's laid-out rect.
fn painted_texts(h: &egui_kittest::Harness<'static, Box<PhotocraftApp>>) -> Vec<(String, egui::Rect)> {
    fn collect(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) => out.push((text.galley.text().trim_end_matches(':').to_owned(), text.galley.rect.translate(text.pos.to_vec2()))),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output().shapes.iter().for_each(|clipped| collect(&clipped.shape, &mut out));
    out
}

/// The rect of the painted text `name`, which has to be painted for the trial to say anything.
fn at(texts: &[(String, egui::Rect)], name: &str) -> egui::Rect {
    texts.iter().find(|(text, _)| text == name).map(|(_, rect)| *rect).unwrap_or_else(|| panic!("{name:?} is painted: {texts:?}"))
}

/// Show `sentence` in the window, and say how many times the frame painted it.
fn count_shown(h: &egui_kittest::Harness<'static, Box<PhotocraftApp>>, sentence: &str) -> usize {
    painted_texts(h).into_iter().filter(|(text, _)| text == sentence).count()
}

// --- the rows the page has today, one measured sentence each -----------------------------------------------------------------

/// The reference page's "Show Crosshair in Brush Tip" tooltip, painted under its own row.
#[test]
fn the_show_crosshair_in_brush_tip_row_shows_its_measured_tooltips_sentence() {
    let (_, files) = app_with_store();
    let h = prefs_open(&files, "cursors");
    let sentence = "Always show crosshair in center of brush cursor";
    assert_eq!(count_shown(&h, sentence), 1, "the row shows the measured sentence once");
    let texts = painted_texts(&h);
    let help = at(&texts, sentence);
    let row = at(&texts, "Show crosshair in brush tip");
    let next = at(&texts, "Show only crosshair while painting");
    assert!(help.top() > row.top() && help.top() < next.top(), "under \"Show crosshair in brush tip\" and above the next row: {row:?} {help:?} {next:?}");
}

/// The reference page's "Show Only Crosshair While Painting" tooltip, painted under its own row.
#[test]
fn the_show_only_crosshair_while_painting_row_shows_its_measured_tooltips_sentence() {
    let (_, files) = app_with_store();
    let h = prefs_open(&files, "cursors");
    let sentence = "Switch to only a crosshair while painting to improve performance with large brushes";
    assert_eq!(count_shown(&h, sentence), 1, "the row shows the measured sentence once");
    let texts = painted_texts(&h);
    let help = at(&texts, sentence);
    let row = at(&texts, "Show only crosshair while painting");
    let next = at(&texts, "Other");
    assert!(
        help.top() > row.top() && help.top() < next.top(),
        "under \"Show only crosshair while painting\" and above the Other row: {row:?} {help:?} {next:?}"
    );
}

/// The reference page's "Brush Preview Color" tooltip, painted under its own row.
#[test]
fn the_brush_preview_color_row_shows_its_measured_tooltips_sentence() {
    let (_, files) = app_with_store();
    let h = prefs_open(&files, "cursors");
    let sentence = "Color to use for brush editing preview";
    assert_eq!(count_shown(&h, sentence), 1, "the row shows the measured sentence once");
    let texts = painted_texts(&h);
    let help = at(&texts, sentence);
    let row = at(&texts, "Brush preview color");
    let above = at(&texts, "Other");
    assert!(help.top() > row.top() && row.top() > above.top(), "under \"Brush preview color\", the page's last row: {above:?} {row:?} {help:?}");
}

// --- what the rows that have no measured sentence of their own keep ---------------------------------------------------------

/// The Painting and Other rules stand for several reference radios each, so no single measured sentence belongs
/// to them, and they keep today's help-less state. The Brush Tip Outline, No Cursor While Painting and Brush
/// Leash settings do not exist yet, so their sentences wait with the items that add those rows.
#[test]
fn the_cursors_rows_without_a_measured_sentence_keep_their_help_as_it_is() {
    let ctx = egui::Context::default();
    let found = |key: &str| rules(&ctx).into_iter().find(|rule| rule.key == key);
    for key in ["cursors.painting", "cursors.other"] {
        let rule = found(key).unwrap_or_else(|| panic!("{key} has a rule"));
        assert_eq!(rule.help, None, "{key}: the measured table splits its sentences per radio, not per row");
    }
    for key in ["cursors.brushTipOutline", "cursors.noCursorWhilePainting", "cursors.showBrushLeash", "cursors.brushLeashColor"] {
        assert!(found(key).is_none(), "{key}: the page has no such setting yet");
    }
}

/// The three sentences, word for word as the issue's measured table gives them, on the rules of the rows that
/// have them; no other page's rule gains one.
#[test]
fn only_the_three_measured_cursors_rules_carry_a_help_line() {
    let ctx = egui::Context::default();
    let rules = rules(&ctx);
    let wanted: [(&str, &str); 3] = [
        ("cursors.showCrosshairInBrushTip", "Always show crosshair in center of brush cursor"),
        ("cursors.showOnlyCrosshairWhilePainting", "Switch to only a crosshair while painting to improve performance with large brushes"),
        ("cursors.brushPreviewColor", "Color to use for brush editing preview"),
    ];
    for (key, sentence) in wanted {
        let rule = rules.iter().find(|rule| rule.key == key).unwrap_or_else(|| panic!("{key} has a rule"));
        assert_eq!(rule.help.as_deref(), Some(sentence), "{key}");
    }
    let with_help: Vec<&str> = rules.iter().filter(|rule| rule.help.is_some()).map(|rule| rule.key.as_str()).collect();
    assert_eq!(
        with_help,
        ["cursors.brushPreviewColor", "cursors.showCrosshairInBrushTip", "cursors.showOnlyCrosshairWhilePainting"],
        "the help lines of the whole window"
    );
}

// --- the other pages, byte for byte ---------------------------------------------------------------------------------------

/// The rule state of every page but Cursors: the fields PhotoCraft itself sets, one line per rule, in key order.
fn other_pages_rule_state(ctx: &egui::Context) -> String {
    let mut lines: Vec<String> = Vec::new();
    for rule in rules(ctx).iter().filter(|rule| !rule.key.starts_with("cursors.")) {
        let kind = match &rule.kind {
            None => "-".to_string(),
            Some(RuleKind::Int { range, unit, .. }) => format!("int {range:?} {unit:?}"),
            Some(RuleKind::Float { range, decimals, unit, .. }) => format!("float {range:?} {decimals} {unit:?}"),
            Some(RuleKind::Choice(options)) => format!("choice {:?}", options.iter().map(|(id, label)| (id, label)).collect::<Vec<_>>()),
            Some(RuleKind::Color) => "color".to_string(),
            Some(RuleKind::Custom) => "custom".to_string(),
            Some(RuleKind::CustomRows) => "customRows".to_string(),
            Some(other) => format!("other {other:?}"),
        };
        lines.push(format!(
            "{}|label={:?}|help={:?}|kind={kind}|when={:?}|unless={:?}|hidden={}|linux={}|restart={}|keep={}|group={:?}",
            rule.key, rule.label, rule.help, rule.shown_when, rule.shown_unless, rule.hidden, rule.linux_only, rule.restart, rule.keep_on_reset, rule.group,
        ));
    }
    lines.join("\n")
}

/// A stable checksum of the rule state (FNV-1a, 64 bit), so a change to any other page's rule turns this red.
fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// #16833 touches the `cursors.*` rules only: every other page's rules come out byte-identical. The pinned
/// length and checksum are that state as `upstream/main` gives it.
#[test]
fn every_other_pages_rules_stay_byte_identical() {
    let ctx = egui::Context::default();
    let state = other_pages_rule_state(&ctx);
    let (lines, bytes) = (state.lines().count(), state.len());
    let (wanted_lines, wanted_bytes, wanted_hash) = (105usize, 18912usize, 10297778473294635895u64);
    assert_eq!((lines, bytes, fnv1a(&state)), (wanted_lines, wanted_bytes, wanted_hash), "{lines} rules of the other pages, {bytes} bytes");
}
