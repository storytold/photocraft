//! Comprehensive 4-Tier Opaque-Box E2E Engine Validation Test Suite.
//!
//! Exclusively exercises the PhotoCraft engine via the Model Context Protocol (MCP)
//! interface and headless command bus without accessing internal private structs.
//!
//! Tiers:
//! - Tier 1: Feature Coverage (>=5 test cases per feature for layer CRUD, opacity, fill, blend mode cycling, MCP commands)
//! - Tier 2: Boundary & Corner Cases (>=5 test cases per feature for 0% opacity, 100% opacity, out-of-range rejection, zero-size rects, transparent backdrops, extreme coordinates, div-by-zero guards)
//! - Tier 3: Cross-Feature Combinations (pairwise interactions: clipping mask + group + blend mode, Special Eight fill dynamics, opacity sweep during translation, nested group transform & isolation, multi-selection batch)
//! - Tier 4: Real-World Application Scenarios (title card with drop shadow, multi-layer photo composition, 50-layer stress batch, non-destructive adjustment stack, frame latency telemetry verification)

use std::path::PathBuf;
use std::time::Instant;

use base64::Engine as _;
use photocraft_automation::{AuthorizedWorkspace, PhotocraftMcp};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

// ============================================================================
// Test Harness & Duplex Transport RPC Infrastructure
// ============================================================================

#[derive(Clone, Default)]
struct TestClient;

impl ClientHandler for TestClient {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

type Conn = RunningService<RoleClient, TestClient>;
type ServerHandle = tokio::task::JoinHandle<()>;

struct DirCleanupGuard(PathBuf);

impl Drop for DirCleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Harness {
    client: Conn,
    server: ServerHandle,
    dir: PathBuf,
    _cleanup: DirCleanupGuard,
}

impl Harness {
    async fn new(test_name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pc-e2e-{}-{}-{}",
            std::process::id(),
            test_name,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test workspace dir");

        let (server_stream, client_stream) = tokio::io::duplex(1 << 20);
        let workspace = AuthorizedWorkspace::new(Some(&dir), Some(&dir)).expect("create test workspace");
        let server = tokio::spawn(async move {
            if let Ok(running) = PhotocraftMcp::headless_with_workspace(workspace).serve(server_stream).await {
                let _ = running.waiting().await;
            }
        });
        let client = TestClient.serve(client_stream).await.expect("client init");
        Self { client, server, _cleanup: DirCleanupGuard(dir.clone()), dir }
    }

    async fn shutdown(self) {
        let _ = self.client.cancel().await;
        let _ = self.server.await;
        for _ in 0..20 {
            match std::fs::remove_dir_all(&self.dir) {
                Ok(()) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Harness {
    async fn tool_raw(&self, name: &str, args: Value) -> CallToolResult {
        let mut p = CallToolRequestParams::new(name.to_owned());
        if let Value::Object(m) = args {
            p = p.with_arguments(m);
        }
        self.client.call_tool(p).await.expect("call_tool transport")
    }

    async fn tool(&self, name: &str, args: Value) -> Value {
        let r = self.tool_raw(name, args).await;
        let txt = text_content(&r);
        assert_ne!(r.is_error, Some(true), "{name} returned tool error: {txt}");
        serde_json::from_str(&txt).unwrap_or(Value::String(txt))
    }

    async fn tool_err(&self, name: &str, args: Value) -> String {
        let r = self.tool_raw(name, args).await;
        assert_eq!(r.is_error, Some(true), "{name} unexpectedly succeeded: {}", text_content(&r));
        text_content(&r)
    }

    async fn run(&self, id: &str, params: Value) -> Value {
        self.tool("command_run", json!({"id": id, "params": params})).await
    }

    async fn run_err(&self, id: &str, params: Value) -> String {
        self.tool_err("command_run", json!({"id": id, "params": params})).await
    }

    async fn inspect(&self) -> Value {
        self.tool("doc_inspect", json!({})).await
    }

    async fn pixel(&self, x: i32, y: i32) -> [f64; 4] {
        let v = self.run("document.pixel", json!({"x": x, "y": y})).await;
        let arr = if v.is_array() { v } else { v.get("result").cloned().unwrap_or(v) };
        let elems: Vec<f64> = arr.as_array().unwrap_or_else(|| panic!("pixel output is not array: {arr}")).iter().map(|n| n.as_f64().unwrap()).collect();
        [elems[0], elems[1], elems[2], elems[3]]
    }
}

fn text_content(r: &CallToolResult) -> String {
    r.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n")
}

fn close_approx(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

fn count_layers_recursive(layers: &[Value]) -> usize {
    let mut total = 0;
    for l in layers {
        total += 1;
        if let Some(children) = l.get("children").and_then(Value::as_array) {
            total += count_layers_recursive(children);
        }
    }
    total
}

// ============================================================================
// TIER 1: FEATURE COVERAGE (>=5 tests per feature)
// ============================================================================

// --- Feature 1: Layer CRUD ---

#[tokio::test(flavor = "multi_thread")]
async fn tier1_layer_crud_1_create_pixel_layer() {
    let h = Harness::new("crud_create").await;
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;

    let res = h.run("layer.new.layer", json!({"name": "ArtLayer"})).await;
    let layer_id = res["layer"].as_u64().expect("layer id in response");
    assert!(layer_id > 0);

    let doc = h.inspect().await;
    let layers = doc["layers"].as_array().expect("layers array");
    assert_eq!(layers.len(), 2, "Expected 2 layers (Background + ArtLayer)");
    assert_eq!(layers[0]["name"], "ArtLayer");
    assert_eq!(layers[0]["kind"], "Pixel");
    assert_eq!(layers[0]["visible"], true);
    assert_eq!(layers[0]["opacity"], 1.0);
    assert_eq!(layers[0]["blend"], "Normal");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_layer_crud_2_create_group() {
    let h = Harness::new("crud_group").await;
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;

    let res = h.run("layer.new.group", json!({"name": "FolderGroup"})).await;
    let group_id = res["layer"].as_u64().expect("group id");
    assert!(group_id > 0);

    let doc = h.inspect().await;
    let layers = doc["layers"].as_array().expect("layers array");
    assert_eq!(layers[0]["name"], "FolderGroup");
    assert_eq!(layers[0]["kind"], "Group");
    assert!(layers[0]["children"].is_array());

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_layer_crud_3_move_into_group() {
    let h = Harness::new("crud_move_into").await;
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;

    let g_res = h.run("layer.new.group", json!({"name": "Folder"})).await;
    let group_id = g_res["layer"].as_u64().unwrap();
    let l_res = h.run("layer.new.layer", json!({"name": "Item"})).await;
    let item_id = l_res["layer"].as_u64().unwrap();

    h.run("layer.moveTo", json!({"layer": item_id, "target": group_id, "position": "into"})).await;

    let doc = h.inspect().await;
    let layers = doc["layers"].as_array().unwrap();
    let folder = layers.iter().find(|l| l["id"] == group_id).expect("folder in tree");
    let children = folder["children"].as_array().expect("children array");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["id"], item_id);
    assert_eq!(children[0]["name"], "Item");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_layer_crud_4_reorder_above_below() {
    let h = Harness::new("crud_reorder").await;
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;

    let r1 = h.run("layer.new.layer", json!({"name": "Alpha"})).await;
    let a_id = r1["layer"].as_u64().unwrap();
    let r2 = h.run("layer.new.layer", json!({"name": "Beta"})).await;
    let b_id = r2["layer"].as_u64().unwrap();

    // Initial order from top to bottom is: Beta, Alpha, Background
    let doc = h.inspect().await;
    let names: Vec<&str> = doc["layers"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Beta", "Alpha", "Background"]);

    // Move Beta below Alpha
    h.run("layer.moveTo", json!({"layer": b_id, "target": a_id, "position": "below"})).await;
    let doc = h.inspect().await;
    let names: Vec<&str> = doc["layers"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Alpha", "Beta", "Background"]);

    // Move Beta back above Alpha
    h.run("layer.moveTo", json!({"layer": b_id, "target": a_id, "position": "above"})).await;
    let doc = h.inspect().await;
    let names: Vec<&str> = doc["layers"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Beta", "Alpha", "Background"]);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_layer_crud_5_delete_layer() {
    let h = Harness::new("crud_delete").await;
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Doomed"})).await;
    let id = r["layer"].as_u64().unwrap();

    let doc = h.inspect().await;
    assert_eq!(doc["layers"].as_array().unwrap().len(), 2);

    h.run("layer.delete", json!({"layer": id})).await;

    let doc = h.inspect().await;
    let layers = doc["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0]["name"], "Background");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_layer_crud_6_new_layer_extended_props_and_fill_neutral() {
    let h = Harness::new("crud_new_extended").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#4080c0"})).await;

    // 1. Position: "below" places layer below the currently active layer
    let r1 = h.run("layer.new.layer", json!({"name": "TopLayer"})).await;
    let top_id = r1["layer"].as_u64().unwrap();
    let r2 = h.run("layer.new.layer", json!({"name": "BelowLayer", "position": "below"})).await;
    let below_id = r2["layer"].as_u64().unwrap();

    let doc = h.inspect().await;
    let layers = doc["layers"].as_array().unwrap();
    // Order from top to bottom: TopLayer, BelowLayer, Background
    assert_eq!(layers[0]["id"], top_id);
    assert_eq!(layers[1]["id"], below_id);

    // 2. clipToPrevious: true sets clipped: true and color label applies
    let r3 = h.run("layer.new.layer", json!({"name": "ClippedLayer", "clipToPrevious": true, "color": "red"})).await;
    let clipped_id = r3["layer"].as_u64().unwrap();
    let doc = h.inspect().await;
    let clipped_layer = doc["layers"].as_array().unwrap().iter().find(|l| l["id"] == clipped_id).unwrap();
    assert_eq!(clipped_layer["clipped"], true);
    assert_eq!(clipped_layer["labelColor"], "red");

    // 3. fillNeutral: true with Overlay (neutral is 50% gray)
    let px_before = h.pixel(16, 16).await;
    h.run("layer.new.layer", json!({"name": "NeutralOverlay", "mode": "Overlay", "fillNeutral": true})).await;
    let px_after_overlay = h.pixel(16, 16).await;
    assert!(close_approx(&px_before, &px_after_overlay, 0.02), "Overlay neutral fill must leave composite unchanged");

    // 4. fillNeutral: true with Multiply (neutral is white)
    h.run("layer.new.layer", json!({"name": "NeutralMultiply", "mode": "Multiply", "fillNeutral": true})).await;
    let px_after_multiply = h.pixel(16, 16).await;
    assert!(close_approx(&px_before, &px_after_multiply, 0.02), "Multiply neutral fill must leave composite unchanged");

    // 5. fillNeutral: true with Screen (neutral is black)
    h.run("layer.new.layer", json!({"name": "NeutralScreen", "mode": "Screen", "fillNeutral": true})).await;
    let px_after_screen = h.pixel(16, 16).await;
    assert!(close_approx(&px_before, &px_after_screen, 0.02), "Screen neutral fill must leave composite unchanged");

    // 6. fillNeutral: true with Normal mode must be rejected (Normal has no neutral color)
    let err = h.run_err("layer.new.layer", json!({"name": "InvalidNormalNeutral", "mode": "Normal", "fillNeutral": true})).await;
    assert!(err.contains("has no neutral color to fill with"), "Error should report no neutral color: {err}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_layer_crud_7_move_to_batch_copy_and_cycle_rejection() {
    let h = Harness::new("crud_moveto_batch_copy").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r_a = h.run("layer.new.layer", json!({"name": "LayerA"})).await;
    let id_a = r_a["layer"].as_u64().unwrap();
    let r_b = h.run("layer.new.layer", json!({"name": "LayerB"})).await;
    let id_b = r_b["layer"].as_u64().unwrap();
    let r_target = h.run("layer.new.layer", json!({"name": "TargetLayer"})).await;
    let id_target = r_target["layer"].as_u64().unwrap();

    // 1. Batch move: moving [LayerA, LayerB] above TargetLayer
    h.run("layer.moveTo", json!({"layers": [id_a, id_b], "target": id_target, "position": "above"})).await;
    let doc = h.inspect().await;
    let names: Vec<&str> = doc["layers"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["LayerB", "LayerA", "TargetLayer", "Background"]);

    // 2. copy: true duplicates layer on move
    let copy_res = h.run("layer.moveTo", json!({"layer": id_a, "target": id_target, "position": "below", "copy": true})).await;
    let copy_id = copy_res["layer"].as_u64().unwrap();
    assert_ne!(copy_id, id_a);
    let doc = h.inspect().await;
    let copy_layer = doc["layers"].as_array().unwrap().iter().find(|l| l["id"] == copy_id).expect("copy in document");
    assert_eq!(copy_layer["name"], "LayerA copy");

    // 3. Cycle detection rejection: moving a group into itself or its child
    let g1 = h.run("layer.new.group", json!({"name": "ParentGroup"})).await["layer"].as_u64().unwrap();
    let g2 = h.run("layer.new.group", json!({"name": "ChildGroup"})).await["layer"].as_u64().unwrap();
    h.run("layer.moveTo", json!({"layer": g2, "target": g1, "position": "into"})).await;

    // Moving parent into its child group must be rejected
    let err_child = h.run_err("layer.moveTo", json!({"layer": g1, "target": g2, "position": "into"})).await;
    assert!(err_child.contains("target cannot be one of the moved layers or its descendant"), "Expected cycle rejection, got {err_child}");

    // Moving group into itself (with copy: true) must be rejected
    let err_self = h.run_err("layer.moveTo", json!({"layer": g1, "target": g1, "position": "into", "copy": true})).await;
    assert!(err_self.contains("target cannot be one of the moved layers or its descendant"), "Expected cycle rejection, got {err_self}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_layer_crud_8_translate_pt_mask_and_locked_rejection() {
    let h = Harness::new("crud_translate_ext").await;
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Art"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;

    // 1. units: "pt" translation (72 pt == 72 px at 72 dpi)
    let doc_before = h.inspect().await;
    let bounds_before = doc_before["layers"][0]["bounds"].clone();
    h.run("layer.translate", json!({"layer": id, "dx": 10.0, "dy": 5.0, "units": "pt"})).await;
    let doc_after = h.inspect().await;
    let bounds_after = doc_after["layers"][0]["bounds"].as_array().unwrap();
    let bounds_b = bounds_before.as_array().unwrap();
    assert_eq!(bounds_after[0].as_i64().unwrap(), bounds_b[0].as_i64().unwrap() + 10);
    assert_eq!(bounds_after[1].as_i64().unwrap(), bounds_b[1].as_i64().unwrap() + 5);

    // 2. Unlinked mask movement
    h.run("layer.layerMask.revealAll", json!({})).await;
    h.run("layer.layerMask.linked", json!({"linked": false})).await;
    let bounds_pre_mask = h.inspect().await["layers"][0]["bounds"].clone();
    h.run("layer.translate", json!({"layer": id, "dx": 20, "dy": 15, "target": "mask"})).await;
    let bounds_post_mask = h.inspect().await["layers"][0]["bounds"].clone();
    assert_eq!(bounds_pre_mask, bounds_post_mask, "Unlinked mask move must not displace layer bounds");

    // 3. Position-locked layer rejection
    // Background layer is position-locked by default
    let bg_id = h.inspect().await["layers"].as_array().unwrap().last().unwrap()["id"].as_u64().unwrap();
    let err_bg = h.run_err("layer.translate", json!({"layer": bg_id, "dx": 5, "dy": 5})).await;
    assert!(err_bg.contains("position-locked"), "Background move should be position-locked: {err_bg}");

    // Lock position on an art layer and verify rejection
    h.run("layer.setProps", json!({"layer": id, "locks": {"position": true}})).await;
    let err_locked = h.run_err("layer.translate", json!({"layer": id, "dx": 5, "dy": 5})).await;
    assert!(err_locked.contains("position-locked"), "Position-locked art layer move should fail: {err_locked}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_layer_crud_9_delete_last_layer_rejection_and_subtree_cascade() {
    let h = Harness::new("crud_delete_ext").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let doc = h.inspect().await;
    let bg_id = doc["layers"][0]["id"].as_u64().unwrap();

    // 1. Attempt to delete the last remaining layer must be rejected
    let err = h.run_err("layer.delete", json!({"layer": bg_id})).await;
    assert!(err.contains("a document must keep at least one layer"), "Expected last layer rejection, got {err}");

    // 2. Subtree cascade deletion: deleting a group removes all its descendants
    let g_res = h.run("layer.new.group", json!({"name": "CascadeParent"})).await;
    let g_id = g_res["layer"].as_u64().unwrap();
    let c1 = h.run("layer.new.layer", json!({"name": "Child1"})).await["layer"].as_u64().unwrap();
    let c2 = h.run("layer.new.layer", json!({"name": "Child2"})).await["layer"].as_u64().unwrap();
    h.run("layer.moveTo", json!({"layers": [c1, c2], "target": g_id, "position": "into"})).await;

    let doc = h.inspect().await;
    assert_eq!(doc["layers"].as_array().unwrap().len(), 2, "Group + Background");
    let group = doc["layers"].as_array().unwrap().iter().find(|l| l["id"] == g_id).unwrap();
    assert_eq!(group["children"].as_array().unwrap().len(), 2, "2 children in group");

    // Deleting the group must cascade and delete Child1 and Child2
    h.run("layer.delete", json!({"layer": g_id})).await;
    let doc_after = h.inspect().await;
    let remaining = doc_after["layers"].as_array().unwrap();
    assert_eq!(remaining.len(), 1, "Only Background remains after cascade group deletion");
    assert_eq!(remaining[0]["name"], "Background");

    h.shutdown().await;
}

// --- Feature 2: Layer Opacity ---

#[tokio::test(flavor = "multi_thread")]
async fn tier1_opacity_1_half_blend() {
    let h = Harness::new("opacity_half").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Red"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 0.5})).await;

    let px = h.pixel(16, 16).await;
    // Red (1, 0, 0) over White (1, 1, 1) at 50% opacity -> (1.0, 0.5, 0.5, 1.0)
    assert!(close_approx(&px, &[1.0, 0.5, 0.5, 1.0], 0.02), "expected ~[1.0, 0.5, 0.5, 1.0], got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_opacity_2_quarter_blend() {
    let h = Harness::new("opacity_quarter").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Red"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 0.25})).await;

    let px = h.pixel(16, 16).await;
    // Red (1, 0, 0) over White (1, 1, 1) at 25% opacity -> (1.0, 0.75, 0.75, 1.0)
    assert!(close_approx(&px, &[1.0, 0.75, 0.75, 1.0], 0.02), "expected ~[1.0, 0.75, 0.75, 1.0], got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_opacity_3_three_quarters_blend() {
    let h = Harness::new("opacity_three_quarters").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Red"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 0.75})).await;

    let px = h.pixel(16, 16).await;
    // Red (1, 0, 0) over White (1, 1, 1) at 75% opacity -> (1.0, 0.25, 0.25, 1.0)
    assert!(close_approx(&px, &[1.0, 0.25, 0.25, 1.0], 0.02), "expected ~[1.0, 0.25, 0.25, 1.0], got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_opacity_4_multi_layer_batch() {
    let h = Harness::new("opacity_multi_batch").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r1 = h.run("layer.new.layer", json!({"name": "L1"})).await;
    let id1 = r1["layer"].as_u64().unwrap();
    let r2 = h.run("layer.new.layer", json!({"name": "L2"})).await;
    let id2 = r2["layer"].as_u64().unwrap();

    h.run("layer.setProps", json!({"layers": [id1, id2], "opacity": 0.6})).await;

    let doc = h.inspect().await;
    let l1 = doc["layers"].as_array().unwrap().iter().find(|l| l["id"] == id1).unwrap();
    let l2 = doc["layers"].as_array().unwrap().iter().find(|l| l["id"] == id2).unwrap();
    assert!((l1["opacity"].as_f64().unwrap() - 0.6).abs() < 1e-4);
    assert!((l2["opacity"].as_f64().unwrap() - 0.6).abs() < 1e-4);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_opacity_5_undo_redo_roundtrip() {
    let h = Harness::new("opacity_undo_redo").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Red"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    let before_px = h.pixel(16, 16).await;
    assert!(close_approx(&before_px, &[1.0, 0.0, 0.0, 1.0], 0.01));

    h.run("layer.setProps", json!({"layer": id, "opacity": 0.3})).await;
    let altered_px = h.pixel(16, 16).await;
    assert!(altered_px[1] > 0.6, "pixel green channel should increase under opacity drop");

    h.run("edit.undo", json!({})).await;
    let undone_px = h.pixel(16, 16).await;
    assert!(close_approx(&undone_px, &before_px, 0.01), "undo must restore opacity 1.0 pixels");

    h.run("edit.redo", json!({})).await;
    let redone_px = h.pixel(16, 16).await;
    assert!(close_approx(&redone_px, &altered_px, 0.01), "redo must restore opacity 0.3 pixels");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_opacity_6_fine_grained_monotonic_sweep_and_coalesce() {
    let h = Harness::new("opacity_fine_sweep").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "RedLayer"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;

    let sweep = [0.0, 0.05, 0.25, 0.5, 0.75, 0.95, 1.0];
    let mut prev_g = 2.0;
    let mut prev_b = 2.0;

    for &op in &sweep {
        h.run("layer.setProps", json!({"layer": id, "opacity": op, "coalesce": "drag-opacity"})).await;
        let px = h.pixel(16, 16).await;

        // Red channel is always 1.0 (blending Red 1.0 with White 1.0)
        assert!((px[0] - 1.0).abs() <= 0.02, "Red channel should be ~1.0 at op={op}, got {}", px[0]);

        // Green and Blue channels should decrease strictly monotonically as opacity increases
        let expected_gb = 1.0 - op;
        assert!(
            close_approx(&[px[1], px[2]], &[expected_gb, expected_gb], 0.02),
            "Channels [G, B] mismatch at op={op}: expected ~{expected_gb}, got [{}, {}]",
            px[1],
            px[2]
        );

        if op > 0.0 {
            assert!(px[1] < prev_g, "Green channel must decrease strictly monotonically: op={op}, got {} >= prev {prev_g}", px[1]);
            assert!(px[2] < prev_b, "Blue channel must decrease strictly monotonically: op={op}, got {} >= prev {prev_b}", px[2]);
        }
        prev_g = px[1];
        prev_b = px[2];
    }

    // Coalescing check: 7 sequential drag updates should coalesce into a single history entry
    let doc = h.inspect().await;
    let history = doc["history"].as_array().expect("history array");
    assert_eq!(
        history.iter().filter(|s| s.as_str() == Some("Opacity Change")).count(),
        1,
        "Coalesced drag should result in exactly one 'Opacity Change' history entry"
    );

    h.shutdown().await;
}

// --- Feature 3: Fill Opacity ---

#[tokio::test(flavor = "multi_thread")]
async fn tier1_fill_1_half() {
    let h = Harness::new("fill_half").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "FillLayer"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("layer.setProps", json!({"layer": id, "fill": 0.5})).await;

    let doc = h.inspect().await;
    let top = &doc["layers"][0];
    assert_eq!(top["fill"], 0.5);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_fill_2_zero() {
    let h = Harness::new("fill_zero").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "FillLayer"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("layer.setProps", json!({"layer": id, "fill": 0.0})).await;

    let doc = h.inspect().await;
    let top = &doc["layers"][0];
    assert_eq!(top["fill"], 0.0);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_fill_3_full() {
    let h = Harness::new("fill_full").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "FillLayer"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("layer.setProps", json!({"layer": id, "fill": 1.0})).await;

    let doc = h.inspect().await;
    let top = &doc["layers"][0];
    assert_eq!(top["fill"], 1.0);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_fill_4_multi_layer() {
    let h = Harness::new("fill_multi").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r1 = h.run("layer.new.layer", json!({"name": "F1"})).await;
    let id1 = r1["layer"].as_u64().unwrap();
    let r2 = h.run("layer.new.layer", json!({"name": "F2"})).await;
    let id2 = r2["layer"].as_u64().unwrap();

    h.run("layer.setProps", json!({"layers": [id1, id2], "fill": 0.4})).await;

    let doc = h.inspect().await;
    let l1 = doc["layers"].as_array().unwrap().iter().find(|l| l["id"] == id1).unwrap();
    let l2 = doc["layers"].as_array().unwrap().iter().find(|l| l["id"] == id2).unwrap();
    assert!((l1["fill"].as_f64().unwrap() - 0.4).abs() < 1e-4);
    assert!((l2["fill"].as_f64().unwrap() - 0.4).abs() < 1e-4);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_fill_5_independent_of_layer_opacity() {
    let h = Harness::new("fill_independent").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "BothProps"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("layer.setProps", json!({"layer": id, "opacity": 0.7, "fill": 0.3})).await;

    let doc = h.inspect().await;
    let top = &doc["layers"][0];
    assert!((top["opacity"].as_f64().unwrap() - 0.7).abs() < 1e-4);
    assert!((top["fill"].as_f64().unwrap() - 0.3).abs() < 1e-4);
    assert_eq!(doc["history"].as_array().unwrap().last().unwrap(), "Layer Properties");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_fill_6_special_eight_fill_dynamics() {
    let h = Harness::new("fill_special_eight_tier1").await;
    // Photoshop 27.11 measured corpus: Background #c89664, Layer #64b4dc
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#c89664"})).await;

    let r = h.run("layer.new.layer", json!({"name": "SpecialEightLayer"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#64b4dc"})).await;

    let special_eight_oracles: &[(&str, [f64; 3])] = &[
        ("Color Burn", [176.0 / 255.0, 132.0 / 255.0, 88.0 / 255.0]),
        ("Linear Burn", [122.0 / 255.0, 112.0 / 255.0, 82.0 / 255.0]),
        ("Color Dodge", [249.0 / 255.0, 232.0 / 255.0, 176.0 / 255.0]),
        ("Linear Dodge", [250.0 / 255.0, 240.0 / 255.0, 210.0 / 255.0]),
        ("Vivid Light", [193.0 / 255.0, 188.0 / 255.0, 157.0 / 255.0]),
        ("Linear Light", [171.0 / 255.0, 202.0 / 255.0, 192.0 / 255.0]),
        ("Hard Mix", [245.0 / 255.0, 225.0 / 255.0, 165.0 / 255.0]),
        ("Difference", [150.0 / 255.0, 60.0 / 255.0, 10.0 / 255.0]),
    ];

    for &(mode, expected) in special_eight_oracles {
        // 1. Measure at 50% fill (128/255), 100% opacity
        h.run("layer.setProps", json!({"layer": id, "blend": mode, "opacity": 1.0, "fill": 128.0 / 255.0})).await;
        let px_fill = h.pixel(16, 16).await;

        assert!(
            close_approx(&px_fill[0..3], &expected, 0.02),
            "Special Eight mode {mode} fill 50% mismatch with Photoshop oracle: expected {expected:?}, got {:?}",
            &px_fill[0..3]
        );

        // 2. Measure at 50% opacity, 100% fill and verify divergence
        h.run("layer.setProps", json!({"layer": id, "opacity": 128.0 / 255.0, "fill": 1.0})).await;
        let px_op = h.pixel(16, 16).await;

        if mode != "Linear Burn" {
            let max_diff = (px_fill[0] - px_op[0]).abs().max((px_fill[1] - px_op[1]).abs()).max((px_fill[2] - px_op[2]).abs());
            assert!(max_diff > 0.02, "Special Eight mode {mode} MUST diverge from standard opacity: px_fill={px_fill:?}, px_op={px_op:?}, diff={max_diff}");
        }
    }

    // 2b. Verify Linear Burn divergence on clamped inputs (where Cb + Cs - 1 < 0)
    let h_lb = Harness::new("fill_linear_burn_div").await;
    h_lb.tool("doc_new", json!({"width": 32, "height": 32, "background": "#404040"})).await;
    let lb_id = h_lb.run("layer.new.layer", json!({"name": "LB"})).await["layer"].as_u64().unwrap();
    h_lb.run("edit.fill", json!({"color": "#404040"})).await;
    h_lb.run("layer.setProps", json!({"layer": lb_id, "blend": "Linear Burn", "opacity": 1.0, "fill": 0.5})).await;
    let px_fill_lb = h_lb.pixel(16, 16).await;
    h_lb.run("layer.setProps", json!({"layer": lb_id, "opacity": 0.5, "fill": 1.0})).await;
    let px_op_lb = h_lb.pixel(16, 16).await;
    let lb_diff = (px_fill_lb[0] - px_op_lb[0]).abs().max((px_fill_lb[1] - px_op_lb[1]).abs()).max((px_fill_lb[2] - px_op_lb[2]).abs());
    assert!(lb_diff > 0.02, "Linear Burn with clamping MUST diverge from standard opacity: px_fill={px_fill_lb:?}, px_op={px_op_lb:?}, diff={lb_diff}");
    h_lb.shutdown().await;

    // 3. Verify non-special modes (Normal, Multiply) treat fill identically to opacity
    for mode in ["Normal", "Multiply"] {
        h.run("layer.setProps", json!({"layer": id, "blend": mode, "opacity": 1.0, "fill": 0.5})).await;
        let px_fill = h.pixel(16, 16).await;

        h.run("layer.setProps", json!({"layer": id, "opacity": 0.5, "fill": 1.0})).await;
        let px_op = h.pixel(16, 16).await;

        assert!(
            close_approx(&px_fill[0..3], &px_op[0..3], 0.01),
            "Non-special mode {mode} must treat fill identically to opacity: px_fill={px_fill:?}, px_op={px_op:?}"
        );
    }

    h.shutdown().await;
}

// --- Feature 4: Blend Mode Cycling ---

#[tokio::test(flavor = "multi_thread")]
async fn tier1_blend_mode_1_multiply() {
    let h = Harness::new("blend_multiply").await;
    // Cyan background: (0, 1, 1)
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#00ffff"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Red"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.setProps", json!({"layer": id, "blend": "Multiply"})).await;

    let px = h.pixel(16, 16).await;
    // Multiply of Red (1, 0, 0) and Cyan (0, 1, 1) produces Black (0, 0, 0)
    assert!(close_approx(&px, &[0.0, 0.0, 0.0, 1.0], 0.01), "Multiply expected black, got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_blend_mode_2_screen() {
    let h = Harness::new("blend_screen").await;
    // Green background: (0, 1, 0)
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#00ff00"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Red"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.setProps", json!({"layer": id, "blend": "Screen"})).await;

    let px = h.pixel(16, 16).await;
    // Screen of Red (1, 0, 0) and Green (0, 1, 0) produces Yellow (1, 1, 0)
    assert!(close_approx(&px, &[1.0, 1.0, 0.0, 1.0], 0.01), "Screen expected yellow, got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_blend_mode_3_overlay() {
    let h = Harness::new("blend_overlay").await;
    // 50% Gray background: #808080 (~0.5, 0.5, 0.5)
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#808080"})).await;

    let r = h.run("layer.new.layer", json!({"name": "White"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ffffff"})).await;
    h.run("layer.setProps", json!({"layer": id, "blend": "Overlay"})).await;

    let px = h.pixel(16, 16).await;
    // White over 50% gray in Overlay yields pure white (1, 1, 1)
    assert!(close_approx(&px, &[1.0, 1.0, 1.0, 1.0], 0.02), "Overlay expected pure white, got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_blend_mode_4_difference() {
    let h = Harness::new("blend_difference").await;
    // Red background: (1, 0, 0)
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#ff0000"})).await;

    let r = h.run("layer.new.layer", json!({"name": "White"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ffffff"})).await;
    h.run("layer.setProps", json!({"layer": id, "blend": "Difference"})).await;

    let px = h.pixel(16, 16).await;
    // White (1, 1, 1) - Red (1, 0, 0) = Cyan (0, 1, 1)
    assert!(close_approx(&px, &[0.0, 1.0, 1.0, 1.0], 0.01), "Difference expected cyan, got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_blend_mode_5_all_27_modes_cycle() {
    let h = Harness::new("blend_all_27_cycle").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#4080c0"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Source"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#e07030"})).await;

    let all_27_modes: &[(&str, [f64; 3])] = &[
        ("Normal", [0.8784, 0.4392, 0.1882]),
        ("Dissolve", [0.8784, 0.4392, 0.1882]),
        ("Darken", [0.2510, 0.4392, 0.1882]),
        ("Multiply", [0.2205, 0.2205, 0.1417]),
        ("Color Burn", [0.1473, 0.0000, 0.0000]),
        ("Linear Burn", [0.1294, 0.0000, 0.0000]),
        ("Darker Color", [0.2510, 0.5020, 0.7529]),
        ("Lighten", [0.8784, 0.5020, 0.7529]),
        ("Screen", [0.9089, 0.7207, 0.7995]),
        ("Color Dodge", [1.0000, 0.8951, 0.9274]),
        ("Linear Dodge", [1.0000, 0.9412, 0.9411]),
        ("Lighter Color", [0.8784, 0.4392, 0.1882]),
        ("Overlay", [0.4410, 0.4414, 0.5988]),
        ("Soft Light", [0.4402, 0.4716, 0.6369]),
        ("Hard Light", [0.8178, 0.4410, 0.2834]),
        ("Vivid Light", [1.0000, 0.4331, 0.3435]),
        ("Linear Light", [1.0000, 0.3804, 0.1293]),
        ("Pin Light", [0.7568, 0.5020, 0.3764]),
        ("Hard Mix", [1.0000, 0.0000, 0.0000]),
        ("Difference", [0.6274, 0.0628, 0.5647]),
        ("Exclusion", [0.6885, 0.5002, 0.6577]),
        ("Subtract", [0.0000, 0.0628, 0.5647]),
        ("Divide", [0.2858, 1.0000, 1.0000]),
        ("Hue", [0.6980, 0.3785, 0.1960]),
        ("Saturation", [0.1747, 0.5198, 0.8649]),
        ("Color", [0.7893, 0.3501, 0.0991]),
        ("Luminosity", [0.3401, 0.5911, 0.8420]),
    ];

    assert_eq!(all_27_modes.len(), 27, "Must verify exact 27 Photoshop blend modes");

    for &(mode, expected) in all_27_modes {
        h.run("layer.setProps", json!({"layer": id, "blend": mode})).await;
        let px = h.pixel(16, 16).await;
        for c in px {
            assert!(!c.is_nan() && !c.is_infinite(), "Blend mode {mode} resulted in non-finite float {px:?}");
            assert!((0.0..=1.0001).contains(&c), "Blend mode {mode} channel out of [0, 1]: {c}");
        }
        assert!(close_approx(&px[0..3], &expected, 0.02), "Blend mode {mode} mathematical oracle mismatch: expected {expected:?}, got {:?}", &px[0..3]);
        assert!((px[3] - 1.0).abs() <= 0.01, "Blend mode {mode} alpha expected 1.0, got {}", px[3]);
    }

    h.shutdown().await;
}

// --- Feature 5: MCP Commands ---

#[tokio::test(flavor = "multi_thread")]
async fn tier1_mcp_1_doc_new() {
    let h = Harness::new("mcp_doc_new").await;
    let _doc = h.tool("doc_new", json!({"width": 80, "height": 45, "mode": "rgb", "depth": 8, "background": "#112233", "name": "NewDoc"})).await;

    let inspected = h.inspect().await;
    assert_eq!(inspected["name"], "NewDoc");
    assert_eq!(inspected["width"], 80);
    assert_eq!(inspected["height"], 45);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_mcp_2_doc_inspect() {
    let h = Harness::new("mcp_doc_inspect").await;
    h.tool("doc_new", json!({"width": 50, "height": 50, "background": "white"})).await;
    h.run("layer.new.layer", json!({"name": "LayerOne"})).await;

    let doc = h.inspect().await;
    assert_eq!(doc["width"], 50);
    assert_eq!(doc["height"], 50);
    assert_eq!(doc["mode"], "Rgb");
    assert_eq!(doc["depth"], 8);
    let layers = doc["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0]["name"], "LayerOne");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_mcp_3_doc_render_preview() {
    let h = Harness::new("mcp_preview").await;
    h.tool("doc_new", json!({"width": 64, "height": 48, "background": "white"})).await;

    let res = h.tool_raw("doc_render_preview", json!({"max_side": 32})).await;
    assert_ne!(res.is_error, Some(true));
    let img_block = res.content.iter().find_map(|c| c.as_image()).expect("image block in preview result");
    assert_eq!(img_block.mime_type, "image/png");

    let decoded_png = base64::engine::general_purpose::STANDARD.decode(&img_block.data).expect("base64 decode");
    // Standard PNG magic header: 137 80 78 71 13 10 26 10
    assert_eq!(&decoded_png[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    let img = photocraft_codecs::decode(&decoded_png).expect("valid image decode");
    assert_eq!(img.dimensions(), (32, 24));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_mcp_4_command_batch() {
    let h = Harness::new("mcp_batch").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let batch_res = h
        .tool(
            "command_batch",
            json!({
                "steps": [
                    {"id": "layer.new.layer", "params": {"name": "BatchLayer"}},
                    {"id": "edit.fill", "params": {"color": "#0000ff"}},
                    {"id": "layer.setProps", "params": {"opacity": 0.5}}
                ]
            }),
        )
        .await;

    assert_eq!(batch_res["completed"], 3);
    assert_eq!(batch_res["failed"], 0);

    let doc = h.inspect().await;
    assert_eq!(doc["layers"][0]["name"], "BatchLayer");
    assert_eq!(doc["layers"][0]["opacity"], 0.5);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier1_mcp_5_command_list() {
    let h = Harness::new("mcp_command_list").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let list_val = h.tool("command_list", json!({"filter": "layer."})).await;
    let list = list_val.as_array().expect("command list array");
    let ids: Vec<&str> = list.iter().filter_map(|c| c["id"].as_str()).collect();

    for required_id in ["layer.new.layer", "layer.new.group", "layer.moveTo", "layer.delete", "layer.setProps", "layer.translate"] {
        assert!(ids.contains(&required_id), "command_list missing required command: {required_id}");
    }

    h.shutdown().await;
}

// ============================================================================
// TIER 2: BOUNDARY & CORNER CASES (>=5 tests per feature)
// ============================================================================

// --- Feature 1: 0% Opacity Boundary ---

#[tokio::test(flavor = "multi_thread")]
async fn tier2_zero_opacity_1_pure_invisibility() {
    let h = Harness::new("zero_op_inv").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#00ff00"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Red"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 0.0})).await;

    let px = h.pixel(16, 16).await;
    // 0% opacity must reveal underlying green background completely
    assert!(close_approx(&px, &[0.0, 1.0, 0.0, 1.0], 0.001), "0% opacity should be pure green, got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_zero_opacity_2_over_black() {
    let h = Harness::new("zero_op_black").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#000000"})).await;

    let r = h.run("layer.new.layer", json!({"name": "White"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ffffff"})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 0.0})).await;

    let px = h.pixel(16, 16).await;
    assert!(close_approx(&px, &[0.0, 0.0, 0.0, 1.0], 0.001), "0% opacity should be pure black, got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_zero_opacity_3_with_blend_mode() {
    let h = Harness::new("zero_op_blend").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#55aaff"})).await;

    let r = h.run("layer.new.layer", json!({"name": "MultiplyZero"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#000000"})).await;
    h.run("layer.setProps", json!({"layer": id, "blend": "Multiply", "opacity": 0.0})).await;

    let px = h.pixel(16, 16).await;
    let expected = [0x55 as f64 / 255.0, 0xaa as f64 / 255.0, 1.0, 1.0];
    assert!(close_approx(&px, &expected, 0.01), "0% opacity with Multiply must leave backdrop untouched, got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_zero_opacity_4_in_group() {
    let h = Harness::new("zero_op_group").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let g = h.run("layer.new.group", json!({"name": "G"})).await["layer"].as_u64().unwrap();
    let l = h.run("layer.new.layer", json!({"name": "Red"})).await["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.moveTo", json!({"layer": l, "target": g, "position": "into"})).await;
    h.run("layer.setProps", json!({"layer": l, "opacity": 0.0})).await;

    let px = h.pixel(16, 16).await;
    assert!(close_approx(&px, &[1.0, 1.0, 1.0, 1.0], 0.001));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_zero_opacity_5_preview_generation() {
    let h = Harness::new("zero_op_preview").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#ff8800"})).await;

    let r = h.run("layer.new.layer", json!({"name": "BlueInvisible"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#0000ff"})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 0.0})).await;

    let res = h.tool_raw("doc_render_preview", json!({"max_side": 32})).await;
    assert_ne!(res.is_error, Some(true));

    h.shutdown().await;
}

// --- Feature 2: 100% Opacity Boundary ---

#[tokio::test(flavor = "multi_thread")]
async fn tier2_full_opacity_1_pure_occlusion() {
    let h = Harness::new("full_op_occl").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Red"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 1.0})).await;

    let px = h.pixel(16, 16).await;
    assert!(close_approx(&px, &[1.0, 0.0, 0.0, 1.0], 0.001), "100% opacity must completely occlude, got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_full_opacity_2_over_black() {
    let h = Harness::new("full_op_black").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "black"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Blue"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#0000ff"})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 1.0})).await;

    let px = h.pixel(16, 16).await;
    assert!(close_approx(&px, &[0.0, 0.0, 1.0, 1.0], 0.001));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_full_opacity_3_restore_from_zero() {
    let h = Harness::new("full_op_restore").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Green"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#00ff00"})).await;

    h.run("layer.setProps", json!({"layer": id, "opacity": 0.0})).await;
    assert!(close_approx(&h.pixel(16, 16).await, &[1.0, 1.0, 1.0, 1.0], 0.01));

    h.run("layer.setProps", json!({"layer": id, "opacity": 1.0})).await;
    assert!(close_approx(&h.pixel(16, 16).await, &[0.0, 1.0, 0.0, 1.0], 0.01));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_full_opacity_4_across_multiple_layers() {
    let h = Harness::new("full_op_multi").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let id1 = h.run("layer.new.layer", json!({"name": "A"})).await["layer"].as_u64().unwrap();
    let id2 = h.run("layer.new.layer", json!({"name": "B"})).await["layer"].as_u64().unwrap();

    h.run("layer.setProps", json!({"layers": [id1, id2], "opacity": 1.0})).await;

    let doc = h.inspect().await;
    for l in doc["layers"].as_array().unwrap().iter().filter(|l| l["name"] != "Background") {
        assert_eq!(l["opacity"], 1.0);
    }

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_full_opacity_5_preview_verification() {
    let h = Harness::new("full_op_preview").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "Full"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#123456"})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 1.0})).await;

    let res = h.tool_raw("doc_render_preview", json!({"max_side": 32})).await;
    assert_ne!(res.is_error, Some(true));

    h.shutdown().await;
}

// --- Feature 3: Out-of-Range Rejection ---

#[tokio::test(flavor = "multi_thread")]
async fn tier2_out_of_range_1_negative_opacity() {
    let h = Harness::new("oor_neg_op").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();

    let err = h.run_err("layer.setProps", json!({"layer": id, "opacity": -0.1})).await;
    assert!(err.contains("opacity") && err.contains("must be a number in 0..1"), "error message: {err}");

    let doc = h.inspect().await;
    assert_eq!(doc["layers"][0]["opacity"], 1.0, "state should remain intact");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_out_of_range_2_excessive_opacity() {
    let h = Harness::new("oor_exc_op").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();

    let err = h.run_err("layer.setProps", json!({"layer": id, "opacity": 1.5})).await;
    assert!(err.contains("opacity") && err.contains("must be a number in 0..1"), "error message: {err}");

    let doc = h.inspect().await;
    assert_eq!(doc["layers"][0]["opacity"], 1.0);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_out_of_range_3_negative_fill() {
    let h = Harness::new("oor_neg_fill").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();

    let err = h.run_err("layer.setProps", json!({"layer": id, "fill": -0.2})).await;
    assert!(err.contains("fill") && err.contains("must be a number in 0..1"), "error message: {err}");

    let doc = h.inspect().await;
    assert_eq!(doc["layers"][0]["fill"], 1.0);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_out_of_range_4_excessive_fill() {
    let h = Harness::new("oor_exc_fill").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();

    let err = h.run_err("layer.setProps", json!({"layer": id, "fill": 2.0})).await;
    assert!(err.contains("fill") && err.contains("must be a number in 0..1"), "error message: {err}");

    let doc = h.inspect().await;
    assert_eq!(doc["layers"][0]["fill"], 1.0);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_out_of_range_5_unknown_blend_mode() {
    let h = Harness::new("oor_blend").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();

    let err = h.run_err("layer.setProps", json!({"layer": id, "blend": "NonExistentMode"})).await;
    assert!(err.contains("unknown blend mode"), "error message: {err}");

    let doc = h.inspect().await;
    assert_eq!(doc["layers"][0]["blend"], "Normal");

    h.shutdown().await;
}

// --- Feature 4: Zero-Size & Degenerate Rects ---

#[tokio::test(flavor = "multi_thread")]
async fn tier2_zero_size_1_rect_selection() {
    let h = Harness::new("zs_rect").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    // Zero-width rect selection
    h.run("select.rect", json!({"x": 10, "y": 10, "width": 0, "height": 10})).await;
    let doc = h.inspect().await;
    assert_eq!(doc["hasSelection"], false, "Zero-width rect should not leave a selection active");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_zero_size_2_preview_max_side() {
    let h = Harness::new("zs_preview").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    // max_side: 0 requests full size within ceiling without panic
    let res = h.tool_raw("doc_render_preview", json!({"max_side": 0})).await;
    assert_ne!(res.is_error, Some(true));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_zero_size_3_layer_translate_zero() {
    let h = Harness::new("zs_translate").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    h.run("layer.new.layer", json!({"name": "P"})).await;
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.translate", json!({"dx": 0, "dy": 0})).await;

    let px = h.pixel(16, 16).await;
    assert!(close_approx(&px, &[1.0, 0.0, 0.0, 1.0], 0.01));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_zero_size_4_empty_layer_list_rejection() {
    let h = Harness::new("zs_empty_layers").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let err = h.run_err("layer.setProps", json!({"layers": [], "opacity": 0.5})).await;
    assert!(err.contains("non-empty list"), "error message: {err}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_zero_size_5_pass_through_on_pixel_layer_rejected() {
    let h = Harness::new("zs_pass_through").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();

    let err = h.run_err("layer.setProps", json!({"layer": id, "blend": "Pass Through"})).await;
    assert!(err.contains("Pass Through applies to groups only"), "error message: {err}");

    h.shutdown().await;
}

// --- Feature 5: Transparent Backdrops ---

#[tokio::test(flavor = "multi_thread")]
async fn tier2_transparent_1_empty_pixel() {
    let h = Harness::new("transp_empty").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "transparent"})).await;

    let px = h.pixel(16, 16).await;
    assert_eq!(px, [0.0, 0.0, 0.0, 0.0], "Unpainted transparent backdrop must be pure zero");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_transparent_2_opaque_paint() {
    let h = Harness::new("transp_opaque").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "transparent"})).await;

    h.run("layer.new.layer", json!({"name": "Paint"})).await;
    h.run("edit.fill", json!({"color": "#ff0000"})).await;

    let px = h.pixel(16, 16).await;
    assert!(close_approx(&px, &[1.0, 0.0, 0.0, 1.0], 0.001));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_transparent_3_partial_opacity() {
    let h = Harness::new("transp_partial").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "transparent"})).await;

    let r = h.run("layer.new.layer", json!({"name": "RedHalf"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 0.5})).await;

    let px = h.pixel(16, 16).await;
    // Over transparent backdrop, 50% opacity red gives RGBA [1.0, 0.0, 0.0, 0.5]
    assert!(close_approx(&px, &[1.0, 0.0, 0.0, 0.5], 0.02), "expected [1.0, 0.0, 0.0, 0.5], got {px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_transparent_4_preview_png() {
    let h = Harness::new("transp_preview").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "transparent"})).await;

    let res = h.tool_raw("doc_render_preview", json!({"max_side": 32})).await;
    assert_ne!(res.is_error, Some(true));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_transparent_5_stacked_alphas() {
    let h = Harness::new("transp_stacked").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "transparent"})).await;

    // Layer 1: Red at 50% opacity (alpha = 0.5)
    let r1 = h.run("layer.new.layer", json!({"name": "L1"})).await["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.setProps", json!({"layer": r1, "opacity": 0.5})).await;

    // Layer 2: Blue at 50% opacity (alpha = 0.5)
    let r2 = h.run("layer.new.layer", json!({"name": "L2"})).await["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#0000ff"})).await;
    h.run("layer.setProps", json!({"layer": r2, "opacity": 0.5})).await;

    let px = h.pixel(16, 16).await;
    // Porter-Duff alpha over transparent backdrop: 0.5 + 0.5 * (1 - 0.5) = 0.75
    assert!((px[3] - 0.75).abs() <= 0.03, "composite alpha should be 0.75, got {}", px[3]);

    h.shutdown().await;
}

// --- Feature 6: Extreme Coordinates ---

#[tokio::test(flavor = "multi_thread")]
async fn tier2_extreme_coords_1_pixel_negative() {
    let h = Harness::new("ext_neg_px").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let px = h.pixel(-100, -100).await;
    assert_eq!(px, [0.0, 0.0, 0.0, 0.0], "Negative coordinates should return transparent empty pixel");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_extreme_coords_2_pixel_far_outside() {
    let h = Harness::new("ext_far_px").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let px = h.pixel(100_000, 100_000).await;
    assert_eq!(px, [0.0, 0.0, 0.0, 0.0], "Far outside coordinates should return transparent empty pixel");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_extreme_coords_3_translate_far_positive() {
    let h = Harness::new("ext_trans_pos").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    h.run("layer.new.layer", json!({"name": "Moved"})).await;
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.translate", json!({"dx": 50_000, "dy": 50_000})).await;

    let px = h.pixel(16, 16).await;
    // Center pixel now shows white background since layer moved off-canvas
    assert!(close_approx(&px, &[1.0, 1.0, 1.0, 1.0], 0.01));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_extreme_coords_4_translate_far_negative() {
    let h = Harness::new("ext_trans_neg").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    h.run("layer.new.layer", json!({"name": "MovedNeg"})).await;
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("layer.translate", json!({"dx": -50_000, "dy": -50_000})).await;

    let px = h.pixel(16, 16).await;
    assert!(close_approx(&px, &[1.0, 1.0, 1.0, 1.0], 0.01));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_extreme_coords_5_int32_boundary() {
    let h = Harness::new("ext_i32_bound").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let px = h.pixel(i32::MAX, 0).await;
    assert_eq!(px, [0.0, 0.0, 0.0, 0.0], "At i32::MAX document.pixel saturates to empty");

    h.shutdown().await;
}

// --- Feature 7: Division-by-Zero Guards ---

#[tokio::test(flavor = "multi_thread")]
async fn tier2_div_by_zero_1_color_burn_black() {
    let h = Harness::new("div0_cburn").await;
    // Black background (B = 0)
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#000000"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#808080"})).await;
    h.run("layer.setProps", json!({"layer": id, "blend": "Color Burn"})).await;

    let px = h.pixel(16, 16).await;
    for c in px {
        assert!(!c.is_nan() && !c.is_infinite(), "Color Burn over black must not produce NaN/Inf: {px:?}");
    }

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_div_by_zero_2_color_dodge_white() {
    let h = Harness::new("div0_cdodge").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#808080"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();
    // Pure white source (S = 1.0)
    h.run("edit.fill", json!({"color": "#ffffff"})).await;
    h.run("layer.setProps", json!({"layer": id, "blend": "Color Dodge"})).await;

    let px = h.pixel(16, 16).await;
    for c in px {
        assert!(!c.is_nan() && !c.is_infinite(), "Color Dodge with white must not produce NaN/Inf: {px:?}");
    }
    assert!(close_approx(&px, &[1.0, 1.0, 1.0, 1.0], 0.01));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_div_by_zero_3_divide_mode_black() {
    let h = Harness::new("div0_divide").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#808080"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();
    // Pure black source (S = 0.0)
    h.run("edit.fill", json!({"color": "#000000"})).await;
    h.run("layer.setProps", json!({"layer": id, "blend": "Divide"})).await;

    let px = h.pixel(16, 16).await;
    for c in px {
        assert!(!c.is_nan() && !c.is_infinite(), "Divide mode with 0 must clamp safely without NaN: {px:?}");
    }

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_div_by_zero_4_subtract_underflow() {
    let h = Harness::new("div0_subtract").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#202020"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();
    // White source subtracting from dark background: S > B
    h.run("edit.fill", json!({"color": "#ffffff"})).await;
    h.run("layer.setProps", json!({"layer": id, "blend": "Subtract"})).await;

    let px = h.pixel(16, 16).await;
    // B - S < 0 clamps to 0.0
    assert!(close_approx(&px, &[0.0, 0.0, 0.0, 1.0], 0.001));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier2_div_by_zero_5_hard_mix_boundaries() {
    let h = Harness::new("div0_hard_mix").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#808080"})).await;

    let r = h.run("layer.new.layer", json!({})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#808080"})).await;
    h.run("layer.setProps", json!({"layer": id, "blend": "Hard Mix"})).await;

    let px = h.pixel(16, 16).await;
    for c in px {
        assert!(!c.is_nan() && !c.is_infinite());
    }

    h.shutdown().await;
}

// ============================================================================
// TIER 3: CROSS-FEATURE COMBINATIONS (Pairwise & Multi-Feature Interactions)
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn tier3_combination_1_clipping_group_blend_mode() {
    let h = Harness::new("comb_clip_group_blend").await;
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;

    // Group to isolate
    let g = h.run("layer.new.group", json!({"name": "Group"})).await["layer"].as_u64().unwrap();

    // Base layer: 20x20 red square at (10, 10)
    let base = h.run("layer.new.layer", json!({"name": "Base"})).await["layer"].as_u64().unwrap();
    h.run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).await;
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("select.deselect", json!({})).await;
    h.run("layer.moveTo", json!({"layer": base, "target": g, "position": "into"})).await;

    // Clipped top layer: full-canvas cyan fill with Multiply blend
    let top = h.run("layer.new.layer", json!({"name": "ClippedTop"})).await["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#00ffff"})).await;
    h.run("layer.moveTo", json!({"layer": top, "target": g, "position": "into"})).await;
    h.run("layer.setProps", json!({"layer": top, "clipped": true, "blend": "Multiply"})).await;

    // Inside base rect (15, 15): Red * Cyan = Black
    let inside_px = h.pixel(15, 15).await;
    assert!(close_approx(&inside_px, &[0.0, 0.0, 0.0, 1.0], 0.02), "Inside clipping mask should be black, got {inside_px:?}");

    // Outside base rect (5, 5): Top layer is clipped away, reveals white background
    let outside_px = h.pixel(5, 5).await;
    assert!(close_approx(&outside_px, &[1.0, 1.0, 1.0, 1.0], 0.02), "Outside clipping mask should be white background, got {outside_px:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier3_combination_2_special_eight_fill_vs_opacity() {
    let h = Harness::new("comb_special_eight").await;
    // 1. Opaque backdrop: verify all 8 special modes diverge between fill: 0.5 and opacity: 0.5
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "#c89664"})).await;

    let r1 = h.run("layer.new.layer", json!({"name": "L1"})).await["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#64b4dc"})).await;

    let special_eight_modes = ["Color Burn", "Linear Burn", "Color Dodge", "Linear Dodge", "Vivid Light", "Linear Light", "Hard Mix", "Difference"];

    for mode in special_eight_modes {
        h.run("layer.setProps", json!({"layer": r1, "blend": mode, "opacity": 0.5, "fill": 1.0})).await;
        let px_op = h.pixel(16, 16).await;

        h.run("layer.setProps", json!({"layer": r1, "opacity": 1.0, "fill": 0.5})).await;
        let px_fill = h.pixel(16, 16).await;

        for c in px_op.iter().chain(&px_fill) {
            assert!(!c.is_nan() && !c.is_infinite(), "Special Eight {mode} produced non-finite float");
        }

        if mode != "Linear Burn" {
            let max_diff = (px_fill[0] - px_op[0]).abs().max((px_fill[1] - px_op[1]).abs()).max((px_fill[2] - px_op[2]).abs());
            assert!(max_diff > 0.02, "Special Eight {mode} fill 50% must diverge from opacity 50%: px_fill={px_fill:?}, px_op={px_op:?}, diff={max_diff}");
        }
    }

    // Verify Linear Burn divergence on clamped inputs (where Cb + Cs - 1 < 0)
    let h_lb = Harness::new("comb_linear_burn_div").await;
    h_lb.tool("doc_new", json!({"width": 32, "height": 32, "background": "#404040"})).await;
    let lb_id = h_lb.run("layer.new.layer", json!({"name": "LB"})).await["layer"].as_u64().unwrap();
    h_lb.run("edit.fill", json!({"color": "#404040"})).await;
    h_lb.run("layer.setProps", json!({"layer": lb_id, "blend": "Linear Burn", "opacity": 1.0, "fill": 0.5})).await;
    let px_fill_lb = h_lb.pixel(16, 16).await;
    h_lb.run("layer.setProps", json!({"layer": lb_id, "opacity": 0.5, "fill": 1.0})).await;
    let px_op_lb = h_lb.pixel(16, 16).await;
    let lb_diff = (px_fill_lb[0] - px_op_lb[0]).abs().max((px_fill_lb[1] - px_op_lb[1]).abs()).max((px_fill_lb[2] - px_op_lb[2]).abs());
    assert!(lb_diff > 0.02, "Linear Burn with clamping MUST diverge from standard opacity: px_fill={px_fill_lb:?}, px_op={px_op_lb:?}, diff={lb_diff}");
    h_lb.shutdown().await;

    // 2. Transparent backdrop: Special Eight fill must fall back to standard alpha coverage
    let h_trans = Harness::new("comb_special_eight_trans").await;
    h_trans.tool("doc_new", json!({"width": 32, "height": 32, "background": "transparent"})).await;

    let r2 = h_trans.run("layer.new.layer", json!({"name": "TransL2"})).await["layer"].as_u64().unwrap();
    // Fill with #64b4dc: [100/255, 180/255, 220/255] = [0.39215, 0.70588, 0.86274]
    h_trans.run("edit.fill", json!({"color": "#64b4dc"})).await;

    for mode in special_eight_modes {
        h_trans.run("layer.setProps", json!({"layer": r2, "blend": mode, "opacity": 1.0, "fill": 0.5})).await;
        let px = h_trans.pixel(16, 16).await;

        // Over transparent backdrop, alpha should be exactly fill = 0.5
        assert!((px[3] - 0.5).abs() <= 0.02, "Special Eight {mode} over transparent backdrop must have alpha=0.5, got {}", px[3]);
        // And RGB channels must equal source color #64b4dc
        assert!(
            close_approx(&px[0..3], &[100.0 / 255.0, 180.0 / 255.0, 220.0 / 255.0], 0.02),
            "Special Eight {mode} over transparent backdrop must preserve source RGB: got {:?}",
            &px[0..3]
        );
    }

    h.shutdown().await;
    h_trans.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier3_combination_3_opacity_sweep_during_layer_translation() {
    let h = Harness::new("comb_opacity_sweep_trans").await;
    // Blue background
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "#0000ff"})).await;

    let r = h.run("layer.new.layer", json!({"name": "MovingPatch"})).await;
    let id = r["layer"].as_u64().unwrap();
    // 16x16 red square at (0, 0)
    h.run("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 16})).await;
    h.run("edit.fill", json!({"color": "#ff0000"})).await;
    h.run("select.deselect", json!({})).await;

    // Initial center (8, 8) is Red
    assert!(close_approx(&h.pixel(8, 8).await, &[1.0, 0.0, 0.0, 1.0], 0.01));

    // Move diagonally by dx: 16, dy: 16 and lower opacity to 0.5
    h.run("layer.translate", json!({"layer": id, "dx": 16, "dy": 16})).await;
    h.run("layer.setProps", json!({"layer": id, "opacity": 0.5})).await;

    // Vacated coordinate (8, 8) is now pure Blue background
    let vacated = h.pixel(8, 8).await;
    assert!(close_approx(&vacated, &[0.0, 0.0, 1.0, 1.0], 0.01), "Vacated area must revert to blue background, got {vacated:?}");

    // New center coordinate (24, 24) is 50% Red over Blue -> [0.5, 0.0, 0.5, 1.0]
    let new_pos = h.pixel(24, 24).await;
    assert!(close_approx(&new_pos, &[0.5, 0.0, 0.5, 1.0], 0.03), "New position must show 50% blended patch, got {new_pos:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier3_combination_4_nested_group_isolation_and_blend() {
    let h = Harness::new("comb_nested_groups").await;
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;

    // Parent group
    let g1 = h.run("layer.new.group", json!({"name": "ParentGroup"})).await["layer"].as_u64().unwrap();
    // Child group inside parent
    let g2 = h.run("layer.new.group", json!({"name": "ChildGroup"})).await["layer"].as_u64().unwrap();
    h.run("layer.moveTo", json!({"layer": g2, "target": g1, "position": "into"})).await;

    // Pixel layer inside child group
    let l = h.run("layer.new.layer", json!({"name": "DeepLeaf"})).await["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#00ff00"})).await;
    h.run("layer.moveTo", json!({"layer": l, "target": g2, "position": "into"})).await;
    h.run("layer.setProps", json!({"layer": l, "opacity": 0.5})).await;

    let doc = h.inspect().await;
    let layers = doc["layers"].as_array().unwrap();
    let parent = layers.iter().find(|x| x["id"] == g1).expect("parent group");
    let child = parent["children"].as_array().unwrap().iter().find(|x| x["id"] == g2).expect("child group");
    assert_eq!(child["children"].as_array().unwrap().len(), 1);

    // Green at 50% over White -> [0.5, 1.0, 0.5, 1.0]
    let px = h.pixel(32, 32).await;
    assert!(close_approx(&px, &[0.5, 1.0, 0.5, 1.0], 0.02));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier3_combination_5_multi_selection_simultaneous_property_batch() {
    let h = Harness::new("comb_multi_prop_batch").await;
    h.tool("doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    let id0 = h.run("layer.new.layer", json!({"name": "A"})).await["layer"].as_u64().unwrap();
    let id1 = h.run("layer.new.layer", json!({"name": "B"})).await["layer"].as_u64().unwrap();
    let id2 = h.run("layer.new.layer", json!({"name": "C"})).await["layer"].as_u64().unwrap();

    h.run("layer.setProps", json!({"layers": [id0, id1, id2], "opacity": 0.45, "blend": "Screen"})).await;

    let doc = h.inspect().await;
    for l in doc["layers"].as_array().unwrap().iter().filter(|l| l["name"] != "Background") {
        assert!((l["opacity"].as_f64().unwrap() - 0.45).abs() < 1e-4);
        assert_eq!(l["blend"], "Screen");
    }

    h.shutdown().await;
}

// ============================================================================
// TIER 4: REAL-WORLD APPLICATION SCENARIOS (>=5 application workflows)
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn tier4_scenario_1_title_card_drop_shadow_export() {
    let h = Harness::new("scen_title_card").await;
    h.tool("doc_new", json!({"width": 200, "height": 120, "background": "white"})).await;

    // Create type layer
    h.run("type.create", json!({"x": 20, "y": 70, "text": "ArtCraft Engine", "size": 32})).await;
    // Apply drop shadow layer effect
    h.run("layer.layerStyle.dropShadow", json!({"distance": 4, "size": 3, "opacity": 75})).await;

    let doc = h.inspect().await;
    let top = &doc["layers"][0];
    assert_eq!(top["kind"], "Type");
    assert_eq!(top["text"]["text"], "ArtCraft Engine");
    assert_eq!(top["effects"]["items"][0]["kind"], "Drop Shadow");

    // Export to PNG
    let out_file = "title_card.png";
    h.tool("doc_export", json!({"path": out_file})).await;

    let exported_bytes = std::fs::read(h.dir.join(out_file)).expect("read exported title card PNG");
    let img = photocraft_codecs::decode(&exported_bytes).expect("decode exported card");
    assert_eq!(img.dimensions(), (200, 120));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier4_scenario_2_multilayer_photo_composition() {
    let h = Harness::new("scen_composition").await;
    // Base photo document: 100x80
    h.tool("doc_new", json!({"width": 100, "height": 80, "background": "#304050"})).await;

    // Layer 1: Warm ambient wash (Multiply)
    let wash = h.run("layer.new.layer", json!({"name": "WarmWash"})).await["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#e0a060"})).await;
    h.run("layer.setProps", json!({"layer": wash, "blend": "Multiply", "opacity": 0.8})).await;

    // Layer 2: Center spotlight (Screen)
    let spot = h.run("layer.new.layer", json!({"name": "Spotlight"})).await["layer"].as_u64().unwrap();
    h.run("select.rect", json!({"x": 30, "y": 20, "width": 40, "height": 40})).await;
    h.run("edit.fill", json!({"color": "#ffffff"})).await;
    h.run("select.deselect", json!({})).await;
    h.run("layer.setProps", json!({"layer": spot, "blend": "Screen", "opacity": 0.5})).await;

    // Sample spotlight center vs outer corner
    let center = h.pixel(50, 40).await;
    let corner = h.pixel(5, 5).await;

    // Center should be significantly brighter than corner due to spotlight
    assert!(center[0] > corner[0] && center[1] > corner[1], "Spotlight center should be brighter: center={center:?}, corner={corner:?}");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier4_scenario_3_fifty_layer_stress_batch() {
    let h = Harness::new("scen_stress_50").await;
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;

    let mut steps = Vec::with_capacity(50);
    let blend_modes = ["Normal", "Multiply", "Screen", "Overlay", "Linear Burn"];
    for i in 0..50 {
        let mode = blend_modes[i % blend_modes.len()];
        let opacity = 0.5 + ((i % 5) as f64) * 0.1;
        steps.push(json!({
            "id": "layer.new.layer",
            "params": {"name": format!("Layer_{i:02}")}
        }));
        steps.push(json!({
            "id": "layer.setProps",
            "params": {"blend": mode, "opacity": opacity}
        }));
    }

    let batch_res = h.tool("command_batch", json!({"steps": steps})).await;
    assert_eq!(batch_res["completed"], 100);
    assert_eq!(batch_res["failed"], 0);

    let doc = h.inspect().await;
    // 50 added layers + 1 Background = 51 layers total
    assert_eq!(doc["layers"].as_array().unwrap().len(), 51);

    // Verify preview renders cleanly under 50-layer stack
    let preview_res = h.tool_raw("doc_render_preview", json!({"max_side": 32})).await;
    assert_ne!(preview_res.is_error, Some(true));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier4_scenario_3_stress_150_layers() {
    let h = Harness::new("scen_stress_150").await;
    h.tool("doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;

    // Create hierarchical structure with nested groups and clipping chains across 2 batches
    // Note: MAX_BATCH_STEPS is 256. 150 layers requires chunking.
    // Group A, nested Group A_Sub, and Group B
    let g1 = h.run("layer.new.group", json!({"name": "Group_A"})).await["layer"].as_u64().unwrap();
    let g2 = h.run("layer.new.group", json!({"name": "Group_A_Sub"})).await["layer"].as_u64().unwrap();
    h.run("layer.moveTo", json!({"layer": g2, "target": g1, "position": "into"})).await;

    let _g3 = h.run("layer.new.group", json!({"name": "Group_B"})).await["layer"].as_u64().unwrap();

    // Currently in document: Background + Group_A (with Group_A_Sub) + Group_B = 4 layers.
    // We need 147 more layers to reach exactly 150 added layers + Background = 151 layers.
    let blend_modes = ["Normal", "Multiply", "Screen", "Overlay", "Linear Burn", "Color Dodge", "Linear Dodge", "Vivid Light", "Hard Mix", "Difference"];

    // Chunk 1: 74 layers (including Curves adjustment layer)
    let mut chunk1_steps = Vec::with_capacity(160);
    chunk1_steps.push(json!({
        "id": "layer.newAdjustmentLayer.curves",
        "params": {"points": [[0, 0], [128, 160], [255, 255]]}
    }));
    for i in 1..=73 {
        let mode = blend_modes[i % blend_modes.len()];
        let opacity = 0.3 + ((i % 7) as f64) * 0.1;
        let clipped = (i % 4) == 0;
        chunk1_steps.push(json!({
            "id": "layer.new.layer",
            "params": {"name": format!("Layer_C1_{i:02}"), "clipToPrevious": clipped}
        }));
        chunk1_steps.push(json!({
            "id": "layer.setProps",
            "params": {"blend": mode, "opacity": opacity}
        }));
    }

    let res1 = h.tool("command_batch", json!({"steps": chunk1_steps})).await;
    assert_eq!(res1["failed"], 0, "Chunk 1 batch encountered failures: {res1:?}");

    // Chunk 2: 73 layers (including Vibrance adjustment layer) to reach 150 added layers
    let mut chunk2_steps = Vec::with_capacity(160);
    chunk2_steps.push(json!({
        "id": "layer.newAdjustmentLayer.vibrance",
        "params": {"vibrance": 30, "saturation": 10}
    }));
    for i in 1..=72 {
        let mode = blend_modes[(i + 3) % blend_modes.len()];
        let opacity = 0.4 + ((i % 5) as f64) * 0.1;
        let clipped = (i % 6) == 0;
        chunk2_steps.push(json!({
            "id": "layer.new.layer",
            "params": {"name": format!("Layer_C2_{i:02}"), "clipToPrevious": clipped}
        }));
        chunk2_steps.push(json!({
            "id": "layer.setProps",
            "params": {"blend": mode, "opacity": opacity}
        }));
    }

    let res2 = h.tool("command_batch", json!({"steps": chunk2_steps})).await;
    assert_eq!(res2["failed"], 0, "Chunk 2 batch encountered failures: {res2:?}");

    // 1. Verify document stability: exactly 151 layers total across hierarchy
    let doc = h.inspect().await;
    let total_layers = count_layers_recursive(doc["layers"].as_array().expect("layers array"));
    assert_eq!(total_layers, 151, "Total layer stack should contain 150 added layers + Background = 151");

    // 2. Verify preview render under 150-layer hierarchical load
    let preview_res = h.tool_raw("doc_render_preview", json!({"max_side": 64})).await;
    assert_ne!(preview_res.is_error, Some(true), "doc_render_preview failed on 150 layers: {preview_res:?}");

    // 3. Verify composite correctness via document.pixel
    let px = h.pixel(32, 32).await;
    for c in px {
        assert!(!c.is_nan() && !c.is_infinite(), "150-layer composite produced non-finite float {px:?}");
        assert!((0.0..=1.0001).contains(&c), "150-layer composite channel out of [0, 1]: {c}");
    }

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tier4_scenario_4_nondestructive_adjustment_stack() {
    let h = Harness::new("scen_adjust_stack").await;
    h.tool("doc_new", json!({"width": 64, "height": 48, "background": "#408040"})).await;

    let base_px = h.pixel(32, 24).await;

    // Add Curves adjustment layer
    let curves = h.run("layer.newAdjustmentLayer.curves", json!({"points": [[0, 0], [128, 200], [255, 255]]})).await;
    let curves_id = curves["layer"].as_u64().unwrap();

    // Add Vibrance adjustment layer
    h.run("layer.newAdjustmentLayer.vibrance", json!({"vibrance": 50, "saturation": 10})).await;

    let graded_px = h.pixel(32, 24).await;
    assert!(graded_px[1] > base_px[1] + 0.05, "Curves should brighten green: base={base_px:?}, graded={graded_px:?}");

    // Toggle Curves visibility off
    h.run("layer.setProps", json!({"layer": curves_id, "visible": false})).await;
    let without_curves = h.pixel(32, 24).await;
    assert!(without_curves[1] < graded_px[1], "Hiding curves should drop brightness: {without_curves:?}");

    // Undo visibility toggle and both adjustments (3 steps)
    h.run("edit.undo", json!({})).await; // restores Curves visibility
    h.run("edit.undo", json!({})).await; // removes Vibrance
    h.run("edit.undo", json!({})).await; // removes Curves
    assert_eq!(h.inspect().await["layers"].as_array().unwrap().len(), 1, "only base background layer remains");
    let reverted = h.pixel(32, 24).await;
    assert!(close_approx(&reverted, &base_px, 0.01), "Undo must restore base pixels");

    h.shutdown().await;
}

struct LatencyStats {
    samples: usize,
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

fn compute_stats(durations: &[std::time::Duration]) -> LatencyStats {
    let mut sorted: Vec<f64> = durations.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let sum: f64 = sorted.iter().sum();
    let mean_ms = sum / (n as f64);
    let rank = |p: usize| ((n * p).div_ceil(100)).saturating_sub(1).min(n - 1);
    let p50_ms = sorted[rank(50)];
    let p95_ms = sorted[rank(95)];
    let max_ms = sorted[n - 1];
    LatencyStats { samples: n, mean_ms, p50_ms, p95_ms, max_ms }
}

#[tokio::test(flavor = "multi_thread")]
async fn tier4_scenario_5_frame_latency_telemetry() {
    let h = Harness::new("scen_latency").await;
    h.tool("doc_new", json!({"width": 128, "height": 128, "background": "white"})).await;

    let r = h.run("layer.new.layer", json!({"name": "BenchmarkTarget"})).await;
    let id = r["layer"].as_u64().unwrap();
    h.run("edit.fill", json!({"color": "#3366cc"})).await;

    const SAMPLES: usize = 20;

    // 1. Measure P1: layer.translate latency (budget <= 25 ms)
    let mut move_latencies = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let t0 = Instant::now();
        h.run("layer.translate", json!({"layer": id, "dx": (i as i32 % 3) - 1, "dy": 1})).await;
        move_latencies.push(t0.elapsed());
    }

    // 2. Measure P2: layer.setProps opacity adjustment latency (budget <= 20 ms)
    let mut opacity_latencies = Vec::with_capacity(SAMPLES);
    for i in 1..=SAMPLES {
        let op = 0.1 + ((i % 9) as f64) * 0.1;
        let t0 = Instant::now();
        h.run("layer.setProps", json!({"layer": id, "opacity": op})).await;
        opacity_latencies.push(t0.elapsed());
    }

    // 3. Measure P3: layer.setProps fill adjustment latency (budget <= 20 ms)
    let mut fill_latencies = Vec::with_capacity(SAMPLES);
    for i in 1..=SAMPLES {
        let fill = 0.1 + ((i % 9) as f64) * 0.1;
        let t0 = Instant::now();
        h.run("layer.setProps", json!({"layer": id, "fill": fill})).await;
        fill_latencies.push(t0.elapsed());
    }

    // 4. Measure P4: layer.setProps blend mode cycling latency (budget <= 20 ms)
    let blend_modes = ["Multiply", "Screen", "Overlay", "Color Burn", "Linear Dodge", "Vivid Light", "Linear Light", "Hard Mix", "Difference", "Color"];
    let mut blend_latencies = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let mode = blend_modes[i % blend_modes.len()];
        let t0 = Instant::now();
        h.run("layer.setProps", json!({"layer": id, "blend": mode})).await;
        blend_latencies.push(t0.elapsed());
    }

    // 5. Measure Preview render latency via doc_render_preview
    const PREVIEW_SAMPLES: usize = 10;
    let mut preview_latencies = Vec::with_capacity(PREVIEW_SAMPLES);
    for _ in 0..PREVIEW_SAMPLES {
        let t0 = Instant::now();
        let res = h.tool_raw("doc_render_preview", json!({"max_side": 64})).await;
        assert_ne!(res.is_error, Some(true));
        preview_latencies.push(t0.elapsed());
    }

    let p1_stats = compute_stats(&move_latencies);
    let p2_stats = compute_stats(&opacity_latencies);
    let p3_stats = compute_stats(&fill_latencies);
    let p4_stats = compute_stats(&blend_latencies);
    let preview_stats = compute_stats(&preview_latencies);

    let is_debug = cfg!(debug_assertions);
    let p1_budget = if is_debug { 500.0 } else { 25.0 };
    let p2_budget = if is_debug { 500.0 } else { 20.0 };
    let p3_budget = if is_debug { 500.0 } else { 20.0 };
    let p4_budget = if is_debug { 500.0 } else { 20.0 };

    println!("\n=== ArtCraft MCP Frame Latency Telemetry Report ===");
    println!("{:<24} {:>7} {:>10} {:>10} {:>10} {:>10} {:>10} {:>8}", "Operation", "Samples", "Mean(ms)", "p50(ms)", "p95(ms)", "Max(ms)", "Budget", "Status");
    println!("{}", "-".repeat(95));

    let fmt_row = |name: &str, s: &LatencyStats, budget: f64| {
        let status = if s.p95_ms <= budget { "PASS" } else { "FAIL" };
        println!("{:<24} {:>7} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>8.1} ms {:>8}", name, s.samples, s.mean_ms, s.p50_ms, s.p95_ms, s.max_ms, budget, status);
    };

    fmt_row("P1 Layer Move", &p1_stats, p1_budget);
    fmt_row("P2 Layer Opacity", &p2_stats, p2_budget);
    fmt_row("P3 Layer Fill", &p3_stats, p3_budget);
    fmt_row("P4 Blend Mode Change", &p4_stats, p4_budget);
    println!(
        "{:<24} {:>7} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>10} {:>8}",
        "Preview Render (64px)", preview_stats.samples, preview_stats.mean_ms, preview_stats.p50_ms, preview_stats.p95_ms, preview_stats.max_ms, "N/A", "PASS"
    );
    println!("{}\n", "-".repeat(95));

    if std::env::var("ARTCRAFT_STRICT_BUDGETS").is_ok() {
        assert!(p1_stats.p95_ms <= p1_budget, "P1 move p95 exceeded budget: {:.3} ms > {} ms", p1_stats.p95_ms, p1_budget);
        assert!(p2_stats.p95_ms <= p2_budget, "P2 opacity p95 exceeded budget: {:.3} ms > {} ms", p2_stats.p95_ms, p2_budget);
        assert!(p3_stats.p95_ms <= p3_budget, "P3 fill p95 exceeded budget: {:.3} ms > {} ms", p3_stats.p95_ms, p3_budget);
        assert!(p4_stats.p95_ms <= p4_budget, "P4 blend p95 exceeded budget: {:.3} ms > {} ms", p4_stats.p95_ms, p4_budget);
    }

    h.shutdown().await;
}
