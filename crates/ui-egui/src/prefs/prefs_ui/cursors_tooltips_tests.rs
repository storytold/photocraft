//! Preferences › Cursors: the reference application's tooltips (#16833). Each of the three rows whose reference row
//! is one control carries that row's measured sentence; a trial reads the rule the window is built from and the open
//! window's painted frame, so the sentence is pinned whole and the page's own rows are pinned unchanged. The review
//! asks for the sentence as hover text rather than as a line under the row, which the shared pattern has to carry
//! first - see the PR for the seam and for the trials that hover the control once it does.
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

// --- the rows the page has today, one measured sentence each -----------------------------------------------------------------
//
// The review of this PR asks for these sentences as hover text on the control, with no line painted under the row.
// The window can only paint a rule's help text as a line today, so these trials pin the sentence the row carries and
// that the page's five rows keep their own order; the hover form (pointer on the control, nothing painted in a
// resting frame) replaces them as soon as the shared pattern carries hover text.

/// The row for "Show Crosshair in Brush Tip" carries the measured sentence word for word.
#[test]
fn the_show_crosshair_in_brush_tip_row_carries_its_measured_sentence() {
    assert_rule_says("cursors.showCrosshairInBrushTip", "Always show crosshair in center of brush cursor");
}

/// The row for "Show Only Crosshair While Painting" carries the measured sentence word for word.
#[test]
fn the_show_only_crosshair_while_painting_row_carries_its_measured_sentence() {
    assert_rule_says("cursors.showOnlyCrosshairWhilePainting", "Switch to only a crosshair while painting to improve performance with large brushes");
}

/// The row for "Brush Preview Color" carries the measured sentence word for word.
#[test]
fn the_brush_preview_color_row_carries_its_measured_sentence() {
    assert_rule_says("cursors.brushPreviewColor", "Color to use for brush editing preview");
}

/// The rule of `key` carries `sentence` exactly, and the open window paints it once.
fn assert_rule_says(key: &str, sentence: &str) {
    let ctx = egui::Context::default();
    let rule = rules(&ctx).into_iter().find(|rule| rule.key == key).unwrap_or_else(|| panic!("{key} has a rule"));
    assert_eq!(rule.help.as_deref(), Some(sentence), "{key}: the measured sentence, word for word");
    let (_, files) = app_with_store();
    let h = prefs_open(&files, "cursors");
    let painted = painted_texts(&h).into_iter().filter(|(text, _)| text == sentence).count();
    assert_eq!(painted, 1, "{key}: the sentence shows once in the open window");
}

/// The Cursors page keeps its five rows of today, in the reference application's order, and gains no other text.
#[test]
fn the_cursors_page_keeps_its_five_rows_in_order() {
    let (_, files) = app_with_store();
    let h = prefs_open(&files, "cursors");
    let texts = painted_texts(&h);
    let order = ["Painting", "Show crosshair in brush tip", "Show only crosshair while painting", "Other", "Brush preview color"];
    let tops: Vec<f32> = order.iter().map(|name| at(&texts, name).top()).collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "top to bottom {order:?}: {tops:?}");
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

/// The rules whose state the build target or the host's font stack decides, so they are pinned on their own in
/// `the_target_and_host_dependent_rows_stay_as_they_are` and stay out of the byte-for-byte state: the language
/// picker shows a locale's native name only while the active fonts can draw it (`fields.rs`, `language_label`),
/// WinTab is a Windows-only row, and the title bar only follows its setting where the app draws its own frame.
const TARGET_AND_HOST_DEPENDENT: [&str; 3] = ["interface.language", "interface.systemTitleBar", "tools.tabletInput"];

/// The rule state of every page but Cursors: the fields PhotoCraft itself sets, one line per rule, in key order.
fn other_pages_rule_state(ctx: &egui::Context) -> String {
    let mut lines: Vec<String> = Vec::new();
    for rule in rules(ctx).iter().filter(|rule| !rule.key.starts_with("cursors.") && !TARGET_AND_HOST_DEPENDENT.contains(&rule.key.as_str())) {
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
/// length and checksum are that state as `upstream/main` ed09854be0 gives it; the web build hides its own set
/// of rows, so this state is the native one.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_other_pages_rules_stay_byte_identical() {
    let ctx = egui::Context::default();
    let state = other_pages_rule_state(&ctx);
    let (lines, bytes) = (state.lines().count(), state.len());
    let (wanted_lines, wanted_bytes, wanted_hash) = (102usize, 18452usize, 13006842060618656633u64);
    assert_eq!((lines, bytes, fnv1a(&state)), (wanted_lines, wanted_bytes, wanted_hash), "{lines} rules of the other pages, {bytes} bytes");
}

/// The three rows a build target or the host's fonts decide, kept exactly as they are today.
#[test]
fn the_target_and_host_dependent_rows_stay_as_they_are() {
    let ctx = egui::Context::default();
    let rules = rules(&ctx);
    let find = |key: &str| rules.iter().find(|rule| rule.key == key).unwrap_or_else(|| panic!("{key} has a rule"));

    let language = find("interface.language");
    let options = match &language.kind {
        Some(RuleKind::Choice(options)) => options,
        other => panic!("interface.language offers a choice, got {other:?}"),
    };
    assert_eq!(options.first().map(|(id, label)| (id.as_str(), label.as_str())), Some(("auto", "Auto")), "the picker still starts with Auto");
    assert!(options.iter().any(|(id, label)| id == "en" && label == "English"), "English names itself: {options:?}");

    let tablet = find("tools.tabletInput");
    if cfg!(target_os = "windows") {
        assert_eq!(tablet.kind, Some(RuleKind::CustomRows), "WinTab draws its own row on Windows");
        assert!(!tablet.hidden && tablet.restart, "and its context opens at launch there");
    } else {
        assert!(tablet.hidden, "WinTab is Windows-only");
        assert!(!tablet.restart, "so nothing opens its context elsewhere");
    }

    let title_bar = find("interface.systemTitleBar");
    let draws_its_own_frame = cfg!(not(target_arch = "wasm32")) && cfg!(not(target_os = "macos"));
    assert_eq!(title_bar.restart, draws_its_own_frame, "the title bar applies at next launch where the app draws the frame");
    assert!(title_bar.help.is_none() && title_bar.label.is_none(), "and it gains no wording here");
}
