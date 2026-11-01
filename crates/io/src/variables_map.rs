//! Image › Variables and Data Sets ↔ PSD image resources 7000 and 7001.
//!
//! The Adobe Photoshop File Formats Specification lists resource 7000 as "Image Ready variables.
//! XML representation of variables definition" and 7001 as "Image Ready data sets". Both hold XML
//! in Adobe's public variables vocabulary (the `http://ns.adobe.com/Variables/1.0/` namespace that
//! Photoshop's File › Export › Data Sets and Illustrator's variable libraries use):
//!
//! ```xml
//! <variableSets xmlns="http://ns.adobe.com/Variables/1.0/">
//!   <variableSet varSetName="binding1" locked="none">
//!     <variables>
//!       <variable varName="title" trait="textcontent" docRef="id('12')" category="…"/>
//!     </variables>
//!   </variableSet>
//! </variableSets>
//! ```
//!
//! `trait` is `visibility`, `textcontent` or `fileref`; `docRef` names the bound layer by its PSD
//! layer id (`lyid`). A Pixel Replacement variable also carries `placementMethod`, `align` and
//! `clip`. The data sets resource holds `sampleDataSet` rows whose children are named after the
//! variables (text split into `<p>` lines, visibility as `true`/`false`, a pixel value as a path).
//! A variable name that isn't an XML name is written as `<v:value v:varName="…">` instead.
//!
//! Import reads both resources after layer ids are known. Export keeps the preserved resources
//! byte for byte while they still decode to the document's variables, else writes fresh ones
//! (or none when the document has no variables).

use std::collections::{HashMap, HashSet};

use photocraft_doc::variables::Value;
use photocraft_doc::{DataSet, DataValue, Document, LayerId, PixelAlign, PixelMethod, VarKind, VariableDef, Variables};
use roxmltree::Node;

/// Image resource id of the variable definitions.
pub const VARIABLES: u16 = 7000;
/// Image resource id of the data sets.
pub const DATA_SETS: u16 = 7001;

const NS_VARS: &str = "http://ns.adobe.com/Variables/1.0/";
const NS_FLOWS: &str = "http://ns.adobe.com/Flows/1.0/";
const NS_CUSTOM: &str = "http://ns.adobe.com/GenericCustomNamespace/1.0/";
/// Entities Adobe's variables XML declares in its DOCTYPE, expanded before parsing (an internal
/// DTD subset is never handed to the parser).
const ENTITIES: [(&str, &str); 7] = [
    ("&ns_vars;", NS_VARS),
    ("&ns_flows;", NS_FLOWS),
    ("&ns_custom;", NS_CUSTOM),
    ("&ns_imrep;", "http://ns.adobe.com/ImageReplacement/1.0/"),
    ("&ns_graphs;", "http://ns.adobe.com/Graphs/1.0/"),
    ("&ns_extend;", "http://ns.adobe.com/Extensibility/1.0/"),
    ("&ns_sfw;", "http://ns.adobe.com/SaveForWeb/1.0/"),
];

fn raw(doc: &Document, id: u16) -> Option<&[u8]> {
    doc.metadata.psd_resources.iter().find(|(rid, _, _)| *rid == id).map(|(_, _, d)| d.as_slice())
}

/// The resource's XML text: UTF-8 (a BOM, leading bytes before the first tag and trailing NULs
/// dropped). Anything else is unreadable.
fn xml_text(data: &[u8]) -> Option<String> {
    let start = data.iter().position(|&b| b == b'<')?;
    let end = data.iter().rposition(|&b| b != 0).map_or(start, |e| e.saturating_add(1));
    let text = std::str::from_utf8(data.get(start..end)?).ok()?;
    Some(strip_doctype(text))
}

/// `text` without its DOCTYPE (and the internal subset Adobe puts there), with the known Adobe
/// namespace entities expanded.
fn strip_doctype(text: &str) -> String {
    let mut out = text.to_string();
    if let Some(at) = out.find("<!DOCTYPE") {
        let rest = out.get(at..).unwrap_or("");
        let end = match (rest.find('['), rest.find('>')) {
            (Some(open), Some(gt)) if open < gt => rest.find("]>").map(|e| e + 2),
            (_, Some(gt)) => Some(gt + 1),
            _ => None,
        };
        let end = end.map_or(out.len(), |e| at.saturating_add(e).min(out.len()));
        out.replace_range(at..end, "");
    }
    for (entity, uri) in ENTITIES {
        out = out.replace(entity, uri);
    }
    out
}

fn parse_kind(n: Node<'_, '_>) -> Option<VarKind> {
    match n.attribute("trait")? {
        "visibility" => Some(VarKind::Visibility),
        "textcontent" => Some(VarKind::TextReplacement),
        "fileref" => {
            let method = match n.attribute("placementMethod") {
                Some("fill") => PixelMethod::Fill,
                Some("as-is" | "asIs") => PixelMethod::AsIs,
                Some("conform") => PixelMethod::Conform,
                _ => PixelMethod::Fit,
            };
            let align = n.attribute("align").and_then(align_from).unwrap_or_default();
            let clip = n.attribute("clip") == Some("true");
            Some(VarKind::PixelReplacement { method, align, clip })
        }
        _ => None,
    }
}

const ALIGNS: [(PixelAlign, &str); 9] = [
    (PixelAlign::TopLeft, "top-left"),
    (PixelAlign::TopCenter, "top-center"),
    (PixelAlign::TopRight, "top-right"),
    (PixelAlign::CenterLeft, "center-left"),
    (PixelAlign::Center, "center"),
    (PixelAlign::CenterRight, "center-right"),
    (PixelAlign::BottomLeft, "bottom-left"),
    (PixelAlign::BottomCenter, "bottom-center"),
    (PixelAlign::BottomRight, "bottom-right"),
];

fn align_from(s: &str) -> Option<PixelAlign> {
    ALIGNS.iter().find(|(_, n)| *n == s).map(|(a, _)| *a)
}

fn align_name(a: PixelAlign) -> &'static str {
    ALIGNS.iter().find(|(x, _)| *x == a).map_or("center", |(_, n)| n)
}

fn method_name(m: PixelMethod) -> &'static str {
    match m {
        PixelMethod::Fit => "fit",
        PixelMethod::Fill => "fill",
        PixelMethod::AsIs => "as-is",
        PixelMethod::Conform => "conform",
    }
}

/// The PSD layer id in a `docRef` such as `id('12')` (or a bare number).
fn doc_ref_id(s: &str) -> Option<u32> {
    let s = s.trim();
    let inner = s.strip_prefix("id(").and_then(|r| r.strip_suffix(')')).unwrap_or(s);
    inner.trim().trim_matches(|c| c == '\'' || c == '"').trim().parse().ok()
}

fn decode_defs(xml: &str, by_psd_id: &HashMap<u32, LayerId>, warnings: &mut Vec<String>) -> Vec<VariableDef> {
    let doc = match crate::xml::parse(xml, Default::default()) {
        Ok(d) => d,
        Err(e) => {
            warnings.push(format!("image variables (resource {VARIABLES}) couldn't be read: {e}"));
            return Vec::new();
        }
    };
    let mut defs: Vec<VariableDef> = Vec::new();
    // Bindings already taken, and each name's kind: the resource comes from the file, so lookups
    // stay O(1) (#10128). Photoshop links several layers to one variable, so a name may repeat on
    // other layers; it can't change type, since a data set's value is typed by the name.
    let mut seen: HashSet<(&str, LayerId)> = HashSet::new();
    let mut kinds: HashMap<&str, std::mem::Discriminant<VarKind>> = HashMap::new();
    let mut skipped = 0usize;
    for n in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "variable") {
        let Some(name) = n.attribute("varName").filter(|s| !s.is_empty()) else {
            skipped += 1;
            continue;
        };
        let layer = n.attribute("docRef").and_then(doc_ref_id).and_then(|id| by_psd_id.get(&id).copied());
        let (Some(layer), Some(kind)) = (layer, parse_kind(n)) else {
            skipped += 1;
            continue;
        };
        let ty = std::mem::discriminant(&kind);
        if *kinds.entry(name).or_insert(ty) != ty || !seen.insert((name, layer)) {
            skipped += 1;
            continue;
        }
        defs.push(VariableDef { name: name.to_string(), layer, kind });
    }
    if skipped > 0 {
        warnings.push(format!("{skipped} image variable(s) without a known layer or type were not imported"));
    }
    defs
}

/// The text of a text-replacement value: its `<p>` lines joined by newlines, else its text.
fn element_text(n: Node<'_, '_>) -> String {
    let paras: Vec<String> = n
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "p")
        .map(|p| p.descendants().filter(Node::is_text).filter_map(|d| d.text()).collect())
        .collect();
    if paras.is_empty() { n.descendants().filter(Node::is_text).filter_map(|d| d.text()).collect::<String>().trim().to_string() } else { paras.join("\n") }
}

fn decode_sets(xml: &str, defs: &[VariableDef], warnings: &mut Vec<String>) -> Vec<DataSet> {
    let doc = match crate::xml::parse(xml, Default::default()) {
        Ok(d) => d,
        Err(e) => {
            warnings.push(format!("image data sets (resource {DATA_SETS}) couldn't be read: {e}"));
            return Vec::new();
        }
    };
    let mut sets = Vec::new();
    // The first definition of a name types its values, as a linear search would find it.
    let mut by_name: HashMap<&str, &VariableDef> = HashMap::new();
    for d in defs {
        by_name.entry(d.name.as_str()).or_insert(d);
    }
    for row in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "sampleDataSet") {
        let name = row.attribute("dataSetName").unwrap_or_default().to_string();
        let mut values: Vec<DataValue> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for cell in row.children().filter(Node::is_element) {
            let tag = cell.tag_name();
            let variable = if tag.namespace() == Some(NS_VARS) && tag.name() == "value" {
                cell.attribute((NS_VARS, "varName")).or_else(|| cell.attribute("varName")).unwrap_or_default()
            } else {
                tag.name()
            };
            // A value is typed by its variable; one with no definition can't be typed.
            let Some(def) = by_name.get(variable).copied() else { continue };
            let value = match def.kind {
                VarKind::Visibility => Value::Visibility(element_text(cell) != "false"),
                VarKind::TextReplacement => Value::Text(element_text(cell)),
                VarKind::PixelReplacement { .. } => Value::Pixels(element_text(cell)),
            };
            if seen.insert(variable) {
                values.push(DataValue { variable: variable.to_string(), value });
            }
        }
        sets.push(DataSet { name, values });
    }
    sets
}

/// The document's variables from the raw resources, plus warnings for what couldn't be read.
pub fn variables_from_psd(defs_raw: Option<&[u8]>, sets_raw: Option<&[u8]>, doc: &Document) -> (Variables, Vec<String>) {
    let mut warnings = Vec::new();
    let Some(defs_raw) = defs_raw else { return (Variables::default(), warnings) };
    let by_psd_id: HashMap<u32, LayerId> = doc.walk().into_iter().filter_map(|(_, _, l)| l.psd_id.map(|p| (p, l.id))).collect();
    let Some(xml) = xml_text(defs_raw) else {
        warnings.push(format!("image variables (resource {VARIABLES}) aren't UTF-8 XML and were not imported"));
        return (Variables::default(), warnings);
    };
    let defs = decode_defs(&xml, &by_psd_id, &mut warnings);
    let data_sets = match sets_raw.map(xml_text) {
        Some(Some(xml)) => decode_sets(&xml, &defs, &mut warnings),
        Some(None) => {
            warnings.push(format!("image data sets (resource {DATA_SETS}) aren't UTF-8 XML and were not imported"));
            Vec::new()
        }
        None => Vec::new(),
    };
    (Variables { defs, data_sets, active: None }, warnings)
}

/// Sets `doc.variables` from its preserved resources (called after layers and ids are imported).
pub fn import(doc: &mut Document) -> Vec<String> {
    let (vars, warnings) = variables_from_psd(raw(doc, VARIABLES), raw(doc, DATA_SETS), doc);
    doc.variables = vars;
    warnings
}

/// Do the document's variables still match the preserved resources? (`active` isn't stored.)
pub fn unchanged(doc: &Document) -> bool {
    let (v, _) = variables_from_psd(raw(doc, VARIABLES), raw(doc, DATA_SETS), doc);
    v.defs == doc.variables.defs && v.data_sets == doc.variables.data_sets
}

/// XML text with the special characters escaped (control characters other than tab and newline
/// dropped: XML 1.0 can't hold them).
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Can `s` be an element name as is (an ASCII NCName that doesn't start with `xml`)?
fn is_element_name(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !s.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("xml"))
}

fn defs_xml(defs: &[(&VariableDef, u32)]) -> String {
    let mut x = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<variableSets xmlns=\"{NS_VARS}\">\n<variableSet varSetName=\"binding1\" locked=\"none\">\n<variables>\n"
    );
    for (d, id) in defs {
        let (tr, category) = match d.kind {
            VarKind::Visibility => ("visibility", NS_VARS),
            VarKind::TextReplacement => ("textcontent", NS_FLOWS),
            VarKind::PixelReplacement { .. } => ("fileref", NS_VARS),
        };
        x.push_str(&format!("<variable varName=\"{}\" trait=\"{tr}\" category=\"{category}\" docRef=\"id('{id}')\"", escape(&d.name)));
        if let VarKind::PixelReplacement { method, align, clip } = d.kind {
            x.push_str(&format!(" placementMethod=\"{}\" align=\"{}\" clip=\"{clip}\"", method_name(method), align_name(align)));
        }
        x.push_str("/>\n");
    }
    x.push_str("</variables>\n</variableSet>\n</variableSets>\n");
    x
}

fn sets_xml(sets: &[DataSet], written: &HashSet<&str>) -> String {
    let mut x = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<v:sampleDataSets xmlns:v=\"{NS_VARS}\" xmlns=\"{NS_CUSTOM}\">\n");
    for s in sets {
        x.push_str(&format!("<v:sampleDataSet dataSetName=\"{}\">\n", escape(&s.name)));
        for dv in s.values.iter().filter(|v| written.contains(v.variable.as_str())) {
            let body = match &dv.value {
                Value::Visibility(b) => b.to_string(),
                Value::Text(t) => t.split('\n').map(|line| format!("<p>{}</p>", escape(line))).collect(),
                Value::Pixels(p) => escape(p),
            };
            if is_element_name(&dv.variable) {
                x.push_str(&format!("<{0}>{body}</{0}>\n", dv.variable));
            } else {
                x.push_str(&format!("<v:value v:varName=\"{}\">{body}</v:value>\n", escape(&dv.variable)));
            }
        }
        x.push_str("</v:sampleDataSet>\n");
    }
    x.push_str("</v:sampleDataSets>\n");
    x
}

/// An image resource to write: its id and data.
pub type Resource = (u16, Vec<u8>);

/// Is `id` one of the resources this module reads and writes?
pub fn is_variables_resource(id: u16) -> bool {
    matches!(id, VARIABLES | DATA_SETS)
}

/// The resources to write as `(id, data)`: `None` keeps the preserved ones, `Some` replaces them
/// (empty when the document has no variables, so they're dropped). `layer_ids` maps document
/// layers to the PSD layer ids being written; a variable whose layer isn't written is dropped
/// with a warning.
pub fn export_resources(doc: &Document, layer_ids: &HashMap<LayerId, u32>) -> (Option<Vec<Resource>>, Option<String>) {
    if unchanged(doc) {
        return (None, None);
    }
    let defs: Vec<(&VariableDef, u32)> = doc.variables.defs.iter().filter_map(|d| layer_ids.get(&d.layer).map(|id| (d, *id))).collect();
    let dropped = doc.variables.defs.len().saturating_sub(defs.len());
    let warning = (dropped > 0).then(|| format!("{dropped} image variable(s) bound to a layer that isn't saved were dropped"));
    if defs.is_empty() {
        return (Some(Vec::new()), warning);
    }
    let written: HashSet<&str> = defs.iter().map(|(d, _)| d.name.as_str()).collect();
    let mut out = vec![(VARIABLES, defs_xml(&defs).into_bytes())];
    if !doc.variables.data_sets.is_empty() {
        out.push((DATA_SETS, sets_xml(&doc.variables.data_sets, &written).into_bytes()));
    }
    (Some(out), warning)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Layer, SampleType, Size};

    fn doc_with_layers() -> (Document, LayerId, LayerId, HashMap<LayerId, u32>) {
        let mut d = Document::new("t.psd", Size::new(40, 30), ColorMode::Rgb, SampleType::U8);
        let mut a = Layer::raster("Title", d.pixel_format());
        a.psd_id = Some(12);
        let mut b = Layer::raster("Photo", d.pixel_format());
        b.psd_id = Some(7);
        let (ia, ib) = (a.id, b.id);
        d.layers.push(a);
        d.layers.push(b);
        (d, ia, ib, [(ia, 12), (ib, 7)].into_iter().collect())
    }

    fn sample(a: LayerId, b: LayerId) -> Variables {
        Variables {
            defs: vec![
                VariableDef { name: "VenueName".into(), layer: a, kind: VarKind::TextReplacement },
                VariableDef { name: "show".into(), layer: b, kind: VarKind::Visibility },
                VariableDef {
                    name: "photo 1".into(),
                    layer: b,
                    kind: VarKind::PixelReplacement { method: PixelMethod::Fill, align: PixelAlign::BottomRight, clip: true },
                },
            ],
            data_sets: vec![
                DataSet {
                    name: "Row <1> & \"more\"".into(),
                    values: vec![
                        DataValue { variable: "VenueName".into(), value: Value::Text("The Hall\nLine 2 & <b>".into()) },
                        DataValue { variable: "show".into(), value: Value::Visibility(false) },
                        DataValue { variable: "photo 1".into(), value: Value::Pixels("/tmp/a b.png".into()) },
                    ],
                },
                DataSet { name: "empty".into(), values: vec![] },
            ],
            active: None,
        }
    }

    fn with_resources(d: &Document, res: Vec<(u16, Vec<u8>)>) -> Document {
        let mut back = d.clone();
        back.variables = Variables::default();
        for (id, data) in res {
            back.metadata.psd_resources.push((id, String::new(), std::sync::Arc::new(data)));
        }
        back
    }

    #[test]
    fn variables_and_data_sets_round_trip() {
        let (mut d, a, b, ids) = doc_with_layers();
        d.variables = sample(a, b);
        let (res, warning) = export_resources(&d, &ids);
        assert!(warning.is_none());
        let mut back = with_resources(&d, res.unwrap());
        let w = import(&mut back);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back.variables, d.variables);
        assert!(unchanged(&back));
        assert_eq!(export_resources(&back, &ids), (None, None), "unchanged → keep the raw resources");
    }

    #[test]
    fn reads_adobe_style_xml_with_a_doctype() {
        let (d, a, _, _) = doc_with_layers();
        let defs = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 20001102//EN\" \"x.dtd\" [\n<!ENTITY ns_vars \"http://ns.adobe.com/Variables/1.0/\">\n<!ENTITY ns_flows \"http://ns.adobe.com/Flows/1.0/\">\n]>\n<variableSets xmlns=\"&ns_vars;\"><variableSet locked=\"none\" varSetName=\"binding1\"><variables><variable varName=\"title\" trait=\"textcontent\" category=\"&ns_flows;\" docRef=\"id('12')\"></variable></variables></variableSet></variableSets>\0";
        let sets = "<v:sampleDataSets xmlns:v=\"http://ns.adobe.com/Variables/1.0/\" xmlns=\"http://ns.adobe.com/GenericCustomNamespace/1.0/\"><v:sampleDataSet dataSetName=\"Data Set 1\"><title><p>Hello</p><p>World</p></title></v:sampleDataSet></v:sampleDataSets>";
        let mut back = with_resources(&d, vec![(VARIABLES, defs.as_bytes().to_vec()), (DATA_SETS, sets.as_bytes().to_vec())]);
        let w = import(&mut back);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back.variables.defs, vec![VariableDef { name: "title".into(), layer: a, kind: VarKind::TextReplacement }]);
        assert_eq!(back.variables.data_sets[0].get("title"), Some(&Value::Text("Hello\nWorld".into())));
    }

    /// Photoshop links several layers to one variable: every binding survives import and a
    /// re-save, so editing the variables doesn't drop the later layers from the file.
    #[test]
    fn a_variable_shared_by_two_layers_keeps_both_bindings() {
        let (mut d, a, b, ids) = doc_with_layers();
        d.variables = Variables {
            defs: vec![
                VariableDef { name: "show".into(), layer: a, kind: VarKind::Visibility },
                VariableDef { name: "show".into(), layer: b, kind: VarKind::Visibility },
            ],
            data_sets: vec![DataSet { name: "Row".into(), values: vec![DataValue { variable: "show".into(), value: Value::Visibility(false) }] }],
            active: None,
        };
        let (res, warning) = export_resources(&d, &ids);
        assert!(warning.is_none());
        let mut back = with_resources(&d, res.unwrap());
        let w = import(&mut back);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back.variables, d.variables);
        // A repeated binding of one layer, or a second type for the name, is still refused.
        let defs = "<variableSets xmlns=\"http://ns.adobe.com/Variables/1.0/\"><variableSet><variables>\
            <variable varName=\"show\" trait=\"visibility\" docRef=\"id('12')\"/>\
            <variable varName=\"show\" trait=\"visibility\" docRef=\"id('12')\"/>\
            <variable varName=\"show\" trait=\"textcontent\" docRef=\"id('7')\"/>\
            </variables></variableSet></variableSets>";
        let mut back = with_resources(&d, vec![(VARIABLES, defs.as_bytes().to_vec())]);
        let w = import(&mut back);
        assert_eq!(back.variables.defs, vec![VariableDef { name: "show".into(), layer: a, kind: VarKind::Visibility }]);
        assert_eq!(w.len(), 1, "{w:?}");
    }

    /// A file with many variables imports in linear time and keeps the first of each duplicate:
    /// the first definition of a name, and the first value a data set gives it.
    #[test]
    fn many_variables_import_and_duplicates_keep_the_first() {
        let (d, a, b, _) = doc_with_layers();
        const N: usize = 20_000;
        let mut defs = String::from("<variableSets xmlns=\"http://ns.adobe.com/Variables/1.0/\"><variableSet><variables>");
        for i in 0..N {
            defs.push_str(&format!("<variable varName=\"v{i}\" trait=\"textcontent\" docRef=\"id('12')\"/>"));
        }
        defs.push_str("<variable varName=\"v0\" trait=\"visibility\" docRef=\"id('7')\"/></variables></variableSet></variableSets>");
        let mut sets = String::from("<v:sampleDataSets xmlns:v=\"http://ns.adobe.com/Variables/1.0/\"><v:sampleDataSet dataSetName=\"Row\">");
        for i in 0..N {
            sets.push_str(&format!("<v{i}>t{i}</v{i}>"));
        }
        sets.push_str("<v0>again</v0><undefined>x</undefined></v:sampleDataSet></v:sampleDataSets>");
        let mut back = with_resources(&d, vec![(VARIABLES, defs.into_bytes()), (DATA_SETS, sets.into_bytes())]);
        let w = import(&mut back);
        assert_eq!(w.len(), 1, "the duplicate is reported: {w:?}");
        assert_eq!(back.variables.defs.len(), N);
        assert_eq!(back.variables.defs[0], VariableDef { name: "v0".into(), layer: a, kind: VarKind::TextReplacement });
        assert!(back.variables.defs.iter().all(|v| v.layer != b));
        let row = &back.variables.data_sets[0];
        assert_eq!(row.values.len(), N, "the undefined cell and the repeat are dropped");
        assert_eq!(row.get("v0"), Some(&Value::Text("t0".into())));
        assert_eq!(row.get(&format!("v{}", N - 1)), Some(&Value::Text(format!("t{}", N - 1))));
    }

    #[test]
    fn malformed_resources_are_ignored_with_a_warning() {
        let (d, _, _, ids) = doc_with_layers();
        let deep = format!("<a>{}{}</a>", "<g>".repeat(100_000), "</g>".repeat(100_000));
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0xff, 0xfe, 0x00],
            b"<variableSets".to_vec(),
            b"<!DOCTYPE x [ <!ENTITY a \"b\"".to_vec(),
            b"<variableSets><variable varName=\"x\" trait=\"textcontent\" docRef=\"id('99999999999')\"/><variable trait=\"visibility\"/></variableSets>"
                .to_vec(),
            deep.into_bytes(),
            vec![b'<', 0xc3],
        ];
        for raw in cases {
            let mut back = with_resources(&d, vec![(VARIABLES, raw.clone()), (DATA_SETS, raw)]);
            let _ = import(&mut back);
            assert!(back.variables.is_empty());
            // Unreadable resources are kept verbatim.
            assert_eq!(export_resources(&back, &ids), (None, None));
        }
    }

    #[test]
    fn variables_on_unsaved_layers_are_dropped_with_a_warning() {
        let (mut d, a, b, _) = doc_with_layers();
        d.variables = sample(a, b);
        let ids: HashMap<LayerId, u32> = [(a, 12)].into_iter().collect();
        let (res, warning) = export_resources(&d, &ids);
        assert!(warning.is_some());
        let mut back = with_resources(&d, res.unwrap());
        let _ = import(&mut back);
        assert_eq!(back.variables.defs.len(), 1);
        assert_eq!(back.variables.data_sets[0].values.len(), 1);
    }

    #[test]
    fn clearing_variables_drops_the_resources() {
        let (mut d, a, b, ids) = doc_with_layers();
        d.variables = sample(a, b);
        let (res, _) = export_resources(&d, &ids);
        let mut back = with_resources(&d, res.unwrap());
        let _ = import(&mut back);
        back.variables = Variables::default();
        assert_eq!(export_resources(&back, &ids), (Some(Vec::new()), None));
    }
}
