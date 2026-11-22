//! The Measurement Log panel body.

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, IconId, ItemSource, Selection};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Bind, ColumnSpec, Props as _, Show as _, ShowCollection as _};
use craft::ui::list::ListRow;
use craft::ui::runtime::env::Label;
use craft::ui::table::Table;
use photocraft_engine::analysis_cmds;
use serde_json::{Value, json};

use super::export_log;
use crate::PhotocraftApp;

/// One log row as the table shows it: the row id and the text of each column.
struct Row {
    id: u64,
    cells: Vec<(ColumnKey, String)>,
}

/// The rows keyed by their log id.
struct Rows<'a>(&'a [Row]);

impl DisplayOrder<u64> for Rows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn key(&self, at: usize) -> u64 {
        self.0[at].id
    }
    fn index_of(&self, key: &u64) -> Option<usize> {
        self.0.iter().position(|r| r.id == *key)
    }
    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.0.get(at).and_then(|r| r.cells.first()).map(|(_, text)| std::borrow::Cow::Borrowed(text.as_str()))
    }
}

impl ItemSource<u64> for Rows<'_> {}

fn cell_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// The panel body: the actions row over the table of recorded measurements.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let log = &app.session.analysis.log;
    // The column keys are the static ones of `COLUMNS`, which the engine's own column list names.
    let columns: Vec<(&'static str, &'static str)> = analysis_cmds::log_columns(log)
        .into_iter()
        .filter(|c| *c != "histogram")
        .filter_map(|c| analysis_cmds::COLUMNS.iter().find(|x| x.0 == c).map(|x| (x.0, x.1)))
        .collect();
    let rows: Vec<Row> =
        log.iter().map(|r| Row { id: r.id, cells: columns.iter().map(|(key, _)| (ColumnKey::new(key), cell_text(r.values.get(*key)))).collect() }).collect();
    let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();

    ui.horizontal(|ui| {
        if Button::new(Label::from(tl!("Record Measurements"))).show(ui).clicked() {
            let ctx = ui.ctx().clone();
            let _ = crate::menus::invoke(app, &ctx, "image.analysis.recordMeasurements", json!({}));
        }
        if Button::new(Label::from(tl!("Select All"))).show(ui).clicked() {
            app.ui.analysis.log_selected = Selection::new(ids.clone(), ids.last().copied(), ids.first().copied());
        }
        if Button::new(Label::from(tl!("Deselect All"))).show(ui).clicked() {
            app.ui.analysis.log_selected = Selection::default();
        }
        let selected = app.ui.analysis.log_selected.items.clone();
        if IconButton::new(IconId::new("trash")).label(Label::from(tl!("Delete selected measurements"))).enabled(!selected.is_empty()).show(ui).clicked()
            && app.run("measurementLog.delete", json!({"rows": selected})).is_ok()
        {
            app.ui.analysis.log_selected = Selection::default();
        }
        if IconButton::new(IconId::new("file")).label(Label::from(tl!("Export selected measurements (CSV)"))).show(ui).clicked() {
            // PLAN: assumed `export_log(app, rows: Option<Vec<u64>>) -> Result<Value, String>`.
            let chosen = app.ui.analysis.log_selected.items.clone();
            if let Err(e) = export_log(app, (!chosen.is_empty()).then_some(chosen))
                && e != "cancelled"
            {
                app.ui.status = e;
            }
        }
    });

    let specs: Vec<ColumnSpec> = columns.iter().map(|(key, name)| ColumnSpec::new(ColumnKey::new(key), tl!(name), ColumnWidth::Content).resizable()).collect();
    // PLAN: assumed `AnalysisUi::log_selected` is a `Selection<u64>`.
    Table::<u64>::new("Measurement Log", specs).striped(true).selection(Bind::bound(&mut app.ui.analysis.log_selected)).show_items(
        ui,
        &Rows(&rows),
        &mut |key, icx| {
            let mut row = ListRow::new(String::new()).cells(icx.cells()).striped(icx.row.striped);
            if let Some(r) = rows.iter().find(|r| r.id == *key) {
                for (column, text) in &r.cells {
                    row = row.cell(column.clone(), text.clone());
                }
            }
            row
        },
    );
}
