//! The `json` feature — exact conversions between `serde_json::Value` and
//! `temple_dsl::Value` in both directions, with no f64 detour for decimals.
#![cfg(feature = "json")]

use rust_decimal::Decimal;
use temple_dsl::{Template, Value};

fn dec(s: &str) -> Value {
    Value::Decimal(s.parse::<Decimal>().unwrap())
}

#[test]
fn json_input_flows_straight_into_render() {
    // No hand-rolled converter — serde_json::Value satisfies Into<Value>.
    let input: serde_json::Value =
        serde_json::from_str(r#"{ "price": 129.99, "rate": 0.0825 }"#).unwrap();
    let out = Template::compile("{{ input.price * input.rate }}")
        .unwrap()
        .render_value(input)
        .unwrap();
    // exact decimal math — would be 10.7241749… through f64
    assert_eq!(out, dec("10.724175"));
}

#[test]
fn json_numbers_convert_verbatim() {
    let j: serde_json::Value =
        serde_json::from_str(r#"{ "i": 7, "d": 0.1, "big": 99999999999999999999 }"#).unwrap();
    let v = Value::from(j);
    assert_eq!(
        v,
        Value::obj([
            ("i", Value::Int(7)),
            ("d", dec("0.1")), // the token "0.1", not 0.1000000000000000055511…
            ("big", dec("99999999999999999999")), // > i64::MAX, still exact
        ])
    );
}

#[test]
fn out_of_range_number_becomes_null_not_garbage() {
    let j: serde_json::Value = serde_json::from_str(r#"{ "huge": 1e400 }"#).unwrap();
    assert_eq!(Value::from(j), Value::obj([("huge", Value::Null)]));
}

#[test]
fn value_to_json_emits_exact_number_tokens() {
    let v = Value::obj([
        ("n", Value::Int(-3)),
        ("d", dec("10.724175")),
        ("s", Value::from("x")),
        ("b", Value::Bool(true)),
        ("nil", Value::Null),
        ("a", Value::Arr(vec![dec("0.10")])),
    ]);
    let j = serde_json::Value::from(v);
    // decimals are real number tokens carrying the exact digits — not strings
    assert_eq!(
        serde_json::to_string(&j).unwrap(),
        r#"{"n":-3,"d":10.724175,"s":"x","b":true,"nil":null,"a":[0.10]}"#
    );
}

#[test]
fn round_trip_preserves_value_and_key_order() {
    let src: serde_json::Value =
        serde_json::from_str(r#"{ "z": 1, "a": [true, null, "s", 2.50], "m": { "k": 0.3 } }"#)
            .unwrap();
    let back = serde_json::Value::from(Value::from(src.clone()));
    assert_eq!(back, src);
    // preserve_order keeps "z" before "a"
    assert_eq!(
        serde_json::to_string(&back).unwrap(),
        r#"{"z":1,"a":[true,null,"s",2.50],"m":{"k":0.3}}"#
    );
}

#[test]
fn full_pipeline_json_in_json_out() {
    let template =
        Template::compile(r#"{ "total": {{ input.items.map(it -> it.qty * it.price).sum() }} }"#)
            .unwrap();
    let input: serde_json::Value = serde_json::from_str(
        r#"{ "items": [ { "qty": 3, "price": 9.99 }, { "qty": 1, "price": 0.02 } ] }"#,
    )
    .unwrap();
    let out = serde_json::Value::from(template.render_value(input).unwrap());
    assert_eq!(serde_json::to_string(&out).unwrap(), r#"{"total":29.99}"#);
}
