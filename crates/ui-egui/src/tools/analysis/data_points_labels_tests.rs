//! #16345: Image › Analysis › Select Data Points draws every check box with its data point's name, and each column's
//! title above its boxes.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::analysis_cmds;
use serde_json::json;

use super::sources;
use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

fn open_dialog() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).with_context(app.ctx().clone()).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "image.analysis.selectDataPoints", json!({})).unwrap();
    h.run_steps(6);
    h
}

/// The shown name of data point `column`, as the engine lists it.
fn name(column: &str) -> &str {
    analysis_cmds::COLUMNS.iter().find(|c| c.0 == column).map_or(column, |c| c.1)
}

/// A check box of the dialog: where it is, its name, and where the text that names it is drawn.
struct Shown {
    rect: egui::Rect,
    name: String,
    text: Option<egui::Rect>,
}

fn rect(r: egui::accesskit::Rect) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(r.x0 as f32, r.y0 as f32), egui::pos2(r.x1 as f32, r.y1 as f32))
}

#[test]
fn every_check_box_draws_its_data_point_name_and_each_column_its_title_above() {
    let h = open_dialog();
    let mut boxes: Vec<Shown> = h
        .query_all_by_role(Role::CheckBox)
        .map(|b| {
            let node = b.accesskit_node();
            let name = node.label().unwrap_or_default();
            let text =
                node.labelled_by().find(|l| l.role() == Role::Label && l.value().as_deref() == Some(name.as_str())).and_then(|l| l.bounding_box()).map(rect);
            Shown { rect: b.rect(), name, text }
        })
        .collect();
    // In columns by their left edge, each column top to bottom.
    boxes.sort_by(|a, b| a.rect.left().total_cmp(&b.rect.left()).then(a.rect.top().total_cmp(&b.rect.top())));
    let mut columns: Vec<Vec<Shown>> = Vec::new();
    for shown in boxes {
        match columns.last_mut() {
            Some(column) if (column[0].rect.left() - shown.rect.left()).abs() < 0.5 => column.push(shown),
            _ => columns.push(vec![shown]),
        }
    }
    let names: Vec<Vec<&str>> = columns.iter().map(|c| c.iter().map(|s| s.name.as_str()).collect()).collect();
    assert_eq!(columns.len(), sources().len(), "one column of boxes per source: {names:?}");
    for (((source, title), column), shown) in sources().into_iter().zip(&columns).zip(&names) {
        let want: Vec<&str> = analysis_cmds::available(source).into_iter().map(name).collect();
        assert_eq!(shown, &want, "{source}: each box is named after its data point, in order");
        // Each box draws its name: the text that names it stands after the box, on its line.
        for b in column {
            let text = b.text.unwrap_or_else(|| panic!("{source}: {:?} draws no name", b.name));
            assert!(
                text.left() >= b.rect.right() - 0.5 && text.top() < b.rect.bottom() && text.bottom() > b.rect.top(),
                "{source}: {:?} is named beside its box",
                b.name
            );
        }
        // The title sits over its column's first box, under the dialog's "Data Points:" rather than on its line.
        let heading = h.query_by_label("Data Points:").expect("\"Data Points:\" is drawn").rect();
        let first = column[0].rect;
        let title = h
            .query_all_by_label(title)
            .find(|n| n.accesskit_node().role() != Role::CheckBox && n.rect().left() <= first.center().x && n.rect().right() >= first.center().x)
            .unwrap_or_else(|| panic!("{title} heads its column"))
            .rect();
        assert!(title.bottom() <= first.top() + 0.5, "{source}: the title {title:?} is above the first box {first:?}");
        assert!(heading.bottom() <= title.top() + 0.5, "{source}: \"Data Points:\" {heading:?} is above the title {title:?}");
    }
}
