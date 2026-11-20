use super::*;
use crate::PhotocraftApp;
use craft::imaging::ui::scopes::ScopePanelState;
use egui::{Rect, vec2};
use serde_json::json;

fn app_with(params: Value) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", params).unwrap();
    app
}

/// One panel frame through a minimal egui pass.
// PLAN: `show` now builds the framework's `ScopePanel`, which reads `Env::of(ui)`; this bare context has no host installed. If it panics, the frame needs `Fixture::host`.
fn frame(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    ctx.begin_pass(Default::default());
    let mut ui =
        egui::Ui::new(ctx.clone(), egui::Id::unique("hist-test"), egui::UiBuilder::new().max_rect(Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 900.0))));
    show(app, &mut ui);
    ctx.end_pass().textures_delta.clear();
}

/// Frames until the panel shows the histogram it wants (the throttle skipped), or panics after
/// 60 s: an uncached count of the 18 MP document below takes seconds in a debug build on a loaded
/// CI runner, and the loop returns as soon as the histogram lands.
fn settle(app: &mut PhotocraftApp) {
    let t0 = std::time::Instant::now();
    loop {
        for e in [app.doc_hist.as_mut(), app.doc_hist_preview.as_mut()].into_iter().flatten() {
            e.at = -1e12;
        }
        frame(app);
        let done = app.hist_job.is_none() && shown(app).is_some_and(|s| !s.stale);
        if done {
            return;
        }
        assert!(t0.elapsed() < std::time::Duration::from_secs(60), "the histogram never settled");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn ui_set(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Value {
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", params);
    match crate::control::handle(app, ctx, &req) {
        crate::control::Reply::Done(v) => v,
        _ => panic!("ui.set: expected an immediate reply"),
    }
}

fn inspect_stats(app: &PhotocraftApp) -> Value {
    inspect(app)["stats"].clone()
}

#[test]
fn channels_follow_the_colour_mode() {
    let ids = |m| channel_options(m).into_iter().map(|(c, _)| c).collect::<Vec<_>>();
    use HistChannel::*;
    assert_eq!(ids(ColorMode::Rgb), [Composite, Red, Green, Blue, Luminosity, Colors]);
    assert_eq!(ids(ColorMode::Cmyk), [Composite, Cyan, Magenta, Yellow, Black, Luminosity, Colors]);
    assert_eq!(ids(ColorMode::Lab), [Composite, A, B, Luminosity]);
    assert_eq!(ids(ColorMode::Grayscale), [Composite]);
    assert_eq!(resolve_channel(ColorMode::Cmyk, Red), Composite, "a channel the mode lacks shows its composite");
    assert_eq!(resolve_channel(ColorMode::Rgb, Red), Red);
    assert!(color_channels(ColorMode::Grayscale).is_empty(), "All Channels lists colour channels only");
    assert_eq!(color_channels(ColorMode::Rgb), [Red, Green, Blue]);
    // Pointer marks bin pixels the way the histogram does.
    let px = [1.0, 0.5, 0.0, 1.0];
    assert_eq!(channel_level(ColorMode::Rgb, Red, px), 255);
    assert_eq!(channel_level(ColorMode::Rgb, Green, px), 128);
    assert_eq!(channel_level(ColorMode::Rgb, Composite, px), 127);
    assert_eq!(channel_level(ColorMode::Rgb, Luminosity, px), ((0.30 + 0.59 * 0.5) * 255.0f32).round() as u8);
    assert_eq!(channel_level(ColorMode::Rgb, Blue, [f32::NAN, 0.0, 2.0, 1.0]), 255, "hostile samples clamp, never panic");
}

#[test]
fn luminosity_and_rgb_channels_are_counted() {
    let mut app = app_with(json!({"width": 40, "height": 20}));
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    // The whole input: the selection left over the fill would mask the panel's count.
    let h = tone::compute_histograms(doc, tone::IMAGE, ToneSpace::Rgb, HistScope::WholeInput);
    assert_eq!(bins(&h, doc.mode, HistChannel::Red)[255], 800);
    assert_eq!(bins(&h, doc.mode, HistChannel::Green)[0], 400);
    let red_lum = (0.30f32 * 255.0).round() as usize;
    assert_eq!(h.lum[red_lum], 400, "red is 30% luminosity");
    assert_eq!(h.lum[255], 400, "white is full luminosity");
}

#[test]
fn sources_count_the_image_the_layer_or_the_adjustment_composite() {
    let mut app = app_with(json!({"width": 32, "height": 32})); // white
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 32})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let st = app.session.active().unwrap();
    let invert = st.doc.layers[1].id;
    let doc = &st.doc;
    // The whole input: the selection left over the fill would mask the panel's count.
    let image = tone::compute_histograms(doc, tone::IMAGE, ToneSpace::Rgb, HistScope::WholeInput);
    assert_eq!((image[0][0], image[0][128]), (512, 512), "black (inverted white) and the grey above it");
    let upto = tone::compute_histograms(doc, HistSource::UpTo(invert), ToneSpace::Rgb, HistScope::WholeInput);
    assert_eq!(upto[0][0], 1024, "the adjustment composite hides the grey layer above");
    // The panel offers Adjustment Composite only with an adjustment layer selected.
    assert_eq!(resolve_source(st, HistSourceChoice::AdjustmentComposite).0, HistSourceChoice::EntireImage);
    assert_eq!(resolve_source(st, HistSourceChoice::SelectedLayer).1, HistSource::Layer(st.active_layer.unwrap()));
    app.run("layer.select", json!({"layer": invert.0})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(invert));
    assert_eq!(resolve_source(st, HistSourceChoice::AdjustmentComposite), (HistSourceChoice::AdjustmentComposite, HistSource::UpTo(invert)));
    // A single-layer document counts the entire image whatever is chosen.
    let one = app_with(json!({"width": 8, "height": 8}));
    let st = one.session.active().unwrap();
    assert_eq!(resolve_source(st, HistSourceChoice::SelectedLayer).0, HistSourceChoice::EntireImage);
}

#[test]
fn cache_level_follows_size_and_the_preference_cap() {
    let big = app_with(json!({"width": 6000, "height": 3000}));
    let doc = &big.session.active().unwrap().doc;
    let auto = tone::cache_level(doc, tone::MAX_CACHE_LEVEL);
    assert!(auto > 1, "a large document counts a proxy, level {auto}");
    assert_eq!(tone::cache_level(doc, 2), 2, "capped by Cache Levels");
    assert_eq!(tone::cache_level(doc, 0), 1, "out-of-range caps clamp");
    let exact = tone::compute_histograms_at(doc, tone::IMAGE, ToneSpace::Rgb, HistScope::Selection, 1);
    assert_eq!(exact.level, 1);
    assert_eq!(exact[0][255], 6000 * 3000, "level 1 counts every pixel");
    let small = app_with(json!({"width": 64, "height": 64}));
    assert_eq!(tone::cache_level(&small.session.active().unwrap().doc, 8), 1, "small documents are exact");
}

#[test]
fn panel_counts_off_thread_and_recounts_after_an_edit() {
    let mut app = app_with(json!({"width": 64, "height": 48}));
    settle(&mut app);
    let s = inspect_stats(&app);
    assert_eq!(s["pixels"], 64 * 48);
    assert_eq!(s["median"], 255);
    assert_eq!(inspect(&app)["current"], true);
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    assert_eq!(inspect(&app)["current"], false, "stale until recounted");
    frame(&mut app);
    // The old histogram stays shown, marked stale, while the new one counts.
    assert!(shown(&mut app).is_some());
    settle(&mut app);
    assert_eq!(inspect_stats(&app)["median"], 0, "refreshed without a manual refresh");
}

#[test]
fn a_dead_histogram_worker_clears_the_job() {
    let mut app = app_with(json!({"width": 64, "height": 48}));
    let st = app.session.active().unwrap();
    let key = HistKey { doc: st.doc.id, revision: st.revision, source: 0, preview: 0, max_level: 4 };
    let (tx, rx) = std::sync::mpsc::channel();
    drop(tx);
    app.hist_job = Some((key, rx));
    let ctx = app.ctx().clone();
    update(&mut app, &ctx);
    assert!(app.doc_hist.is_none(), "nothing landed from the dead worker");
    // …and the next count was scheduled in its place.
    assert!(app.hist_job.is_some());
}

#[test]
fn uncached_refresh_counts_every_pixel_for_the_current_state() {
    let mut app = app_with(json!({"width": 6000, "height": 3000}));
    app.run("prefs.set", json!({"path": "performance.cacheLevels", "value": 8})).unwrap();
    settle(&mut app);
    assert!(inspect(&app)["cacheLevel"].as_u64().unwrap() > 1, "cached first");
    uncached_refresh(&mut app);
    settle(&mut app);
    assert_eq!(inspect(&app)["cacheLevel"], 1);
    assert_eq!(inspect_stats(&app)["pixels"], 6000u64 * 3000);
    // Cache Levels caps what the panel counts at.
    app.run("prefs.set", json!({"path": "performance.cacheLevels", "value": 2})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    settle(&mut app);
    assert_eq!(inspect(&app)["cacheLevel"], 2);
    assert_eq!(inspect(&app)["maxCacheLevel"], 2);
}

#[test]
fn adjustment_preview_is_counted_over_the_original() {
    let mut app = app_with(json!({"width": 32, "height": 32})); // white
    app.run("layer.newAdjustmentLayer.levels", json!({})).unwrap();
    settle(&mut app);
    assert_eq!(inspect_stats(&app)["median"], 255);
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.live_adjust = Some((id, json!({"outWhite": 100})));
    settle(&mut app);
    assert_eq!(inspect(&app)["preview"], true);
    assert_eq!(inspect_stats(&app)["median"], 100, "the previewed levels");
    let s = shown(&mut app).unwrap();
    let original = s.original.expect("the original is kept for the overlay");
    assert_eq!(original[0][255], 1024, "the original is the image before the preview");
    app.live_adjust = None;
    settle(&mut app);
    assert_eq!(inspect(&app)["preview"], false);
    assert_eq!(inspect_stats(&app)["median"], 255);
}

#[test]
fn views_keep_the_channel_and_the_menu_toggles_options() {
    let mut app = app_with(json!({"width": 16, "height": 16}));
    app.ui.histogram.settings.channel = HistChannel::Green.key().into();
    let ctx = app.ctx().clone();
    let fire = |app: &mut PhotocraftApp, command: ScopeCommand| assert!(run_fired(app, &ctx, &CommandRef::from(command)));
    fire(&mut app, ScopeCommand::AllChannelsView);
    assert_eq!(app.ui.histogram.settings.view, ScopeView::AllChannels);
    assert_eq!(app.ui.histogram.settings.channel, "green", "the channel is kept");
    fire(&mut app, ScopeCommand::ShowChannelsInColor);
    assert!(app.ui.histogram.settings.show_channels_in_color);
    fire(&mut app, ScopeCommand::ShowStatistics);
    assert!(!app.ui.histogram.settings.show_statistics);
    // The states mirror the settings: the radio mark follows the view.
    let state = |app: &PhotocraftApp, command: ScopeCommand| command_states(app).into_iter().find(|(c, _)| *c == CommandRef::from(command)).map(|(_, s)| s);
    assert_eq!(state(&app, ScopeCommand::AllChannelsView), Some(CommandState::new().checked(true)));
    assert_eq!(state(&app, ScopeCommand::CompactView), Some(CommandState::new().checked(false)));
    // Kept with the preferences.
    let mut again = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let saved = app.session.prefs().dialogs.get(OPTIONS_PREF).cloned().unwrap();
    again.session.prefs.edit(|p| {
        p.dialogs.insert(OPTIONS_PREF.into(), saved);
    });
    restore_options(&mut again);
    assert_eq!(again.ui.histogram.settings.view, ScopeView::AllChannels);
    assert!(again.ui.histogram.settings.show_channels_in_color);
    assert!(!again.ui.histogram.settings.show_statistics);
}

#[test]
fn ui_set_drives_the_panel() {
    let mut app = app_with(json!({"width": 16, "height": 16}));
    let ctx = app.ctx().clone();
    let r = ui_set(
        &mut app,
        &ctx,
        json!({"histogramPanel": {"view": "allChannels", "channel": "luminosity"}, "histogramRange": [10, 20], "histogramRefresh": true}),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.ui.histogram.settings.view, ScopeView::AllChannels);
    assert_eq!(app.ui.histogram.settings.channel, "luminosity");
    assert_eq!(range_levels(app.ui.histogram.range), Some([10, 20]));
    assert!(app.hist_uncached.is_some());
    for bad in [
        json!({"histogramPanel": {"view": "tiny"}}),
        json!({"histogramPanel": {"nope": 1}}),
        json!({"histogramRange": [0, 300]}),
        json!({"histogramRefresh": "yes"}),
    ] {
        let r = ui_set(&mut app, &ctx, bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
    }
    assert_eq!(app.ui.histogram.settings.view, ScopeView::AllChannels, "a rejected call changes nothing");
    assert_eq!(app.ui.histogram.settings.channel, "luminosity", "a rejected call changes nothing");
    assert_eq!(range_levels(app.ui.histogram.range), Some([10, 20]), "a rejected call changes nothing");
}

/// A white Background under a black layer, with the Background selected: the entire image is
/// black, the selected layer white.
fn black_over_white_background_selected() -> Box<PhotocraftApp> {
    let mut app = app_with(json!({"width": 32, "height": 32}));
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    let background = app.session.active().unwrap().doc.layers[0].id;
    app.run("layer.select", json!({"layer": background.0})).unwrap();
    app
}

/// Preferences › Cache Levels 1 counts at level 1 like an Uncached Refresh, but only the refresh
/// was asked for: edits in a row still wait out the 250 ms throttle.
#[test]
fn cache_levels_one_still_throttles_recounts_and_an_uncached_refresh_does_not() {
    let mut app = app_with(json!({"width": 64, "height": 48}));
    app.run("prefs.set", json!({"path": "performance.cacheLevels", "value": 1})).unwrap();
    settle(&mut app);
    // The count just landed, so the next edit falls inside the throttle. Each pass without an input
    // time moves the context's clock on by one frame, so "just now" is the context's time, not 0.
    let now = app.ctx().input(|i| i.time) * 1000.0;
    app.doc_hist.as_mut().unwrap().at = now;
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    frame(&mut app);
    assert!(app.hist_job.is_none(), "an edit within 250 ms of the last count waits");
    uncached_refresh(&mut app);
    frame(&mut app);
    assert!(app.hist_job.is_some(), "an Uncached Refresh counts at once");
}

/// The Levels, Curves and Threshold dialogs count uncapped: their proxy is as small as before
/// Cache Levels (about 768 pixels on the longest side), and only a cap halves by powers of two.
#[test]
fn uncapped_counts_keep_their_proxy_and_a_cap_reduces_by_powers_of_two() {
    let big = app_with(json!({"width": 6000, "height": 3000}));
    let doc = &big.session.active().unwrap().doc;
    let pixels = |h: &Histograms| h[0].iter().map(|&c| u64::from(c)).sum::<u64>();
    // 6000 / 768 = 7: a 7x proxy, 858 x 429 (rounded up), not an 8x one.
    let auto = tone::compute_histograms(doc, tone::IMAGE, ToneSpace::Rgb, HistScope::Selection);
    assert_eq!(pixels(&auto), 858 * 429);
    let capped = tone::compute_histograms_at(doc, tone::IMAGE, ToneSpace::Rgb, HistScope::Selection, 3);
    assert_eq!(capped.level, 3);
    assert_eq!(pixels(&capped), 1500 * 750, "level 3 is a 4x proxy");
}

/// Photoshop opens the panel in Compact View, which shows no Source menu and represents the
/// entire image whatever Source the Expanded View was left at.
#[test]
fn compact_view_is_the_default_and_counts_the_entire_image() {
    assert_eq!(ScopePanelState::default().settings.view, ScopeView::Compact);
    let mut app = black_over_white_background_selected();
    app.ui.histogram.settings.source = HistSourceChoice::SelectedLayer.key().into();
    settle(&mut app);
    let i = inspect(&app);
    assert_eq!(i["source"], "entireImage");
    assert_eq!(i["stats"]["median"], 0, "the black layer over the Background, not the Background alone");
    app.ui.histogram.settings.view = ScopeView::Expanded;
    settle(&mut app);
    assert_eq!(inspect(&app)["source"], "selectedLayer");
    assert_eq!(inspect_stats(&app)["median"], 255, "Expanded View counts the chosen Source");
}

/// `current` says whether the shown histogram is the one the panel wants: a Source change makes
/// it stale until the recount lands, though the document's revision didn't move.
#[test]
fn inspect_current_follows_the_source_not_only_the_revision() {
    let mut app = black_over_white_background_selected();
    app.ui.histogram.settings.view = ScopeView::Expanded;
    settle(&mut app);
    assert_eq!(inspect(&app)["current"], true);
    let ctx = app.ctx().clone();
    let r = ui_set(&mut app, &ctx, json!({"histogramPanel": {"source": "selectedLayer"}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(inspect(&app)["current"], false, "the entire image's histogram is not the selected layer's");
    settle(&mut app);
    assert_eq!(inspect(&app)["current"], true);
    assert_eq!(inspect_stats(&app)["median"], 255);
}

/// `preview` reports only the active document's preview, not one left from another document.
#[test]
fn inspect_preview_is_the_active_documents() {
    let mut app = app_with(json!({"width": 16, "height": 16}));
    settle(&mut app);
    let mut other = app.doc_hist.clone().unwrap();
    other.key.preview = 1;
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert_ne!(app.session.active().unwrap().doc.id, other.key.doc);
    app.doc_hist_preview = Some(other);
    assert_eq!(inspect(&app)["preview"], false);
}

/// A selection changes no pixels, but the panel counts different ones: it must recompute for
/// the selection instead of riding the #125 view-only fast path, and its Pixels statistic
/// reports the selection's size. A later view-only edit without a selection change keeps the
/// selection's histogram.
#[test]
fn the_panel_follows_the_selection() {
    let mut app = app_with(json!({"width": 640, "height": 480}));
    settle(&mut app);
    let whole = app.doc_hist.clone().expect("landed");
    assert_eq!(inspect_stats(&app)["pixels"], 640 * 480);
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 10})).unwrap();
    assert_eq!(inspect(&app)["current"], false, "the selection changed what the panel counts");
    settle(&mut app);
    let selected = app.doc_hist.clone().expect("landed");
    assert_eq!(selected.key.revision, app.session.active().unwrap().revision, "fresh for the selection revision");
    assert_eq!(inspect_stats(&app)["pixels"], 200, "Pixels counts the selection");
    assert_ne!(selected.key.source, whole.key.source, "the selection changed the histogram's identity");
    // A later view-only edit (no selection change) keeps the selection histogram.
    app.run("layer.new.layer", json!({})).unwrap();
    settle(&mut app);
    let after = app.doc_hist.clone().expect("landed");
    assert_eq!(after.key.source, selected.key.source, "same selection, same histogram identity");
    assert_eq!(inspect_stats(&app)["pixels"], 200);
}

/// A worker result computed before a selection change is dropped at the door: it must not be
/// cached for the selection now on screen (which would reset the 250 ms throttle and delay the
/// recomputation).
#[test]
fn a_result_computed_before_a_selection_change_is_not_cached() {
    let mut app = app_with(json!({"width": 64, "height": 64}));
    settle(&mut app);
    let before = app.doc_hist.clone().expect("landed");
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    // A worker still holding the pre-selection result lands: the panel must reject it.
    let (tx, rx) = std::sync::mpsc::channel();
    let stale = Arc::new((*before.hist).clone());
    let _ = tx.send((before.key, 1.0, stale.clone()));
    app.hist_job = Some((before.key, rx));
    let ctx = app.ctx().clone();
    update(&mut app, &ctx);
    assert!(app.hist_job.is_none(), "the stale job was consumed, not kept");
    let after = app.doc_hist.clone().expect("the previous entry stands");
    assert_eq!(after.key.revision, before.key.revision, "the stale result was not cached over it");
    assert!(!Arc::ptr_eq(&after.hist, &stale), "the stale result was not accepted");
    assert!(Arc::ptr_eq(&after.hist, &before.hist), "the previous histogram still stands");
}

/// A result computed for the revision just before a pixel edit still lands: the panel shows the
/// newer data, marked stale, instead of dropping it and lagging behind continuous edits. Only
/// another document or selection is dropped at the door.
#[test]
fn a_result_one_revision_behind_still_lands() {
    let mut app = app_with(json!({"width": 64, "height": 64}));
    settle(&mut app);
    let before = app.doc_hist.clone().expect("landed");
    app.run("edit.fill", json!({"color": "#000000"})).unwrap(); // a newer revision exists
    let (tx, rx) = std::sync::mpsc::channel();
    let newer = Arc::new((*before.hist).clone());
    let _ = tx.send((before.key, 1.0, newer.clone()));
    app.hist_job = Some((before.key, rx));
    let ctx = app.ctx().clone();
    update(&mut app, &ctx);
    assert!(app.hist_job.is_none(), "the job was consumed");
    let landed = app.doc_hist.clone().expect("the one-revision-old result lands");
    assert_eq!(landed.key.revision, before.key.revision, "landed as computed");
    assert!(Arc::ptr_eq(&landed.hist, &newer));
    assert_eq!(inspect(&app)["current"], false, "still marked stale for the newer revision");
}

/// The panel's Pixels statistic counts the pixels the selection covers, transparent ones
/// included, while the histogram bins keep skipping them: a selection over a transparent canvas
/// reports the selection's size.
#[test]
fn the_panel_reports_the_selection_size_over_transparent_pixels() {
    let mut app = app_with(json!({"width": 32, "height": 32, "background": "transparent"}));
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 32})).unwrap();
    settle(&mut app);
    assert_eq!(inspect_stats(&app)["pixels"], 16 * 32, "the transparent selected pixels still count");
    app.ui.histogram.settings.view = ScopeView::Expanded;
    frame(&mut app);
}

/// The tone editors' histogram shows the adjustment's whole input (Photoshop's dialogs are not
/// selection-masked), so a selection change — view-only, no pixels moved — keeps the cached
/// histogram (#125). The panel's masked histogram is covered above.
#[test]
fn the_tone_histogram_cache_ignores_selection_changes() {
    let mut app = app_with(json!({"width": 64, "height": 64}));
    let src = tone::IMAGE;
    let first = tone::histograms(&mut app, src, ToneSpace::Rgb);
    assert!(Arc::ptr_eq(&first, &tone::histograms(&mut app, src, ToneSpace::Rgb)), "unchanged state reuses the cache");
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    assert!(Arc::ptr_eq(&first, &tone::histograms(&mut app, src, ToneSpace::Rgb)), "a selection change is view-only for the whole-input histogram");
    app.run("select.deselect", json!({})).unwrap();
    assert!(Arc::ptr_eq(&first, &tone::histograms(&mut app, src, ToneSpace::Rgb)), "deselecting keeps it too");
}
