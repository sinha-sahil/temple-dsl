use super::*;

#[test]
fn reads_report_paths_and_optionality() {
    let u = ExprUnit::compile(
        "input.data.order.lines[0].qty + (input.payload?.note ?? 0).len() + input.x.map(v -> v).len()",
        &Names::new(),
    )
    .unwrap();
    let reads = u.reads();
    let paths: Vec<(Vec<&str>, bool)> = reads
        .iter()
        .map(|r| (r.path.iter().map(|s| s.as_str()).collect(), r.optional))
        .collect();
    assert!(
        paths.contains(&(vec!["data", "order", "lines"], false)),
        "{paths:?}"
    );
    assert!(
        paths.contains(&(vec!["payload", "note"], true)),
        "{paths:?}"
    );
    assert!(paths.contains(&(vec!["x"], false)), "{paths:?}");
    let bare: Vec<InputRead> = ExprUnit::compile("input", &Names::new()).unwrap().reads();
    assert_eq!(bare[0].path.len(), 0);
}
