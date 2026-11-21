use super::*;
use std::cell::Cell;

fn check_markup_namespace_work(markup: &str, work_left: usize) -> Result<()> {
    let limits = craft_xml::Limits { max_input: super::super::MAX_STRING, max_depth: MAX_DEPTH, max_attrs: 256, ..Default::default() };
    super::super::xml_work::check_namespace_work(markup, "source text markup", limits, work_left, &photocraft_raster::Interrupt::NONE)
}

fn attributes(count: usize) -> String {
    (0..count).map(|index| format!("a{index}='inert' ")).collect()
}

fn parsed_elements(markup: &str, calls: &Cell<usize>) -> Result<usize> {
    parse_markup_with(markup, &photocraft_raster::Interrupt::NONE, |source| {
        calls.set(calls.get() + 1);
        let xml = roxmltree::Document::parse_with_options(
            source,
            roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES as u32, ..Default::default() },
        )
        .map_err(|error| invalid(error.to_string()))?;
        Ok(xml.descendants().filter(roxmltree::Node::is_element).count())
    })
}

#[test]
fn twenty_thousand_attributes_never_reach_the_dom_parser() {
    let markup = format!("<markup><span {}>x</span></markup>", attributes(20_000));
    assert!(markup.len() < super::super::MAX_STRING);
    let calls = Cell::new(0);
    // A spy deliberately replaces the quadratic parser in the negative control.
    let result = parse_markup_with(&markup, &photocraft_raster::Interrupt::NONE, |_| {
        calls.set(calls.get() + 1);
        Ok(())
    });
    assert!(matches!(result, Err(crate::IoError::Unsupported(message)) if message.contains("XML attribute-parse")));
    assert_eq!(calls.get(), 0);
}

#[test]
fn xml_attribute_width_boundary_keeps_valid_native_styles() {
    let calls = Cell::new(0);
    let admitted = format!("<markup><span {}>x</span></markup>", attributes(256));
    assert_eq!(parsed_elements(&admitted, &calls).unwrap(), 2);
    assert_eq!(calls.get(), 1);
    let refused = format!("<markup><span {}>x</span></markup>", attributes(257));
    assert!(parsed_elements(&refused, &calls).is_err());
    assert_eq!(calls.get(), 1);
    let known = format!("<markup><span {} weight='bold'>x</span></markup>", attributes(255));
    let (text, runs) = markup_runs(&known, &CharStyle::default(), &HashMap::new(), 72., &mut Vec::new()).unwrap();
    assert_eq!(text, "x");
    assert_eq!(runs[0].style.weight, 700);
}

#[test]
fn aggregate_squared_attribute_work_is_fenced_before_dom() {
    let element = format!("<span {}/>", attributes(256));
    let calls = Cell::new(0);
    let admitted = format!("<markup>{}</markup>", element.repeat(16));
    assert_eq!(parsed_elements(&admitted, &calls).unwrap(), 17);
    let refused = format!("<markup>{}</markup>", element.repeat(17));
    assert!(matches!(parsed_elements(&refused, &calls), Err(crate::IoError::Unsupported(message)) if message.contains("XML attribute-parse")));
    assert_eq!(calls.get(), 1, "16 * 256 squared fits; the next element must not reach the parser");
}

#[test]
fn xml_depth_boundary_is_fenced_before_dom_even_without_text_leaves() {
    let calls = Cell::new(0);
    let admitted = format!("<markup>{}x{}</markup>", "<span>".repeat(31), "</span>".repeat(31));
    assert_eq!(parsed_elements(&admitted, &calls).unwrap(), 32);
    let refused = format!("<markup>{}{}</markup>", "<span>".repeat(32), "</span>".repeat(32));
    assert!(matches!(parsed_elements(&refused, &calls), Err(crate::IoError::Unsupported(message)) if message.contains("XML nesting")));
    assert_eq!(calls.get(), 1);
}

#[test]
fn quoted_markers_comments_and_cdata_do_not_inflate_xml_work() {
    let calls = Cell::new(0);
    let fake = format!("<span {}>{}", attributes(300), "<span>".repeat(64));
    let markup = format!("<markup><!--{fake}--><![CDATA[{fake}]]><span title='&lt;not-a-tag a=&#34;x&#34; >'>x</span></markup>");
    assert_eq!(parsed_elements(&markup, &calls).unwrap(), 2);
    // The lexical fence must also ignore literal delimiters inside a quote;
    // actual XML validity is still the parser's responsibility afterwards.
    let quoted = format!("<markup title=\"{fake}\"/>");
    assert!(!craft_svg::guard::too_wide(&quoted, 256, 1 << 20));
    assert!(!craft_svg::guard::deeper_than(&quoted, MAX_DEPTH));
    assert_eq!(calls.get(), 1);
}

#[test]
fn preparse_fences_do_not_enable_dtds() {
    let calls = Cell::new(0);
    assert!(parsed_elements("<!DOCTYPE markup [<!ENTITY x 'inert'>]><markup>&x;</markup>", &calls).is_err());
    assert_eq!(calls.get(), 0, "neither admission nor the DTD-disabled DOM parser may expand declarations");
}

#[test]
fn namespace_inheritance_work_boundary_counts_scope_copying_and_prefix_lookups() {
    let markup = "<markup xmlns:a='urn:a' xmlns:b='urn:b'><span xmlns:c='urn:c'>x</span></markup>";
    // Root: 2 squared + 2*(2 attributes + name) + 2*3 insertions = 16.
    // Child: 3 squared + 3*(1 attribute + name) + 1*4 insertions = 19.
    check_markup_namespace_work(markup, 35).unwrap();
    assert!(matches!(check_markup_namespace_work(markup, 34), Err(crate::IoError::Unsupported(message)) if message.contains("XML namespace-parse")));
    let calls = Cell::new(0);
    assert_eq!(parsed_elements(markup, &calls).unwrap(), 2);
    let (text, _) = markup_runs(markup, &CharStyle::default(), &HashMap::new(), 72., &mut Vec::new()).unwrap();
    assert_eq!(text, "x", "ordinary unused namespace declarations retain their existing meaning");
}

#[test]
fn inherited_namespace_amplification_never_reaches_the_dom_parser() {
    let mut markup = String::from("<markup>");
    // Keep every inherited scope inside craft-xml's 256-attribute cap and
    // accumulate enough repeated inheritance for the DOM admission guard.
    for level in 0..15 {
        markup.push_str("<span ");
        for index in 0..15 {
            markup.push_str(&format!("xmlns:n{level}_{index}='urn:{level}:{index}' "));
        }
        markup.push('>');
    }
    for leaf in 0..24 {
        markup.push_str(&format!("<span xmlns:leaf='urn:leaf:{leaf}'/>"));
    }
    markup.push_str(&"</span>".repeat(15));
    markup.push_str("</markup>");
    assert!(!craft_svg::guard::too_wide(&markup, 256, 1 << 20), "local attribute widths alone admit the hostile inherited work");
    assert!(!craft_svg::guard::deeper_than(&markup, MAX_DEPTH));
    let calls = Cell::new(0);
    let result = parse_markup_with(&markup, &photocraft_raster::Interrupt::NONE, |_| {
        calls.set(calls.get() + 1);
        Ok(())
    });
    assert!(matches!(result, Err(crate::IoError::Unsupported(message)) if message.contains("XML namespace-parse")));
    assert_eq!(calls.get(), 0);
}

#[test]
fn shallow_namespace_insertions_never_reach_the_dom_parser() {
    let mut markup = String::from("<markup>");
    for index in (0..2048).rev() {
        markup.push_str(&format!("<span xmlns:n{index:04}='urn:{index:04}'/>"));
    }
    markup.push_str("</markup>");
    assert!(!craft_svg::guard::too_wide(&markup, 256, 1 << 20));
    let calls = Cell::new(0);
    let result = parse_markup_with(&markup, &photocraft_raster::Interrupt::NONE, |_| {
        calls.set(calls.get() + 1);
        Ok(())
    });
    assert!(matches!(result, Err(crate::IoError::Unsupported(message)) if message.contains("XML namespace-parse")));
    assert_eq!(calls.get(), 0, "a shallow tree can still cause quadratic sorted namespace insertion");
}

#[test]
fn cancellation_prevents_dom_parse_and_interrupts_the_style_walk() {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    let markup = "<markup><span weight='bold'>first</span><span foreground='#fff'>second</span></markup>";
    let polls = AtomicUsize::new(0);
    let stop = || polls.fetch_add(1, Relaxed) >= 3;
    let ctl = photocraft_raster::Interrupt::cancel_only(&stop);
    let result: Result<()> = parse_markup_with(markup, &ctl, |_| panic!("cancelled preflight must never build a DOM"));
    assert!(matches!(result, Err(crate::IoError::Cancelled)));

    polls.store(0, Relaxed);
    let count = || {
        polls.fetch_add(1, Relaxed);
        false
    };
    let ctl = photocraft_raster::Interrupt::cancel_only(&count);
    parse_markup_with(markup, &ctl, |_| Ok(())).unwrap();
    let before_walk = polls.swap(0, Relaxed);
    let stop = || polls.fetch_add(1, Relaxed) >= before_walk + 3;
    let ctl = photocraft_raster::Interrupt::cancel_only(&stop);
    let mut work = MarkupWork::new();
    let initial_visits = work.visits_left;
    let result = markup_runs_with_attribute_allowance(markup, &CharStyle::default(), &HashMap::new(), 72., &mut Vec::new(), &mut work, &ctl);
    assert!(matches!(result, Err(crate::IoError::Cancelled)));
    assert!(work.visits_left < initial_visits);
}
