//! Structural features — conditional field omission (`"k"?:`), computed object
//! keys (`{ [expr]: v }`), object indexing, `let … in …`, and method/index
//! chains on any expression (not just identifier paths).

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

fn compile_fails(src: &str) -> bool {
    Template::compile(src).is_err()
}

// ── conditional field omission ──

#[test]
fn omits_null_output_key() {
    let out = render(
        r#"{ "always": {{ input.n }}, "coupon"?: {{ input.coupon }} }"#,
        Value::obj([("n", Value::Int(7)), ("coupon", Value::Null)]),
    );
    assert_eq!(out, Value::obj([("always", Value::Int(7))]));
}

#[test]
fn keeps_non_null_optional_key() {
    let out = render(
        r#"{ "coupon"?: {{ input.coupon }} }"#,
        Value::obj([("coupon", Value::Str("SAVE".into()))]),
    );
    assert_eq!(out, Value::obj([("coupon", Value::Str("SAVE".into()))]));
}

#[test]
fn omits_in_object_literal() {
    let out = render(
        r#"{{ { "a": input.n, "b"?: input.coupon } }}"#,
        Value::obj([("n", Value::Int(1)), ("coupon", Value::Null)]),
    );
    assert_eq!(out, Value::obj([("a", Value::Int(1))]));
}

// ── computed object keys ──

#[test]
fn computed_object_key() {
    let out = render(
        r#"{{ { [input.k]: input.v } }}"#,
        Value::obj([("k", Value::Str("dyn".into())), ("v", Value::Int(9))]),
    );
    assert_eq!(out, Value::obj([("dyn", Value::Int(9))]));
}

#[test]
fn computed_non_string_key_errors() {
    assert!(render_fails(
        r#"{{ { [input.n]: 1 } }}"#,
        Value::obj([("n", Value::Int(5))])
    ));
}

#[test]
fn duplicate_computed_key_is_a_render_error() {
    // A computed key colliding with a static one (or another computed one)
    // must error, never silently overwrite.
    assert!(render_fails(r#"{{ { "a": 1, ["a"]: 2 } }}"#, Value::Null));
    assert!(render_fails(
        r#"{{ { [input.k1]: 1, [input.k2]: 2 } }}"#,
        Value::obj([("k1", Value::from("x")), ("k2", Value::from("x"))])
    ));
    // distinct keys stay fine
    assert!(!render_fails(r#"{{ { "a": 1, ["b"]: 2 } }}"#, Value::Null));
}

#[test]
fn group_by_via_fold_merge_dynamic_key() {
    let items = Value::Arr(vec![
        Value::obj([("cat", Value::from("a"))]),
        Value::obj([("cat", Value::from("b"))]),
        Value::obj([("cat", Value::from("a"))]),
    ]);
    let out = render(
        r#"{{ input.items.fold({}, (acc, x) -> acc.merge({ [x.cat]: (acc.get(x.cat) ?? 0) + 1 })) }}"#,
        Value::obj([("items", items)]),
    );
    assert_eq!(
        out,
        Value::obj([("a", Value::Int(2)), ("b", Value::Int(1))])
    );
}

// ── object indexing ──

#[test]
fn object_index_strict() {
    let i = Value::obj([("o", Value::obj([("a", Value::Int(1))]))]);
    assert_eq!(render(r#"{{ input.o["a"] }}"#, i.clone()), Value::Int(1));
    assert!(render_fails(r#"{{ input.o["missing"] }}"#, i)); // strict: errors when absent
}

// ── let … in … ──

#[test]
fn let_in_basic_and_scoped() {
    assert_eq!(eval("{{ let x = 2 in x * 10 }}"), Value::Int(20));
    // body sees the binding; binding doesn't leak (compile error if it did)
    assert_eq!(
        eval("{{ let a = 3 in let b = a + 1 in a * b }}"),
        Value::Int(12)
    );
}

#[test]
fn let_in_inside_when() {
    let out = render(
        r#"{{ when { let s = input.score in s >= 80: "high", else: "low" } }}"#,
        Value::obj([("score", Value::Int(85))]),
    );
    assert_eq!(out, Value::Str("high".into()));
}

#[test]
fn let_in_shadows() {
    assert_eq!(
        eval("{{ let x = 1 in let x = x + 10 in x }}"),
        Value::Int(11)
    );
}

#[test]
fn let_in_reserved_name_is_compile_error() {
    assert!(compile_fails("{{ let input = 5 in input }}"));
}

#[test]
fn in_is_reserved_everywhere() {
    // `in` is the let-in separator — binding it would make grammar soup.
    assert!(compile_fails("let in = 5\n{ \"a\": 1 }"));
    assert!(compile_fails("{{ let in = 5 in in }}"));
    assert!(compile_fails("{{ [1].map(in -> in * 2) }}"));
}

#[test]
fn let_in_body_can_read_this() {
    // `this.a` referenced inside a let-in body must still wire the dependency
    // graph (b depends on a), so ordering resolves regardless of declaration order.
    let out = render(
        r#"{ "b": {{ let k = this.a in k * 2 }}, "a": {{ input.n }} }"#,
        Value::obj([("n", Value::Int(5))]),
    );
    assert_eq!(
        out,
        Value::obj([("b", Value::Int(10)), ("a", Value::Int(5))])
    );
}

// ── postfix on any expression ──

#[test]
fn methods_chain_on_non_identifier_base() {
    assert_eq!(
        eval("{{ [3, 1, 2].sort() }}"),
        Value::Arr(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        eval(r#"{{ concat("x", "y").replace("x", "X") }}"#),
        Value::Str("Xy".into())
    );
    assert_eq!(eval(r#"{{ ([1, 2].concat([3])).len() }}"#), Value::Int(3));
}

// ── format round-trips the new syntax ──

#[test]
fn format_is_stable_for_new_syntax() {
    for src in [
        "{{ input.n % 3 }}",
        r#"{ "coupon"?: {{ input.c }} }"#,
        r#"{{ { [input.k]: input.v, "x"?: input.y } }}"#,
        "{{ let x = input.n + 1 in x * 2 }}",
        r#"{{ [3, 1].sort().reverse() }}"#,
    ] {
        let once = Template::format(src).expect("format");
        let twice = Template::format(&once).expect("reformat");
        assert_eq!(once, twice, "not idempotent for: {src}");
        assert!(
            Template::compile(&once).is_ok(),
            "formatted does not compile: {once}"
        );
    }
}
