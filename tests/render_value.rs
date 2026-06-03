//! `render_value` returns the dynamic `Value` directly (no serde round-trip).
//! Rendering into `Value` via `render::<Value>` is intentionally a compile error.

use rust_decimal::Decimal;
use temple_dsl::{Template, Value};

fn rv(src: &str, input: Value) -> Value {
    Template::compile(src).unwrap().render_value(input).unwrap()
}

#[test]
fn scalar_shapes() {
    assert_eq!(
        rv("{{ input.n }}", Value::obj([("n", Value::Int(5))])),
        Value::Int(5)
    );
    assert_eq!(
        rv(
            "{{ input.s }}",
            Value::obj([("s", Value::Str("hi".into()))])
        ),
        Value::Str("hi".into())
    );
    assert_eq!(rv("{{ 1 == 1 }}", Value::Null), Value::Bool(true));
    assert_eq!(rv("{{ null }}", Value::Null), Value::Null);
    assert_eq!(
        rv("{{ 10 / 4 }}", Value::Null),
        Value::Decimal("2.5".parse::<Decimal>().unwrap())
    );
}

#[test]
fn array_shape() {
    assert_eq!(
        rv(
            "[{{ input.a }}, {{ input.b }}]",
            Value::obj([("a", Value::Int(1)), ("b", Value::Int(2))]),
        ),
        Value::Arr(vec![Value::Int(1), Value::Int(2)]),
    );
}

#[test]
fn object_shape_and_order() {
    let out = rv(
        r#"{ "z": {{ input.n }}, "a": {{ input.m }} }"#,
        Value::obj([("n", Value::Int(9)), ("m", Value::Int(8))]),
    );
    assert_eq!(
        out,
        Value::obj([("z", Value::Int(9)), ("a", Value::Int(8))])
    );
    // declared key order preserved
    if let Value::Obj(o) = &out {
        let keys: Vec<&str> = o.keys().map(|s| s.as_str()).collect();
        assert_eq!(keys, vec!["z", "a"]);
    } else {
        panic!("expected object");
    }
}

#[test]
fn matches_typed_render() {
    // render_value and render::<T> agree on the same template.
    let src = r#"{ "total": {{ input.a + input.b }} }"#;
    let input = Value::obj([("a", Value::Int(3)), ("b", Value::Int(4))]);
    let t = Template::compile(src).unwrap();
    #[derive(serde::Deserialize)]
    struct O {
        total: i64,
    }
    let typed: O = t.render(input.clone()).unwrap();
    let dynamic = t.render_value(input).unwrap();
    assert_eq!(typed.total, 7);
    assert_eq!(dynamic, Value::obj([("total", Value::Int(7))]));
}
