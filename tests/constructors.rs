//! Expression-position object literals, string interpolation, and the
//! convenience collection methods (sum/any/all/len) from DESIGN §2-§3.

use rust_decimal::Decimal;
use serde::Deserialize;
use temple_dsl::{Template, Value};

fn render<T: serde::de::DeserializeOwned>(src: &str, input: Value) -> T {
    Template::compile(src)
        .expect("compile")
        .render::<T>(input)
        .expect("render")
}

fn arr(xs: &[i64]) -> Value {
    Value::Arr(xs.iter().copied().map(Value::Int).collect())
}

#[test]
fn object_literal_in_map_lambda() {
    #[derive(Deserialize, Debug, PartialEq)]
    struct Line {
        sku: String,
        total: i64,
    }
    let input = Value::obj([(
        "items",
        Value::Arr(vec![
            Value::obj([
                ("sku", Value::Str("a".into())),
                ("qty", Value::Int(2)),
                ("price", Value::Int(5)),
            ]),
            Value::obj([
                ("sku", Value::Str("b".into())),
                ("qty", Value::Int(3)),
                ("price", Value::Int(4)),
            ]),
        ]),
    )]);
    let out: Vec<Line> = render(
        r#"{{ input.items.map(it -> { "sku": it.sku, "total": it.qty * it.price }) }}"#,
        input,
    );
    assert_eq!(
        out,
        vec![
            Line {
                sku: "a".into(),
                total: 10
            },
            Line {
                sku: "b".into(),
                total: 12
            },
        ]
    );
}

#[test]
fn object_literal_bare_hole() {
    #[derive(Deserialize, Debug, PartialEq)]
    struct O {
        a: i64,
        b: i64,
    }
    assert_eq!(
        render::<O>(r#"{{ { "a": 1, "b": 2 } }}"#, Value::Null),
        O { a: 1, b: 2 }
    );
}

#[test]
fn object_literal_duplicate_key_rejected() {
    let err = Template::compile(r#"{{ { "a": 1, "a": 2 } }}"#).expect_err("dup");
    assert!(format!("{err:?}").contains("duplicate key"));
}

#[test]
fn duplicate_output_key_rejected() {
    let err = Template::compile(r#"{ "a": {{ 1 }}, "a": {{ 2 }} }"#).expect_err("dup");
    assert!(format!("{err:?}").contains("duplicate output key"));
}

#[test]
fn string_interpolation_basic() {
    #[derive(Deserialize)]
    struct O {
        note: String,
    }
    let o: O = render(
        r#"{ "note": "Thanks, {{ input.name }}!" }"#,
        Value::obj([("name", Value::Str("Ada".into()))]),
    );
    assert_eq!(o.note, "Thanks, Ada!");
}

#[test]
fn scalar_string_interpolation_template() {
    let out: String = render(
        r#""Hello {{ input.name }}, you have {{ input.n }} messages""#,
        Value::obj([("name", Value::Str("Ada".into())), ("n", Value::Int(3))]),
    );
    assert_eq!(out, "Hello Ada, you have 3 messages");
}

#[test]
fn string_interpolation_mixes_types() {
    #[derive(Deserialize)]
    struct O {
        msg: String,
    }
    let o: O = render(
        r#"{ "msg": "{{ input.name }} owes {{ input.amt }} ({{ input.vip }})" }"#,
        Value::obj([
            ("name", Value::Str("Bob".into())),
            ("amt", Value::Decimal("12.50".parse().unwrap())),
            ("vip", Value::Bool(true)),
        ]),
    );
    assert_eq!(o.msg, "Bob owes 12.50 (true)");
}

#[test]
fn string_interpolation_reads_this() {
    #[derive(Deserialize)]
    struct O {
        label: String,
    }
    let o: O = render(
        r#"{ "total": {{ input.a + input.b }}, "label": "total is {{ this.total }}" }"#,
        Value::obj([
            ("a", Value::Decimal("10".parse().unwrap())),
            ("b", Value::Decimal("5".parse().unwrap())),
        ]),
    );
    assert_eq!(o.label, "total is 15");
}

#[test]
fn plain_quoted_string_stays_literal() {
    #[derive(Deserialize)]
    struct O {
        s: String,
    }
    assert_eq!(
        render::<O>(r#"{ "s": "no holes here" }"#, Value::Null).s,
        "no holes here"
    );
}

#[test]
fn interpolating_a_collection_errors() {
    let t = Template::compile(r#"{ "s": "x={{ input.xs }}" }"#).unwrap();
    #[derive(Deserialize)]
    struct O {
        #[allow(dead_code)]
        s: String,
    }
    assert!(t.render::<O>(Value::obj([("xs", arr(&[1, 2]))])).is_err());
}

#[test]
fn sum_ints_and_empty() {
    assert_eq!(
        render::<i64>(
            "{{ input.xs.sum() }}",
            Value::obj([("xs", arr(&[1, 2, 3, 4]))])
        ),
        10
    );
    assert_eq!(
        render::<i64>(
            "{{ input.xs.sum() }}",
            Value::obj([("xs", Value::Arr(vec![]))])
        ),
        0
    );
}

#[test]
fn sum_decimals_exact() {
    let out: Decimal = render(
        "{{ input.xs.sum() }}",
        Value::obj([(
            "xs",
            Value::Arr(vec![
                Value::Decimal("1.5".parse().unwrap()),
                Value::Decimal("2.25".parse().unwrap()),
            ]),
        )]),
    );
    assert_eq!(out, "3.75".parse::<Decimal>().unwrap());
}

#[test]
fn any_and_all() {
    let inp = Value::obj([("xs", arr(&[1, 2, 3, 4]))]);
    assert!(render::<bool>(
        "{{ input.xs.any(x -> x > 3) }}",
        inp.clone()
    ));
    assert!(!render::<bool>(
        "{{ input.xs.any(x -> x > 10) }}",
        inp.clone()
    ));
    assert!(render::<bool>(
        "{{ input.xs.all(x -> x > 0) }}",
        inp.clone()
    ));
    assert!(!render::<bool>("{{ input.xs.all(x -> x > 2) }}", inp));
}

#[test]
fn any_and_all_on_empty() {
    let inp = Value::obj([("xs", Value::Arr(vec![]))]);
    assert!(!render::<bool>(
        "{{ input.xs.any(x -> x > 0) }}",
        inp.clone()
    ));
    assert!(render::<bool>("{{ input.xs.all(x -> x > 0) }}", inp));
}

#[test]
fn len_method_aliases_length() {
    let inp = Value::obj([("xs", arr(&[1, 2, 3]))]);
    assert_eq!(render::<i64>("{{ input.xs.len() }}", inp.clone()), 3);
    assert_eq!(render::<i64>("{{ input.xs.len() }}", inp), 3);
}

#[test]
fn min_max_promote_to_decimal_when_mixed() {
    assert_eq!(
        render::<Decimal>("{{ max(2.0, 1) }}", Value::Null),
        "2".parse::<Decimal>().unwrap()
    );
    assert_eq!(
        render::<Decimal>("{{ min(1, 2.0) }}", Value::Null),
        "1".parse::<Decimal>().unwrap()
    );
}
