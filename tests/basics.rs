//! Foundational output — value literals, bare holes, path access, nested
//! object/array shapes, comments/trailing commas, and the typed round-trip.

use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;
use temple_dsl::{Template, Value};

fn empty_input() -> Value {
    Value::Obj(IndexMap::new())
}

#[test]
fn empty_object_template() {
    let template = Template::compile("{}").expect("compile");

    #[derive(Deserialize, Debug)]
    struct Empty {}

    let _: Empty = template.render(empty_input()).expect("render");
}

#[test]
fn all_literal_value_types() {
    let src = r#"{
        "s": "hello",
        "n": 42,
        "d": 3.14,
        "b": true,
        "z": null
    }"#;
    let template = Template::compile(src).expect("compile");

    #[derive(Deserialize, Debug)]
    struct Lit {
        s: String,
        n: i64,
        d: Decimal,
        b: bool,
        z: Option<String>,
    }

    let result: Lit = template.render(empty_input()).expect("render");
    assert_eq!(result.s, "hello");
    assert_eq!(result.n, 42);
    assert_eq!(result.d, Decimal::from_str("3.14").unwrap());
    assert!(result.b);
    assert_eq!(result.z, None);
}

#[test]
fn bare_hole_from_input() {
    let template = Template::compile(r#"{ "id": {{ input.id }} }"#).expect("compile");

    #[derive(Deserialize, Debug)]
    struct Out {
        id: i64,
    }

    let input = Value::obj([("id", Value::Int(7))]);
    let result: Out = template.render(input).expect("render");
    assert_eq!(result.id, 7);
}

#[test]
fn nested_path() {
    let template = Template::compile(r#"{ "x": {{ input.a.b.c }} }"#).expect("compile");

    #[derive(Deserialize, Debug)]
    struct Out {
        x: String,
    }

    let input = Value::obj([(
        "a",
        Value::obj([("b", Value::obj([("c", Value::Str("deep".into()))]))]),
    )]);
    let result: Out = template.render(input).expect("render");
    assert_eq!(result.x, "deep");
}

#[test]
fn array_literal_in_output() {
    let template = Template::compile(r#"{ "list": [1, 2, 3] }"#).expect("compile");

    #[derive(Deserialize, Debug)]
    struct Out {
        list: Vec<i64>,
    }

    let result: Out = template.render(empty_input()).expect("render");
    assert_eq!(result.list, vec![1, 2, 3]);
}

#[test]
fn array_with_holes() {
    let template =
        Template::compile(r#"{ "list": [{{ input.a }}, {{ input.b }}, 99] }"#).expect("compile");

    #[derive(Deserialize, Debug)]
    struct Out {
        list: Vec<i64>,
    }

    let input = Value::obj([("a", Value::Int(1)), ("b", Value::Int(2))]);
    let result: Out = template.render(input).expect("render");
    assert_eq!(result.list, vec![1, 2, 99]);
}

#[test]
fn nested_object_output() {
    let src = r#"{ "outer": { "inner": {{ input.value }} } }"#;
    let template = Template::compile(src).expect("compile");

    #[derive(Deserialize, Debug)]
    struct Out {
        outer: Inner,
    }
    #[derive(Deserialize, Debug)]
    struct Inner {
        inner: String,
    }

    let input = Value::obj([("value", Value::Str("deep".into()))]);
    let result: Out = template.render(input).expect("render");
    assert_eq!(result.outer.inner, "deep");
}

#[test]
fn missing_field_error() {
    let template = Template::compile(r#"{ "x": {{ input.missing }} }"#).expect("compile");

    #[derive(Deserialize, Debug)]
    struct Out {
        #[allow(dead_code)]
        x: String,
    }

    let input = Value::obj([("other", Value::Str("value".into()))]);
    let result: Result<Out, _> = template.render(input);
    let err = result.expect_err("expected MissingPath");
    assert!(err.to_string().contains("missing"), "got: {err}");
}

#[test]
fn typed_struct_round_trip() {
    let src = r#"{
        "id":       {{ input.id }},
        "name":     {{ input.name }},
        "active":   true,
        "amount":   1.23,
        "tags":     ["a", "b"],
        "metadata": { "src": {{ input.source }} }
    }"#;
    let template = Template::compile(src).expect("compile");

    #[derive(Deserialize, Debug)]
    struct Out {
        id: i64,
        name: String,
        active: bool,
        amount: Decimal,
        tags: Vec<String>,
        metadata: Meta,
    }
    #[derive(Deserialize, Debug)]
    struct Meta {
        src: String,
    }

    let input = Value::obj([
        ("id", Value::Int(42)),
        ("name", Value::Str("Ada".into())),
        ("source", Value::Str("manual".into())),
    ]);
    let out: Out = template.render(input).expect("render");
    assert_eq!(out.id, 42);
    assert_eq!(out.name, "Ada");
    assert!(out.active);
    assert_eq!(out.amount, Decimal::from_str("1.23").unwrap());
    assert_eq!(out.tags, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(out.metadata.src, "manual");
}

#[test]
fn trailing_commas_and_comments() {
    let src = r#"# top-level comment
    {
        # inside the object
        "a": {{ input.a }},
        "b": [1, 2, 3,],
    }
    "#;
    let template = Template::compile(src).expect("compile");

    #[derive(Deserialize, Debug)]
    struct Out {
        a: String,
        b: Vec<i64>,
    }

    let input = Value::obj([("a", Value::Str("ok".into()))]);
    let result: Out = template.render(input).expect("render");
    assert_eq!(result.a, "ok");
    assert_eq!(result.b, vec![1, 2, 3]);
}
