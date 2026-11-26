//! Read-only, paged view of the engine's historical conversion report.

use photocraft_doc::DocId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const PAGE_SIZE: u64 = 20;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub open: bool,
    pub offset: u64,
    #[serde(skip)]
    cache: Option<CachedPage>,
}

impl PartialEq for State {
    /// Compare visible report state while ignoring the derived page cache.
    fn eq(&self, other: &Self) -> bool {
        self.open == other.open && self.offset == other.offset
    }
}

#[derive(Clone, Debug)]
struct CachedPage {
    key: (Option<DocId>, u64, u64),
    page: Result<Value, String>,
}

/// Offer report inspection whenever the active native document carries conversion metadata, including after native
/// reopen.
pub fn available(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|s| s.doc.metadata.import_conversion.is_some())
}

/// Open report inspection at its first page with a fresh cache.
pub fn open(app: &mut PhotocraftApp) {
    app.ui.import_report = State { open: true, ..Default::default() };
}

/// Fetch a bounded page through the read-only engine command only when document, revision or page offset changes.
fn refresh(app: &mut PhotocraftApp) {
    let active = app.session.active();
    let offset = app.ui.import_report.offset;
    let key = (active.map(|s| s.doc.id), active.map_or(0, |s| s.revision), offset);
    if app.ui.import_report.cache.as_ref().is_some_and(|c| c.key == key) {
        return;
    }
    let page = app.session.execute("document.importReport", json!({"offset": offset, "limit": PAGE_SIZE})).map_err(|e| e.to_string());
    app.ui.import_report.cache = Some(CachedPage { key, page });
}

/// Truncate display text by Unicode characters and append an ellipsis only when more characters remain.
fn short(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

/// Translate stored outcome keys into stable localizable labels with an explicit unknown fallback.
fn disposition<T>(key: &str, translate: impl FnOnce(&'static str) -> T) -> T {
    translate(match key {
        "native_equivalent" => "Native equivalent",
        "native_approximation" => "Native approximation",
        "retained_unapplied" => "Retained, not applied",
        "skipped_nonessential" => "Nonessential data skipped",
        _ => "Unknown outcome",
    })
}

/// Translate stored conversion reasons into localizable UI labels without interpreting retained payloads.
fn reason<T>(key: &str, translate: impl FnOnce(&'static str) -> T) -> T {
    translate(match key {
        "native_policy" => "Native rules",
        "native_limitation" => "Native capability limit",
        "missing_mapping" => "Conversion not implemented",
        "resource_unavailable" => "Resource unavailable",
        "invalid_optional_data" => "Invalid optional data",
        "retention_limit" => "Retention limit",
        "numerical_conversion" => "Precision conversion",
        _ => "Unknown reason",
    })
}

/// Display the cached read-only report as a modal with bounded paging and release its cache when closed.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.import_report.open {
        app.ui.import_report.cache = None;
        return;
    }
    refresh(app);
    let tokens = crate::theme::Tokens::get(ctx);
    let mut close = false;
    let mut offset = None;
    let modal = egui::Modal::new(egui::Id::unique("import-conversion-report")).backdrop_color(egui::Color32::TRANSPARENT).show(ctx, |ui| {
        ui.set_width(640.0f32.min((ctx.content_rect().width() - 48.0).max(280.0)));
        ui.heading(tl!("Import Conversion Report"));
        if let Some(active) = app.session.active() {
            ui.label(short(&active.doc.name, 100));
        }
        ui.label(egui::RichText::new(tl!("Import-time outcomes. Editing uses native PhotoCraft rules.")).color(tokens.text_dim));
        ui.separator();
        match app.ui.import_report.cache.as_ref().map(|c| &c.page) {
            Some(Ok(page)) => {
                if page["opaque"] == true {
                    ui.label(tl!("A newer report format is retained. Its details cannot be displayed."));
                } else if page["summary"].is_null() {
                    ui.label(tl!("No conversion report was recorded."));
                } else {
                    egui::ScrollArea::vertical().id_salt("import-report-entries").max_height((ctx.content_rect().height() - 280.0).clamp(100.0, 460.0)).show(
                        ui,
                        |ui| {
                            for entry in page["entries"].as_array().into_iter().flatten().take(PAGE_SIZE as usize) {
                                let source = &entry["source"];
                                let kind = match source["kind"].as_str() {
                                    Some("layer") => tl!("Layer"),
                                    Some("image") => tl!("Image"),
                                    Some("channel") => tl!("Channel"),
                                    Some("layer_mask") => tl!("Layer mask"),
                                    Some("selection") => tl!("Selection"),
                                    _ => tl!("Source item"),
                                };
                                let index = source["ordinal"].as_u64().unwrap_or(0).saturating_add(1);
                                let name = short(source["name"].as_str().unwrap_or(""), 160);
                                let title = if name.is_empty() { format!("{kind} #{index}") } else { format!("{kind} #{index} · {name}") };
                                ui.label(egui::RichText::new(title).strong().color(tokens.text));
                                let outcome = disposition(entry["disposition"].as_str().unwrap_or(""), crate::i18n::t);
                                let caption = if entry["disposition"] == "native_equivalent" {
                                    outcome.to_string()
                                } else {
                                    format!("{} · {}", outcome, reason(entry["reason"].as_str().unwrap_or(""), crate::i18n::t))
                                };
                                ui.label(egui::RichText::new(caption).color(tokens.text_dim));
                                for field in ["sourceDescription", "nativeDescription"] {
                                    ui.add(egui::Label::new(short(entry[field].as_str().unwrap_or(""), 768)).wrap().selectable(true));
                                }
                                for target in entry["targets"].as_array().into_iter().flatten().take(8) {
                                    if let Some(id) = target["layerId"].as_u64() {
                                        let text = if target["present"] == false {
                                            crate::i18n::fmt(tl!("Native layer {id} (removed; historical reference)"), &[("id", &id.to_string())])
                                        } else {
                                            crate::i18n::fmt(
                                                tl!("Native layer: {name}"),
                                                &[("name", &short(target["currentName"].as_str().unwrap_or(""), 120))],
                                            )
                                        };
                                        ui.label(egui::RichText::new(text).small().color(tokens.text_faint));
                                    }
                                }
                                if let Some(targets) = entry["targets"].as_array().filter(|targets| targets.len() > 8) {
                                    ui.label(crate::i18n::fmt(tl!("…and {n} more"), &[("n", &(targets.len() - 8).to_string())]));
                                }
                                ui.add_space(4.0);
                                ui.separator();
                                ui.add_space(4.0);
                            }
                            if page["entries"].as_array().is_none_or(Vec::is_empty) {
                                ui.label(tl!("No entries on this page."));
                            }
                        },
                    );
                    ui.horizontal(|ui| {
                        let current = app.ui.import_report.offset;
                        if ui.add_enabled(current > 0, egui::Button::new(tl!("First"))).clicked() {
                            offset = Some(0);
                        }
                        if ui.add_enabled(current > 0, egui::Button::new(tl!("Previous"))).clicked() {
                            offset = Some(current.saturating_sub(PAGE_SIZE));
                        }
                        let next = page["nextOffset"].as_u64();
                        if ui.add_enabled(next.is_some(), egui::Button::new(tl!("Next"))).clicked() {
                            offset = next;
                        }
                        let count = page["entries"].as_array().map_or(0, Vec::len) as u64;
                        ui.label(crate::i18n::fmt(
                            tl!("{first}–{last} of {total}"),
                            &[
                                ("first", &if count > 0 { current.saturating_add(1) } else { 0 }.to_string()),
                                ("last", &current.saturating_add(count).to_string()),
                                ("total", &page["total"].as_u64().unwrap_or(0).to_string()),
                            ],
                        ));
                    });
                }
            }
            Some(Err(error)) => {
                ui.colored_label(tokens.warning, short(error, 768));
            }
            None => {}
        }
        ui.add_space(8.0);
        close = ui.button(tl!("Close")).clicked();
    });
    if let Some(offset) = offset {
        app.ui.import_report.offset = offset;
    }
    if close || modal.should_close() {
        app.ui.import_report = State::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::import_conversion::{
        ConversionDisposition, ConversionEntry, ConversionReason, ConversionReport, ImportConversion, NativeTargetRef, RetainedImportData, SourceItemRef,
    };
    use std::sync::Arc;

    /// Construct a native document with synthetic report entries and private retained payloads for UI paging
    /// assertions.
    fn app(count: u32) -> Box<crate::PhotocraftApp> {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", serde_json::json!({"width": 3, "height": 2})).unwrap();
        let doc = Arc::make_mut(session.active_mut().unwrap().doc_mut());
        doc.metadata.import_conversion = Some(Arc::new(ImportConversion::Report(ConversionReport {
            source_format: "xcf".into(),
            source_version: Some(11),
            policy_version: 1,
            entries: (0..count)
                .map(|ordinal| ConversionEntry {
                    feature_code: "xcf.test".into(),
                    source: SourceItemRef { kind: "layer".into(), ordinal, name: "Duplicate λ ".repeat(100), ..Default::default() },
                    targets: vec![NativeTargetRef { layer_id: Some(photocraft_doc::LayerId::fresh()), ..Default::default() }],
                    disposition: ConversionDisposition::NativeApproximation,
                    reason: ConversionReason::NativePolicy,
                    source_description: "Stored source configuration".into(),
                    native_description: "Native editable layer".into(),
                    attachments: vec![0],
                })
                .collect(),
            attachments: vec![RetainedImportData {
                owner: SourceItemRef::default(),
                kind: "config".into(),
                encoding: "opaque".into(),
                bytes: Arc::new(b"private bytes C:\\resources\\unavailable.asset".to_vec()),
            }],
        })));
        crate::PhotocraftApp::new(session, crate::Services::default())
    }

    #[test]
    /// Verify page limits, stale-target flags and Unicode truncation without exposing private payloads or modifying
    /// document revision.
    fn pages_are_bounded_read_only_and_hide_retained_payloads() {
        let mut app = app(61);
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        refresh(&mut app);
        let first = app.ui.import_report.cache.as_ref().unwrap().page.as_ref().unwrap();
        assert_eq!(first["entries"].as_array().unwrap().len(), PAGE_SIZE as usize);
        assert_eq!(first["nextOffset"], PAGE_SIZE);
        assert_eq!(first["entries"][0]["targets"][0]["present"], false);
        assert!(!first.to_string().contains("unavailable.asset"));
        assert!(!first.to_string().contains("private bytes"));
        app.ui.import_report.offset = PAGE_SIZE;
        refresh(&mut app);
        assert_eq!(app.ui.import_report.cache.as_ref().unwrap().page.as_ref().unwrap()["entries"][0]["source"]["ordinal"], PAGE_SIZE);
        assert!(app.session.active().unwrap().doc == before, "read-only paging changed the native document");
        assert_eq!(app.session.active().unwrap().revision, revision);
        let state = serde_json::to_string(&app.ui.import_report).unwrap();
        assert!(!state.contains("entries"));
        assert!(!state.contains("private"));
        assert!(short(&"λ".repeat(1000), 80).chars().count() <= 81);
    }

    #[test]
    /// Keep empty and persisted reports inspectable independently of the session's live source-read-only flag.
    fn empty_and_reopened_native_reports_are_accessible_without_source_flags() {
        let mut app = app(0);
        refresh(&mut app);
        assert_eq!(app.ui.import_report.cache.as_ref().unwrap().page.as_ref().unwrap()["entries"], serde_json::json!([]));
        let bytes = photocraft_format::save_to_bytes(&app.session.active().unwrap().doc, &Default::default()).unwrap();
        let native = photocraft_format::load_from_bytes(&bytes).unwrap();
        app.session.add_document(native, Some("native.pcraft".into()));
        assert!(available(&app));
        assert!(!app.session.active().unwrap().source_read_only);
        refresh(&mut app);
        assert_eq!(app.ui.import_report.cache.as_ref().unwrap().page.as_ref().unwrap()["summary"]["sourceFormat"], "xcf");
    }

    #[test]
    /// Feed an undo shortcut beneath the report modal and require the native document to remain unchanged.
    fn report_modal_does_not_run_document_shortcuts_beneath_it() {
        let mut app = app(1);
        app.session.execute("layer.setProps", json!({"opacity": 0.4})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        open(&mut app);
        assert!(app.dialog_open(), "canvas and menu gates see the report modal too");
        let ctx = egui::Context::default();
        let modifiers = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers },
                ],
                ..Default::default()
            },
            |ui| {
                crate::shortcuts::handle(&mut app, ui.ctx());
            },
        );
        output.textures_delta.clear();
        assert!(app.session.active().unwrap().doc == before, "a document shortcut ran beneath the report modal");
    }

    #[test]
    /// Drive report visibility through control requests and reject non-boolean state updates without partial UI
    /// changes.
    fn control_can_open_and_close_the_view_and_rejects_bad_values_atomically() {
        use crate::control::{ControlRequest, Reply, handle};
        let mut app = app(1);
        let ctx = egui::Context::default();
        for value in [true, false] {
            let (request, _) = ControlRequest::new("ui.set", json!({"importReport": value}));
            let Reply::Done(result) = handle(&mut app, &ctx, &request) else { panic!("immediate view-state reply expected") };
            assert_eq!(result["ok"], true);
            assert_eq!(app.ui.import_report.open, value);
            assert_eq!(crate::control::inspect(&app, &ctx)["importReport"], value);
        }
        let before = app.ui.clone();
        let (request, _) = ControlRequest::new("ui.set", json!({"importReport": "yes", "maskTarget": true}));
        let Reply::Done(result) = handle(&mut app, &ctx, &request) else { panic!("immediate validation reply expected") };
        assert_eq!(result["ok"], false);
        assert_eq!(app.ui, before);
    }

    /// Dynamic labels use the same translation path as static UI text. New English keys may
    /// legitimately be absent from a pinned catalog (see i18n's feature-PR fallback policy).
    #[test]
    fn dynamic_outcome_labels_use_translation_with_english_fallback() {
        let outcomes = [
            ("native_equivalent", "Native equivalent"),
            ("native_approximation", "Native approximation"),
            ("retained_unapplied", "Retained, not applied"),
            ("skipped_nonessential", "Nonessential data skipped"),
            ("unknown", "Unknown outcome"),
        ];
        let reasons = [
            ("native_policy", "Native rules"),
            ("native_limitation", "Native capability limit"),
            ("missing_mapping", "Conversion not implemented"),
            ("resource_unavailable", "Resource unavailable"),
            ("invalid_optional_data", "Invalid optional data"),
            ("retention_limit", "Retention limit"),
            ("numerical_conversion", "Precision conversion"),
            ("unknown", "Unknown reason"),
        ];
        for (key, english) in outcomes {
            assert_eq!(disposition(key, |label| format!("translated:{label}")), format!("translated:{english}"));
            for lang in crate::i18n::Lang::all() {
                let actual = disposition(key, |label| crate::i18n::tr(lang, label));
                assert_eq!(actual, crate::i18n::tr(lang, english));
                assert!(!actual.is_empty());
                if !crate::i18n::has(lang, english) {
                    assert_eq!(actual, english);
                }
            }
        }
        for (key, english) in reasons {
            assert_eq!(reason(key, |label| format!("translated:{label}")), format!("translated:{english}"));
            for lang in crate::i18n::Lang::all() {
                let actual = reason(key, |label| crate::i18n::tr(lang, label));
                assert_eq!(actual, crate::i18n::tr(lang, english));
                assert!(!actual.is_empty());
                if !crate::i18n::has(lang, english) {
                    assert_eq!(actual, english);
                }
            }
        }
    }

    #[test]
    fn page_cache_follows_document_identity_revision_and_offset() {
        let mut app = app(23);
        refresh(&mut app);
        let first = app.session.active().unwrap().doc.id;
        let first_revision = app.session.active().unwrap().revision;
        assert_eq!(app.ui.import_report.cache.as_ref().unwrap().key, (Some(first), first_revision, 0));
        let other = self::app(2);
        app.session.add_document((*other.session.active().unwrap().doc).clone(), None);
        let second = app.session.active().unwrap().doc.id;
        assert_ne!(first, second);
        refresh(&mut app);
        assert_eq!(app.ui.import_report.cache.as_ref().unwrap().page.as_ref().unwrap()["total"], 2);
        app.session.set_active(0);
        app.ui.import_report.offset = PAGE_SIZE;
        refresh(&mut app);
        assert_eq!(app.ui.import_report.cache.as_ref().unwrap().page.as_ref().unwrap()["entries"][0]["source"]["ordinal"], PAGE_SIZE);
        app.session.execute("layer.new.layer", json!({"name":"Later native edit"})).unwrap();
        refresh(&mut app);
        assert_eq!(app.ui.import_report.cache.as_ref().unwrap().key, (Some(first), app.session.active().unwrap().revision, PAGE_SIZE));
        assert_ne!(app.ui.import_report.cache.as_ref().unwrap().key.1, first_revision);
        app.ui.import_report.open = false;
        show(&mut app, &egui::Context::default());
        assert!(app.ui.import_report.cache.is_none(), "closing releases the derived page");
    }

    #[test]
    fn file_info_footer_opens_report_without_applying_metadata_or_unrelated_actions() {
        let mut app = app(1);
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &ctx, "file.fileInfo", json!({})).unwrap();
        let dialog = app.top_dialog().unwrap();
        crate::menus::view_cmds::dialogs::footer_action(&mut app, &ctx, dialog, "unrelated");
        assert!(!app.ui.import_report.open);
        crate::menus::view_cmds::dialogs::footer_action(&mut app, &ctx, dialog, "importReport");
        assert!(app.ui.import_report.open && app.dialog_open());
        assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &before));
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert_eq!(app.top_dialog(), Some(dialog), "File Info remains uncommitted below the report");
        app.ui.import_report = Default::default();
        app.session.execute("file.new", json!({"width":1,"height":1})).unwrap();
        crate::menus::view_cmds::dialogs::footer_action(&mut app, &ctx, dialog, "importReport");
        assert!(!app.ui.import_report.open, "a report action cannot expose a previous document's report");
    }
}
