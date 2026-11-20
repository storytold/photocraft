//! CMYK regressions through the framework's tool and hosted picker paths.
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use crate::{PhotocraftApp, Services, color_picker_ui};
use serde_json::json;

fn chart(depth: u32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width":16,"height":8,"mode":"cmyk","depth":depth,"background":"white"})).unwrap();
    app.session
        .edit("CMYK chart", |doc, _| {
            let surface = doc.layers[0].surface_mut().unwrap();
            surface.fill_rect(photocraft_geom::Rect::new(0, 0, 8, 8), &[0.95, 0.1, 0., 0., 1.]);
            surface.fill_rect(photocraft_geom::Rect::new(8, 0, 16, 8), &[0.95, 0.2, 0., 0., 1.]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Eyedropper;
    app.sync_views();
    app
}

#[test]
fn framework_eyedropper_and_picker_preserve_native_inks_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut app = chart(depth);
        for (x, which, mods, role) in [(3., "foreground", egui::Modifiers::NONE, 0), (11., "background", egui::Modifiers::ALT, 1)] {
            let expected = app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(x as i32, 3);
            tool_event(&mut app, ToolEvent::Down { x, y: 3., pressure: 1. }, mods);
            tool_event(&mut app, ToolEvent::Up { x, y: 3. }, mods);
            let inks = app.session.tools.picked_role_cmyk(role).expect("native pick association");
            assert!(inks.iter().zip(&expected).all(|(a, b)| (a - b).abs() < 1e-6), "depth {depth}: {inks:?} vs {expected:?}");
            let picker = color_picker_ui::open(&mut app, which);
            let picked = color_picker_ui::testing::current(&app, picker).unwrap();
            assert!(color_picker_ui::same_inks(color_picker_ui::inks_of(&picked).unwrap(), inks));
            crate::dialogs::testing::cancel(&mut app, picker).unwrap();
        }
    }
}

#[test]
fn framework_dialog_dropper_keeps_native_model_values() {
    let mut app = chart(16);
    let id = color_picker_ui::open(&mut app, "foreground");
    crate::dialogs::canvas_input::arm_dropper(&mut app, id, "foreground".into());
    assert!(crate::dialogs::canvas_input::pick_color(&mut app, [3., 3.]));
    let picked = color_picker_ui::testing::current(&app, id).unwrap();
    let inks = color_picker_ui::inks_of(&picked).unwrap();
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    assert!(color_picker_ui::same_inks(app.session.tools.picked_role_cmyk(0).unwrap(), inks));
    let before = app.session.tools.foreground;
    assert_eq!(&before[..3], &picked.color.to_rgb_array());

    // The hosted Fill colour row must carry the same model through its child picker to edit.fill.
    let (rgb, expected) =
        crate::canvas::sampling::native_color(&app, [11., 3.], app.ui.tool_options.eyedropper.size, craft::color_ui::ui::SampleSource::All).unwrap();
    let fill = crate::fill_ui::open(&mut app);
    crate::dialogs::testing::show(&mut app);
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, fill, "contents", json!("color")).unwrap();
    let color = craft::theme::Srgba::from_rgb_array(rgb);
    let child = color_picker_ui::open_for_request(&mut app, fill, craft::ui::runtime::requests::ColorRequest::new("color", color));
    app.framework
        .dialogs
        .get_mut_by_id::<craft::color_ui::ui::ColorPickerDialog>(child)
        .unwrap()
        .set_color(craft::color_ui::ui::PickedColor::cmyk(color, expected.map(|v| v * 100.)));
    crate::dialogs::testing::confirm(&mut app, child).unwrap();
    crate::dialogs::testing::show(&mut app);
    crate::dialogs::testing::confirm(&mut app, fill).unwrap();
    let pixel = app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(3, 3);
    assert!(pixel.iter().zip(expected).all(|(actual, want)| (actual - want).abs() <= 1. / 65535. + 1e-6), "native Fill inks: {pixel:?} vs {expected:?}");
}
