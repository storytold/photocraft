use super::*;

fn attributes(visits: usize, bytes: usize, copies: usize) -> AttributeWork {
    AttributeWork { visits_left: visits, bytes_left: bytes, copies_left: copies }
}

#[test]
fn common_budget_preserves_style_rejection_order_and_exhausts_only_failed_bucket() {
    let mut local = 3;
    let mut file = attributes(11, 13, 20);
    let error = charge_style_copy(4, &mut local, Some(&mut file)).unwrap_err();
    assert!(error.to_string().contains("bounded formatting allowance"));
    assert_eq!((local, file.copies_left, file.visits_left, file.bytes_left), (0, 20, 11, 13));
    let mut local = 10;
    file.copies_left = 3;
    let error = charge_style_copy(4, &mut local, Some(&mut file)).unwrap_err();
    assert!(error.to_string().contains("document text style-copy work allowance"));
    assert_eq!((local, file.copies_left), (10, 0));
    file.copies_left = 10;
    charge_style_copy(4, &mut local, Some(&mut file)).unwrap();
    assert_eq!((local, file.copies_left), (6, 6));
    assert!(charge_style_copy(7, &mut local, Some(&mut file)).is_err());
    assert_eq!((local, file.copies_left), (0, 6), "already performed file work is not refunded or charged on failed local admission");
    let mut local = 3;
    assert!(charge_style_copy(4, &mut local, None).is_err());
    assert_eq!(local, 0, "local-only refusal keeps the explicit exhaustion policy");
}

#[test]
fn common_budget_attribute_transaction_preserves_local_and_file_allowances_on_refusal() {
    let xml = roxmltree::Document::parse("<span a='value'/>").unwrap();
    let attribute = xml.root_element().attributes().next().unwrap();
    let bytes = attribute.name().len() + attribute.value().len();
    let mut file = attributes(2, bytes - 1, 17);
    let mut work = MarkupWork { visits_left: 2, bytes_left: bytes, shared: Some(&mut file) };
    let error = charge_markup_attribute(&attribute, &mut work).unwrap_err();
    assert!(error.to_string().contains("document text attribute-byte work allowance"));
    assert_eq!((work.visits_left, work.bytes_left), (2, bytes));
    let shared = work.shared.as_mut().unwrap();
    assert_eq!((shared.visits_left, shared.bytes_left, shared.copies_left), (2, bytes - 1, 17));
    shared.bytes_left = bytes;
    charge_markup_attribute(&attribute, &mut work).unwrap();
    assert_eq!((work.visits_left, work.bytes_left), (1, 0));
    let shared = work.shared.as_ref().unwrap();
    assert_eq!((shared.visits_left, shared.bytes_left, shared.copies_left), (1, 0, 17));
    let error = charge_markup_attribute(&attribute, &mut work).unwrap_err();
    assert!(error.to_string().contains("source text markup exceeds the bounded attribute-byte allowance"));
    assert_eq!((work.visits_left, work.bytes_left), (1, 0));
    let shared = work.shared.as_ref().unwrap();
    assert_eq!((shared.visits_left, shared.bytes_left), (1, 0));
}
