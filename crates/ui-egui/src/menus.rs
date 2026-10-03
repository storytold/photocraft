//! Menu bar generated from the engine command registry plus UI-level commands.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{DialogKind, UiState};

/// Top-level menus in Photoshop order.
pub const TOP_MENUS: [&str; 10] = ["File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"];

/// UI-level commands (handled by the shell rather than the engine): id, label, menu, shortcut.
pub const UI_COMMANDS: &[(&str, &str, &[&str], Option<&str>)] = &[
    ("file.open", "Open…", &["File"], Some("Cmd+O")),
    ("file.save", "Save", &["File"], Some("Cmd+S")),
    ("file.saveAs", "Save As…", &["File"], Some("Cmd+Shift+S")),
    ("edit.freeTransform", "Free Transform", &["Edit"], Some("Cmd+T")),
    ("file.export.exportAs", "Export As…", &["File", "Export"], Some("Cmd+Alt+Shift+W")),
    ("file.export.quickExportAsPng", "Quick Export as PNG", &["File", "Export"], None),
    ("edit.transform.scale", "Scale", &["Edit", "Transform"], None),
    ("edit.transform.rotate", "Rotate", &["Edit", "Transform"], None),
    ("edit.transform.skew", "Skew", &["Edit", "Transform"], None),
    ("edit.transform.distort", "Distort", &["Edit", "Transform"], None),
    ("edit.transform.perspective", "Perspective", &["Edit", "Transform"], None),
    ("select.selectAndMask", "Select and Mask…", &["Select"], Some("Cmd+Alt+R")),
    ("view.rulers", "Rulers", &["View"], Some("Cmd+R")),
    ("view.show.grid", "Grid", &["View", "Show"], Some("Cmd+'")),
    ("view.show.guides", "Guides", &["View", "Show"], Some("Cmd+;")),
    ("view.snap", "Snap", &["View"], Some("Cmd+Shift+;")),
    ("view.lockGuides", "Lock Guides", &["View"], Some("Cmd+Alt+;")),
    ("view.extras", "Extras", &["View"], Some("Cmd+H")),
    ("view.show.targetPath", "Target Path", &["View", "Show"], Some("Cmd+Shift+H")),
    ("view.screenMode.cycle", "Cycle Screen Mode", &[], Some("F")),
    ("view.zoomIn", "Zoom In", &["View"], Some("Cmd+=")),
    ("view.zoomOut", "Zoom Out", &["View"], Some("Cmd+-")),
    ("view.fitOnScreen", "Fit on Screen", &["View"], Some("Cmd+0")),
    ("view.actualPixels", "100%", &["View"], Some("Cmd+1")),
    ("window.newWindowForDocument", "New Window for Document", &["Window", "Arrange"], None),
    ("window.toggle.layers", "Layers", &["Window"], Some("F7")),
    ("window.toggle.history", "History", &["Window"], None),
    ("window.toggle.properties", "Properties", &["Window"], None),
    ("window.toggle.color", "Color", &["Window"], Some("F6")),
    ("window.toggle.brushSettings", "Brush Settings", &["Window"], Some("F5")),
    ("window.toggle.navigator", "Navigator", &["Window"], None),
    ("window.toggle.toolbar", "Tools", &["Window"], None),
    ("window.toggle.options", "Options", &["Window"], None),
    ("window.theme.toggle", "Next Theme", &["Window"], None),
    ("window.theme.pro", "Pro Theme", &["Window", "Theme"], None),
    ("window.theme.proMedium", "Pro Medium Gray Theme", &["Window", "Theme"], None),
    ("window.theme.studio", "Studio Theme", &["Window", "Theme"], None),
    ("window.theme.studioLight", "Studio Light Theme", &["Window", "Theme"], None),
    ("window.theme.classic", "Classic Theme", &["Window", "Theme"], None),
    ("edit.search", "Search…", &["Edit"], Some("Cmd+K")),
    ("help.discord", "Join the ArtCraft Discord…", &["Help"], None),
    ("help.website", "PhotoCraft Website", &["Help"], None),
    ("help.artcraftWebsite", "ArtCraft Website", &["Help"], None),
    ("help.github", "PhotoCraft on GitHub", &["Help"], None),
    ("help.reportIssue", "Report an Issue…", &["Help"], None),
    ("help.about", "About PhotoCraft", &["Help"], None),
];

/// Photoshop's Window › <panel> ids for the panels the shell already has, as `window.toggle.*`.
fn panel_alias(id: &str) -> Option<&'static str> {
    Some(match id.strip_prefix("window.panel.")? {
        "layers" => "window.toggle.layers",
        "history" => "window.toggle.history",
        "properties" | "adjustments" => "window.toggle.properties",
        "color" | "swatches" => "window.toggle.color",
        "navigator" | "info" | "histogram" => "window.toggle.navigator",
        "tools" => "window.toggle.toolbar",
        "options" => "window.toggle.options",
        "brushSettings" | "brushes" => "window.toggle.brushSettings",
        _ => return None,
    })
}

/// View › Proof Setup presets we can simulate, as `view.proofSetup` profiles.
fn proof_preset(id: &str) -> Option<&'static str> {
    Some(match id.strip_prefix("view.proofSetup.")? {
        "workingCmyk" => "working-cmyk",
        "internetStandardRgb" | "monitorRgb" => "srgb",
        _ => return None,
    })
}

/// Window › Workspace presets by id.
fn workspace_name(id: &str) -> Option<&'static str> {
    Some(match id.strip_prefix("window.workspace.")? {
        "essentials" | "resetWorkspace" => "",
        "photography" => "Photography",
        "painting" => "Painting",
        "graphicAndWeb" => "Graphic and Web",
        "pixelArt" => "Pixel Art",
        "motion" => "Motion",
        _ => return None,
    })
}

/// Run a command id from any source (menu, shortcut, palette, automation).
pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {
    // Help › Discord, website, GitHub, Report an Issue.
    if let Some(url) = crate::links::url_for(id) {
        return Ok(crate::links::open(app, ctx, url));
    }
    if id == "view.proofSetup.custom" {
        return Ok(json!({"dialog": crate::filter_dialog::open(app, "view.proofSetup")}));
    }
    // Image › Analysis tools/dialogs, Measurement Log and Notes panels, File › Import › Notes.
    if let Some(r) = crate::analysis_ui::menu(app, id, &params) {
        return r;
    }
    // Workspace New/Delete/Lock, Modifier Keys, custom pixel aspect, Extras/32-bit options.
    if let Some(r) = crate::workspace_ui::menu(app, id, &params) {
        return r;
    }
    // Window › Gradients, Patterns, Styles, Shapes, Tool Presets, Clone Source.
    if let Some(r) = crate::preset_panels::menu(app, id, &params) {
        return r;
    }
    // Character/Paragraph Styles, Glyphs, Edit › Check Spelling dialog.
    if let Some(r) = crate::type_panels_ui::menu(app, id, &params) {
        return r;
    }
    // Image > Variables (Define / Data Sets), Apply Data Set.
    if let Some(r) = crate::variables_ui::menu(app, id, &params) {
        return r;
    }
    // Window > Timeline panel.
    if let Some(r) = crate::timeline_ui::menu(app, id, &params) {
        return r;
    }
    if id == "window.panel.brushes" {
        // Window › Brushes opens the Brush Settings window on its presets tab.
        app.ui.panels.brush_settings = true;
        app.ui.brush_tab = 1;
        return Ok(Value::Null);
    }
    if id == "window.panel.brushSettings" {
        app.ui.brush_tab = 0;
    }
    // View/Window/Type shell items, and dialogs/pickers in front of File commands.
    // Edit › Preferences, Keyboard Shortcuts, Color Settings and other Edit dialogs.
    if let Some(r) = crate::prefs_ui::invoke(app, ctx, id, &params) {
        return r;
    }
    // Save for Web, Print and the other File-menu dialogs added with slices.
    if let Some(r) = crate::file_ui::invoke(app, ctx, id, &params) {
        return r;
    }
    if let Some(r) = crate::view_cmds::invoke(app, ctx, id, &params) {
        return r;
    }
    // Liquify dialog, Puppet Warp and Perspective Warp modes (and their control params).
    if let Some(r) = crate::distort_ui::menu(app, ctx, id, &params) {
        return r;
    }
    // Camera Raw Filter dialog (and its control params).
    if let Some(r) = crate::camera_raw_ui::menu(app, ctx, id, &params) {
        return r;
    }
    if let Some(r) = crate::wide_angle_ui::menu(app, ctx, id, &params) {
        return r;
    }
    if let Some(profile) = proof_preset(id) {
        // View › Proof Setup presets: set the proof profile and turn Proof Colors on.
        app.run("view.proofSetup", json!({"profile": profile}))?;
        return app.run("view.proofColors", json!({"on": true}));
    }
    if let Some(alias) = panel_alias(id) {
        return invoke(app, ctx, alias, params);
    }
    if let Some(ws) = workspace_name(id) {
        // Reset re-applies the current workspace; Essentials is the default layout.
        match (ws, id) {
            (_, "window.workspace.resetWorkspace") => {}
            ("", _) => app.ui.workspace = "Essentials".into(),
            (name, _) => app.ui.workspace = name.into(),
        }
        apply_workspace(app);
        return Ok(json!({"workspace": app.ui.workspace}));
    }
    match id {
        // Layer › Rename Layer from a menu starts in-place renaming in the Layers panel.
        "layer.renameLayer" if params.get("name").is_none() => {
            let st = app.session.active().ok_or("no document")?;
            let id = params.get("layer").and_then(Value::as_u64).or(st.active_layer.map(|l| l.0)).ok_or("no active layer")?;
            let name = st.doc.layer(photocraft_doc::LayerId(id)).map(|l| l.name.clone()).ok_or("no such layer")?;
            ctx.data_mut(|d| d.insert_temp(egui::Id::new(("rename", id)), name));
            Ok(Value::Null)
        }
        "file.new" if params.as_object().is_none_or(|o| o.is_empty()) => {
            let d = app.ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
            Ok(json!({"dialog": d}))
        }
        "file.open" => {
            if let Some(path) = params.get("path").and_then(Value::as_str) {
                open_path(app, path)
            } else {
                app.open_dialog_file();
                Ok(Value::Null)
            }
        }
        "file.save" => {
            let path = params.get("path").and_then(Value::as_str).map(str::to_string).or_else(|| app.session.active().and_then(|d| d.path.clone()).filter(|p| p.ends_with(".psd") || p.ends_with(".psb")));
            app.save_as(path).map(|p| json!({"path": p}))
        }
        "file.saveAs" => app.save_as(params.get("path").and_then(Value::as_str).map(str::to_string)).map(|p| json!({"path": p})),
        "view.zoomIn" | "view.zoomOut" | "view.fitOnScreen" | "view.actualPixels" => {
            let i = app.session.active_index().ok_or("no document")?;
            let v = &mut app.ui.views[i];
            match id {
                "view.zoomIn" => v.zoom = crate::canvas::zoom_step(v.zoom, 1),
                "view.zoomOut" => v.zoom = crate::canvas::zoom_step(v.zoom, -1),
                "view.fitOnScreen" => v.fit_pending = true,
                _ => v.zoom = 1.0,
            }
            Ok(Value::Null)
        }
        "window.newWindowForDocument" => {
            let doc = app.session.active_index().ok_or("no document")?;
            let wid = app.ui.alloc_id();
            let view = app.ui.views[doc].clone();
            app.ui.windows.push(crate::state::DocWindow { id: wid, document: doc, view, open: true });
            Ok(json!({"window": wid}))
        }
        "window.theme.toggle" => {
            let next = app.ui.theme.next();
            app.set_theme(ctx, next);
            Ok(Value::Null)
        }
        "window.theme.pro" | "window.theme.proMedium" | "window.theme.studio" | "window.theme.studioLight" | "window.theme.classic" => {
            let k = crate::theme::ThemeKind::from_name(&id["window.theme.".len()..]).unwrap_or_default();
            app.set_theme(ctx, k);
            Ok(Value::Null)
        }
        "edit.search" => {
            app.ui.palette_open = !app.ui.palette_open;
            Ok(Value::Null)
        }
        "help.about" => Ok(json!({"dialog": app.ui.open_dialog(DialogKind::About, Default::default())})),
        "file.export.exportAs" => Ok(json!({"dialog": crate::export_dialog::open(app)?})),
        "file.export.quickExportAsPng" => crate::export_dialog::quick_export_png(app),
        // Layer › Export As… / Quick Export as PNG: the export pipeline on just the active layer.
        l if matches!(l, "layer.exportAs" | "layer.quickExportAsPng") && params.as_object().is_none_or(|o| o.is_empty()) => {
            let layer = app.session.active().and_then(|d| d.active_layer).ok_or("no active layer")?;
            if l == "layer.exportAs" { Ok(json!({"dialog": crate::export_dialog::open_layer(app, layer)?})) } else { crate::export_dialog::quick_export_layer_png(app, layer) }
        }
        "layer.layerStyle.blendingOptions" if params.as_object().is_none_or(|o| o.is_empty()) => {
            crate::layer_style::open(app, Some(crate::layer_style::BLENDING)).map(|d| json!({"dialog": d})).ok_or_else(|| "no active layer".to_string())
        }
        // Layer Content Options…: the adjustment / fill controls live in Properties.
        "layer.layerContentOptions" => {
            let r = app.run(id, params)?;
            app.ui.panels.properties = true;
            app.ui.dock_tabs.properties = 0;
            Ok(r)
        }
        // Photoshop's "Select and Mask…" is the engine's select.refineEdge.
        "select.selectAndMask" => Ok(json!({"dialog": crate::filter_dialog::open(app, "select.refineEdge")})),
        // Select › Transform Selection from the menu: the interactive box (with params: the engine).
        "select.transformSelection" if params.as_object().is_none_or(|o| o.is_empty()) => {
            crate::transform_tool::begin_selection(app, ctx).map(|_| json!({"transform": app.ui.transform}))
        }
        "edit.paste" if params.as_object().is_none_or(|o| o.is_empty()) => {
            // Photoshop: paste in place when the copied area is visible, else centred in the view;
            // images from other apps are always centred.
            let external = app.import_os_clipboard();
            let visible = !external && app.session.active_index().zip(app.session.clipboard.as_ref()).is_some_and(|(i, clip)| {
                let v = &app.ui.views[i];
                let (hw, hh) = (app.last_canvas_rect.width() / 2.0 / v.zoom, app.last_canvas_rect.height() / 2.0 / v.zoom);
                let r = photocraft_geom::Rect::new((v.center[0] - hw) as i32, (v.center[1] - hh) as i32, (v.center[0] + hw) as i32, (v.center[1] + hh) as i32);
                !clip.bounds.intersect(&r).is_empty()
            });
            let p = match app.session.active_index() {
                Some(i) if !visible => json!({"center": app.ui.views[i].center}),
                _ => json!({}),
            };
            app.run("edit.paste", p)
        }
        "view.rulers" | "view.show.grid" | "view.show.guides" | "view.snap" | "view.lockGuides" => {
            let e = &mut app.ui.extras;
            let slot = match id {
                "view.rulers" => &mut e.rulers,
                "view.show.grid" => &mut e.grid,
                "view.show.guides" => &mut e.guides,
                "view.snap" => &mut e.snap,
                _ => &mut e.lock_guides,
            };
            *slot = !*slot;
            Ok(json!(*slot))
        }
        "edit.freeTransform" | "edit.transform.scale" | "edit.transform.rotate" | "edit.transform.skew" | "edit.transform.distort" | "edit.transform.perspective" => {
            crate::transform_tool::begin(app, ctx).map(|_| json!({"transform": app.ui.transform}))
        }
        // Edit › Transform › Warp from the menu: interactive Warp mode (with params: the engine).
        "edit.transform.warp" | "layer.smartObjects.warp" if params.as_object().is_none_or(|o| o.is_empty()) => {
            crate::transform_tool::begin_warp(app, ctx).map(|_| json!({"transform": app.ui.transform}))
        }
        // Split warps edit the mesh of an active Warp session.
        "edit.transform.splitWarpCrosswise" | "edit.transform.splitWarpHorizontally" | "edit.transform.splitWarpVertically" | "edit.transform.removeWarpSplit"
            if app.ui.transform.as_ref().is_some_and(|t| t.warp.is_some()) && params.get("warp").is_none() =>
        {
            let at = params.get("at").and_then(Value::as_array).and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?]));
            crate::transform_tool::split(app, id, at).map(|_| json!({"transform": app.ui.transform}))
        }
        sz if crate::sizing::is_sizing(sz) && params.as_object().is_none_or(|o| o.is_empty()) => {
            Ok(json!({"dialog": crate::sizing::open(app, sz).ok_or("no document")?}))
        }
        fl if crate::filter_dialog::has_dialog(fl) && params.as_object().is_none_or(|o| o.is_empty()) => {
            Ok(json!({"dialog": crate::filter_dialog::open(app, fl)}))
        }
        a if a.starts_with("image.adjustments.")
            && params.as_object().is_none_or(|o| o.is_empty())
            && !crate::panels::adjustment_sliders(a.rsplit('.').next().unwrap_or("")).is_empty() =>
        {
            let label = photocraft_engine::commands::find(a).map(|c| c.label).unwrap_or(a);
            Ok(json!({"dialog": crate::dialogs::open_command_dialog(app, a, label)}))
        }
        t if t.starts_with("window.toggle.") => {
            let p = &mut app.ui.panels;
            let slot = match &t["window.toggle.".len()..] {
                "layers" => &mut p.layers,
                "history" => &mut p.history,
                "properties" => &mut p.properties,
                "color" => &mut p.color,
                "navigator" => &mut p.navigator,
                "toolbar" => &mut p.toolbar,
                "options" => &mut p.options_bar,
                "brushSettings" => &mut p.brush_settings,
                _ => return Err(format!("unknown panel in {t}")),
            };
            *slot = !*slot;
            Ok(Value::Null)
        }
        // Layer › Layer Style › <effect>… opens the Layer Style dialog on that effect.
        ls if params.as_object().is_none_or(|o| o.is_empty())
            && ls.strip_prefix("layer.layerStyle.").is_some_and(|k| crate::layer_style::KINDS.iter().any(|(kind, _)| *kind == k)) =>
        {
            let kind = &ls["layer.layerStyle.".len()..];
            crate::layer_style::open(app, Some(kind)).map(|d| json!({"dialog": d})).ok_or_else(|| "no active layer".to_string())
        }
        _ => app.run(id, params),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn open_path(app: &mut PhotocraftApp, path: &str) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(path.to_string());
    app.open_bytes(&name, &bytes)?;
    if let Some(st) = app.session.active_mut() {
        st.path = Some(path.to_string());
    }
    Ok(Value::Null)
}

#[cfg(target_arch = "wasm32")]
fn open_path(_app: &mut PhotocraftApp, path: &str) -> Result<Value, String> {
    Err(format!("cannot open paths on the web: {path}"))
}

pub fn is_enabled(app: &PhotocraftApp, id: &str) -> bool {
    // Photoshop greys these for the Background layer, other layer kinds or single-layer documents.
    if crate::enable_rules::disabled(app, id) {
        return false;
    }
    if let Some(e) = crate::workspace_ui::is_enabled(app, id) {
        return e;
    }
    if crate::analysis_ui::handles(id) {
        return true;
    }
    if crate::preset_panels::handles(id) || crate::type_panels_ui::handles(id) || crate::timeline_ui::handles(id) {
        return true;
    }
    if let Some(e) = crate::view_cmds::is_enabled(app, id) {
        return e;
    }
    match id {
        "file.open" | "help.about" | "edit.search" => true,
        i if crate::links::url_for(i).is_some() => true,
        i if i.starts_with("window.theme.") => true,
        "file.save" | "file.saveAs" | "file.export.exportAs" | "file.export.quickExportAsPng" => app.session.active().is_some() && app.services.export.is_some(),
        i if i.starts_with("window.toggle.") => true,
        i if panel_alias(i).is_some() || workspace_name(i).is_some() => true,
        i if proof_preset(i).is_some() => app.session.active().is_some(),
        // "Custom…" is the full Proof Setup dialog.
        "view.proofSetup.custom" => app.session.active().is_some(),
        "view.rulers" | "view.show.grid" | "view.show.guides" | "view.snap" | "view.lockGuides" => true,
        "select.selectAndMask" => app.session.is_enabled("select.refineEdge"),
        "select.transformSelection" => app.ui.transform.is_none() && app.session.is_enabled("select.transformSelection"),
        i if (i.starts_with("view.zoom") || i == "view.fitOnScreen" || i == "view.actualPixels") || i == "window.newWindowForDocument" => app.session.active().is_some(),
        "edit.freeTransform" | "edit.transform.scale" | "edit.transform.rotate" | "edit.transform.skew" | "edit.transform.distort" | "edit.transform.perspective" => {
            app.ui.transform.is_none() && app.session.active().and_then(|s| s.active_layer).is_some()
        }
        i => app.session.is_enabled(i),
    }
}

/// Is a UI-level panel toggle currently on (for checkmarks)?
fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    use photocraft_doc::{ColorMode, SampleType};
    if let Some(c) = crate::view_cmds::checked(app, id) {
        return Some(c);
    }
    if let Some(c) = crate::analysis_ui::checked(app, id).or_else(|| crate::workspace_ui::checked(app, id)).or_else(|| crate::file_ui::checked(app, id)) {
        return Some(c);
    }
    if let Some(c) = crate::preset_panels::checked(app, id) {
        return Some(c);
    }
    if let Some(c) = crate::timeline_ui::checked(app, id) {
        return Some(c);
    }
    if let Some(c) = crate::type_panels_ui::checked(app, id) {
        return Some(c);
    }
    if let Some(alias) = panel_alias(id) {
        return checked(app, alias);
    }
    if id == "select.isolateLayers" {
        return Some(!app.session.active()?.isolated_layers.is_empty());
    }
    if id == "view.proofColors" || id == "view.gamutWarning" {
        let d = app.session.active()?;
        let pv = app.session.color.proof(d.doc.id);
        return Some(if id == "view.proofColors" { pv.enabled } else { pv.gamut_warning });
    }
    if id.starts_with("window.workspace.") && id != "window.workspace.resetWorkspace" {
        let want = workspace_name(id)?;
        return Some(app.ui.workspace == if want.is_empty() { "Essentials" } else { want });
    }
    if let Some(m) = id.strip_prefix("image.mode.") {
        let d = &app.session.active()?.doc;
        return match m {
            "rgb" => Some(d.mode == ColorMode::Rgb),
            "grayscale" => Some(d.mode == ColorMode::Grayscale),
            "cmyk" => Some(d.mode == ColorMode::Cmyk),
            "lab" => Some(d.mode == ColorMode::Lab),
            "multichannel" => Some(d.mode == ColorMode::Multichannel),
            "indexedColor" => Some(d.mode == ColorMode::Indexed),
            "bitmap" => Some(d.mode == ColorMode::Bitmap),
            "duotone" => Some(d.mode == ColorMode::Duotone),
            "bits8" => Some(d.depth == SampleType::U8),
            "bits16" => Some(d.depth == SampleType::U16),
            "bits32" => Some(d.depth == SampleType::F32),
            _ => None,
        };
    }
    let e = &app.ui.extras;
    match id {
        "view.rulers" => return Some(e.rulers),
        "view.show.grid" => return Some(e.grid),
        "view.show.guides" => return Some(e.guides),
        "view.snap" => return Some(e.snap),
        "view.lockGuides" => return Some(e.lock_guides),
        _ => {}
    }
    let p = &app.ui.panels;
    Some(match id {
        "window.toggle.layers" => p.layers,
        "window.toggle.history" => p.history,
        "window.toggle.properties" => p.properties,
        "window.toggle.color" => p.color,
        "window.toggle.navigator" => p.navigator,
        "window.toggle.toolbar" => p.toolbar,
        "window.toggle.options" => p.options_bar,
        "window.toggle.brushSettings" => p.brush_settings,
        _ => return None,
    })
}

/// Menu tree entry for rendering and for `ui.inspect`.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub path: Vec<String>,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
    /// Edit › Menus colour (red, orange, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Is `id` implemented by the engine or the shell (a live menu item)? Shared by the menus and
/// the parity report ([`crate::parity`]).
pub fn is_live(id: &str) -> bool {
    photocraft_engine::commands::find(id).is_some() || UI_COMMANDS.iter().any(|c| c.0 == id) || panel_alias(id).is_some() || workspace_name(id).is_some() || proof_preset(id).is_some() || id == "view.proofSetup.custom" || crate::view_cmds::handles(id) || crate::analysis_ui::handles(id) || crate::workspace_ui::handles(id) || crate::preset_panels::handles(id) || crate::type_panels_ui::handles(id) || crate::timeline_ui::handles(id)
}

pub fn menu_items(app: &PhotocraftApp) -> Vec<MenuItem> {
    // 1) Photoshop's full menu tree, in Photoshop order; live where we implement the command.
    let known = is_live;
    let mut items: Vec<MenuItem> = crate::menu_catalog::CATALOG
        .iter()
        .map(|&(path, label, sc, id)| MenuItem {
            id: id.to_string(),
            label: label.to_string(),
            path: path.iter().map(|s| s.to_string()).collect(),
            shortcut: sc.map(Into::into),
            enabled: known(id) && is_enabled(app, id),
            checked: checked(app, id),
            color: None,
        })
        .collect();
    // 2) Our commands that Photoshop's tree doesn't list (or lists under another id).
    let mut extra: Vec<MenuItem> = Vec::new();
    for &(id, label, path, sc) in UI_COMMANDS {
        extra.push(MenuItem { id: id.into(), label: label.into(), path: path.iter().map(|s| s.to_string()).collect(), shortcut: sc.map(Into::into), enabled: is_enabled(app, id), checked: checked(app, id), color: None });
    }
    for c in photocraft_engine::command_specs().iter().filter(|c| !c.menu.is_empty()) {
        extra.push(MenuItem { id: c.id.into(), label: c.label.into(), path: c.menu.iter().map(|s| s.to_string()).collect(), shortcut: c.shortcut.map(Into::into), enabled: is_enabled(app, c.id), checked: None, color: None });
    }
    for e in extra {
        let dup = items.iter().any(|i| i.id == e.id || (i.path == e.path && i.label.trim_end_matches('…') == e.label.trim_end_matches('…')));
        if !dup {
            // Insert after the last item of the same top-level menu, keeping menus contiguous.
            let top = e.path.first().cloned();
            let at = items.iter().rposition(|i| i.path.first() == top.as_ref()).map_or(items.len(), |p| p + 1);
            items.insert(at, e);
        }
    }
    // Help: the link items, a separator, then About.
    if let Some(at) = items.iter().position(|i| i.id == "help.about") {
        items.insert(at, MenuItem { id: "---".into(), label: "---".into(), path: vec!["Help".into()], shortcut: None, enabled: false, checked: None, color: None });
    }
    // Edit › Keyboard Shortcuts overrides, Edit › Menus hidden items and colours.
    let prefs = app.session.prefs();
    if !prefs.shortcuts.is_empty() {
        for it in &mut items {
            if prefs.shortcuts.contains_key(&it.id) {
                it.shortcut = prefs.shortcut(&it.id, None).map(str::to_string);
            }
        }
    }
    if !prefs.menus.hidden.is_empty() {
        items.retain(|i| !prefs.menus.hidden.contains(&i.id));
    }
    if prefs.interface.show_menu_colors {
        for it in &mut items {
            it.color = prefs.menus.colors.get(&it.id).cloned();
        }
    }
    items
}

/// Background tint of an Edit › Menus colour name.
fn menu_tint(name: &str) -> Option<egui::Color32> {
    Some(match name {
        "red" => egui::Color32::from_rgb(190, 60, 60),
        "orange" => egui::Color32::from_rgb(200, 120, 40),
        "yellow" => egui::Color32::from_rgb(190, 170, 40),
        "green" => egui::Color32::from_rgb(60, 150, 70),
        "blue" => egui::Color32::from_rgb(50, 110, 200),
        "violet" => egui::Color32::from_rgb(130, 80, 190),
        "gray" => egui::Color32::from_rgb(120, 120, 120),
        _ => return None,
    })
}

pub fn menu_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let items = menu_items(app);
    let mut clicked: Option<String> = None;
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let v = &mut ui.style_mut().visuals;
        v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, t.text_dim);
        v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
        egui::MenuBar::new().ui(ui, |ui| {
            ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
            for top in TOP_MENUS {
                let mine: Vec<&MenuItem> = items.iter().filter(|i| i.path.first().map(String::as_str) == Some(top)).collect();
                ui.menu_button(egui::RichText::new(top).color(t.text_dim), |ui| {
                    ui.set_min_width(220.0);
                    if mine.is_empty() {
                        ui.weak("(coming soon)");
                    }
                    render_level(ui, &mine, 1, &mut clicked);
                });
            }
        });
    });
    if let Some(id) = clicked {
        let ctx = ui.ctx().clone();
        let _ = invoke(app, &ctx, &id, json!({}));
    }
}

fn render_level(ui: &mut egui::Ui, items: &[&MenuItem], depth: usize, clicked: &mut Option<String>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    if t.pro {
        // Spectrum/macOS menus: blue highlight row with white text.
        let v = &mut ui.style_mut().visuals;
        v.widgets.hovered.weak_bg_fill = t.accent;
        v.widgets.hovered.bg_fill = t.accent;
        v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        v.widgets.hovered.corner_radius = egui::CornerRadius::same(3);
        ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);
    }
    // Walk in Photoshop order: leaves and separators at this depth; a submenu appears at the position
    // of its first child.
    let mut shown_subs: Vec<&str> = Vec::new();
    let mut last_was_sep = true;
    for (i, it) in items.iter().enumerate() {
        if it.path.len() == depth {
            if it.label == "---" {
                if !last_was_sep && i + 1 < items.len() {
                    ui.separator();
                    last_was_sep = true;
                }
                continue;
            }
            let mut text = it.label.clone();
            if let Some(c) = it.checked {
                text = format!("{} {}", if c { "✔" } else { "  " }, text);
            }
            let mut b = egui::Button::new(text);
            if let Some(tint) = it.color.as_deref().and_then(menu_tint) {
                b = b.fill(tint.gamma_multiply(0.55));
            }
            if let Some(sc) = &it.shortcut {
                b = b.shortcut_text(crate::shortcuts::pretty(sc));
            }
            if ui.add_enabled(it.enabled, b).clicked() {
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
            let any_enabled = child.iter().any(|c| c.enabled && c.label != "---");
            ui.add_enabled_ui(any_enabled || !child.is_empty(), |ui| {
                ui.menu_button(name, |ui| render_level(ui, &child, depth + 1, clicked));
            });
            last_was_sep = false;
        }
    }
}

/// Workspace presets (Window → Workspace): which panels are visible.
pub fn apply_workspace(app: &mut PhotocraftApp) {
    // Saved workspaces (Window › Workspace › New Workspace…) restore their own layout.
    if crate::workspace_ui::apply_custom(app) {
        return;
    }
    let p = &mut app.ui.panels;
    let (nav, color, layers, history, props) = match app.ui.workspace.as_str() {
        "Photography" => (true, false, true, true, true),
        "Painting" => (false, true, true, false, false),
        "Graphic and Web" => (false, true, true, false, true),
        _ => (false, true, true, false, true),
    };
    p.navigator = nav;
    p.color = color;
    p.layers = layers;
    p.history = history;
    p.properties = props;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_bar_labels_have_horizontal_padding_and_open_menus() {
        use egui_kittest::{Harness, kittest::Queryable};

        for theme in crate::theme::ThemeKind::ALL {
            for width in [640.0, 1440.0] {
                let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                let mut harness = Harness::builder().with_size(egui::vec2(width, 200.0)).build_ui_state(|ui, app| menu_bar(app, ui), app);
                PhotocraftApp::setup_context(&harness.ctx, theme);
                harness.run_steps(3);

                let font = egui::TextStyle::Button.resolve(&harness.ctx.global_style());
                let mut right = 0.0;
                for label in TOP_MENUS {
                    let rect = harness.get_by_label(label).rect();
                    let text_width = harness.ctx.fonts_mut(|fonts| fonts.layout_no_wrap(label.into(), font.clone(), egui::Color32::WHITE).size().x);
                    assert!(rect.width() >= text_width + 11.5, "{theme:?}: {label} lacks horizontal padding");
                    assert!(rect.left() >= right && rect.right() <= width, "{theme:?}: {label} overlaps or overflows at width {width}");
                    right = rect.right();
                }

                harness.get_by_label("File").click();
                harness.run_steps(3);
                assert!(harness.query_by_label_contains("Open…").is_some(), "{theme:?}: File menu did not open");
            }
        }
    }

    #[test]
    fn window_panel_and_workspace_ids_drive_the_shell() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        assert!(!app.ui.panels.history);
        invoke(&mut app, &ctx, "window.panel.history", Value::Null).unwrap();
        assert!(app.ui.panels.history);
        assert_eq!(checked(&app, "window.panel.history"), Some(true));
        invoke(&mut app, &ctx, "window.workspace.painting", Value::Null).unwrap();
        assert_eq!(app.ui.workspace, "Painting");
        assert!(!app.ui.panels.properties);
        assert_eq!(checked(&app, "window.workspace.painting"), Some(true));
        invoke(&mut app, &ctx, "window.workspace.essentials", Value::Null).unwrap();
        assert!(app.ui.panels.properties);
        // Live in the menu, and no duplicate "Layers" entry from the window.toggle.* commands.
        let items = menu_items(&app);
        assert!(items.iter().any(|i| i.id == "window.panel.layers" && i.enabled));
        assert_eq!(items.iter().filter(|i| i.path == ["Window"] && i.label == "Layers").count(), 1);
        assert!(items.iter().any(|i| i.id == "edit.stroke"));
    }

    #[test]
    fn proof_setup_presets_and_checkmarks() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.sync_views();
        assert_eq!(checked(&app, "view.proofColors"), Some(false));
        invoke(&mut app, &ctx, "view.proofSetup.workingCmyk", Value::Null).unwrap();
        assert_eq!(checked(&app, "view.proofColors"), Some(true));
        invoke(&mut app, &ctx, "view.gamutWarning", json!({"on": true})).unwrap();
        assert_eq!(checked(&app, "view.gamutWarning"), Some(true));
        let doc = app.session.active().unwrap().doc.clone();
        let lut = app.session.color.canvas_lut(&doc, 9).unwrap().unwrap();
        assert_eq!(lut.len(), 9 * 9 * 9 * 4);
        // Pure sRGB green is outside coated CMYK; mid grey is inside.
        let at = |r: usize, g: usize, b: usize| lut[(r + g * 9 + b * 81) * 4 + 3];
        assert_eq!((at(0, 8, 0), at(4, 4, 4)), (255, 0));
        // Custom… opens the generated Proof Setup dialog with a profile choice.
        assert!(invoke(&mut app, &ctx, "view.proofSetup.custom", Value::Null).unwrap()["dialog"].is_u64());
        let spec = photocraft_engine::commands::find("edit.convertToProfile").unwrap();
        let params = crate::filter_dialog::parse_spec(spec.params);
        assert!(matches!(&params[0].kind, crate::filter_dialog::Kind::Choice(c) if c[0] == "srgb" && !c.iter().any(|v| v.contains('/'))));
    }
}
