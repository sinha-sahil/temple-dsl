//! Built-in functions — `abs`/`round`/`floor`/`ceil`/`min`/`max`/`upper`/
//! `lower`/`trim`/`to_string`/`len`, plus arity and type errors.

use rust_decimal::Decimal;
use temple_dsl::{RenderError, Template, Value};

fn render<T: serde::de::DeserializeOwned>(src: &str, input: Value) -> T {
    let template = Template::compile(src).expect("compile");
    template.render::<T>(input).expect("render")
}

#[test]
fn abs_negative_int() {
    let out: i64 = render("{{ abs(-7) }}", Value::Null);
    assert_eq!(out, 7);
}

#[test]
fn abs_negative_decimal() {
    let out: Decimal = render("{{ abs(-3.14) }}", Value::Null);
    assert_eq!(out, "3.14".parse::<Decimal>().unwrap());
}

#[test]
fn abs_positive_passthrough() {
    let out: i64 = render("{{ abs(5) }}", Value::Null);
    assert_eq!(out, 5);
}

#[test]
fn round_to_integer() {
    let out: Decimal = render("{{ round(3.567) }}", Value::Null);
    assert_eq!(out, "4".parse::<Decimal>().unwrap());
}

#[test]
fn round_to_n_decimals() {
    let out: Decimal = render("{{ round(3.567, 2) }}", Value::Null);
    assert_eq!(out, "3.57".parse::<Decimal>().unwrap());
}

#[test]
fn round_int_unchanged() {
    let out: i64 = render("{{ round(42) }}", Value::Null);
    assert_eq!(out, 42);
}

#[test]
fn floor_decimal() {
    let out: Decimal = render("{{ floor(3.9) }}", Value::Null);
    assert_eq!(out, "3".parse::<Decimal>().unwrap());
}

#[test]
fn ceil_decimal() {
    let out: Decimal = render("{{ ceil(3.1) }}", Value::Null);
    assert_eq!(out, "4".parse::<Decimal>().unwrap());
}

#[test]
fn floor_negative() {
    let out: Decimal = render("{{ floor(-3.1) }}", Value::Null);
    assert_eq!(out, "-4".parse::<Decimal>().unwrap());
}

#[test]
fn ceil_negative() {
    let out: Decimal = render("{{ ceil(-3.9) }}", Value::Null);
    assert_eq!(out, "-3".parse::<Decimal>().unwrap());
}

#[test]
fn min_two_args() {
    let out: i64 = render("{{ min(5, 3) }}", Value::Null);
    assert_eq!(out, 3);
}

#[test]
fn max_two_args() {
    let out: i64 = render("{{ max(5, 3) }}", Value::Null);
    assert_eq!(out, 5);
}

#[test]
fn min_variadic() {
    let out: i64 = render("{{ min(7, 3, 5, 1, 9) }}", Value::Null);
    assert_eq!(out, 1);
}

#[test]
fn max_variadic() {
    let out: i64 = render("{{ max(7, 3, 5, 1, 9) }}", Value::Null);
    assert_eq!(out, 9);
}

#[test]
fn min_decimal() {
    let out: Decimal = render("{{ min(1.5, 0.5, 2.5) }}", Value::Null);
    assert_eq!(out, "0.5".parse::<Decimal>().unwrap());
}

#[test]
fn min_mixed_int_decimal() {
    let out: Decimal = render("{{ min(1, 0.5) }}", Value::Null);
    assert_eq!(out, "0.5".parse::<Decimal>().unwrap());
}

#[test]
fn upper_string() {
    let out: String = render(r#"{{ upper("hello") }}"#, Value::Null);
    assert_eq!(out, "HELLO");
}

#[test]
fn lower_string() {
    let out: String = render(r#"{{ lower("WORLD") }}"#, Value::Null);
    assert_eq!(out, "world");
}

#[test]
fn trim_string() {
    let out: String = render(r#"{{ trim("  hello  ") }}"#, Value::Null);
    assert_eq!(out, "hello");
}

#[test]
fn to_string_int() {
    let out: String = render("{{ to_string(42) }}", Value::Null);
    assert_eq!(out, "42");
}

#[test]
fn to_string_decimal() {
    let out: String = render("{{ to_string(3.14) }}", Value::Null);
    assert_eq!(out, "3.14");
}

#[test]
fn to_string_bool() {
    let out: String = render("{{ to_string(true) }}", Value::Null);
    assert_eq!(out, "true");
}

#[test]
fn to_string_null() {
    let out: String = render("{{ to_string(null) }}", Value::Null);
    assert_eq!(out, "null");
}

#[test]
fn to_string_string_passthrough() {
    let out: String = render(r#"{{ to_string("hi") }}"#, Value::Null);
    assert_eq!(out, "hi");
}

#[test]
fn len_is_a_method_not_a_function() {
    // One spelling: `.len()`. The function form does not exist.
    let out: i64 = render(r#"{{ "hello".len() }}"#, Value::Null);
    assert_eq!(out, 5);
    let out: i64 = render(
        "{{ input.xs.len() }}",
        Value::obj([(
            "xs",
            Value::Arr(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        )]),
    );
    assert_eq!(out, 3);
    assert!(Template::compile("{{ len(input.xs) }}").is_err());
}

#[test]
fn unknown_function_compile_error() {
    let err = Template::compile("{{ nope(5) }}").expect_err("expected unknown function error");
    assert!(format!("{err:?}").contains("unknown function"));
}

#[test]
fn function_arity_mismatch_zero_args() {
    let template = Template::compile("{{ abs() }}").expect("compile");
    let result = template.render::<i64>(Value::Null);
    assert!(matches!(result, Err(RenderError::ArityMismatch { .. })));
}

#[test]
fn function_arity_mismatch_too_many() {
    let template = Template::compile("{{ abs(1, 2, 3) }}").expect("compile");
    let result = template.render::<i64>(Value::Null);
    assert!(matches!(result, Err(RenderError::ArityMismatch { .. })));
}

#[test]
fn function_wrong_type_errors() {
    let template = Template::compile(r#"{{ abs("hello") }}"#).expect("compile");
    let result = template.render::<i64>(Value::Null);
    assert!(matches!(result, Err(RenderError::TypeMismatch { .. })));
}

#[test]
fn min_empty_args_errors() {
    let template = Template::compile("{{ min() }}").expect("compile");
    let result = template.render::<i64>(Value::Null);
    assert!(matches!(result, Err(RenderError::ArityMismatch { .. })));
}

#[test]
fn function_inside_lambda() {
    let input = Value::obj([(
        "xs",
        Value::Arr(vec![
            Value::Int(-3),
            Value::Int(5),
            Value::Int(-7),
            Value::Int(2),
        ]),
    )]);
    let out: Vec<i64> = render("{{ input.xs.map(x -> abs(x)) }}", input);
    assert_eq!(out, vec![3, 5, 7, 2]);
}

#[test]
fn function_with_path_arg() {
    let input = Value::obj([("price", Value::Decimal("19.999".parse().unwrap()))]);
    let out: Decimal = render("{{ round(input.price, 2) }}", input);
    assert_eq!(out, "20".parse::<Decimal>().unwrap());
}

#[test]
fn nested_function_calls() {
    let out: i64 = render("{{ abs(min(-3, -5, -1)) }}", Value::Null);
    assert_eq!(out, 5);
}

#[test]
fn function_in_when_branch() {
    let src = r#"{{ when {
        input.x > 0: upper("positive"),
        input.x < 0: upper("negative"),
        else: upper("zero")
    } }}"#;
    let input = Value::obj([("x", Value::Int(-5))]);
    let out: String = render(src, input);
    assert_eq!(out, "NEGATIVE");
}

#[test]
fn full_pipeline_with_functions() {
    let src = r#"
        let prices = input.lines.map(line -> line.price)
        {
            "customer": {{ upper(input.customer) }},
            "subtotal": {{ prices.fold(0, (acc, p) -> acc + p) }},
            "tax":      {{ round(this.subtotal * 0.0825, 2) }},
            "total":    {{ this.subtotal + this.tax }},
            "highest":  {{ prices.fold(0, (acc, p) -> max(acc, p)) }},
            "rounded":  {{ floor(this.total) }}
        }
    "#;
    let input = Value::obj([
        ("customer", Value::Str("ada lovelace".into())),
        (
            "lines",
            Value::Arr(vec![
                Value::obj([("price", Value::Decimal("9.99".parse().unwrap()))]),
                Value::obj([("price", Value::Decimal("4.50".parse().unwrap()))]),
                Value::obj([("price", Value::Decimal("19.99".parse().unwrap()))]),
            ]),
        ),
    ]);
    #[derive(serde::Deserialize)]
    struct Out {
        customer: String,
        subtotal: Decimal,
        tax: Decimal,
        total: Decimal,
        highest: Decimal,
        rounded: Decimal,
    }
    let out: Out = render(src, input);
    assert_eq!(out.customer, "ADA LOVELACE");
    assert_eq!(out.subtotal, "34.48".parse::<Decimal>().unwrap());
    assert_eq!(out.tax, "2.84".parse::<Decimal>().unwrap());
    assert_eq!(out.total, "37.32".parse::<Decimal>().unwrap());
    assert_eq!(out.highest, "19.99".parse::<Decimal>().unwrap());
    assert_eq!(out.rounded, "37".parse::<Decimal>().unwrap());
}
