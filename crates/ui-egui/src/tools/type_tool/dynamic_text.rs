//! The Dynamic Text toggle and its Word Gap / Line Gap rows, shared by the Type options bar and
//! the Properties type section.

use super::defaults::{drag_key, target, target_text};
use super::{PhotocraftApp, Stroke, json};

/// Whether the target type layer is Dynamic Text (its size comes from the box).
pub(super) fn is_fitted(app: &PhotocraftApp) -> bool {
    target_text(app).is_some_and(|t| t.fit.is_some())
}

/// The Dynamic Text toggle: a "T" struck by a lightning bolt, highlighted while the layer is fitted.
fn dynamic_text_button(ui: &mut egui::Ui, on: bool, unavailable: Option<&str>) -> egui::Response {
    let tip = tl!("Dynamic Text");
    let enabled = unavailable.is_none();
    let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let col = crate::icons::button_chrome(ui, r, on, enabled && resp.hovered());
    let col = if enabled { col } else { col.gamma_multiply(0.4) };
    let c = r.center();
    let st = Stroke::new(1.6, col);
    let p = ui.painter();
    // The T, left of centre.
    p.line_segment([egui::pos2(c.x - 8.0, c.y - 6.0), egui::pos2(c.x + 1.0, c.y - 6.0)], st);
    p.line_segment([egui::pos2(c.x - 3.5, c.y - 6.0), egui::pos2(c.x - 3.5, c.y + 6.0)], st);
    // The bolt, right of the T.
    let bolt = [egui::pos2(c.x + 6.5, c.y - 7.0), egui::pos2(c.x + 2.5, c.y + 0.5), egui::pos2(c.x + 6.0, c.y + 0.5), egui::pos2(c.x + 3.0, c.y + 7.0)];
    p.add(egui::Shape::line(bolt.to_vec(), Stroke::new(1.4, col)));
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, tip));
    resp.on_hover_text(unavailable.unwrap_or(tip))
}

/// Why the Dynamic Text toggle can't act on the target, or `None`. A fitted layer can always be
/// released; an unfitted one must exist and pass the fit's own refusal check.
fn dynamic_text_unavailable(app: &PhotocraftApp) -> Option<&'static str> {
    let Some(t) = target_text(app) else { return Some(tl!("Select a type layer")) };
    if t.fit.is_some() {
        return None;
    }
    photocraft_engine::type_fit_cmds::fit_refusal(t).map(|_| tl!("Not available for vertical, right-to-left or path text"))
}

/// The Dynamic Text toggle wired to Convert / Release Dynamic Text on the target layer. The
/// options bar and the Properties type section both show it, as in Photoshop.
pub(super) fn dynamic_text_toggle(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let fitted = is_fitted(app);
    let unavailable = dynamic_text_unavailable(app);
    if dynamic_text_button(ui, fitted, unavailable).clicked()
        && unavailable.is_none()
        && let Some((layer, _)) = target(app)
    {
        // Off puts the text back to its own sizes, as Photoshop's toggle does; the Type menu's
        // Release Dynamic Text keeps the fitted look instead.
        let (cmd, p) = if fitted {
            ("type.releaseDynamicText", json!({ "layer": layer, "keepLook": false }))
        } else {
            ("type.convertToDynamicText", json!({ "layer": layer }))
        };
        if let Err(e) = app.run(cmd, p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Word Gap and Line Gap while the target layer is Dynamic Text. A drag is one history step.
pub(super) fn dynamic_text_gaps(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    use photocraft_doc::text::TextFit;
    // Only the two gaps: the fit's solved lines are not copied every frame.
    let Some((word_gap, line_gap)) = target_text(app).and_then(|t| t.fit.as_ref()).map(|f| (f.word_gap, f.line_gap)) else { return };
    let Some((layer, _)) = target(app) else { return };
    // `slider_row` translates its label, so it gets the English key.
    for (key, label, current, min) in [("wordGap", "Word Gap", word_gap, TextFit::WORD_GAP_MIN), ("lineGap", "Line Gap", line_gap, TextFit::LINE_GAP_MIN)] {
        let mut v = current;
        ui.push_id(("dynamic-text-gap", key), |ui| {
            if crate::widgets::slider_row(ui, label, &mut v, min..=TextFit::GAP_MAX, "%", None).changed() {
                let mut p = json!({"layer": layer, key: v.round()});
                if let Some(k) = drag_key(ui.ctx()) {
                    p["coalesce"] = json!(k);
                }
                if let Err(e) = app.run("type.setDynamicText", p) {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
            }
        });
        ui.add_space(crate::theme::ROW_GAP);
    }
}
