//! File › Automate › PDF Presentation… (#6637): Source Files (Add Open Files, the list in page
//! order, reordered by dragging, with Browse…, Duplicate, Remove and Sort By Name), Output
//! Options (Save As Multi-Page Document or Presentation, Background) and Presentation Options
//! (Advance Every, Loop after Last Page, Transition; only for a presentation). Save… asks where
//! to write the PDF and runs `file.automate.pdfPresentation` with the list.
use egui::{RichText, Sense, Stroke, pos2, vec2};
use photocraft_io::pdf::Transition;
use serde_json::{Map, Value, json};

use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
use crate::{PhotocraftApp, state::DialogKind, theme::Tokens, widgets};

const MARK: &str = "__pdfPresentation";
pub const COMMAND: &str = "file.automate.pdfPresentation";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Open the dialog with an empty list. Returns its id.
pub fn open(app: &mut PhotocraftApp) -> u64 {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("PDF Presentation"));
    f.insert("files".into(), json!([]));
    f.insert("addOpenFiles".into(), json!(false));
    f.insert("saveAs".into(), json!("multiPage"));
    f.insert("background".into(), json!("white"));
    f.insert("advanceOn".into(), json!(true));
    f.insert("advance".into(), json!(DEFAULT_ADVANCE));
    f.insert("loop".into(), json!(false));
    f.insert("transition".into(), json!("none"));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// Advance Every's starting value, in seconds.
const DEFAULT_ADVANCE: f64 = 5.0;
const MAX_ADVANCE: f32 = photocraft_io::pdf::MAX_ADVANCE as f32;

/// Save As Presentation is chosen: the Presentation Options apply.
fn presenting(f: &Map<String, Value>) -> bool {
    f.get("saveAs").and_then(Value::as_str) == Some("presentation")
}

/// `t`'s name in the Transition list, in the UI language.
fn transition_label(t: Transition) -> &'static str {
    match t {
        Transition::None => tl!("None"),
        Transition::BlindsHorizontal => tl!("Blinds Horizontal"),
        Transition::BlindsVertical => tl!("Blinds Vertical"),
        Transition::BoxIn => tl!("Box In"),
        Transition::BoxOut => tl!("Box Out"),
        Transition::Dissolve => tl!("Dissolve"),
        Transition::GlitterDown => tl!("Glitter Down"),
        Transition::GlitterRight => tl!("Glitter Right"),
        Transition::GlitterRightDown => tl!("Glitter Right-Down"),
        Transition::Random => tl!("Random"),
        Transition::SplitHorizontalIn => tl!("Split Horizontal In"),
        Transition::SplitHorizontalOut => tl!("Split Horizontal Out"),
        Transition::SplitVerticalIn => tl!("Split Vertical In"),
        Transition::SplitVerticalOut => tl!("Split Vertical Out"),
        Transition::WipeDown => tl!("Wipe Down"),
        Transition::WipeLeft => tl!("Wipe Left"),
        Transition::WipeRight => tl!("Wipe Right"),
        Transition::WipeUp => tl!("Wipe Up"),
    }
}

fn files(f: &Map<String, Value>) -> Vec<String> {
    f.get("files").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

fn set_files(f: &mut Map<String, Value>, list: Vec<String>) {
    f.insert("files".into(), json!(list));
}

fn selected(f: &Map<String, Value>) -> Option<usize> {
    f.get("selected").and_then(Value::as_u64).map(|i| i as usize).filter(|i| *i < files(f).len())
}

/// Add `paths` to the list, skipping those already in it (Duplicate is how a file goes in twice).
fn add(f: &mut Map<String, Value>, paths: impl IntoIterator<Item = String>) {
    let mut list = files(f);
    for p in paths {
        if !list.contains(&p) {
            list.push(p);
        }
    }
    set_files(f, list);
}

fn file_name(p: &str) -> &str {
    std::path::Path::new(p).file_name().and_then(|n| n.to_str()).unwrap_or(p)
}

/// The open documents that have a file, in tab order. The PDF is made from the files on disk,
/// as last saved.
fn open_files(app: &PhotocraftApp) -> Vec<String> {
    app.session.documents().iter().filter_map(|d| d.path.clone()).collect()
}

/// Where a row dragged from `from` lands when dropped on row `target`'s upper or lower half.
fn drop_index(from: usize, target: usize, after: bool) -> usize {
    let slot = target + usize::from(after);
    if from < slot { slot - 1 } else { slot }
}

/// Move the row at `from` to `to`, keeping it selected.
fn move_row(f: &mut Map<String, Value>, from: usize, to: usize) {
    let mut list = files(f);
    if from >= list.len() || to >= list.len() {
        return;
    }
    let p = list.remove(from);
    list.insert(to, p);
    set_files(f, list);
    f.insert("selected".into(), json!(to));
}

/// The row being dragged, in dialog `dialog`.
#[derive(Clone, Copy)]
struct DragRow {
    dialog: u64,
    row: usize,
}

/// Save… needs at least one page.
pub fn can_confirm(f: &Map<String, Value>) -> bool {
    !owns(f) || !files(f).is_empty()
}

/// Browse…: files to add to dialog `id` once the file dialog answers.
fn browse(app: &mut PhotocraftApp, id: u64) {
    let initial_dir = crate::file_dialog::last_used_dir(&app.ui.recent_files);
    let r = app.ask_file(FileDialogRequest::Open { multiple: true, initial_dir, extensions: None }, move |app, answer| {
        let FileDialogAnswer::Paths(paths) = answer else { return Err("PDF Presentation reads files from disk".into()) };
        let d = app.ui.dialog_mut(id).ok_or("the dialog was closed")?;
        add(&mut d.fields, paths);
        Ok(Value::Null)
    });
    if let Err(e) = r
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status = e;
    }
}

/// The dialog's body (dialog `id`); Save… and Cancel are the standard buttons below it.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, id: u64) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Source Files")).font(crate::theme::semibold(13.0)));
    ui.add_space(4.0);
    let open = open_files(app);
    let mut add_open = f.get("addOpenFiles").and_then(Value::as_bool).unwrap_or(false);
    if ui.add_enabled_ui(!open.is_empty() || add_open, |ui| widgets::checkbox(ui, &mut add_open, tl!("Add Open Files"))).inner.changed() {
        f.insert("addOpenFiles".into(), json!(add_open));
        if add_open {
            add(f, open);
        } else {
            let list = files(f).into_iter().filter(|p| !open.contains(p)).collect();
            set_files(f, list);
            f.remove("selected");
        }
    }
    ui.add_space(6.0);
    let mut browse_now = false;
    ui.horizontal_top(|ui| {
        let list = files(f);
        let sel = selected(f);
        let mut clicked = None;
        let mut moved = None;
        egui::Frame::NONE.fill(t.field).stroke(Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(4.0).show(ui, |ui| {
            ui.set_min_size(vec2(300.0, 180.0));
            ui.set_max_width(300.0);
            egui::ScrollArea::vertical().id_salt("pdf-presentation-files").max_height(180.0).auto_shrink([false, false]).show(ui, |ui| {
                // One row per page, top to bottom (the frame sits in a horizontal row).
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    let row_h = ui.spacing().interact_size.y;
                    for (i, p) in list.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click_and_drag());
                        let on = sel == Some(i);
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, file_name(p)));
                        if on {
                            ui.painter().rect_filled(rect, t.radius_sm, t.accent.gamma_multiply(0.35));
                        }
                        ui.painter().text(
                            pos2(rect.left() + 4.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            file_name(p),
                            egui::FontId::proportional(12.5),
                            t.text,
                        );
                        let resp = resp.on_hover_text(p.as_str());
                        if resp.clicked() {
                            clicked = Some(i);
                        }
                        resp.dnd_set_drag_payload(DragRow { dialog: id, row: i });
                        if let Some(source) = resp.dnd_hover_payload::<DragRow>()
                            && source.dialog == id
                        {
                            let after = ui.input(|inp| inp.pointer.hover_pos().is_some_and(|pt| pt.y >= rect.center().y));
                            let to = drop_index(source.row, i, after);
                            let y = if after { rect.bottom() } else { rect.top() };
                            ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(2.0, t.accent));
                            if resp.dnd_release_payload::<DragRow>().is_some() && source.row != to {
                                moved = Some((source.row, to));
                            }
                        }
                    }
                });
            });
        });
        if let Some(i) = clicked {
            f.insert("selected".into(), json!(i));
        }
        if let Some((from, to)) = moved {
            move_row(f, from, to);
        }
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            if widgets::secondary_button(ui, tl!("Browse…"), 120.0).clicked() {
                browse_now = true;
            }
            let sel = selected(f);
            if ui.add_enabled_ui(sel.is_some(), |ui| widgets::secondary_button(ui, tl!("Duplicate"), 120.0)).inner.clicked()
                && let Some(i) = sel
                && let Some(p) = files(f).get(i).cloned()
            {
                let mut list = files(f);
                list.insert(i + 1, p);
                set_files(f, list);
                f.insert("selected".into(), json!(i + 1));
            }
            if ui.add_enabled_ui(sel.is_some(), |ui| widgets::secondary_button(ui, tl!("Remove"), 120.0)).inner.clicked()
                && let Some(i) = sel
            {
                let mut list = files(f);
                list.remove(i);
                set_files(f, list);
                f.remove("selected");
            }
            if ui.add_enabled_ui(files(f).len() > 1, |ui| widgets::secondary_button(ui, tl!("Sort By Name"), 120.0)).inner.clicked() {
                let mut list = files(f);
                list.sort_by_key(|p| file_name(p).to_lowercase());
                set_files(f, list);
                f.remove("selected");
            }
        });
    });
    // The pages come from the files on disk: say so when an open one has changes not saved yet.
    let listed = files(f);
    let unsaved: Vec<&str> = app
        .session
        .documents()
        .iter()
        .filter(|d| d.is_dirty())
        .filter_map(|d| d.path.as_deref())
        .filter(|p| listed.iter().any(|l| l == p))
        .map(file_name)
        .collect();
    if !unsaved.is_empty() {
        ui.add_space(4.0);
        let text = crate::i18n::fmt(tl!("Uses the saved file: save {name} first to include its latest changes"), &[("name", &unsaved.join(", "))]);
        ui.label(RichText::new(text).color(t.warning).size(11.5));
    }
    ui.add_space(10.0);
    ui.label(RichText::new(tl!("Output Options")).font(crate::theme::semibold(13.0)));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Save As"));
        let mut save_as = f.get("saveAs").and_then(Value::as_str).unwrap_or("multiPage").to_string();
        ui.radio_value(&mut save_as, "multiPage".to_string(), tl!("Multi-Page Document"));
        ui.radio_value(&mut save_as, "presentation".to_string(), tl!("Presentation"));
        f.insert("saveAs".into(), json!(save_as));
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Background"));
        let mut bg = f.get("background").and_then(Value::as_str).unwrap_or("white").to_string();
        let options = [("white".to_string(), tl!("White")), ("gray".to_string(), tl!("Gray")), ("black".to_string(), tl!("Black"))];
        if widgets::dropdown(ui, "pdf-presentation-background", tl!("Background"), &mut bg, &options, 120.0) {
            f.insert("background".into(), json!(bg));
        }
    });
    ui.add_space(10.0);
    let show = presenting(f);
    ui.add_enabled_ui(show, |ui| {
        ui.label(RichText::new(tl!("Presentation Options")).font(crate::theme::semibold(13.0)));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let mut on = f.get("advanceOn").and_then(Value::as_bool).unwrap_or(true);
            if widgets::checkbox(ui, &mut on, tl!("Advance Every")).changed() {
                f.insert("advanceOn".into(), json!(on));
            }
            let mut secs = f.get("advance").and_then(Value::as_f64).unwrap_or(DEFAULT_ADVANCE) as f32;
            if ui.add_enabled_ui(on, |ui| widgets::value_field_prec(ui, &mut secs, 0.1..=MAX_ADVANCE, "", 56.0, 1)).inner.changed() {
                f.insert("advance".into(), json!(f64::from(secs)));
            }
            ui.label(tl!("Seconds"));
        });
        let mut looping = f.get("loop").and_then(Value::as_bool).unwrap_or(false);
        // PDF has no setting for it: a one-line script that only Acrobat and Adobe Reader run.
        if widgets::checkbox(ui, &mut looping, tl!("Loop after Last Page")).on_hover_text(tl!("Acrobat and Adobe Reader only")).changed() {
            f.insert("loop".into(), json!(looping));
        }
        ui.horizontal(|ui| {
            ui.label(tl!("Transition"));
            let mut id = f.get("transition").and_then(Value::as_str).unwrap_or("none").to_string();
            let options: Vec<(String, &str)> = Transition::ALL.iter().map(|t| (t.id().to_string(), transition_label(*t))).collect();
            if widgets::dropdown(ui, "pdf-presentation-transition", tl!("Transition"), &mut id, &options, 180.0) {
                f.insert("transition".into(), json!(id));
            }
        });
    });
    if browse_now {
        // The file dialog's answer lands in the dialog's own fields: save this frame's edits first.
        if let Some(d) = app.ui.dialog_mut(id) {
            d.fields = f.clone();
        }
        browse(app, id);
    }
}

/// Save…: ask where to write the PDF, then make it from the list.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let list = files(f);
    if list.is_empty() {
        return Err("Add the files to place".into());
    }
    let background = f.get("background").and_then(Value::as_str).unwrap_or("white");
    let mut params = json!({"files": list, "saveAs": "multiPage", "background": background});
    if presenting(f) {
        params["saveAs"] = json!("presentation");
        if f.get("advanceOn").and_then(Value::as_bool).unwrap_or(true) {
            params["advance"] = json!(f.get("advance").and_then(Value::as_f64).unwrap_or(DEFAULT_ADVANCE));
        }
        params["loop"] = json!(f.get("loop").and_then(Value::as_bool).unwrap_or(false));
        params["transition"] = json!(f.get("transition").and_then(Value::as_str).unwrap_or("none"));
    }
    app.pick_save("Untitled.pdf", move |app, path| {
        params["path"] = json!(path);
        let r = app.run(COMMAND, params);
        match &r {
            Ok(v) => {
                let skipped = v["errors"].as_array().map_or(0, Vec::len);
                if skipped > 0 {
                    app.ui.status = format!("PDF Presentation: {skipped} file(s) could not be read and were left out");
                    app.ui.status_error = true;
                }
            }
            Err(e) => {
                app.ui.status = e.clone();
                app.ui.status_error = true;
            }
        }
        r
    })
}

#[cfg(test)]
#[path = "pdf_presentation_ui_tests.rs"]
mod tests;
