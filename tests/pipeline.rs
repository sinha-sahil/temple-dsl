//! End-to-end pipeline — a realistic template exercising the full feature set,
//! rendered into a typed Rust struct, round-tripped through the compiled blob,
//! and stressed for the no-panic guarantee on the new surface.

use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::BTreeMap;
use temple_dsl::{Template, Value};

const SUMMARY_SRC: &str = r#"
let items = input.items
{
  "customer":   {{ concat(upper(input.first), " ", input.last) }},
  "item_count": {{ items.len() }},
  "skus":       {{ items.map(it -> it.sku).sort() }},
  "subtotal":   {{ items.map(it -> it.qty * it.price).sum() }},
  "expensive":  {{ items.filter(it -> it.price > 10).map(it -> it.sku) }},
  "by_cat":     {{ items.fold({}, (acc, it) -> acc.merge({ [it.cat]: (acc.get(it.cat) ?? 0) + it.qty })) }},
  "tier":       {{ let s = this.subtotal in when { s >= 100: "gold", s >= 50: "silver", else: "bronze" } }},
  "even_items": {{ items.len() % 2 == 0 }},
  "first_cat":  {{ items[0]["cat"] }},
  "coupon"?:    {{ input.coupon }},
  "meta":       {{ json_encode({ "v": 1, "n": items.len() }) }}
}
"#;

#[derive(Deserialize, PartialEq, Debug)]
struct Summary {
    customer: String,
    item_count: i64,
    skus: Vec<String>,
    subtotal: Decimal,
    expensive: Vec<String>,
    by_cat: BTreeMap<String, i64>,
    tier: String,
    even_items: bool,
    first_cat: String,
    coupon: Option<String>,
    meta: String,
}

fn sample_input(coupon: Value) -> Value {
    let items = Value::Arr(vec![
        item("A", "x", 2, "5.00"),
        item("B", "y", 1, "20.00"),
        item("C", "x", 3, "15.00"),
    ]);
    Value::obj([
        ("first", Value::from("ada")),
        ("last", Value::from("lovelace")),
        ("items", items),
        ("coupon", coupon),
    ])
}

fn item(sku: &str, cat: &str, qty: i64, price: &str) -> Value {
    Value::obj([
        ("sku", Value::from(sku)),
        ("cat", Value::from(cat)),
        ("qty", Value::Int(qty)),
        ("price", Value::Decimal(price.parse().unwrap())),
    ])
}

fn expected(coupon: Option<&str>) -> Summary {
    Summary {
        customer: "ADA lovelace".into(),
        item_count: 3,
        skus: vec!["A".into(), "B".into(), "C".into()],
        subtotal: "75.00".parse().unwrap(),
        expensive: vec!["B".into(), "C".into()],
        by_cat: BTreeMap::from([("x".to_string(), 5), ("y".to_string(), 1)]),
        tier: "silver".into(),
        even_items: false,
        first_cat: "x".into(),
        coupon: coupon.map(String::from),
        meta: "{\"v\":1,\"n\":3}".into(),
    }
}

#[test]
fn typed_render_with_all_features() {
    let t = Template::compile(SUMMARY_SRC).expect("compile");

    // coupon omitted (null) → the `coupon` key is absent → Option::None
    let out: Summary = t.render(sample_input(Value::Null)).expect("render");
    assert_eq!(out, expected(None));

    // coupon present → key kept
    let out2: Summary = t
        .render(sample_input(Value::from("SAVE10")))
        .expect("render");
    assert_eq!(out2, expected(Some("SAVE10")));
}

#[test]
fn omitted_key_truly_absent_from_value() {
    let t = Template::compile(SUMMARY_SRC).expect("compile");
    let v = t.render_value(sample_input(Value::Null)).expect("render");
    if let Value::Obj(o) = &v {
        assert!(
            !o.contains_key("coupon"),
            "null optional key must be absent"
        );
    } else {
        panic!("expected object");
    }
}

#[test]
fn compile_once_render_many_is_stable() {
    let t = Template::compile(SUMMARY_SRC).expect("compile");
    let first: Summary = t.render(sample_input(Value::Null)).unwrap();
    for _ in 0..500 {
        let again: Summary = t.render(sample_input(Value::Null)).unwrap();
        assert_eq!(first, again);
    }
}

#[test]
fn blob_round_trips_the_full_template() {
    let t = Template::compile(SUMMARY_SRC).expect("compile");
    let loaded = Template::from_bytes(&t.to_bytes()).expect("reload");
    let a: Summary = t.render(sample_input(Value::from("X"))).unwrap();
    let b: Summary = loaded.render(sample_input(Value::from("X"))).unwrap();
    assert_eq!(a, b);
}

/// Every new AST node must survive the v2 blob (compile → bytes → reload → render
/// equals a direct render).
#[test]
fn blob_round_trips_each_new_feature() {
    let cases: &[(&str, Value)] = &[
        ("{{ input.n % 3 }}", Value::obj([("n", Value::Int(7))])),
        (
            r#"{ "k"?: {{ input.c }} }"#,
            Value::obj([("c", Value::Null)]),
        ),
        (
            r#"{{ { [input.k]: input.v } }}"#,
            Value::obj([("k", Value::from("z")), ("v", Value::Int(1))]),
        ),
        (
            "{{ let x = input.n in x * x }}",
            Value::obj([("n", Value::Int(4))]),
        ),
        (r#"{{ [3, 1, 2].sort().reverse() }}"#, Value::Null),
        (
            r#"{{ input.o["a"] }}"#,
            Value::obj([("o", Value::obj([("a", Value::Int(9))]))]),
        ),
        (
            r#"{{ concat("x=", input.n) }}"#,
            Value::obj([("n", Value::Int(5))]),
        ),
    ];
    for (src, input) in cases {
        let t = Template::compile(src).unwrap_or_else(|e| panic!("compile {src}: {e:?}"));
        let direct = t.render_value(input.clone()).unwrap();
        let reloaded = Template::from_bytes(&t.to_bytes())
            .expect("reload")
            .render_value(input.clone())
            .unwrap();
        assert_eq!(direct, reloaded, "blob round-trip differs for: {src}");
    }
}

/// The no-panic guarantee on the new surface: each of these must return `Err`,
/// never panic. (A panic here unwinds and fails the test.)
#[test]
fn new_surface_errors_never_panic() {
    let n = Value::obj([("n", Value::Int(5))]);
    // an int next to a nested array — not all-comparable, not all-scalar
    let arr = Value::obj([(
        "a",
        Value::Arr(vec![Value::Int(1), Value::Arr(vec![Value::Int(2)])]),
    )]);
    let obj = Value::obj([("o", Value::obj([("a", Value::Int(1))]))]);
    let cases: &[(&str, Value)] = &[
        ("{{ 5 % 0 }}", Value::Null),                  // modulo by zero
        ("{{ to_number(\"nope\") }}", Value::Null),    // unparseable
        ("{{ concat(\"x\", input.a) }}", arr.clone()), // concat a collection
        (r#"{{ { [input.n]: 1 } }}"#, n.clone()),      // non-string computed key
        (r#"{{ input.o["missing"] }}"#, obj.clone()),  // strict index miss
        ("{{ input.a.sort() }}", arr.clone()),         // sort mixed types
        (
            "{{ input.a.min() }}",
            Value::obj([("a", Value::Arr(vec![]))]),
        ), // min of empty
        ("{{ input.a.flatten() }}", arr.clone()),      // flatten non-arrays
        ("{{ input.a.join(\"-\") }}", arr.clone()),    // join a non-scalar elem
        ("{{ \"x\".contains(5) }}", Value::Null),      // non-string arg
        ("{{ input.a.slice(\"x\", 2) }}", arr),        // non-int slice bound
        ("{{ input.o.merge([1]) }}", obj),             // merge a non-object
        ("{{ input.n.keys() }}", n),                   // method on wrong type
    ];
    for (src, input) in cases {
        let r = Template::compile(src)
            .expect("these compile (errors are at render)")
            .render_value(input.clone());
        assert!(r.is_err(), "expected a render error for: {src}");
    }
}

#[test]
fn json_encode_handles_nested_and_escapes() {
    let input = Value::obj([(
        "o",
        Value::obj([
            ("s", Value::from("a\"b")),
            (
                "arr",
                Value::Arr(vec![Value::Int(1), Value::Null, Value::Bool(true)]),
            ),
        ]),
    )]);
    let v = Template::compile("{{ json_encode(input.o) }}")
        .unwrap()
        .render_value(input)
        .unwrap();
    assert_eq!(
        v,
        Value::Str("{\"s\":\"a\\\"b\",\"arr\":[1,null,true]}".into())
    );
}

/// One template exercising EVERY feature in the README support list — every
/// operator, every structure (let / let-in / this / when / ternary / safe
/// access / interp / computed & optional keys / indexing / postfix chains),
/// every builtin, and every string/array/object method. Asserted exactly, then
/// pushed through the blob and format round-trips.
const KITCHEN_SINK_SRC: &str = r#"
# the whole language in one template
let items = input.cart.items
let total = items.map(it -> it.qty * it.price).sum()

{
  "ops_arith":   {{ 1 + 2 * 3 - 7 % 4 }},
  "ops_div":     {{ 10 / 4 }},
  "cmp":         {{ 1 < 2 && 2 <= 2 && 3 > 2 && 3 >= 3 && 1 == 1 && 1 != 2 }},
  "logic":       {{ !(false || false) && true }},
  "neg_not":     {{ -(2 + 3) }},
  "ternary":     {{ input.user.age >= 18 ? 'adult' : 'minor' }},
  "when_tier":   {{ when {
    total >= 100: 'gold'
    total >= 30:  'silver'
    else:         'basic'
  } }},
  "letin":       {{ let half = total / 2 in half * 2 }},
  "safe1":       {{ input.cart.coupon ?? 'none' }},
  "safe2":       {{ input.cart?.missing ?? 'dflt' }},
  "idx_arr":     {{ items[0].sku }},
  "idx_obj":     {{ input.meta["env"] }},
  "this_ref":    {{ this.idx_arr }},
  "interp":      "Hi {{ upper(input.user.first) }}, total {{ total }}!",
  "omitted"?:    {{ input.cart.coupon }},
  "kept"?:       {{ input.meta.env }},
  "computed":    {{ { [concat('k_', input.meta.env)]: 1, "static": 2 } }},
  "nested":      { "deep": [ {{ 1 }}, {{ 'two' }} ] },
  "skus_sorted": {{ items.map(it -> it.sku).sort() }}
  "sku_rev":     {{ items.map(it -> it.sku).sort().reverse() }}
  "by_price":    {{ items.sort_by(it -> it.price).first().sku }},
  "filtered":    {{ items.filter(it -> it.qty >= 2).len() }},
  "folded":      {{ items.fold(0, (acc, it) -> acc + it.qty) }},
  "sum_qty":     {{ items.map(it -> it.qty).sum() }},
  "any_all":     {{ items.any(it -> it.qty > 3) && items.all(it -> it.price > 1) }},
  "count":       {{ items.count(it -> it.cat == 'x') }},
  "found":       {{ items.find(it -> it.cat == 'y').sku }},
  "uniq":        {{ input.user.tags.unique() }},
  "flat":        {{ [[1, 2], [3]].flatten() }},
  "fmapped":     {{ [1, 2].flat_map(x -> [x, x * 10]) }},
  "window":      {{ [1, 2, 3, 4, 5].take(4).drop(1).slice(0, 2) }},
  "joined":      {{ input.user.tags.join('-') }},
  "agg":         {{ [3, 1, 2].min() + [3, 1, 2].max() }},
  "avg":         {{ [2, 4].avg() }},
  "firstlast":   {{ concat(items.first().sku, '/', items.last().sku) }},
  "arrconcat":   {{ [1].concat([2]).len() }},
  "arr_has":     {{ items.map(it -> it.sku).contains('A2') }},
  "arr_pos":     {{ [5, 6].index_of(6) }},
  "str_ops":     {{ input.raw.slice(2, 7) }},
  "str_pred":    {{ 'Hello'.starts_with('He') && 'Hello'.ends_with('lo') && 'Hello'.contains('ell') }},
  "str_repl":    {{ 'a,b'.replace(',', ';').split(';').len() }},
  "str_idx":     {{ 'abc'.index_of('c') }},
  "trimmed":     {{ trim(input.raw) }},
  "cases":       {{ concat(upper('a'), lower('B')) }},
  "nums":        {{ abs(-5) + round(2.4) + floor(2.9) + ceil(2.1) }},
  "minmax_fn":   {{ min(3, 1, 2) + max(3, 1, 2) }},
  "convs":       {{ to_number('42') + 'abc'.len() + [1].len() }},
  "tostr":       {{ to_string(7.5) }},
  "types":       "{{ type_of(1) }}/{{ is_null(null) }}/{{ is_bool(true) }}/{{ is_number(1) }}/{{ is_string('s') }}/{{ is_array([]) }}/{{ is_object(input.meta) }}",
  "encoded":     {{ concat(url_encode('a b'), '|', base64('hi')) }},
  "json":        {{ json_encode({ "n": 1 }) }},
  "obj_keys":    {{ input.meta.keys().join(',') }},
  "obj_vals":    {{ input.meta.values().first() }},
  "obj_ent":     {{ input.meta.entries().first().key }},
  "obj_has":     {{ input.meta.has('env') && !input.meta.has('nope') }},
  "obj_get":     {{ input.meta.get('nope') ?? 'fallback' }},
  "obj_merge":   {{ input.meta.merge({ "x": 1 }).keys().len() }},
  "paren_chain": {{ ([2, 1].concat([3])).sort().join('') }}
}
"#;

fn kitchen_sink_input() -> Value {
    let item = |sku: &str, qty: i64, price: &str, cat: &str| {
        Value::obj([
            ("sku", Value::from(sku)),
            ("qty", Value::Int(qty)),
            ("price", Value::Decimal(price.parse().unwrap())),
            ("cat", Value::from(cat)),
        ])
    };
    Value::obj([
        (
            "user",
            Value::obj([
                ("first", Value::from("ada")),
                ("age", Value::Int(36)),
                ("tags", Value::Arr(vec!["a".into(), "b".into(), "a".into()])),
            ]),
        ),
        (
            "cart",
            Value::obj([
                (
                    "items",
                    Value::Arr(vec![
                        item("B1", 2, "10.50", "x"),
                        item("A2", 1, "5.25", "y"),
                        item("C3", 4, "2.00", "x"),
                    ]),
                ),
                ("coupon", Value::Null),
            ]),
        ),
        ("meta", Value::obj([("env", Value::from("prod"))])),
        ("raw", Value::from("  Hello World  ")),
    ])
}

fn kitchen_sink_expected() -> Value {
    let dec = |s: &str| Value::Decimal(s.parse::<Decimal>().unwrap());
    let strs = |xs: &[&str]| Value::Arr(xs.iter().map(|s| Value::from(*s)).collect());
    let ints = |xs: &[i64]| Value::Arr(xs.iter().map(|n| Value::Int(*n)).collect());
    Value::obj([
        ("ops_arith", Value::Int(4)),
        ("ops_div", dec("2.5")),
        ("cmp", Value::Bool(true)),
        ("logic", Value::Bool(true)),
        ("neg_not", Value::Int(-5)),
        ("ternary", Value::from("adult")),
        ("when_tier", Value::from("silver")),
        ("letin", dec("34.25")),
        ("safe1", Value::from("none")),
        ("safe2", Value::from("dflt")),
        ("idx_arr", Value::from("B1")),
        ("idx_obj", Value::from("prod")),
        ("this_ref", Value::from("B1")),
        ("interp", Value::from("Hi ADA, total 34.25!")),
        // "omitted" is absent — checked below
        ("kept", Value::from("prod")),
        (
            "computed",
            Value::obj([("k_prod", Value::Int(1)), ("static", Value::Int(2))]),
        ),
        (
            "nested",
            Value::obj([("deep", Value::Arr(vec![Value::Int(1), Value::from("two")]))]),
        ),
        ("skus_sorted", strs(&["A2", "B1", "C3"])),
        ("sku_rev", strs(&["C3", "B1", "A2"])),
        ("by_price", Value::from("C3")),
        ("filtered", Value::Int(2)),
        ("folded", Value::Int(7)),
        ("sum_qty", Value::Int(7)),
        ("any_all", Value::Bool(true)),
        ("count", Value::Int(2)),
        ("found", Value::from("A2")),
        ("uniq", strs(&["a", "b"])),
        ("flat", ints(&[1, 2, 3])),
        ("fmapped", ints(&[1, 10, 2, 20])),
        ("window", ints(&[2, 3])),
        ("joined", Value::from("a-b-a")),
        ("agg", Value::Int(4)),
        ("avg", dec("3")),
        ("firstlast", Value::from("B1/C3")),
        ("arrconcat", Value::Int(2)),
        ("arr_has", Value::Bool(true)),
        ("arr_pos", Value::Int(1)),
        ("str_ops", Value::from("Hello")),
        ("str_pred", Value::Bool(true)),
        ("str_repl", Value::Int(2)),
        ("str_idx", Value::Int(2)),
        ("trimmed", Value::from("Hello World")),
        ("cases", Value::from("Ab")),
        ("nums", dec("12")),
        ("minmax_fn", Value::Int(4)),
        ("convs", Value::Int(46)),
        ("tostr", Value::from("7.5")),
        ("types", Value::from("int/true/true/true/true/true/true")),
        ("encoded", Value::from("a%20b|aGk=")),
        ("json", Value::from("{\"n\":1}")),
        ("obj_keys", Value::from("env")),
        ("obj_vals", Value::from("prod")),
        ("obj_ent", Value::from("env")),
        ("obj_has", Value::Bool(true)),
        ("obj_get", Value::from("fallback")),
        ("obj_merge", Value::Int(2)),
        ("paren_chain", Value::from("123")),
    ])
}

#[test]
fn kitchen_sink_uses_every_supported_feature() {
    let t = Template::compile(KITCHEN_SINK_SRC).expect("compile");
    let out = t.render_value(kitchen_sink_input()).expect("render");
    if let Value::Obj(o) = &out {
        assert!(
            !o.contains_key("omitted"),
            "null optional key must be absent"
        );
    }
    assert_eq!(out, kitchen_sink_expected());
}

#[test]
fn kitchen_sink_survives_the_blob() {
    let t = Template::compile(KITCHEN_SINK_SRC).expect("compile");
    let loaded = Template::from_bytes(&t.to_bytes()).expect("reload");
    assert_eq!(
        loaded.render_value(kitchen_sink_input()).expect("render"),
        kitchen_sink_expected()
    );
}

#[test]
fn kitchen_sink_survives_the_formatter() {
    let once = Template::format(KITCHEN_SINK_SRC).expect("format");
    assert_eq!(
        once,
        Template::format(&once).expect("reformat"),
        "idempotent"
    );
    let t = Template::compile(&once).expect("formatted source compiles");
    assert_eq!(
        t.render_value(kitchen_sink_input()).expect("render"),
        kitchen_sink_expected()
    );
}
