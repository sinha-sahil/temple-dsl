//! Milestone 8 — `format`, snippet diagnostics, did-you-mean, and multi-error
//! reporting.

use temple_dsl::{CompileError, Template, Value};

fn fmt(src: &str) -> String {
    Template::format(src).expect("format should succeed")
}

/// Formatting is a fixpoint, the output re-parses, and (for semantically valid
/// templates) it renders identically to the original — the strongest check that
/// re-emission preserved the parse tree, parens and all.
fn assert_format_stable(src: &str, input: Value) {
    let once = fmt(src);
    let twice = fmt(&once);
    assert_eq!(
        once, twice,
        "not idempotent:\n--once--\n{once}\n--twice--\n{twice}"
    );

    let a = Template::compile(src)
        .expect("original compiles")
        .render_value(input.clone())
        .expect("original renders");
    let b = Template::compile(&once)
        .expect("formatted compiles")
        .render_value(input)
        .expect("formatted renders");
    assert_eq!(
        a, b,
        "formatting changed the result for:\n{src}\nformatted to:\n{once}"
    );
}

#[test]
fn format_precedence_keeps_meaning() {
    let input = Value::obj([
        ("a", Value::Int(2)),
        ("b", Value::Int(3)),
        ("c", Value::Int(4)),
    ]);
    for src in [
        "{{ (input.a + input.b) * input.c }}",
        "{{ input.a + input.b * input.c }}",
        "{{ input.a - (input.b - input.c) }}",
        "{{ input.a * input.b + input.c }}",
        "{{ -(input.a + input.b) }}",
        "{{ (input.a + input.b) * (input.b - input.c) }}",
    ] {
        assert_format_stable(src, input.clone());
    }
}

#[test]
fn format_coalesce_associativity() {
    let input = Value::obj([("a", Value::Null), ("b", Value::Null), ("c", Value::Int(9))]);
    // right-assoc: inner parens drop; left-nesting parens stay.
    assert_eq!(
        fmt("{{ input.a ?? (input.b ?? input.c) }}").trim(),
        "{{ input.a ?? input.b ?? input.c }}"
    );
    assert_eq!(
        fmt("{{ (input.a ?? input.b) ?? input.c }}").trim(),
        "{{ (input.a ?? input.b) ?? input.c }}"
    );
    assert_format_stable("{{ (input.a ?? input.b) ?? input.c }}", input);
}

#[test]
fn format_canonical_object_layout() {
    let got = fmt(r#"{"z":{{input.a}},"a":  "hi {{ input.b }}"}"#);
    assert_eq!(
        got,
        "{\n  \"z\": {{ input.a }},\n  \"a\": \"hi {{ input.b }}\"\n}\n"
    );
}

#[test]
fn format_normalizes_string_quotes_and_lets() {
    // double-quoted expression strings become single-quoted; let preamble + blank line.
    let got = fmt("let x = input.v ?? \"none\"\n{ \"k\": {{ x }} }");
    assert_eq!(got, "let x = input.v ?? 'none'\n\n{\n  \"k\": {{ x }}\n}\n");
}

#[test]
fn format_full_language_is_stable() {
    let input = Value::obj([
        ("customer", Value::Str("Ada".into())),
        (
            "items",
            Value::Arr(vec![
                Value::obj([
                    ("qty", Value::Int(3)),
                    ("price", Value::Decimal("9.99".parse().unwrap())),
                ]),
                Value::obj([
                    ("qty", Value::Int(1)),
                    ("price", Value::Decimal("4.50".parse().unwrap())),
                ]),
            ]),
        ),
        ("score", Value::Int(85)),
    ]);
    let src = r#"
        let items = input.items
        {
          "summary":  "{{ input.customer }} x {{ items.len() }}",
          "lines":    {{ items.map(it -> { "n": it.qty, "t": it.qty * it.price }) }},
          "subtotal": {{ items.map(it -> it.qty * it.price).sum() }},
          "grade":    {{ when { input.score >= 90: 'A', input.score >= 80: 'B', else: 'C' } }},
          "bulk":     {{ items.any(it -> it.qty >= 5) }},
          "first":    {{ items[0].qty }},
          "rounded":  {{ round(items.map(it -> it.qty * it.price).sum(), 1) }}
        }
    "#;
    assert_format_stable(src, input);
}

#[test]
fn format_empty_collections() {
    assert_eq!(fmt("{}").trim(), "{}");
    assert_eq!(fmt("[]").trim(), "[]");
}

fn errs(src: &str) -> Vec<CompileError> {
    Template::compile(src).expect_err("should fail to compile")
}

fn joined(errs: &[CompileError]) -> String {
    errs.iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn suggests_for_unknown_identifier() {
    let msg = joined(&errs(r#"{ "x": {{ inputt.id }} }"#));
    assert!(msg.contains("unknown identifier 'inputt'"), "{msg}");
    assert!(msg.contains("did you mean `input`?"), "{msg}");
}

#[test]
fn suggests_for_unknown_function() {
    let msg = joined(&errs(r#"{ "x": {{ rouns(input.a) }} }"#));
    assert!(msg.contains("did you mean `round`?"), "{msg}");
}

#[test]
fn suggests_for_unknown_this_key() {
    let msg = joined(&errs(
        r#"{ "tax": {{ input.a }}, "total": {{ this.taxs }} }"#,
    ));
    assert!(msg.contains("did you mean `tax`?"), "{msg}");
}

#[test]
fn no_suggestion_when_nothing_is_close() {
    let msg = joined(&errs(r#"{ "x": {{ zzzzzz.id }} }"#));
    assert!(msg.contains("unknown identifier 'zzzzzz'"), "{msg}");
    assert!(!msg.contains("did you mean"), "{msg}");
}

#[test]
fn report_renders_underlined_snippet() {
    let src = "{\n  \"id\": {{ inputt.id }}\n}";
    let e = errs(src);
    let report = e[0].report(src);
    assert!(
        report.starts_with("error: unknown identifier 'inputt'"),
        "{report}"
    );
    assert!(report.contains("--> 2:"), "{report}");
    assert!(report.contains("^^^^^^"), "{report}"); // underlines all 6 chars of `inputt`
}

#[test]
fn report_all_lists_every_resolver_error() {
    let src = r#"{ "a": {{ foo }}, "b": {{ bar }} }"#;
    let e = errs(src);
    assert_eq!(
        e.len(),
        2,
        "resolver should report both unknown identifiers"
    );
    let report = CompileError::report_all(src, &e);
    assert!(report.contains("unknown identifier 'foo'"), "{report}");
    assert!(report.contains("unknown identifier 'bar'"), "{report}");
}

#[test]
fn parser_recovers_across_object_fields() {
    // Two malformed holes plus one good field — both syntax errors reported,
    // not just the first.
    let e = errs(r#"{ "a": {{ 1 + }}, "b": {{ 2 * }}, "c": {{ 3 }} }"#);
    assert_eq!(e.len(), 2, "expected both bad holes reported, got: {e:?}");
}

#[test]
fn parser_recovers_across_array_items() {
    let e = errs(r#"[ {{ + }}, {{ * }}, {{ 3 }} ]"#);
    assert_eq!(e.len(), 2, "expected both bad items reported, got: {e:?}");
}

#[test]
fn parser_recovery_terminates_on_garbage() {
    // No panic, no hang — just a non-empty error list.
    for src in [
        "{ \"a\": @@@, \"b\": %%% }",
        "{ \"a\": {{ ]]] }}, \"b\": {{ ((( }} }",
        "[[[[[[",
        "{ \"k\":",
    ] {
        assert!(Template::compile(src).is_err(), "should fail: {src}");
    }
}

#[test]
fn single_bad_hole_reports_one_error() {
    let e = errs(r#"{{ 1 + }}"#);
    assert_eq!(e.len(), 1, "got: {e:?}");
}

#[test]
fn valid_template_still_parses_clean() {
    // Recovery must not invent errors on good input.
    assert!(Template::compile(r#"{ "a": {{ input.x }}, "b": [1, 2, 3,] }"#).is_ok());
}

#[test]
fn recovery_survives_a_hole_in_the_failed_item() {
    // The bad first entry contains a valid hole; recovery must skip the hole
    // whole (not mistake its `}}` for structure) and still report the
    // independent error on line 3.
    let src = "{\n \"a\": [ @ {{ input.x }}, 2, 3 ],\n \"b\": @bad\n}";
    let e = errs(src);
    assert!(e.len() >= 2, "expected both errors, got: {e:?}");
    let spans: Vec<_> = e.iter().filter_map(|err| err.span()).collect();
    assert!(
        spans.iter().any(|s| s.line_col(src).0 == 3),
        "the line-3 error must be reported: {e:?}"
    );
}

#[test]
fn recovery_resyncs_past_comments_in_holes() {
    // A `}}` inside a comment within a broken hole must not end the resync.
    let src = "[ {{ @ # note }} here\n }}, 2 ]";
    assert!(Template::compile(src).is_err());
}

// ── newline-separated entries (DESIGN §2: commas optional) ──

#[test]
fn newline_separates_object_entries() {
    let t = Template::compile("{ \"a\": 1\n  \"b\": 2 }").expect("compile");
    let v = t.render_value(Value::Null).unwrap();
    assert_eq!(v, Value::obj([("a", Value::Int(1)), ("b", Value::Int(2))]));
}

#[test]
fn newline_separates_array_items_and_when_branches() {
    let t = Template::compile("[ 1\n 2 ]").expect("compile");
    assert_eq!(
        t.render_value(Value::Null).unwrap(),
        Value::Arr(vec![Value::Int(1), Value::Int(2)])
    );
    let t =
        Template::compile("{{ when {\n  input.x >= 5: 'hi'\n  else: 'lo'\n} }}").expect("compile");
    assert_eq!(
        t.render_value(Value::obj([("x", Value::Int(7))])).unwrap(),
        Value::Str("hi".into())
    );
}

#[test]
fn same_line_missing_comma_is_still_an_error() {
    assert!(Template::compile(r#"{ "a": 1 "b": 2 }"#).is_err());
    assert!(Template::compile("[ 1 2 ]").is_err());
}

#[test]
fn multiline_expression_is_one_entry() {
    // Inside an expression literal, a trailing operator continues the
    // expression across the line break; a fresh line starts the next item.
    let t = Template::compile("{{ [ 1 +\n 2\n 3 ] }}").expect("compile");
    assert_eq!(
        t.render_value(Value::Null).unwrap(),
        Value::Arr(vec![Value::Int(3), Value::Int(3)])
    );
}

#[test]
fn all_shipped_samples_compile() {
    // Every .temple under samples/ and examples/data/ must parse — the repo
    // never ships a template its own compiler rejects.
    for dir in ["samples", "examples/data"] {
        for entry in std::fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.extension().is_some_and(|e| e == "temple") {
                let src = std::fs::read_to_string(&path).expect("read");
                assert!(
                    Template::compile(&src).is_ok(),
                    "shipped template fails to compile: {}",
                    path.display()
                );
            }
        }
    }
}

// ── format round-trip hardening ──

#[test]
fn format_parenthesizes_numeric_literal_bases() {
    for src in [
        "{{ (5).to_string() }}",
        "{{ (1.5).to_string() }}",
        "{{ (-5).to_string() }}",
    ] {
        let once = fmt(src);
        assert!(
            Template::format(&once).is_ok(),
            "formatted output must re-parse: {src} → {once}"
        );
        assert_eq!(fmt(&once), once, "not idempotent for {src}");
    }
}

#[test]
fn format_escapes_braces_in_interp_text() {
    // `\{` is a literal brace; text containing `{{` must re-emit escaped so it
    // can't open a phantom hole on re-parse.
    let src = r#""a{{ input.x }}\{\{b""#;
    let once = fmt(src);
    let again = Template::format(&once).expect("formatted output must re-parse");
    assert_eq!(once, again);
    // and the rendered text carries the literal braces
    let v = Template::compile(src)
        .unwrap()
        .render_value(Value::obj([("x", Value::Int(1))]))
        .unwrap();
    assert_eq!(v, Value::Str("a1{{b".into()));
}

#[test]
fn format_enforces_the_source_size_cap() {
    let huge = format!("{{ \"k\": \"{}\" }}", "x".repeat(2 * 1024 * 1024));
    assert!(matches!(
        Template::format(&huge),
        Err(ref e) if matches!(e[0], CompileError::TooLarge { .. })
    ));
}

// ── compile-time rejection of nonsense literal access ──

#[test]
fn field_access_on_scalar_literals_is_a_compile_error() {
    for src in [
        "{{ null.foo }}",
        "{{ true.x }}",
        "{{ (5).foo }}",
        "{{ 'a'.foo }}",
    ] {
        assert!(
            Template::compile(src).is_err(),
            "should be a compile error: {src}"
        );
    }
    // …while methods on string literals stay legal
    assert!(Template::compile(r#"{{ "a,b".split(",") }}"#).is_ok());
}
