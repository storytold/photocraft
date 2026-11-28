//! #14113: while a placed file is in its transform box, the options bar shows the reference
//! application's place layout: the reference point on, an Anti-alias checkbox (live for a PDF
//! source, greyed for a raster one) and no H/V skew or Interpolation. An ordinary Free Transform
//! keeps its bar, and the place leaves the persisted Show Reference Point alone.

use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, SmartSource};
use serde_json::json;

use crate::PhotocraftApp;
use crate::transform_tool::{begin, begin_placed, cancel, options_bar};

const SKEW_H: &str = "Horizontal skew";
const SKEW_V: &str = "Vertical skew";
const INTERPOLATION: &str = "Interpolation";
const ANTI_ALIAS: &str = "Anti-alias";
const SHOW_REFERENCE: &str = "Show Reference Point";

/// Where the placed smart object's contents came from.
#[derive(Clone, Copy, Debug)]
enum Source {
    Png,
    Pdf,
    LinkedPdf,
}

/// A 64 × 64 document whose active layer is a smart object made the way a place makes one, its
/// source swapped for `source`.
fn placed_app(source: Source) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(8, 8, 24, 24), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.session.execute("layer.duplicate", json!({"inPlace": true})).unwrap();
    let new_source = match source {
        Source::Png => None,
        Source::Pdf => Some(SmartSource::Embedded { file_name: "logo.pdf".into(), bytes: std::sync::Arc::new(b"%PDF-1.4\n%%EOF\n".to_vec()) }),
        Source::LinkedPdf => Some(SmartSource::Linked { path: "/tmp/Logo.PDF".into() }),
    };
    if let Some(new_source) = new_source {
        app.session
            .edit("source", |doc, a| {
                let Some(LayerContent::Smart(so)) = doc.layer_mut(a.unwrap()).map(|l| &mut l.content) else { panic!("a smart object") };
                so.source = new_source;
                Ok(())
            })
            .unwrap();
    }
    app.sync_views();
    app
}

fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 120.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

/// What the bar shows, as the accessibility tree reads it.
#[derive(Debug, PartialEq)]
struct Bar {
    skew: bool,
    interpolation: bool,
    /// `Some(enabled)` when the Anti-alias checkbox is there.
    anti_alias: Option<bool>,
    anti_alias_checked: bool,
    show_reference: bool,
}

fn bar(h: &Harness<'static, Box<PhotocraftApp>>) -> Bar {
    let anti_alias = h.query_by_role_and_label(Role::CheckBox, ANTI_ALIAS);
    Bar {
        skew: h.query_by_label(SKEW_H).is_some() && h.query_by_label(SKEW_V).is_some(),
        interpolation: h.query_by_label(INTERPOLATION).is_some(),
        anti_alias: anti_alias.as_ref().map(|n| !n.accesskit_node().is_disabled()),
        anti_alias_checked: anti_alias.is_some_and(|n| n.accesskit_node().toggled() == Some(Toggled::True)),
        show_reference: h.get_by_role_and_label(Role::CheckBox, SHOW_REFERENCE).accesskit_node().toggled() == Some(Toggled::True),
    }
}

const FREE_TRANSFORM: Bar = Bar { skew: true, interpolation: true, anti_alias: None, anti_alias_checked: false, show_reference: false };

/// The place bar: reference point on, Anti-alias in place of skew and Interpolation; greyed for a
/// raster source, live and checked for a PDF (embedded or linked).
#[test]
fn placing_shows_the_place_bar() {
    for (source, live) in [(Source::Png, false), (Source::Pdf, true), (Source::LinkedPdf, true)] {
        let mut app = placed_app(source);
        let ctx = app.ctx().clone();
        begin_placed(&mut app, &ctx).unwrap();
        assert!(app.ui.transform.as_ref().unwrap().session.reference_point(), "{source:?}: the session shows the reference point");
        let h = harness(app);
        assert_eq!(bar(&h), Bar { skew: false, interpolation: false, anti_alias: Some(live), anti_alias_checked: true, show_reference: true }, "{source:?}");
    }
}

/// An ordinary Free Transform keeps its bar: skew and Interpolation, no Anti-alias, Show
/// Reference Point off.
#[test]
fn free_transform_keeps_its_bar() {
    let mut app = placed_app(Source::Pdf);
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx).unwrap();
    assert_eq!(bar(&harness(app)), FREE_TRANSFORM);
}

/// The place turns the reference point on for its own box only: the persisted Show Reference
/// Point stays off, and a Free Transform after the place shows today's bar.
#[test]
fn free_transform_after_a_place_keeps_its_bar() {
    let mut app = placed_app(Source::Pdf);
    let ctx = app.ctx().clone();
    begin_placed(&mut app, &ctx).unwrap();
    let mut h = harness(app);
    h.run_steps(2);
    let app = h.state_mut();
    cancel(app);
    assert!(app.ui.transform.is_none());
    assert!(!app.ui.tool_options.transform.show_reference_point, "the persisted setting is untouched");
    let ctx = app.ctx().clone();
    begin(app, &ctx).unwrap();
    assert!(!app.ui.transform.as_ref().unwrap().session.reference_point());
    h.run_steps(4);
    assert_eq!(bar(&h), FREE_TRANSFORM);
}

/// Ticking the live Anti-alias box of a PDF place keeps the new state for that box.
#[test]
fn anti_alias_toggles_for_a_pdf_place() {
    let mut app = placed_app(Source::Pdf);
    let ctx = app.ctx().clone();
    begin_placed(&mut app, &ctx).unwrap();
    let mut h = harness(app);
    h.get_by_role_and_label(Role::CheckBox, ANTI_ALIAS).click();
    h.run_steps(4);
    assert!(!bar(&h).anti_alias_checked, "unticked");
    assert!(!h.state().ui.tool_options.transform.show_reference_point, "the persisted setting is untouched");
}
