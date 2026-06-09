//! Standard-library expansion — the `%` operator, the new builtin functions,
//! and the string / array / object methods.

use rust_decimal::Decimal;
use temple_dsl::{Template, Value};

fn render(src: &str, input: Value) -> Value {
    Template::compile(src)
        .expect("compile")
        .render_value(input)
        .expect("render")
}

fn eval(src: &str) -> Value {
    render(src, Value::Null)
}

fn render_fails(src: &str, input: Value) -> bool {
    Template::compile(src)
        .expect("compile")
        .render_value(input)
        .is_err()
}

fn dec(s: &str) -> Value {
    Value::Decimal(s.parse::<Decimal>().unwrap())
}

fn arr(items: Vec<Value>) -> Value {
    Value::Arr(items)
}

fn ints(ns: &[i64]) -> Value {
    Value::Arr(ns.iter().map(|n| Value::Int(*n)).collect())
}

// ── modulo ──

#[test]
fn modulo() {
    assert_eq!(eval("{{ 7 % 3 }}"), Value::Int(1));
    assert_eq!(eval("{{ 10.5 % 3 }}"), dec("1.5"));
    assert_eq!(eval("{{ 8 % 2 == 0 }}"), Value::Bool(true));
    assert!(render_fails("{{ 5 % 0 }}", Value::Null));
}

// ── builtins ──

#[test]
fn concat_builds_strings() {
    assert_eq!(
        render(
            "{{ concat(\"Bearer \", input.t) }}",
            Value::obj([("t", "abc")])
        ),
        Value::Str("Bearer abc".into())
    );
    // auto-stringifies scalars of any kind
    assert_eq!(
        eval("{{ concat(\"n=\", 7, \" ok=\", true, \" x=\", null) }}"),
        Value::Str("n=7 ok=true x=null".into())
    );
    // collections are not concatenable
    assert!(render_fails(
        "{{ concat(\"x\", input.a) }}",
        Value::obj([("a", vec![1])])
    ));
}

#[test]
fn to_number_parses() {
    assert_eq!(eval("{{ to_number(\"42\") }}"), Value::Int(42));
    assert_eq!(eval("{{ to_number(\"3.14\") }}"), dec("3.14"));
    assert_eq!(eval("{{ to_number(\"42\") + 1 }}"), Value::Int(43));
    assert_eq!(eval("{{ to_number(9) }}"), Value::Int(9)); // number passthrough
    assert!(render_fails("{{ to_number(\"nope\") }}", Value::Null));
}

#[test]
fn type_of_and_is_checks() {
    assert_eq!(eval("{{ type_of(7) }}"), Value::Str("int".into()));
    assert_eq!(eval("{{ type_of(7.5) }}"), Value::Str("decimal".into()));
    assert_eq!(eval("{{ type_of(\"x\") }}"), Value::Str("string".into()));
    assert_eq!(eval("{{ type_of([1]) }}"), Value::Str("array".into()));
    assert_eq!(eval("{{ type_of(null) }}"), Value::Str("null".into()));
    assert_eq!(eval("{{ is_number(7) }}"), Value::Bool(true));
    assert_eq!(eval("{{ is_string(7) }}"), Value::Bool(false));
    assert_eq!(
        render("{{ is_array(input.a) }}", Value::obj([("a", vec![1])])),
        Value::Bool(true)
    );
    assert_eq!(eval("{{ is_null(null) }}"), Value::Bool(true));
}

#[test]
fn json_encode_serializes() {
    let input = Value::obj([(
        "o",
        Value::obj([
            ("a", Value::Int(1)),
            ("b", arr(vec![Value::Int(2), Value::Int(3)])),
        ]),
    )]);
    assert_eq!(
        render("{{ json_encode(input.o) }}", input),
        Value::Str("{\"a\":1,\"b\":[2,3]}".into())
    );
    // strings are JSON-escaped
    assert_eq!(
        render("{{ json_encode(input.s) }}", Value::obj([("s", "a\"b\nc")])),
        Value::Str("\"a\\\"b\\nc\"".into())
    );
}

#[test]
fn url_encode_and_base64() {
    assert_eq!(
        eval("{{ url_encode(\"a b/c\") }}"),
        Value::Str("a%20b%2Fc".into())
    );
    assert_eq!(
        eval("{{ base64(\"abc 123\") }}"),
        Value::Str("YWJjIDEyMw==".into())
    );
}

// ── string methods ──

#[test]
fn string_methods() {
    let i = Value::obj([("s", "Hello World")]);
    assert_eq!(
        render("{{ input.s.contains(\"World\") }}", i.clone()),
        Value::Bool(true)
    );
    assert_eq!(
        render("{{ input.s.starts_with(\"Hell\") }}", i.clone()),
        Value::Bool(true)
    );
    assert_eq!(
        render("{{ input.s.ends_with(\"rld\") }}", i.clone()),
        Value::Bool(true)
    );
    assert_eq!(
        render("{{ input.s.replace(\"l\", \"L\") }}", i.clone()),
        Value::Str("HeLLo WorLd".into())
    );
    assert_eq!(
        render("{{ input.s.slice(0, 5) }}", i.clone()),
        Value::Str("Hello".into())
    );
    assert_eq!(
        render("{{ input.s.index_of(\"World\") }}", i.clone()),
        Value::Int(6)
    );
    assert_eq!(
        render("{{ input.s.index_of(\"zzz\") }}", i.clone()),
        Value::Int(-1)
    );
    assert_eq!(
        render("{{ \"a,b,c\".split(\",\") }}", Value::Null),
        arr(vec!["a".into(), "b".into(), "c".into()])
    );
}

// ── array methods ──

#[test]
fn array_search_and_membership() {
    let i = Value::obj([("a", ints(&[3, 1, 2, 1]))]);
    assert_eq!(
        render("{{ input.a.contains(2) }}", i.clone()),
        Value::Bool(true)
    );
    assert_eq!(
        render("{{ input.a.contains(9) }}", i.clone()),
        Value::Bool(false)
    );
    assert_eq!(
        render("{{ input.a.index_of(2) }}", i.clone()),
        Value::Int(2)
    );
    assert_eq!(
        render("{{ input.a.count(x -> x == 1) }}", i.clone()),
        Value::Int(2)
    );
    assert_eq!(render("{{ input.a.find(x -> x > 1) }}", i), Value::Int(3));
}

#[test]
fn array_transforms() {
    let i = Value::obj([("a", ints(&[3, 1, 2, 1]))]);
    assert_eq!(
        render("{{ input.a.sort() }}", i.clone()),
        ints(&[1, 1, 2, 3])
    );
    assert_eq!(
        render("{{ input.a.reverse() }}", i.clone()),
        ints(&[1, 2, 1, 3])
    );
    assert_eq!(
        render("{{ input.a.unique() }}", i.clone()),
        ints(&[3, 1, 2])
    );
    assert_eq!(render("{{ input.a.take(2) }}", i.clone()), ints(&[3, 1]));
    assert_eq!(render("{{ input.a.drop(2) }}", i.clone()), ints(&[2, 1]));
    assert_eq!(
        render("{{ input.a.slice(1, 3) }}", i.clone()),
        ints(&[1, 2])
    );
    assert_eq!(
        render("{{ input.a.join(\"-\") }}", i),
        Value::Str("3-1-2-1".into())
    );
    assert_eq!(
        render(
            "{{ input.a.flatten() }}",
            Value::obj([("a", arr(vec![ints(&[1, 2]), ints(&[3])]))])
        ),
        ints(&[1, 2, 3])
    );
    assert_eq!(
        render("{{ [1, 2].flat_map(x -> [x, x]) }}", Value::Null),
        ints(&[1, 1, 2, 2])
    );
}

#[test]
fn array_sort_by_and_aggregates() {
    let objs = arr(vec![
        Value::obj([("n", "x"), ("p", "3")]),
        Value::obj([("n", "y"), ("p", "1")]),
    ]);
    // sort_by p ascending → y then x
    assert_eq!(
        render(
            "{{ input.o.sort_by(it -> it.p).map(it -> it.n) }}",
            Value::obj([("o", make_objs())])
        ),
        arr(vec!["y".into(), "x".into()])
    );
    let _ = objs;
    let nums = Value::obj([("a", ints(&[3, 1, 2]))]);
    assert_eq!(render("{{ input.a.min() }}", nums.clone()), Value::Int(1));
    assert_eq!(render("{{ input.a.max() }}", nums.clone()), Value::Int(3));
    assert_eq!(render("{{ input.a.avg() }}", nums), dec("2"));
    assert!(render_fails(
        "{{ input.a.min() }}",
        Value::obj([("a", Value::Arr(vec![]))])
    ));
}

#[test]
fn min_max_strings_and_method_function_agreement() {
    // strings compare lexicographically, in both forms
    assert_eq!(eval("{{ ['b', 'a'].min() }}"), Value::Str("a".into()));
    assert_eq!(eval("{{ max('b', 'a') }}"), Value::Str("b".into()));
    // the method matches the function's Decimal-promotion rule
    assert_eq!(eval("{{ min(1, 2.5) }}"), dec("1"));
    assert_eq!(eval("{{ [1, 2.5].min() }}"), dec("1"));
    // mixed strings and numbers stay an error
    assert!(render_fails("{{ [1, 'a'].min() }}", Value::Null));
}

fn make_objs() -> Value {
    arr(vec![
        Value::obj([("n", Value::from("x")), ("p", Value::Int(3))]),
        Value::obj([("n", Value::from("y")), ("p", Value::Int(1))]),
    ])
}

// ── object methods ──

#[test]
fn object_methods() {
    let i = Value::obj([(
        "o",
        Value::obj([("a", Value::Int(1)), ("b", Value::Int(2))]),
    )]);
    assert_eq!(
        render("{{ input.o.keys() }}", i.clone()),
        arr(vec!["a".into(), "b".into()])
    );
    assert_eq!(render("{{ input.o.values() }}", i.clone()), ints(&[1, 2]));
    assert_eq!(
        render("{{ input.o.has(\"a\") }}", i.clone()),
        Value::Bool(true)
    );
    assert_eq!(
        render("{{ input.o.has(\"z\") }}", i.clone()),
        Value::Bool(false)
    );
    assert_eq!(render("{{ input.o.get(\"a\") }}", i.clone()), Value::Int(1));
    assert_eq!(
        render("{{ input.o.get(\"z\") ?? -1 }}", i.clone()),
        Value::Int(-1)
    );
    assert_eq!(
        render("{{ input.o.merge({ \"c\": 3, \"a\": 9 }) }}", i.clone()),
        Value::obj([
            ("a", Value::Int(9)),
            ("b", Value::Int(2)),
            ("c", Value::Int(3))
        ])
    );
    assert_eq!(
        render("{{ input.o.entries().map(e -> e.key) }}", i),
        arr(vec!["a".into(), "b".into()])
    );
}
