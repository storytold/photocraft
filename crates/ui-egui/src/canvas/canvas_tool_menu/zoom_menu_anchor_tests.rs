use super::*;

#[test]
fn zoom_menu_steps_keep_the_view_centre_and_history() {
    for animated in [false, true] {
        for offset in [vec2(-160.0, -100.0), vec2(140.0, 80.0)] {
            let mut app = zoom_app(2400, 1800);
            app.ui.tool = Tool::Zoom;
            app.run("prefs.set", json!({"values": {"general.animatedZoom": animated, "tools.zoomClickedPointToCenter": false}})).unwrap();
            let mut h = harness(app);
            let v = &mut h.state_mut().ui.views[0];
            v.zoom = 1.0;
            v.center = [928.0, 686.0];
            v.request_zoom_animation(false);
            h.step();
            let center = h.state().ui.views[0].center;
            let st = h.state().session.active().unwrap();
            let history = (st.history.past_len(), st.history.can_redo(), st.revision);
            for (label, zoom) in [("Zoom In", 2.0), ("Zoom Out", 1.0), ("Zoom Out", 2.0 / 3.0)] {
                let p = crate::canvas::ViewXform::active(h.state()).unwrap().rect.center() + offset;
                right_click(&mut h, p, Modifiers::NONE);
                h.get_by_label(label).click();
                h.run_steps(3);
                let v = &h.state().ui.views[0];
                assert_eq!(v.zoom, zoom, "{label}: one ladder stop");
                assert_eq!(v.center, center, "{label}: the right-click must not move the view centre");
                let xf = crate::canvas::ViewXform::active(h.state()).unwrap();
                assert!(xf.to_screen(center[0], center[1]).distance(xf.rect.center()) < 1.0);
                assert!(h.state().ui.canvas_tool_menu.is_none());
                let st = h.state().session.active().unwrap();
                assert_eq!((st.history.past_len(), st.history.can_redo(), st.revision), history);
            }
        }
    }
}
