//! The Curves clipping display (#14383): which channels the black or white end point clips, each in
//! its own colour on the composite curve and one channel on its own curve, from a Curves dialog and
//! from a Curves layer's Properties panel; and that the display ends.

use super::*;
use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point;
use craft::ui::dialog::DialogId;
use photocraft_color::{Color, PixelFormat};
use photocraft_compose::Buffer;
use photocraft_compose::adjust::{Transfer, apply_depth};
use photocraft_doc::{Document, Size};
use serde_json::{Value, json};

use crate::adjust_dialog::HostedAdjustDialog as Hosted;
use crate::adjust_editors::PropertiesAdjust;
use crate::adjust_editors::values::{self, ValueCx};

fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn pt(input: f32, output: f32) -> CurvePoint {
    CurvePoint { input: input / 255.0, output: output / 255.0 }
}

/// A straight curve from (`black`, 0) to (`white`, 255), in levels.
fn ends_at(black: f32, white: f32) -> Vec<CurvePoint> {
    vec![pt(black, 0.0), pt(white, 255.0)]
}

fn curves(master: Vec<CurvePoint>, per_channel: [Vec<CurvePoint>; 3]) -> Adjustment {
    Adjustment::Curves { master, per_channel, space: ToneSpace::Rgb, black: Vec::new(), pencil: false }
}

fn identity() -> Vec<CurvePoint> {
    ends_at(0.0, 255.0)
}

/// Four pixels in a row, in levels: red alone, green and blue, all three, none past an end at 40
/// (black) or 200 (white).
const BLACK_ROW: [[f32; 3]; 4] = [[20.0, 120.0, 180.0], [150.0, 10.0, 30.0], [5.0, 12.0, 35.0], [90.0, 130.0, 160.0]];
const WHITE_ROW: [[f32; 3]; 4] = [[230.0, 120.0, 60.0], [100.0, 210.0, 250.0], [240.0, 220.0, 205.0], [90.0, 130.0, 160.0]];

fn row(px: &[[f32; 3]; 4]) -> Surface {
    let mut s = Surface::new(PixelFormat::RGBA8);
    let data: Vec<f32> = px.iter().flat_map(|p| [p[0] / 255.0, p[1] / 255.0, p[2] / 255.0, 1.0]).collect();
    s.write_region(Rect::new(0, 0, 4, 1), &data);
    s
}

/// The display of `adj` over `px` for `end`, with the curve of `only` shown.
fn shown(adj: &Adjustment, px: &[[f32; 3]; 4], only: Option<usize>, end: LevelsClip) -> Vec<Color32> {
    let thr = thresholds_of(adj, SampleType::U8, only).expect("an RGB Curves has thresholds");
    image_in(&scan(&row(px), None, Rect::new(0, 0, 4, 1), 1), &thr, end, Palette::Curves)
}

/// On the composite curve each clipped channel shows in its colour and two mix, on white for the
/// black end and on black for the white end; all three clipped is pure black or white.
#[test]
fn the_composite_curve_shows_each_channel() {
    let adj = curves(ends_at(40.0, 200.0), [identity(), identity(), identity()]);
    assert_eq!(
        shown(&adj, &BLACK_ROW, None, LevelsClip::Shadows),
        vec![rgb(255, 0, 0), rgb(0, 255, 255), Color32::BLACK, Color32::WHITE],
        "red alone shows red, green and blue show cyan, all three black"
    );
    assert_eq!(
        shown(&adj, &WHITE_ROW, None, LevelsClip::Highlights),
        vec![rgb(255, 0, 0), rgb(0, 255, 255), Color32::WHITE, Color32::BLACK],
        "red alone shows red, green and blue show cyan, all three white"
    );
}

/// On the Red, Green or Blue curve only that channel's clipping shows, in that channel's colour.
#[test]
fn a_channel_curve_shows_that_channel_only() {
    let mut per_channel = [identity(), identity(), identity()];
    per_channel[0] = ends_at(40.0, 200.0);
    let red = curves(identity(), per_channel);
    assert_eq!(shown(&red, &BLACK_ROW, Some(0), LevelsClip::Shadows), vec![rgb(255, 0, 0), Color32::WHITE, rgb(255, 0, 0), Color32::WHITE]);
    assert_eq!(shown(&red, &WHITE_ROW, Some(0), LevelsClip::Highlights), vec![rgb(255, 0, 0), Color32::BLACK, rgb(255, 0, 0), Color32::BLACK]);
    let mut per_channel = [identity(), identity(), identity()];
    per_channel[2] = ends_at(40.0, 200.0);
    let blue = curves(identity(), per_channel);
    // Blue clips at 35 in the third pixel only; green's 10 and 12 don't show on the Blue curve.
    assert_eq!(shown(&blue, &BLACK_ROW, Some(2), LevelsClip::Shadows), vec![Color32::WHITE, rgb(0, 0, 255), rgb(0, 0, 255), Color32::WHITE]);
}

/// With the end points' outputs at 0 and 255, a channel is clipped exactly where the compositor's
/// Curves result sits at 0 or 1, through a channel's curve and the composite curve after it.
#[test]
fn clipping_matches_the_curves_result() {
    let master = ends_at(30.0, 210.0);
    let channels = [ends_at(10.0, 240.0), identity(), ends_at(60.0, 180.0)];
    let adj = curves(master.clone(), channels.clone());
    for n in 0..=255u8 {
        let v = f32::from(n) / 255.0;
        let mut b = Buffer::filled(Rect::new(0, 0, 1, 1), [v, v, v, 1.0]);
        apply_depth(&adj, &mut b, Transfer::for_document(ColorMode::Rgb, SampleType::U8), Some(SampleType::U8));
        for (c, channel) in channels.iter().enumerate() {
            let result = b.px[0][c];
            let (black, white) = classify_curves(&master, channel, v);
            assert_eq!(black, result <= 0.5 / 255.0, "level {n} channel {c}: {result}");
            assert_eq!(white, result >= 1.0 - 0.5 / 255.0, "level {n} channel {c}: {result}");
        }
    }
    let thr = thresholds(&adj, SampleType::U8).unwrap();
    for k in (0..=255u16).map(|n| n * 257) {
        for (c, (channel, (lo, hi))) in channels.iter().zip(thr).enumerate() {
            let (black, white) = classify_curves(&master, channel, f32::from(k) / 65535.0);
            assert_eq!(black, lo.is_some_and(|lo| k <= lo), "key {k} channel {c} black");
            assert_eq!(white, hi.is_some_and(|hi| k >= hi), "key {k} channel {c} white");
        }
    }
}

/// A pencil-drawn curve has no end points to drag, and CMYK has no display; hostile curves never panic.
#[test]
fn pencil_curves_cmyk_and_hostile_curves() {
    let pencil = Adjustment::Curves { master: identity(), per_channel: Default::default(), space: ToneSpace::Rgb, black: Vec::new(), pencil: true };
    assert!(thresholds_of(&pencil, SampleType::U8, None).is_none());
    let cmyk = Adjustment::Curves { master: identity(), per_channel: Default::default(), space: ToneSpace::Cmyk, black: Vec::new(), pencil: false };
    assert!(thresholds_of(&cmyk, SampleType::U8, None).is_none());
    let nan = CurvePoint { input: f32::NAN, output: f32::INFINITY };
    let hostile = curves(vec![nan.clone(), pt(300.0, -20.0)], [vec![pt(255.0, 0.0), pt(0.0, 255.0)], vec![nan], Vec::new()]);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for only in [None, Some(0), Some(2), Some(9)] {
            let _ = thresholds_of(&hostile, depth, only);
        }
    }
}

/// A 4×1 document holding `px` (levels) on its background.
fn row_app(px: &[[f32; 3]; 4]) -> Box<PhotocraftApp> {
    let mut doc = Document::with_background("t", Size::new(4, 1), ColorMode::Rgb, SampleType::U8, Color::rgba(1.0, 1.0, 1.0, 1.0));
    if let Some(s) = doc.layers[0].surface_mut() {
        let data: Vec<f32> = px.iter().flat_map(|p| [p[0] / 255.0, p[1] / 255.0, p[2] / 255.0, 1.0]).collect();
        s.write_region(Rect::new(0, 0, 4, 1), &data);
    }
    let target = doc.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, None);
    session.select_layer(target).unwrap();
    PhotocraftApp::new(session, crate::Services::default())
}

/// The 4×1 document's view at 10 points a pixel.
fn view() -> CanvasView {
    CanvasView::new(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(40.0, 10.0)), Point::new(2.0, 0.5), 10.0)
}

/// The colours of the display's blocks after one paint of [`view`], or None when nothing shows.
fn painted(app: &mut PhotocraftApp, ctx: &egui::Context) -> Option<Vec<Color32>> {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let painter = egui::Painter::new(ui.ctx().clone(), egui::LayerId::background(), egui::Rect::EVERYTHING);
        paint(app, ui.ctx(), &painter, &view(), 0);
    });
    out.textures_delta.clear();
    let c = app.levels_clipping.as_ref()?;
    let (end, thr, palette) = c.drawn?;
    request(app, 0)?;
    Some(image_in(&c.stats, &thr, end, palette))
}

fn hosted(app: &mut PhotocraftApp, id: DialogId) -> &mut Hosted {
    app.framework.dialogs.get_mut_by_id::<Hosted>(id).expect("the hosted Curves dialog is open")
}

/// Opens Image › Adjustments › Curves with `params` written over its curves.
fn open_curves(app: &mut PhotocraftApp, params: Value) -> DialogId {
    let id = crate::adjust_dialog::open(app, "image.adjustments.curves").unwrap();
    let cx = ValueCx::new(ColorMode::Rgb, Value::Null);
    let mut merged = values::to_params(&hosted(app, id).value().value, &cx, true);
    if let (Some(merged), Value::Object(extra)) = (merged.as_object_mut(), params) {
        merged.extend(extra);
    }
    hosted(app, id).value_mut().value = values::from_params("curves", &merged, &cx).unwrap();
    id
}

/// The Curves dialog shows the target layer's clipping for the curve its Channel select shows, and
/// shows nothing once its display is off.
#[test]
fn the_curves_dialog_shows_the_shown_curves_clipping() {
    let mut app = row_app(&BLACK_ROW);
    let ctx = app.ctx().clone();
    let id = open_curves(&mut app, json!({"points": [[40, 0], [200, 255]], "blue": [[40, 0], [255, 255]]}));
    assert_eq!(painted(&mut app, &ctx), None, "no display before Alt");
    hosted(&mut app, id).set_clipping(Some(LevelsClip::Shadows));
    assert_eq!(painted(&mut app, &ctx), Some(vec![rgb(255, 0, 0), rgb(0, 255, 255), Color32::BLACK, Color32::WHITE]), "composite");
    app.framework.dialogs.set_json_field(&ctx, id, "channel", json!(3)).unwrap();
    assert_eq!(hosted(&mut app, id).channel(), 3);
    // Blue's curve clips at 40, and the composite after it at 40 again: 30 and 35 clip, 180 and 160 don't.
    assert_eq!(painted(&mut app, &ctx), Some(vec![Color32::WHITE, rgb(0, 0, 255), rgb(0, 0, 255), Color32::WHITE]), "Blue");
    hosted(&mut app, id).set_clipping(None);
    assert_eq!(painted(&mut app, &ctx), None, "off with the display");
    app.framework.dialogs.cancel_by_id(&ctx, id);
    let _ = painted(&mut app, &ctx);
    assert!(app.levels_clipping.is_none(), "the cache goes with the dialog");
}

/// A Curves layer's Properties panel shows what the layer's input clips: the composite under it,
/// the layer itself and everything above left out. The display needs the panel drawn this pass or
/// the one before, and ends with its record's display.
#[test]
fn the_properties_panel_shows_the_composite_below() {
    let mut app = row_app(&BLACK_ROW);
    let ctx = app.ctx().clone();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    let layer = app.session.active().and_then(|s| s.active_layer).unwrap();
    app.run("layer.setAdjustment", json!({"layer": layer.0, "points": [[40, 0], [200, 255]]})).unwrap();
    // A layer above the Curves layer that would hide every clipped pixel if it counted.
    app.run("layer.newFillLayer.solidColor", json!({"color": "#808080"})).unwrap();
    app.session.select_layer(layer).unwrap();
    let mut record = PropertiesAdjust::new(layer);
    record.clipping = Some(LevelsClip::Shadows);
    app.adjust_session = Some(record);
    // What the panel does in each pass it draws.
    let drawn = |app: &mut PhotocraftApp, channel: usize| {
        let pass = app.ctx().cumulative_pass_nr();
        if let Some(record) = app.adjust_session.as_mut() {
            record.shown = pass;
            record.channel = channel;
        }
    };
    drawn(&mut app, 0);
    assert_eq!(painted(&mut app, &ctx), Some(vec![rgb(255, 0, 0), rgb(0, 255, 255), Color32::BLACK, Color32::WHITE]));
    assert_eq!(app.levels_clipping.as_ref().map(|c| c.key.3), Some(Source::Below));
    drawn(&mut app, 1);
    // The Red curve is straight, and the composite curve after it clips red at 40: red alone shows.
    assert_eq!(painted(&mut app, &ctx), Some(vec![rgb(255, 0, 0), Color32::WHITE, rgb(255, 0, 0), Color32::WHITE]), "Red");
    // The panel stopped drawing (closed mid-drag): the display goes after a pass.
    let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
    let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
    assert_eq!(painted(&mut app, &ctx), None, "a panel that no longer draws shows nothing");
    drawn(&mut app, 0);
    if let Some(record) = app.adjust_session.as_mut() {
        record.clipping = None;
    }
    assert_eq!(painted(&mut app, &ctx), None, "off with the record's display");
    assert!(app.levels_clipping.is_none(), "nothing asks for the display: the cache goes");
}

/// A Properties panel undocked into a window of its own draws its editor in that window's passes, which egui counts
/// apart from the main window's: the display follows the panel's own count, so it shows while the main window has run
/// many more passes than the floating one, and goes once the floating window draws on without the editor.
#[test]
fn a_floating_properties_panel_shows_the_display() {
    let mut app = row_app(&BLACK_ROW);
    let ctx = app.ctx().clone();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    let layer = app.session.active().and_then(|s| s.active_layer).unwrap();
    app.run("layer.setAdjustment", json!({"layer": layer.0, "points": [[40, 0], [200, 255]]})).unwrap();
    app.session.select_layer(layer).unwrap();
    let mut record = PropertiesAdjust::new(layer);
    record.clipping = Some(LevelsClip::Shadows);
    app.adjust_session = Some(record);
    // The floating window is an immediate viewport of the main one, run inside the main window's pass in a pass of
    // its own, as eframe runs it.
    ctx.set_embed_viewports(false);
    egui::Context::set_immediate_viewport_renderer(|ctx, mut viewport| {
        let id = viewport.ids.this;
        let mut input = egui::RawInput { viewport_id: id, ..Default::default() };
        input.viewports.insert(id, egui::ViewportInfo::default());
        ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui)).textures_delta.clear();
    });
    let floating = egui::ViewportId::from_hash_of("floating Properties");
    // One pass of the main window: the floating Properties panel draws the editor or not, then the canvas paints.
    let pass = |app: &mut PhotocraftApp, panel_draws: bool| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.ctx().show_viewport_immediate(floating, egui::ViewportBuilder::default(), |ui, _| {
                if panel_draws && let Some(record) = app.adjust_session.as_mut() {
                    record.shown = ui.ctx().cumulative_pass_nr();
                    record.shown_in = ui.ctx().viewport_id();
                }
            });
            let painter = egui::Painter::new(ui.ctx().clone(), egui::LayerId::background(), egui::Rect::EVERYTHING);
            paint(app, ui.ctx(), &painter, &view(), 0);
        });
        out.textures_delta.clear();
        let c = app.levels_clipping.as_ref()?;
        let (end, thr, palette) = c.drawn?;
        request(app, 0)?;
        Some(image_in(&c.stats, &thr, end, palette))
    };
    // The main window has run many passes before the panel was torn off.
    for _ in 0..20 {
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    }
    let _ = pass(&mut app, true);
    assert!(ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT) > ctx.cumulative_pass_nr_for(floating) + 1, "the counts differ");
    assert_eq!(pass(&mut app, true), Some(vec![rgb(255, 0, 0), rgb(0, 255, 255), Color32::BLACK, Color32::WHITE]));
    assert!(pass(&mut app, true).is_some(), "it stays while the panel draws");
    let _ = pass(&mut app, false);
    assert_eq!(pass(&mut app, false), None, "a floating panel that no longer draws the editor shows nothing");
}

/// Every combination: one channel in its colour, two mixed, all three black or white, none the background.
#[test]
fn curves_colours_follow_the_clipped_channels() {
    let cases = [
        (0b000, Color32::WHITE, Color32::BLACK),
        (0b001, rgb(255, 0, 0), rgb(255, 0, 0)),
        (0b010, rgb(0, 255, 0), rgb(0, 255, 0)),
        (0b100, rgb(0, 0, 255), rgb(0, 0, 255)),
        (0b011, rgb(255, 255, 0), rgb(255, 255, 0)),
        (0b110, rgb(0, 255, 255), rgb(0, 255, 255)),
        (0b101, rgb(255, 0, 255), rgb(255, 0, 255)),
        (0b111, Color32::BLACK, Color32::WHITE),
    ];
    for (clipped, black, white) in cases {
        assert_eq!(curves_colour(LevelsClip::Shadows, clipped), black, "black end, {clipped:03b}");
        assert_eq!(curves_colour(LevelsClip::Highlights, clipped), white, "white end, {clipped:03b}");
    }
}
