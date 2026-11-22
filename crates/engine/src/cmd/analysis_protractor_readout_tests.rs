//! #11990: a protractor's readout is X/Y = the second arm's end with W and H blank, as the
//! reference application shows it; a plain ruler keeps X/Y = start and W/H = end minus start.

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s
}

#[test]
fn a_protractor_reads_the_second_arms_end_with_blank_size() {
    let mut s = session();
    s.execute("image.analysis.rulerTool", json!({"start": [400, 300], "end": [600, 300]})).unwrap();
    let r = s.execute("image.analysis.rulerTool", json!({"protractor": [500, 200]})).unwrap();
    let read = s.execute("image.analysis.rulerTool", json!({})).unwrap();
    let info = s.execute("image.analysis.info", json!({})).unwrap();
    for i in [&r["info"], &read["info"], &info["ruler"]["info"]] {
        assert_eq!((&i["x"], &i["y"]), (&json!(500.0), &json!(200.0)), "{i}");
        assert_eq!((&i["w"], &i["h"]), (&Value::Null, &Value::Null), "{i}");
        assert!((i["angle"].as_f64().unwrap() - 45.0).abs() < 1e-3, "{i}");
        assert_eq!(i["l1"], 200.0);
        assert!((i["l2"].as_f64().unwrap() - 141.4214).abs() < 1e-3, "{i}");
    }
}

#[test]
fn a_plain_ruler_keeps_its_readout() {
    let mut s = session();
    let r = s.execute("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250]})).unwrap();
    let i = &r["info"];
    assert_eq!((&i["x"], &i["y"], &i["w"], &i["h"]), (&json!(100.0), &json!(100.0), &json!(200.0), &json!(150.0)), "{i}");
    assert!((i["angle"].as_f64().unwrap() + 36.8699).abs() < 1e-3, "{i}");
    assert_eq!(i["l1"], 250.0);
    assert_eq!(i["l2"], Value::Null);
}
