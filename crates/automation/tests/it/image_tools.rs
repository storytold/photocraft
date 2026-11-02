//! The task-shaped image tools over a real MCP client/server pair (tokio duplex): protocol
//! shape, pixel effects, the overwrite policy, sandboxed paths and batch processing.

use photocraft_automation::{AuthorizedWorkspace, PhotocraftMcp};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

type Conn = RunningService<RoleClient, Client>;

async fn connect(server: PhotocraftMcp) -> Conn {
    let (s, c) = tokio::io::duplex(1 << 22);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(s).await {
            let _ = running.waiting().await;
        }
    });
    Client.serve(c).await.expect("client init")
}

async fn raw(client: &Conn, name: &str, args: Value) -> Result<CallToolResult, rmcp::ServiceError> {
    let mut p = CallToolRequestParams::new(name.to_owned());
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    client.call_tool(p).await
}

async fn call(client: &Conn, name: &str, args: Value) -> CallToolResult {
    raw(client, name, args).await.expect("call_tool transport")
}

fn text(r: &CallToolResult) -> String {
    r.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n")
}

/// The structured reply of a successful task tool.
fn ok(r: &CallToolResult) -> Value {
    assert_ne!(r.is_error, Some(true), "tool error: {}", text(r));
    let v = r.structured_content.clone().unwrap_or_else(|| panic!("no structuredContent: {}", text(r)));
    assert_eq!(v["ok"], true, "{v}");
    // The text block mirrors the structured content for clients that only read text.
    assert_eq!(serde_json::from_str::<Value>(&text(r)).unwrap(), v);
    v
}

/// The error message of a failed task tool.
fn failed(r: &CallToolResult) -> String {
    assert_eq!(r.is_error, Some(true), "expected an error, got {}", text(r));
    let v = r.structured_content.clone().expect("structured error");
    assert_eq!(v["ok"], false);
    v["error"].as_str().unwrap().to_owned()
}

/// Plain JSON reply of a generic (non-task) tool.
fn generic(r: &CallToolResult) -> Value {
    assert_ne!(r.is_error, Some(true), "tool error: {}", text(r));
    serde_json::from_str(&text(r)).unwrap()
}

struct Root(std::path::PathBuf);
impl Root {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("pc-task-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Root(d)
    }
    fn server(&self) -> PhotocraftMcp {
        PhotocraftMcp::headless_with_workspace(AuthorizedWorkspace::new(Some(&self.0), Some(&self.0)).unwrap())
    }
    fn path(&self, rel: &str) -> std::path::PathBuf {
        self.0.join(rel)
    }
    fn decode(&self, rel: &str) -> photocraft_codecs::Image {
        photocraft_codecs::decode(&std::fs::read(self.path(rel)).unwrap()).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rgb(img: &photocraft_codecs::Image, x: u32, y: u32) -> [f32; 3] {
    [img.get(x, y, 0), img.get(x, y, 1), img.get(x, y, 2)]
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.03)
}

/// New white document, with its left `left` columns filled with `color`.
async fn two_tone(c: &Conn, w: u32, h: u32, left: u32, color: &str) {
    generic(&call(c, "doc_new", json!({"width": w, "height": h, "background": "white"})).await);
    for (id, params) in [
        ("select.rect", json!({"x": 0, "y": 0, "width": left, "height": h, "antiAlias": false})),
        ("edit.fill", json!({"contents": "color", "color": color})),
        ("select.deselect", json!({})),
    ] {
        generic(&call(c, "command_run", json!({"id": id, "params": params})).await);
    }
}

const TASK_TOOLS: [&str; 17] = [
    "open_image",
    "inspect_image",
    "save_image",
    "export_image",
    "resize_image",
    "crop_image",
    "rotate_image",
    "flip_image",
    "adjust_brightness",
    "adjust_contrast",
    "adjust_saturation",
    "apply_blur",
    "apply_sharpen",
    "remove_background",
    "add_text",
    "manage_layers",
    "batch_process",
];

#[tokio::test(flavor = "multi_thread")]
async fn task_tools_are_listed_with_strict_schemas_and_hints() {
    let c = connect(PhotocraftMcp::headless()).await;
    let tools = c.list_all_tools().await.unwrap();
    for name in TASK_TOOLS {
        let t = tools.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("missing tool {name}"));
        assert!(t.description.as_ref().is_some_and(|d| d.len() > 20), "{name} description");
        assert_eq!(t.input_schema.get("additionalProperties"), Some(&json!(false)), "{name} schema");
        assert!(!serde_json::to_string(&t.input_schema).unwrap().contains("$ref"), "{name} schema has unresolved refs");
        let a = t.annotations.as_ref().expect("annotations");
        let read_only = a.read_only_hint == Some(true);
        assert_eq!(read_only, name == "inspect_image", "{name} readOnlyHint");
        assert_eq!(a.open_world_hint, Some(false), "{name} openWorldHint");
    }
    // The generic tools are still there.
    for name in ["command_run", "command_batch", "doc_open", "doc_save"] {
        assert!(tools.iter().any(|t| t.name == name), "{name}");
    }
    let info = c.peer_info().unwrap();
    assert!(info.instructions.as_ref().unwrap().contains("batch_process"));
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_or_ill_typed_arguments_are_protocol_errors() {
    let c = connect(PhotocraftMcp::headless()).await;
    let e = raw(&c, "resize_image", json!({"widht": 10})).await.expect_err("unknown key");
    assert!(e.to_string().contains("widht"), "{e}");
    // A known key with a bad value is a tool error naming the accepted values (rmcp's rule for
    // every tool).
    let r = call(&c, "flip_image", json!({"direction": "diagonal"})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("expected `horizontal` or `vertical`"), "{}", text(&r));
    // A tool error, not a crash, when there is nothing to edit; the session keeps serving.
    let msg = failed(&call(&c, "resize_image", json!({"width": 10})).await);
    assert!(msg.contains("no document open"), "{msg}");
    generic(&call(&c, "session_list", json!({})).await);
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn geometry_tools_change_size_and_pixels() {
    let root = Root::new("geometry");
    let c = connect(root.server()).await;
    two_tone(&c, 40, 20, 20, "#ff0000").await;

    let v = ok(&call(&c, "flip_image", json!({"direction": "horizontal"})).await);
    assert_eq!(v["document"]["width"], 40);
    ok(&call(&c, "export_image", json!({"path": "flipped.png"})).await);
    let img = root.decode("flipped.png");
    assert!(near(rgb(&img, 2, 10), [1.0, 1.0, 1.0]), "left is white after the flip: {:?}", rgb(&img, 2, 10));
    assert!(near(rgb(&img, 37, 10), [1.0, 0.0, 0.0]), "right is red after the flip: {:?}", rgb(&img, 37, 10));

    let v = ok(&call(&c, "resize_image", json!({"width": 20})).await);
    assert_eq!((v["document"]["width"].as_u64(), v["document"]["height"].as_u64()), (Some(20), Some(10)));
    let v = ok(&call(&c, "resize_image", json!({"width": 100, "height": 100, "fit": true, "enlarge": true})).await);
    assert_eq!((v["document"]["width"].as_u64(), v["document"]["height"].as_u64()), (Some(100), Some(50)));
    let v = ok(&call(&c, "rotate_image", json!({"angle": 90})).await);
    assert_eq!((v["document"]["width"].as_u64(), v["document"]["height"].as_u64()), (Some(50), Some(100)));
    let v = ok(&call(&c, "rotate_image", json!({"angle": -30, "interpolation": "bilinear"})).await);
    assert!(v["document"]["width"].as_u64().unwrap() > 50, "an arbitrary rotation grows the canvas: {v}");
    let v = ok(&call(&c, "crop_image", json!({"x": 5, "y": 5, "width": 30, "height": 40})).await);
    assert_eq!((v["document"]["width"].as_u64(), v["document"]["height"].as_u64()), (Some(30), Some(40)));
    let msg = failed(&call(&c, "crop_image", json!({"x": 500, "y": 0, "width": 5, "height": 5})).await);
    assert!(msg.contains("outside"), "{msg}");
    // Every edit is undoable through the ordinary history.
    let d = generic(&call(&c, "doc_inspect", json!({})).await);
    assert!(d["history"].to_string().contains("Crop") || d["history"].as_object().is_some(), "{d}");
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn adjustments_and_filters_change_pixels() {
    let root = Root::new("adjust");
    let c = connect(root.server()).await;

    // Saturation -100 turns red grey.
    two_tone(&c, 20, 10, 20, "#ff0000").await;
    ok(&call(&c, "adjust_saturation", json!({"amount": -100})).await);
    ok(&call(&c, "export_image", json!({"path": "grey.png"})).await);
    let [r, g, b] = rgb(&root.decode("grey.png"), 5, 5);
    assert!((r - g).abs() < 0.03 && (g - b).abs() < 0.03, "desaturated: {r} {g} {b}");

    // Brightness raises a mid grey; contrast moves it.
    two_tone(&c, 20, 10, 20, "#808080").await;
    ok(&call(&c, "adjust_brightness", json!({"amount": 80})).await);
    ok(&call(&c, "export_image", json!({"path": "bright.png"})).await);
    let brighter = rgb(&root.decode("bright.png"), 5, 5)[0];
    assert!(brighter > 0.55, "brighter than 0.5: {brighter}");
    ok(&call(&c, "adjust_contrast", json!({"amount": 60})).await);
    let msg = failed(&call(&c, "adjust_brightness", json!({"amount": 500})).await);
    assert!(msg.contains("between -150 and 150"), "{msg}");

    // A blur mixes the two halves at the edge; sharpening runs.
    two_tone(&c, 40, 10, 20, "#000000").await;
    ok(&call(&c, "apply_blur", json!({"kind": "gaussian", "radius": 4})).await);
    ok(&call(&c, "export_image", json!({"path": "blur.png"})).await);
    let edge = rgb(&root.decode("blur.png"), 20, 5)[0];
    assert!(edge > 0.1 && edge < 0.9, "the edge is mixed: {edge}");
    ok(&call(&c, "apply_sharpen", json!({"kind": "unsharp_mask", "amount": 120, "radius": 2})).await);
    ok(&call(&c, "apply_sharpen", json!({"kind": "smart"})).await);
    ok(&call(&c, "apply_blur", json!({"kind": "motion", "angle": 30, "distance": 5})).await);
    let msg = failed(&call(&c, "apply_blur", json!({"kind": "motion", "radius": 5})).await);
    assert!(msg.contains("does not apply"), "{msg}");
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn text_and_layer_management() {
    let root = Root::new("layers");
    let c = connect(root.server()).await;
    generic(&call(&c, "doc_new", json!({"width": 200, "height": 100, "background": "white"})).await);
    let v = ok(&call(&c, "add_text", json!({"text": "Hello", "x": 10, "y": 60, "size": 36, "color": "#112233", "name": "Title"})).await);
    assert_eq!(v["document"]["layerCount"], 2, "{v}");

    let list = ok(&call(&c, "manage_layers", json!({"action": "list"})).await);
    let layers = list["document"]["layers"].as_array().unwrap();
    assert!(layers.iter().any(|l| l["name"] == "Title" && l["kind"].as_str().is_some_and(|k| k.contains("Text") || k.contains("Type"))), "{list}");

    let v = ok(&call(&c, "manage_layers", json!({"action": "set_props", "layer": "Title", "opacity": 50, "visible": false})).await);
    let title = v["document"]["layers"].as_array().unwrap().iter().find(|l| l["name"] == "Title").unwrap().clone();
    assert_eq!(title["visible"], false);
    assert!((title["opacity"].as_f64().unwrap() - 0.5).abs() < 0.01, "{title}");

    ok(&call(&c, "manage_layers", json!({"action": "duplicate", "layer": "Title", "name": "Copy"})).await);
    let v = ok(&call(&c, "manage_layers", json!({"action": "rename", "layer": "Copy", "name": "Second"})).await);
    assert!(v["document"]["layers"].to_string().contains("Second"));
    let v = ok(&call(&c, "manage_layers", json!({"action": "new", "name": "Paint", "opacity": 80, "blend": "Multiply"})).await);
    assert_eq!(v["document"]["layerCount"], 4, "{v}");
    let v = ok(&call(&c, "manage_layers", json!({"action": "delete", "layer": "Paint"})).await);
    assert_eq!(v["document"]["layerCount"], 3);
    ok(&call(&c, "manage_layers", json!({"action": "send_to_back", "layer": "Second"})).await);

    let msg = failed(&call(&c, "manage_layers", json!({"action": "rename", "layer": "Nope", "name": "x"})).await);
    assert!(msg.contains("no layer named"), "{msg}");
    let msg = failed(&call(&c, "manage_layers", json!({"action": "set_props", "layer": "Title", "opacity": 150})).await);
    assert!(msg.contains("0 to 100"), "{msg}");
    let msg = failed(&call(&c, "add_text", json!({"text": "x", "x": 0, "y": 0, "color": "blue"})).await);
    assert!(msg.contains("#rrggbb"), "{msg}");
    // A filter on the text layer says how to reach pixels instead of only refusing.
    ok(&call(&c, "manage_layers", json!({"action": "select", "layer": "Title"})).await);
    let msg = failed(&call(&c, "apply_blur", json!({"radius": 2})).await);
    assert!(msg.contains("pixel layer") && msg.contains("manage_layers"), "{msg}");

    let v = ok(&call(&c, "manage_layers", json!({"action": "flatten"})).await);
    assert_eq!(v["document"]["layerCount"], 1);
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_background_isolates_a_subject() {
    let root = Root::new("cutout");
    let c = connect(root.server()).await;
    generic(&call(&c, "doc_new", json!({"width": 120, "height": 120, "background": "white"})).await);
    for (id, params) in [
        ("select.rect", json!({"x": 35, "y": 30, "width": 50, "height": 60, "ellipse": true})),
        ("edit.fill", json!({"contents": "color", "color": "#c03020"})),
        ("select.deselect", json!({})),
    ] {
        generic(&call(&c, "command_run", json!({"id": id, "params": params})).await);
    }
    let v = ok(&call(&c, "remove_background", json!({})).await);
    assert!(v["result"]["subjectLayer"].is_u64(), "{v}");
    ok(&call(&c, "export_image", json!({"path": "cut.png"})).await);
    let img = root.decode("cut.png");
    assert_eq!(img.layout().channels(), 4, "the cutout keeps transparency");
    assert!(img.get(2, 2, 3) < 0.1, "the corner is transparent: {}", img.get(2, 2, 3));
    assert!(img.get(60, 60, 3) > 0.9, "the subject stays opaque: {}", img.get(60, 60, 3));

    // Trimmed onto a solid background.
    let v = ok(&call(&c, "remove_background", json!({"trim": true, "background_color": "#00ff00"})).await);
    let w = v["document"]["width"].as_u64().unwrap();
    assert!(w < 120, "trimmed to the subject: {v}");
    let msg = failed(&call(&c, "remove_background", json!({"background_color": "green"})).await);
    assert!(msg.contains("#rrggbb"), "{msg}");
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn save_and_export_never_replace_without_overwrite() {
    let root = Root::new("overwrite");
    let c = connect(root.server()).await;
    two_tone(&c, 8, 8, 4, "#0000ff").await;
    let v = ok(&call(&c, "save_image", json!({"path": "doc.pcraft"})).await);
    assert!(v["path"].as_str().unwrap().ends_with("doc.pcraft"));
    ok(&call(&c, "export_image", json!({"path": "a.png"})).await);
    let before = std::fs::read(root.path("a.png")).unwrap();
    // Missing folders below the write root are created.
    ok(&call(&c, "export_image", json!({"path": "new/nested/dir/b.webp"})).await);
    assert_eq!(root.decode("new/nested/dir/b.webp").width(), 8);

    let msg = failed(&call(&c, "export_image", json!({"path": "a.png", "format": "png"})).await);
    assert!(msg.contains("already exists") && msg.contains("overwrite"), "{msg}");
    let msg = failed(&call(&c, "save_image", json!({"path": "doc.pcraft"})).await);
    assert!(msg.contains("already exists"), "{msg}");
    assert_eq!(std::fs::read(root.path("a.png")).unwrap(), before, "the refused export left the file alone");

    ok(&call(&c, "flip_image", json!({"direction": "vertical"})).await);
    ok(&call(&c, "export_image", json!({"path": "a.png", "overwrite": true})).await);
    // A JPEG with a quality.
    ok(&call(&c, "export_image", json!({"path": "a.jpg", "quality": 70})).await);
    assert!(root.decode("a.jpg").width() == 8);
    let msg = failed(&call(&c, "export_image", json!({"path": "b.jpg", "quality": 0})).await);
    assert!(msg.contains("quality"), "{msg}");

    // Even with overwrite, a non-image file is never replaced (the workspace rule).
    std::fs::write(root.path("notes.txt"), b"keep me").unwrap();
    let msg = failed(&call(&c, "export_image", json!({"path": "notes.txt", "format": "png", "overwrite": true})).await);
    assert!(msg.contains("refused"), "{msg}");
    assert_eq!(std::fs::read(root.path("notes.txt")).unwrap(), b"keep me");
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn paths_stay_inside_the_granted_roots() {
    let outer = Root::new("outer");
    let root_dir = outer.path("root");
    std::fs::create_dir_all(&root_dir).unwrap();
    let server = PhotocraftMcp::headless_with_workspace(AuthorizedWorkspace::new(Some(&root_dir), Some(&root_dir)).unwrap());
    let c = connect(server).await;
    std::fs::write(outer.path("secret.png"), b"not for the agent").unwrap();
    two_tone(&c, 4, 4, 2, "#ff0000").await;

    for bad in ["../secret.png", "a/../../secret.png", "..\\secret.png", "/etc/passwd", outer.path("secret.png").to_str().unwrap()] {
        let msg = failed(&call(&c, "open_image", json!({"path": bad})).await);
        assert!(!msg.is_empty(), "{bad}");
        let msg = failed(&call(&c, "inspect_image", json!({"path": bad})).await);
        assert!(!msg.is_empty(), "{bad}");
    }
    for bad in ["../escape.png", "sub/../../escape.png", outer.path("escape.png").to_str().unwrap()] {
        let msg = failed(&call(&c, "export_image", json!({"path": bad, "overwrite": true})).await);
        assert!(!msg.is_empty(), "{bad}");
    }
    assert!(!outer.path("escape.png").exists(), "nothing was written outside the root");
    let msg = failed(&call(&c, "batch_process", json!({"input_dir": "..", "output_dir": "out"})).await);
    assert!(!msg.is_empty());
    let msg = failed(&call(&c, "batch_process", json!({"input_dir": ".", "output_dir": "../out"})).await);
    assert!(!msg.is_empty());
    assert!(!outer.path("out").exists());

    // A link inside the root that points out of it is refused too.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outer.path("secret.png"), root_dir.join("link.png")).unwrap();
        let msg = failed(&call(&c, "open_image", json!({"path": "link.png"})).await);
        assert!(!msg.is_empty());
    }
    // The root itself, as `.` or by its absolute path, is a valid batch folder.
    ok(&call(&c, "export_image", json!({"path": "src.png"})).await);
    let v = ok(&call(&c, "batch_process", json!({"input_dir": ".", "output_dir": "conv", "format": "webp"})).await);
    assert_eq!(v["processed"], 1, "{v}");
    let v = ok(&call(&c, "batch_process", json!({"input_dir": root_dir.to_str().unwrap(), "output_dir": root_dir.to_str().unwrap(), "format": "bmp"})).await);
    assert_eq!(v["processed"], 1, "{v}");
    assert!(root_dir.join("src.bmp").exists());
    // An absolute path inside the root is fine.
    let inside = root_dir.join("inside.png");
    ok(&call(&c, "export_image", json!({"path": inside.to_str().unwrap()})).await);
    assert!(inside.exists());
    c.cancel().await.unwrap();

    // Without any granted root, file tools fail closed.
    let c = connect(PhotocraftMcp::headless()).await;
    let msg = failed(&call(&c, "open_image", json!({"path": "x.png"})).await);
    assert!(msg.contains("not granted"), "{msg}");
    generic(&call(&c, "doc_new", json!({"width": 4, "height": 4})).await);
    let msg = failed(&call(&c, "export_image", json!({"path": "x.png"})).await);
    assert!(msg.contains("not granted"), "{msg}");
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn inspect_image_leaves_the_session_alone() {
    let root = Root::new("inspect");
    let c = connect(root.server()).await;
    two_tone(&c, 30, 12, 10, "#00ff00").await;
    ok(&call(&c, "export_image", json!({"path": "in.png"})).await);
    let before = generic(&call(&c, "session_list", json!({})).await);

    let v = ok(&call(&c, "inspect_image", json!({"path": "in.png"})).await);
    assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(30), Some(12)));
    assert_eq!(v["file"], "in.png");
    assert!(v["layers"].as_array().is_some_and(|l| !l.is_empty()));
    assert_eq!(generic(&call(&c, "session_list", json!({})).await), before, "no document was left open");

    let v = ok(&call(&c, "open_image", json!({"path": "in.png"})).await);
    assert_eq!(v["document"]["width"], 30);
    let v = ok(&call(&c, "inspect_image", json!({})).await);
    assert_eq!(v["width"], 30);
    let msg = failed(&call(&c, "inspect_image", json!({"path": "in.png", "index": 0})).await);
    assert!(msg.contains("not both"), "{msg}");
    let msg = failed(&call(&c, "inspect_image", json!({"path": "missing.png"})).await);
    assert!(!msg.is_empty());
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_process_converts_a_folder_one_file_at_a_time() {
    let root = Root::new("batch");
    std::fs::create_dir_all(root.path("in")).unwrap();
    let c = connect(root.server()).await;
    for (i, color) in ["#ff0000", "#00ff00", "#0000ff"].iter().enumerate() {
        two_tone(&c, 40, 20, 20, color).await;
        ok(&call(&c, "export_image", json!({"path": format!("in/img{i}.png")})).await);
        generic(&call(&c, "doc_close", json!({})).await);
    }
    std::fs::write(root.path("in/readme.txt"), b"skip me").unwrap();
    std::fs::write(root.path("in/broken.png"), b"not a png").unwrap();
    let sessions = generic(&call(&c, "session_list", json!({})).await);

    let args = json!({
        "input_dir": "in",
        "output_dir": "out/small",
        "operations": [{"tool": "resize_image", "width": 10}, {"tool": "flip_image", "direction": "horizontal"}],
        "format": "webp",
        "suffix": "_s",
    });
    let v = ok(&call(&c, "batch_process", args.clone()).await);
    assert_eq!((v["processed"].as_u64(), v["failed"].as_u64(), v["skipped"].as_u64()), (Some(3), Some(1), Some(0)), "{v}");
    let broken = v["files"].as_array().unwrap().iter().find(|f| f["input"] == "in/broken.png").unwrap();
    assert_eq!(broken["status"], "failed");
    for i in 0..3 {
        let img = root.decode(&format!("out/small/img{i}_s.webp"));
        assert_eq!(img.dimensions(), (10, 5));
    }
    assert!(!root.path("out/small/readme_s.webp").exists());
    assert_eq!(generic(&call(&c, "session_list", json!({})).await), sessions, "batch documents are closed");

    // A rerun skips what exists, and overwrite replaces it.
    let v = ok(&call(&c, "batch_process", args.clone()).await);
    assert_eq!(v["skipped"], 3, "{v}");
    let mut again = args.clone();
    again["overwrite"] = json!(true);
    again["extensions"] = json!(["png"]);
    again["max_files"] = json!(2);
    let v = ok(&call(&c, "batch_process", again).await);
    assert_eq!(v["truncated"], true, "{v}");
    assert_eq!(v["files"].as_array().unwrap().len(), 2);

    // stop_on_error stops at the broken file (sorted first).
    let v = ok(&call(&c, "batch_process", json!({"input_dir": "in", "output_dir": "out/stop", "stop_on_error": true})).await);
    assert_eq!((v["failed"].as_u64(), v["processed"].as_u64()), (Some(1), Some(0)), "{v}");

    // Pure conversion and strict operations.
    let v = ok(&call(&c, "batch_process", json!({"input_dir": "in", "output_dir": "out/jpg", "format": "jpg", "quality": 80, "extensions": ["png"]})).await);
    assert_eq!(v["processed"], 3, "{v}");
    let msg = failed(
        &call(&c, "batch_process", json!({"input_dir": "in", "output_dir": "o", "operations": [{"tool": "resize_image", "width": 5, "index": 0}]})).await,
    );
    assert!(msg.contains("index"), "{msg}");
    let msg = failed(&call(&c, "batch_process", json!({"input_dir": "in", "output_dir": "o", "max_files": 100000})).await);
    assert!(msg.contains("max_files"), "{msg}");
    let e = raw(&c, "batch_process", json!({"input_dir": "in", "output_dir": "o", "operations": [{"tool": "shell", "cmd": "ls"}]})).await;
    assert!(e.is_err() || e.as_ref().is_ok_and(|r| r.is_error == Some(true)), "an unknown operation is rejected");
    c.cancel().await.unwrap();
}

/// A file name the path rules refuse (here `12:30.png`, legal on Unix) fails that file only.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn batch_process_reports_unwritable_names_per_file() {
    let root = Root::new("batch-names");
    std::fs::create_dir_all(root.path("in")).unwrap();
    let c = connect(root.server()).await;
    two_tone(&c, 8, 8, 4, "#ff0000").await;
    ok(&call(&c, "export_image", json!({"path": "in/a.png"})).await);
    std::fs::copy(root.path("in/a.png"), root.path("in/12:30.png")).unwrap();
    std::fs::copy(root.path("in/a.png"), root.path("in/z.png")).unwrap();

    let v = ok(&call(&c, "batch_process", json!({"input_dir": "in", "output_dir": "out"})).await);
    assert_eq!((v["processed"].as_u64(), v["failed"].as_u64()), (Some(2), Some(1)), "{v}");
    let bad = v["files"].as_array().unwrap().iter().find(|f| f["input"] == "in/12:30.png").unwrap();
    assert_eq!(bad["status"], "failed");
    assert!(root.path("out/a.png").exists() && root.path("out/z.png").exists(), "the files after it still ran");

    let v = ok(&call(&c, "batch_process", json!({"input_dir": "in", "output_dir": "out2", "stop_on_error": true})).await);
    assert_eq!((v["processed"].as_u64(), v["failed"].as_u64()), (Some(0), Some(1)), "sorted first, then stops: {v}");
    c.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_process_never_replaces_an_unread_input() {
    let root = Root::new("batch-collide");
    std::fs::create_dir_all(root.path("in")).unwrap();
    let c = connect(root.server()).await;
    two_tone(&c, 8, 8, 4, "#ff0000").await;
    ok(&call(&c, "export_image", json!({"path": "in/photo.jpg"})).await);
    ok(&call(&c, "export_image", json!({"path": "in/photo.png"})).await);
    let original = std::fs::read(root.path("in/photo.png")).unwrap();

    // photo.jpg -> photo.png would replace the input photo.png before it is read.
    let msg = failed(&call(&c, "batch_process", json!({"input_dir": "in", "output_dir": "in", "format": "png", "overwrite": true})).await);
    assert!(msg.contains("before writing anything") && msg.contains("replacing the input"), "{msg}");
    assert_eq!(std::fs::read(root.path("in/photo.png")).unwrap(), original, "the input is untouched");
    // The same request spelled through the root's absolute path is caught too.
    let abs_in = root.path("in").to_str().unwrap().to_owned();
    let msg = failed(&call(&c, "batch_process", json!({"input_dir": "in", "output_dir": abs_in, "format": "png", "overwrite": true})).await);
    assert!(msg.contains("replacing the input"), "{msg}");

    // Two inputs that would write one output.
    let msg = failed(&call(&c, "batch_process", json!({"input_dir": "in", "output_dir": "out", "format": "webp"})).await);
    assert!(msg.contains("would both write"), "{msg}");
    assert!(!root.path("out/photo.webp").exists(), "nothing was written");

    // Writing a file back over its own source is still allowed with overwrite.
    std::fs::remove_file(root.path("in/photo.jpg")).unwrap();
    let v = ok(&call(
        &c,
        "batch_process",
        json!({"input_dir": "in", "output_dir": "in", "overwrite": true, "operations": [{"tool": "flip_image", "direction": "vertical"}]}),
    )
    .await);
    assert_eq!(v["processed"], 1, "{v}");
    c.cancel().await.unwrap();
}

/// A PNG whose header claims `w` x `h` pixels but holds no image data.
fn fake_png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    b
}

#[tokio::test(flavor = "multi_thread")]
async fn huge_declared_images_are_refused_before_decoding() {
    let root = Root::new("huge");
    std::fs::create_dir_all(root.path("in")).unwrap();
    // 20000 x 20000 = 400 megapixels: under the decoder's own limit, over the task tools' budget.
    std::fs::write(root.path("in/huge.png"), fake_png(20_000, 20_000)).unwrap();
    let c = connect(root.server()).await;

    for tool in ["open_image", "inspect_image"] {
        let msg = failed(&call(&c, tool, json!({"path": "in/huge.png"})).await);
        assert!(msg.contains("20000 x 20000") && msg.contains("not decoded"), "{tool}: {msg}");
    }
    assert_eq!(generic(&call(&c, "session_list", json!({})).await)["documents"].as_array().expect("documents").len(), 0, "nothing was opened");

    two_tone(&c, 8, 8, 4, "#00ff00").await;
    ok(&call(&c, "export_image", json!({"path": "in/small.png"})).await);
    let v = ok(&call(&c, "batch_process", json!({"input_dir": "in", "output_dir": "out"})).await);
    assert_eq!((v["processed"].as_u64(), v["failed"].as_u64()), (Some(1), Some(1)), "{v}");
    let huge = v["files"].as_array().unwrap().iter().find(|f| f["input"] == "in/huge.png").unwrap();
    assert!(huge["error"].as_str().unwrap().contains("megapixel"), "{huge}");
    c.cancel().await.unwrap();
}
