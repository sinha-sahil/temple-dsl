use super::*;

fn expr_text(src: &str, start: usize) -> &str {
    let p = ExprUnit::parse_at(src, start).unwrap_or_else(|e| panic!("{src:?}: {}", msg(&e)));
    src[start..p.end()].trim_start()
}

#[test]
fn expression_ends_before_the_hosts_own_punctuation() {
    // guard before ':'
    assert_eq!(
        expr_text("input.event == 'approve': go()", 0),
        "input.event == 'approve'"
    );
    // a ternary inside a guard keeps its own ':'
    assert_eq!(expr_text("a ? b : c: go()", 0), "a ? b : c");
    // payload before ')'
    let src = "ask x({ \"id\": input.id }) -> y()";
    assert_eq!(expr_text(src, 6), "{ \"id\": input.id }");
    // argument list items
    let src = "f(input.a + 1, 'x')";
    assert_eq!(expr_text(src, 2), "input.a + 1");
    assert_eq!(expr_text(src, 15), "'x'");
    // strings may contain the host's punctuation
    assert_eq!(
        expr_text("'a: b) c' == input.x: z", 0),
        "'a: b) c' == input.x"
    );
}

#[test]
fn expression_does_not_swallow_trailing_whitespace_or_comments() {
    let src = "input.a   # a comment\n  when {";
    let p = ExprUnit::parse_at(src, 0).unwrap();
    assert_eq!(&src[..p.end()], "input.a");
    let src = "input.a + 1\n\n";
    assert_eq!(
        &src[..ExprUnit::parse_at(src, 0).unwrap().end()],
        "input.a + 1"
    );
}

#[test]
fn expression_spans_multiple_lines_when_the_grammar_continues() {
    // a method chain continued on the next line
    let src = "input.lines\n  .map(l -> l.qty)\n  .sum()\nwhen {";
    assert_eq!(
        expr_text(src, 0),
        "input.lines\n  .map(l -> l.qty)\n  .sum()"
    );
    // parentheses keep an operator on the next line inside the expression
    let src = "(input.a\n  - input.b)\nlet x = 1";
    assert_eq!(expr_text(src, 0), "(input.a\n  - input.b)");
    // but an array literal on the next line is not an index
    let src = "input.a\n[1, 2]";
    assert_eq!(expr_text(src, 0), "input.a");
}

#[test]
fn every_expression_form_ends_at_its_last_token() {
    for e in [
        "when { input.a: 1, else: 2 }",
        "input.xs.map(x -> x + 1)",
        "let y = 2 in y * 3",
        "-input.n",
        "!input.flag",
        "[1, 2, input.c]",
        "{ \"k\": input.v, [input.key]: 1 }",
        "upper(trim(input.s))",
        "input.items[0].name",
        "input.a?.b ?? 'none'",
        "'text'.len()",
        "(1 + 2).abs()",
    ] {
        let src = format!("{e}   : rest");
        assert_eq!(expr_text(&src, 0), e, "for {e:?}");
    }
}

#[test]
fn template_ends_after_its_literal() {
    let src = "else: { \"status\": \"OK\", \"n\": {{ input.n }} }\n  wake 5";
    let p = TemplateUnit::parse_at(src, 6).unwrap();
    assert_eq!(
        &src[6..p.end()],
        "{ \"status\": \"OK\", \"n\": {{ input.n }} }"
    );
    let src = "[ 1, {{ input.x }} ] ,";
    assert_eq!(
        &src[..TemplateUnit::parse_at(src, 0).unwrap().end()],
        "[ 1, {{ input.x }} ]"
    );
    let src = "\"hello {{ input.name }}\" next";
    assert_eq!(
        &src[..TemplateUnit::parse_at(src, 0).unwrap().end()],
        "\"hello {{ input.name }}\""
    );
}

#[test]
fn spans_are_absolute_in_the_host_source() {
    let src = "line one\nline two\nguard: input.ok && missing: x";
    let start = src.find("input.ok").unwrap();
    let errs = ExprUnit::parse_at(src, start)
        .unwrap()
        .check(&Names::new())
        .unwrap_err();
    let report = errs[0].report(src);
    assert!(report.contains("--> 3:"), "{report}");
    assert!(report.contains("unknown identifier `missing`"), "{report}");
}

#[test]
fn bad_start_offsets_are_errors_not_panics() {
    assert!(ExprUnit::parse_at("abc", 10).is_err());
    assert!(ExprUnit::parse_at("é", 1).is_err()); // inside a character
    assert!(ExprUnit::parse_at("", 0).is_err());
    assert!(TemplateUnit::parse_at("  ", 0).is_err());
}

#[test]
fn compile_rejects_trailing_text() {
    let errs = ExprUnit::compile("1 + 2 3", &Names::new()).unwrap_err();
    assert!(msg(&errs).contains("unexpected text"), "{}", msg(&errs));
    assert!(ExprUnit::compile("1 + 2  # comment\n", &Names::new()).is_ok());
}
