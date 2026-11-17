//! Verify the SVG validator uses the selected renderer backends in this app graph.
//! Resolve the direct IO edge: other pinned craft-svg versions may coexist.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PackageId {
    name: String,
    version: String,
    source: Option<String>,
}
struct Package {
    id: PackageId,
    dependencies: Vec<String>,
}
pub(crate) struct LockGraph {
    packages: Vec<Package>,
}
impl LockGraph {
    pub(crate) fn parse(text: &str) -> Self {
        let lock: toml::Value = toml::from_str(text).unwrap();
        let packages = lock
            .get("package")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|package| {
                let string = |key| package.get(key).unwrap().as_str().unwrap().to_owned();
                Package {
                    id: PackageId { name: string("name"), version: string("version"), source: package.get("source").map(|v| v.as_str().unwrap().to_owned()) },
                    dependencies: package
                        .get("dependencies")
                        .map(|d| d.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect())
                        .unwrap_or_default(),
                }
            })
            .collect();
        Self { packages }
    }
    pub(crate) fn workspace_package(&self, name: &str) -> PackageId {
        let found: Vec<_> = self.packages.iter().filter(|p| p.id.name == name && p.id.source.is_none()).collect();
        assert_eq!(found.len(), 1, "one exact workspace package required: {name}");
        found[0].id.clone()
    }
    pub(crate) fn dependency(&self, from: &PackageId, name: &str) -> PackageId {
        let package = self.packages.iter().find(|p| &p.id == from).unwrap();
        let edges: Vec<_> = package.dependencies.iter().filter(|edge| edge.split_whitespace().next() == Some(name)).collect();
        assert_eq!(edges.len(), 1, "one {name} dependency edge required from {from:?}");
        let fields: Vec<_> = edges[0].split_whitespace().collect();
        let version = fields.get(1).copied();
        let source = fields.get(2).map(|s| s.trim_start_matches('(').trim_end_matches(')'));
        let found: Vec<_> = self
            .packages
            .iter()
            .filter(|p| p.id.name == name && version.is_none_or(|v| p.id.version == v) && source.is_none_or(|s| p.id.source.as_deref() == Some(s)))
            .collect();
        assert_eq!(found.len(), 1, "ambiguous or absent resolved edge {} from {from:?}", edges[0]);
        found[0].id.clone()
    }
}

pub(crate) fn assert_renderer_edges(graph: &LockGraph, svg: &PackageId) {
    let renderer = graph.dependency(svg, "resvg");
    assert_eq!(graph.dependency(svg, "usvg"), graph.dependency(&renderer, "usvg"), "shared and renderer trees must be the same resolved package");
    for codec in ["gif", "image-webp", "zune-jpeg"] {
        assert_eq!(graph.dependency(svg, codec), graph.dependency(&renderer, codec), "validator and selected renderer differ for {codec}");
    }
}

#[test]
fn svg_validator_and_selected_renderer_share_resolved_backend_packages() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let graph = LockGraph::parse(&std::fs::read_to_string(file).unwrap());
    let svg = graph.dependency(&graph.workspace_package("photocraft-io"), "craft-svg");
    assert_renderer_edges(&graph, &svg);
}

#[test]
fn dependency_lookup_follows_the_selected_edge_and_rejects_ambiguity() {
    let graph = LockGraph::parse(
        r#"
[[package]]
name = "consumer"
version = "1"
dependencies = ["codec 1 (registry+one)"]
[[package]]
name = "codec"
version = "1"
source = "registry+one"
[[package]]
name = "codec"
version = "1"
source = "registry+two"
[[package]]
name = "other"
version = "1"
dependencies = ["codec"]
"#,
    );
    let selected = graph.dependency(&graph.workspace_package("consumer"), "codec");
    assert_eq!(selected.source.as_deref(), Some("registry+one"));
    assert!(std::panic::catch_unwind(|| graph.dependency(&graph.workspace_package("other"), "codec")).is_err());
}
