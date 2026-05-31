//! No-panic guarantee: caps, overflow, and diagnostics that must never abort.

use rust_decimal::Decimal;
use serde::Deserialize;
use temple_dsl::{CompileError, RenderError, Template, Value};

fn compile_err(src: &str) -> Vec<CompileError> {
    Template::compile(src).expect_err("expected compile error")
}

#[test]
fn deep_parens_rejected_not_aborted() {
    let src = format!("{{{{ {}1{} }}}}", "(".repeat(50_000), ")".repeat(50_000));
    let err = Template::compile(&src).expect_err("should reject, not abort");
    assert!(matches!(err.first(), Some(CompileError::TooDeep { .. })));
}

#[test]
fn deep_unary_rejected_not_aborted() {
    let src = format!("{{{{ {}true }}}}", "!".repeat(60_000));
    let err = Template::compile(&src).expect_err("should reject, not abort");
    assert!(matches!(err.first(), Some(CompileError::TooDeep { .. })));
}

#[test]
fn deep_output_nesting_rejected_not_aborted() {
    let mut s = String::new();
    for _ in 0..5000 {
        s.push_str("[ ");
    }
    s.push('1');
    for _ in 0..5000 {
        s.push_str(" ]");
    }
    let err = Template::compile(&s).expect_err("should reject, not abort");
    assert!(matches!(err.first(), Some(CompileError::TooDeep { .. })));
}

#[test]
fn moderate_nesting_still_compiles() {
    // Well within the cap: must NOT be rejected.
    let src = format!("{{{{ {}1{} }}}}", "(".repeat(20), ")".repeat(20));
    assert!(Template::compile(&src).is_ok());
}

#[test]
fn oversized_source_rejected() {
    let src = format!("{{{{ {} }}}}", "1+".repeat(600_000));
    let err = Template::compile(&src).expect_err("should reject");
    assert!(matches!(err.first(), Some(CompileError::TooLarge { .. })));
}

#[test]
fn decimal_add_overflow_is_error_not_panic() {
    let t = Template::compile("{{ input.a + input.b }}").unwrap();
    let inp = Value::obj([
        ("a", Value::Decimal(Decimal::MAX)),
        ("b", Value::Decimal(Decimal::MAX)),
    ]);
    assert!(matches!(
        t.render::<Decimal>(inp),
        Err(RenderError::ArithmeticOverflow { .. })
    ));
}

#[test]
fn decimal_mul_overflow_is_error_not_panic() {
    let t = Template::compile("{{ input.a * input.b }}").unwrap();
    let inp = Value::obj([
        ("a", Value::Decimal(Decimal::MAX)),
        ("b", Value::Decimal(Decimal::MAX)),
    ]);
    assert!(matches!(
        t.render::<Decimal>(inp),
        Err(RenderError::ArithmeticOverflow { .. })
    ));
}

#[test]
fn validate_accepts_and_rejects() {
    assert!(Template::validate(r#"{ "x": {{ input.a }} }"#).is_ok());
    assert!(Template::validate("{{ ").is_err());
    assert!(Template::validate(r#"{ "a": {{ this.b }} }"#).is_err());
}

#[test]
fn utf8_error_message_is_not_mojibake() {
    let errs = compile_err("é");
    let msg = format!("{}", errs[0]);
    assert!(msg.contains('é'), "message was: {msg}");
}

#[test]
fn utf8_error_span_is_on_char_boundary() {
    let src = "{{ 你 }}";
    let errs = compile_err(src);
    let CompileError::Syntax { span, .. } = &errs[0] else {
        panic!("expected syntax error, got {:?}", errs[0]);
    };
    assert!(src.is_char_boundary(span.start as usize));
    assert!(src.is_char_boundary(span.end as usize));
}

#[test]
fn trailing_dot_number_has_clear_error() {
    let errs = compile_err("{{ 5. }}");
    let msg = format!("{}", errs[0]);
    assert!(msg.contains("digit after"), "message was: {msg}");
}

#[test]
fn f64_target_field_works() {
    #[derive(Deserialize)]
    struct O {
        x: f64,
    }
    let o: O = Template::compile(r#"{ "x": {{ input.a / input.b }} }"#)
        .unwrap()
        .render(Value::obj([("a", Value::Int(1)), ("b", Value::Int(4))]))
        .unwrap();
    assert!((o.x - 0.25).abs() < 1e-12);
}

#[test]
fn f64_from_decimal_field_works() {
    #[derive(Deserialize)]
    struct O {
        x: f64,
    }
    let o: O = Template::compile(r#"{ "x": {{ input.v }} }"#)
        .unwrap()
        .render(Value::obj([("v", Value::Decimal("2.5".parse().unwrap()))]))
        .unwrap();
    assert!((o.x - 2.5).abs() < 1e-12);
}

#[test]
fn decimal_field_still_exact() {
    let out: Decimal = Template::compile("{{ input.v }}")
        .unwrap()
        .render(Value::obj([("v", Value::Decimal("0.1".parse().unwrap()))]))
        .unwrap();
    assert_eq!(out, "0.1".parse::<Decimal>().unwrap());
}
