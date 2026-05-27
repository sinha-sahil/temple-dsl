use rust_decimal::Decimal;
use serde::Deserialize;
use temple_dsl::{Template, Value};

fn render<T: serde::de::DeserializeOwned>(src: &str, input: Value) -> T {
    let template = Template::compile(src).expect("compile");
    template.render::<T>(input).expect("render")
}

fn render_int(src: &str) -> i64 {
    render::<i64>(src, Value::Null)
}

fn render_bool(src: &str) -> bool {
    render::<bool>(src, Value::Null)
}

#[test]
fn arithmetic_add() {
    assert_eq!(render_int("{{ 2 + 3 }}"), 5);
}

#[test]
fn arithmetic_sub() {
    assert_eq!(render_int("{{ 10 - 7 }}"), 3);
}

#[test]
fn arithmetic_mul() {
    assert_eq!(render_int("{{ 4 * 5 }}"), 20);
}

#[test]
fn arithmetic_precedence() {
    assert_eq!(render_int("{{ 2 + 3 * 4 }}"), 14);
}

#[test]
fn arithmetic_parens() {
    assert_eq!(render_int("{{ (2 + 3) * 4 }}"), 20);
}

#[test]
fn division_promotes_to_decimal() {
    let result: Decimal = render("{{ 10 / 4 }}", Value::Null);
    assert_eq!(result, "2.5".parse::<Decimal>().unwrap());
}

#[test]
fn int_decimal_mixing_promotes() {
    let result: Decimal = render("{{ 1 + 0.5 }}", Value::Null);
    assert_eq!(result, "1.5".parse::<Decimal>().unwrap());
}

#[test]
fn decimal_exact_addition() {
    let result: Decimal = render("{{ 0.1 + 0.2 }}", Value::Null);
    assert_eq!(result, "0.3".parse::<Decimal>().unwrap());
}

#[test]
fn unary_negation_path() {
    let input = Value::obj([("x", Value::Int(7))]);
    let r: i64 = render("{{ -input.x }}", input);
    assert_eq!(r, -7);
}

#[test]
fn unary_not() {
    let input = Value::obj([("flag", Value::Bool(true))]);
    let r: bool = render("{{ !input.flag }}", input);
    assert!(!r);
}

#[test]
fn comparison_lt() {
    assert!(render_bool("{{ 3 < 5 }}"));
    assert!(!render_bool("{{ 5 < 3 }}"));
}

#[test]
fn comparison_le() {
    assert!(render_bool("{{ 5 <= 5 }}"));
    assert!(render_bool("{{ 4 <= 5 }}"));
    assert!(!render_bool("{{ 6 <= 5 }}"));
}

#[test]
fn comparison_gt_ge() {
    assert!(render_bool("{{ 5 > 3 }}"));
    assert!(render_bool("{{ 5 >= 5 }}"));
}

#[test]
fn equality_int() {
    assert!(render_bool("{{ 5 == 5 }}"));
    assert!(render_bool("{{ 5 != 6 }}"));
    assert!(!render_bool("{{ 5 == 6 }}"));
}

#[test]
fn equality_int_decimal_numeric() {
    assert!(render_bool("{{ 5 == 5.0 }}"));
    assert!(!render_bool("{{ 5 == 5.1 }}"));
}

#[test]
fn equality_strings() {
    assert!(render_bool(r#"{{ "abc" == "abc" }}"#));
    assert!(!render_bool(r#"{{ "abc" == "abd" }}"#));
}

#[test]
fn equality_different_types_false() {
    assert!(!render_bool(r#"{{ 5 == "5" }}"#));
    assert!(render_bool(r#"{{ 5 != "5" }}"#));
}

#[test]
fn logical_and() {
    assert!(render_bool("{{ true && true }}"));
    assert!(!render_bool("{{ true && false }}"));
    assert!(!render_bool("{{ false && true }}"));
}

#[test]
fn logical_or() {
    assert!(render_bool("{{ true || false }}"));
    assert!(render_bool("{{ false || true }}"));
    assert!(!render_bool("{{ false || false }}"));
}

#[test]
fn logical_and_short_circuits() {
    assert!(!render_bool("{{ false && (1 / 0 == 0) }}"));
}

#[test]
fn logical_or_short_circuits() {
    assert!(render_bool("{{ true || (1 / 0 == 0) }}"));
}

#[test]
fn ternary_true_branch() {
    assert_eq!(render_int("{{ true ? 1 : 2 }}"), 1);
}

#[test]
fn ternary_false_branch() {
    assert_eq!(render_int("{{ false ? 1 : 2 }}"), 2);
}

#[test]
fn ternary_with_path() {
    let input = Value::obj([("n", Value::Int(7))]);
    let r: String = render(r#"{{ input.n > 5 ? "big" : "small" }}"#, input);
    assert_eq!(r, "big");
}

#[test]
fn ternary_right_associative() {
    let r: i64 = render("{{ false ? 1 : true ? 2 : 3 }}", Value::Null);
    assert_eq!(r, 2);
    let r: i64 = render("{{ false ? 1 : false ? 2 : 3 }}", Value::Null);
    assert_eq!(r, 3);
}

#[test]
fn when_first_match_wins() {
    let src = r#"{{ when {
        input.x > 0: "positive",
        input.x < 0: "negative",
        else: "zero"
    } }}"#;
    let r: String = render(src, Value::obj([("x", Value::Int(5))]));
    assert_eq!(r, "positive");
    let r: String = render(src, Value::obj([("x", Value::Int(-3))]));
    assert_eq!(r, "negative");
    let r: String = render(src, Value::obj([("x", Value::Int(0))]));
    assert_eq!(r, "zero");
}

#[test]
fn when_no_else_no_match_errors() {
    let src = r#"{{ when { input.x > 0: "positive" } }}"#;
    let template = Template::compile(src).expect("compile");
    let result = template.render::<String>(Value::obj([("x", Value::Int(-1))]));
    assert!(matches!(
        result,
        Err(temple_dsl::RenderError::WhenNoMatch { .. })
    ));
}

#[test]
fn when_trailing_comma_ok() {
    let src = r#"{{ when { true: "yes", else: "no", } }}"#;
    let r: String = render(src, Value::Null);
    assert_eq!(r, "yes");
}

#[test]
fn when_only_else_is_allowed() {
    let src = r#"{{ when { else: "always" } }}"#;
    let r: String = render(src, Value::Null);
    assert_eq!(r, "always");
}

#[test]
fn when_duplicate_else_errors() {
    let src = r#"{{ when { else: "a", else: "b" } }}"#;
    let err = Template::compile(src).expect_err("expected duplicate else error");
    assert!(format!("{:?}", err).contains("else"));
}

#[test]
fn divide_by_zero_errors() {
    let template = Template::compile("{{ 1 / 0 }}").expect("compile");
    let result = template.render::<i64>(Value::Null);
    assert!(matches!(
        result,
        Err(temple_dsl::RenderError::DivideByZero { .. })
    ));
}

#[test]
fn arithmetic_overflow_errors() {
    let template = Template::compile("{{ input.x + 1 }}").expect("compile");
    let result = template.render::<i64>(Value::obj([("x", Value::Int(i64::MAX))]));
    assert!(matches!(
        result,
        Err(temple_dsl::RenderError::ArithmeticOverflow { .. })
    ));
}

#[test]
fn type_mismatch_in_logical_errors() {
    let template = Template::compile("{{ 5 && true }}").expect("compile");
    let result = template.render::<bool>(Value::Null);
    assert!(matches!(
        result,
        Err(temple_dsl::RenderError::TypeMismatch { .. })
    ));
}

#[test]
fn type_mismatch_in_arithmetic_errors() {
    let template = Template::compile(r#"{{ "x" + 1 }}"#).expect("compile");
    let result = template.render::<String>(Value::Null);
    assert!(matches!(
        result,
        Err(temple_dsl::RenderError::TypeMismatch { .. })
    ));
}

#[test]
fn complex_output_with_operators() {
    #[derive(Deserialize, Debug)]
    struct Out {
        sum: i64,
        is_positive: bool,
        label: String,
    }

    let src = r#"{
        "sum":         {{ input.a + input.b }},
        "is_positive": {{ input.a > 0 }},
        "label":       {{ input.a > 0 ? "pos" : "neg" }}
    }"#;
    let input = Value::obj([("a", Value::Int(5)), ("b", Value::Int(3))]);
    let out: Out = render(src, input);
    assert_eq!(out.sum, 8);
    assert!(out.is_positive);
    assert_eq!(out.label, "pos");
}

#[test]
fn when_with_decimals() {
    #[derive(Deserialize, Debug)]
    struct Out {
        grade: String,
    }

    let src = r#"{
        "grade": {{ when {
            input.score >= 90: "A",
            input.score >= 80: "B",
            input.score >= 70: "C",
            input.score >= 60: "D",
            else: "F"
        } }}
    }"#;
    let input = Value::obj([("score", Value::Decimal("85.5".parse().unwrap()))]);
    let out: Out = render(src, input);
    assert_eq!(out.grade, "B");
}

#[test]
fn precedence_full_stack() {
    let r: bool = render("{{ true ? 1 + 2 * 3 == 7 && 4 > 3 : false }}", Value::Null);
    assert!(r);
}

#[test]
fn nested_when_in_ternary() {
    let src = r#"{{ input.x > 0 ? when { input.x > 100: "huge", else: "normal" } : "non-positive" }}"#;
    let r: String = render(src, Value::obj([("x", Value::Int(150))]));
    assert_eq!(r, "huge");
    let r: String = render(src, Value::obj([("x", Value::Int(50))]));
    assert_eq!(r, "normal");
    let r: String = render(src, Value::obj([("x", Value::Int(-1))]));
    assert_eq!(r, "non-positive");
}

#[test]
fn double_negation() {
    let input = Value::obj([("x", Value::Int(7))]);
    let r: i64 = render("{{ - -input.x }}", input);
    assert_eq!(r, 7);
}

#[test]
fn not_in_when_condition() {
    let src = r#"{{ when { !input.disabled: "active", else: "off" } }}"#;
    let r: String = render(src, Value::obj([("disabled", Value::Bool(false))]));
    assert_eq!(r, "active");
    let r: String = render(src, Value::obj([("disabled", Value::Bool(true))]));
    assert_eq!(r, "off");
}
