//! Golden fixtures for the palettes and for how the saved appearance maps onto egui's two style
//! slots. Regenerate with `UPDATE_GOLDEN=1 cargo test -p photocraft-ui-egui --lib theme::golden`
//! and review the diff.

use super::*;
use crate::PhotocraftApp;
use serde_json::json;
use std::path::PathBuf;

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/theme/golden").join(name)
}

fn check_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}; run with UPDATE_GOLDEN=1", path.display()));
    assert!(expected == actual, "{name} differs from its golden fixture; run with UPDATE_GOLDEN=1 and review the diff");
}

/// The style a kind produces, as installed by `apply` on a fresh context.
fn kind_style(kind: ThemeKind) -> Arc<egui::Style> {
    let ctx = egui::Context::default();
    apply(&ctx, kind);
    ctx.style_of(egui::Theme::Dark)
}

#[test]
fn every_kind_keeps_its_style_and_tokens() {
    for kind in ThemeKind::ALL {
        let ctx = egui::Context::default();
        apply(&ctx, kind);
        let text = format!("{:#?}\n\n{:#?}\n", ctx.style_of(egui::Theme::Dark), Tokens::get(&ctx));
        check_golden(&format!("{}.txt", kind.id()), &text);
    }
}

fn slot_kind(ctx: &egui::Context, theme: egui::Theme) -> String {
    let visuals = ctx.style_of(theme).visuals.clone();
    let found: Vec<_> = ThemeKind::ALL.into_iter().filter(|k| kind_style(*k).visuals == visuals).collect();
    match found.as_slice() {
        [one] => format!("{one:?}"),
        other => format!("{other:?}"),
    }
}

#[test]
fn appearance_modes_fill_the_style_slots_as_recorded() {
    let pairs = [("proMedium", "proLight"), ("studio", "studioLight"), ("solarizedDark", "adwaita")];
    let mut out = String::new();
    for mode in ["auto", "dark", "light"] {
        for os in [Some(egui::Theme::Light), Some(egui::Theme::Dark), None] {
            for (dark, light) in pairs {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                app.run("prefs.set", json!({"values": {"interface.appearanceMode": mode, "interface.darkTheme": dark, "interface.lightTheme": light}}))
                    .unwrap();
                let ctx = egui::Context::default();
                ctx.run_ui(egui::RawInput { system_theme: os, ..Default::default() }, |ui| {
                    PhotocraftApp::setup_context(ui.ctx(), app.ui.theme);
                    crate::prefs_ui::sync_appearance(&mut app, ui.ctx());
                })
                .textures_delta
                .clear();
                out += &format!(
                    "mode={mode} os={os:?} dark={dark} light={light}: light_slot={} dark_slot={} ctx_theme={:?} tokens={:?} ui_theme={:?}\n",
                    slot_kind(&ctx, egui::Theme::Light),
                    slot_kind(&ctx, egui::Theme::Dark),
                    ctx.theme(),
                    Tokens::get(&ctx).kind,
                    app.ui.theme,
                );
            }
        }
    }
    check_golden("appearance_table.txt", &out);
}
