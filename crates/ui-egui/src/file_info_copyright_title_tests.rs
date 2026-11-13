//! #13821: a document File Info marks Copyrighted shows "© " before its name in the document tab
//! and the floating document window's title; Public Domain and Unknown show no prefix.

use serde_json::json;

use crate::PhotocraftApp;

use super::{pro_tab_title, titled_name};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "name": "oracle"})).unwrap();
    app
}

fn title(app: &PhotocraftApp) -> String {
    pro_tab_title(app, 0)
}

fn status(app: &mut PhotocraftApp, status: &str) {
    app.run("file.fileInfo", json!({"copyrightStatus": status})).unwrap();
}

#[test]
fn copyrighted_puts_the_copyright_sign_before_the_name() {
    let mut app = app();
    assert_eq!(title(&app), "oracle @ 100% (RGB/8)");
    status(&mut app, "copyrighted");
    assert_eq!(title(&app), "\u{a9} oracle @ 100% (RGB/8)*");
    assert_eq!(titled_name(&app.session.documents()[0]), "\u{a9} oracle", "the floating window's title has it too");
}

#[test]
fn public_domain_and_unknown_show_no_prefix() {
    let mut app = app();
    status(&mut app, "copyrighted");
    status(&mut app, "publicDomain");
    assert_eq!(title(&app), "oracle @ 100% (RGB/8)*");
    status(&mut app, "copyrighted");
    status(&mut app, "unknown");
    assert_eq!(title(&app), "oracle @ 100% (RGB/8)*");
    assert_eq!(titled_name(&app.session.documents()[0]), "oracle");
}

#[test]
fn undo_and_redo_take_the_prefix_with_them() {
    let mut app = app();
    status(&mut app, "copyrighted");
    assert!(app.session.undo());
    assert!(title(&app).starts_with("oracle @ "), "{}", title(&app));
    assert!(app.session.redo());
    assert!(title(&app).starts_with("\u{a9} oracle @ "), "{}", title(&app));
}

#[test]
fn a_file_opened_with_the_mark_already_set_shows_the_prefix() {
    let packets = [
        // Element form, as Photoshop writes it.
        (
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/"><xmpRights:Marked> True </xmpRights:Marked></rdf:Description></rdf:RDF></x:xmpmeta>"#,
            true,
        ),
        // Attribute form.
        (r#"<rdf:Description xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/" xmpRights:Marked="True"/>"#, true),
        (r#"<rdf:Description xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/" xmpRights:Marked="False"/>"#, false),
        (r#"<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/"/>"#, false),
        // Single quotes, and the name on its own line or after a tab, with space around the '='.
        ("<rdf:Description xmlns:xmpRights='http://ns.adobe.com/xap/1.0/rights/' xmpRights:Marked='True'/>", true),
        ("<rdf:Description\n   xmpRights:Marked=\"True\"/>", true),
        ("<rdf:Description\txmpRights:Marked = 'True'/>", true),
        ("<rdf:Description\n   xmpRights:Marked='False'/>", false),
        // Not an attribute: a longer name, or the name with no '='.
        ("<rdf:Description xmpRights:MarkedX=\"True\"/>", false),
        ("<rdf:Description xmpRights:Marked/>", false),
    ];
    for (xmp, marked) in packets {
        let mut app = app();
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).metadata.xmp = Some(xmp.into());
        let want = if marked { "\u{a9} oracle @ 100% (RGB/8)" } else { "oracle @ 100% (RGB/8)" };
        assert_eq!(title(&app), want, "{xmp}");
    }
}
