//! Collection methods (`map`/`filter`/`fold`/`first`/`last`/`concat`/…),
//! lambdas, and `arr[i]` indexing.

use rust_decimal::Decimal;
use temple_dsl::{RenderError, Template, Value};

fn render<T: serde::de::DeserializeOwned>(src: &str, input: Value) -> T {
    let template = Template::compile(src).expect("compile");
    template.render::<T>(input).expect("render")
}

fn arr(xs: &[i64]) -> Value {
    Value::Arr(xs.iter().copied().map(Value::Int).collect())
}

#[test]
fn array_length() {
    let out: i64 = render(
        "{{ input.xs.len() }}",
        Value::obj([("xs", arr(&[1, 2, 3, 4]))]),
    );
    assert_eq!(out, 4);
}

#[test]
fn string_length() {
    let out: i64 = render(
        r#"{{ input.name.len() }}"#,
        Value::obj([("name", Value::Str("hello".into()))]),
    );
    assert_eq!(out, 5);
}

#[test]
fn array_first_and_last() {
    let input = Value::obj([("xs", arr(&[10, 20, 30]))]);
    let first: i64 = render("{{ input.xs.first() }}", input.clone());
    let last: i64 = render("{{ input.xs.last() }}", input);
    assert_eq!(first, 10);
    assert_eq!(last, 30);
}

#[test]
fn array_first_on_empty_is_null() {
    let out: Option<i64> = render(
        "{{ input.xs.first() }}",
        Value::obj([("xs", Value::Arr(vec![]))]),
    );
    assert!(out.is_none());
}

#[test]
fn array_indexing() {
    let input = Value::obj([("xs", arr(&[10, 20, 30]))]);
    let zero: i64 = render("{{ input.xs[0] }}", input.clone());
    let two: i64 = render("{{ input.xs[2] }}", input);
    assert_eq!(zero, 10);
    assert_eq!(two, 30);
}

#[test]
fn array_index_out_of_bounds_errors() {
    let template = Template::compile("{{ input.xs[5] }}").expect("compile");
    let result = template.render::<i64>(Value::obj([("xs", arr(&[1, 2, 3]))]));
    assert!(matches!(result, Err(RenderError::IndexOutOfBounds { .. })));
}

#[test]
fn array_negative_index_errors() {
    let template = Template::compile("{{ input.xs[-1] }}").expect("compile");
    let result = template.render::<i64>(Value::obj([("xs", arr(&[1, 2, 3]))]));
    assert!(matches!(result, Err(RenderError::IndexOutOfBounds { .. })));
}

#[test]
fn array_index_expression() {
    let input = Value::obj([("xs", arr(&[10, 20, 30, 40])), ("i", Value::Int(2))]);
    let out: i64 = render("{{ input.xs[input.i] }}", input);
    assert_eq!(out, 30);
}

#[test]
fn array_concat() {
    let input = Value::obj([("a", arr(&[1, 2])), ("b", arr(&[3, 4, 5]))]);
    let out: Vec<i64> = render("{{ input.a.concat(input.b) }}", input);
    assert_eq!(out, vec![1, 2, 3, 4, 5]);
}

#[test]
fn array_map_simple() {
    let input = Value::obj([("xs", arr(&[1, 2, 3]))]);
    let out: Vec<i64> = render("{{ input.xs.map(x -> x * 2) }}", input);
    assert_eq!(out, vec![2, 4, 6]);
}

#[test]
fn array_map_with_parens_param() {
    let input = Value::obj([("xs", arr(&[1, 2, 3]))]);
    let out: Vec<i64> = render("{{ input.xs.map((x) -> x + 10) }}", input);
    assert_eq!(out, vec![11, 12, 13]);
}

#[test]
fn array_filter() {
    let input = Value::obj([("xs", arr(&[1, 2, 3, 4, 5]))]);
    let out: Vec<i64> = render("{{ input.xs.filter(x -> x > 2) }}", input);
    assert_eq!(out, vec![3, 4, 5]);
}

#[test]
fn array_fold_sum() {
    let input = Value::obj([("xs", arr(&[1, 2, 3, 4]))]);
    let out: i64 = render("{{ input.xs.fold(0, (acc, x) -> acc + x) }}", input);
    assert_eq!(out, 10);
}

#[test]
fn array_fold_decimal() {
    let input = Value::obj([(
        "prices",
        Value::Arr(vec![
            Value::Decimal("9.99".parse().unwrap()),
            Value::Decimal("4.50".parse().unwrap()),
            Value::Decimal("0.01".parse().unwrap()),
        ]),
    )]);
    let out: Decimal = render("{{ input.prices.fold(0, (acc, p) -> acc + p) }}", input);
    assert_eq!(out, "14.50".parse::<Decimal>().unwrap());
}

#[test]
fn array_map_filter_chain() {
    let input = Value::obj([("xs", arr(&[1, 2, 3, 4, 5]))]);
    let out: Vec<i64> = render("{{ input.xs.filter(x -> x > 1).map(x -> x * 10) }}", input);
    assert_eq!(out, vec![20, 30, 40, 50]);
}

#[test]
fn lambda_captures_let() {
    let src = r#"
        let factor = input.factor
        { "scaled": {{ input.xs.map(x -> x * factor) }} }
    "#;
    let input = Value::obj([("xs", arr(&[1, 2, 3])), ("factor", Value::Int(3))]);
    #[derive(serde::Deserialize)]
    struct Out {
        scaled: Vec<i64>,
    }
    let out: Out = render(src, input);
    assert_eq!(out.scaled, vec![3, 6, 9]);
}

#[test]
fn lambda_can_reference_this() {
    let src = r#"{
        "factor": {{ input.factor }},
        "scaled": {{ input.xs.map(x -> x * this.factor) }}
    }"#;
    let input = Value::obj([("xs", arr(&[1, 2, 3])), ("factor", Value::Int(10))]);
    #[derive(serde::Deserialize)]
    struct Out {
        factor: i64,
        scaled: Vec<i64>,
    }
    let out: Out = render(src, input);
    assert_eq!(out.factor, 10);
    assert_eq!(out.scaled, vec![10, 20, 30]);
}

#[test]
fn map_over_objects() {
    let input = Value::obj([(
        "users",
        Value::Arr(vec![
            Value::obj([("name", Value::Str("ada".into())), ("age", Value::Int(36))]),
            Value::obj([("name", Value::Str("bob".into())), ("age", Value::Int(50))]),
        ]),
    )]);
    let out: Vec<String> = render("{{ input.users.map(u -> u.name) }}", input);
    assert_eq!(out, vec!["ada", "bob"]);
}

#[test]
fn nested_lambdas() {
    let input = Value::obj([("xs", arr(&[1, 2, 3])), ("ys", arr(&[10, 20]))]);
    let out: Vec<Vec<i64>> = render("{{ input.xs.map(x -> input.ys.map(y -> x + y)) }}", input);
    assert_eq!(out, vec![vec![11, 21], vec![12, 22], vec![13, 23]]);
}

#[test]
fn method_arity_mismatch_errors() {
    let template = Template::compile("{{ input.xs.first(1) }}").expect("compile");
    let result = template.render::<i64>(Value::obj([("xs", arr(&[1]))]));
    assert!(matches!(result, Err(RenderError::ArityMismatch { .. })));
}

#[test]
fn unknown_method_errors() {
    let template = Template::compile("{{ input.xs.nope() }}").expect("compile");
    let result = template.render::<i64>(Value::obj([("xs", arr(&[1]))]));
    assert!(matches!(result, Err(RenderError::UnknownMethod { .. })));
}

#[test]
fn method_on_wrong_type_errors() {
    let template = Template::compile("{{ input.n.map(x -> x) }}").expect("compile");
    let result = template.render::<Vec<i64>>(Value::obj([("n", Value::Int(5))]));
    assert!(matches!(result, Err(RenderError::UnknownMethod { .. })));
}

#[test]
fn indexing_non_array_errors() {
    let template = Template::compile("{{ input.n[0] }}").expect("compile");
    let result = template.render::<i64>(Value::obj([("n", Value::Int(5))]));
    assert!(matches!(result, Err(RenderError::NotIndexable { .. })));
}

#[test]
fn top_level_lambda_rejected() {
    let template = Template::compile("{{ x -> x + 1 }}");
    if let Ok(t) = template {
        let result = t.render::<i64>(Value::Null);
        assert!(result.is_err());
    }
}

#[test]
fn lambda_param_reserved_name_errors() {
    let err = Template::compile("{{ input.xs.map(input -> input) }}")
        .expect_err("expected reserved-name error");
    assert!(format!("{err:?}").contains("reserved"));
}

#[test]
fn array_literal_in_method_arg() {
    let input = Value::obj([("xs", arr(&[1, 2]))]);
    let out: Vec<i64> = render("{{ input.xs.concat([99, 100]) }}", input);
    assert_eq!(out, vec![1, 2, 99, 100]);
}

#[test]
fn fold_appends_via_array_literal_arg() {
    let input = Value::obj([("xs", arr(&[1, 2, 3])), ("empty", Value::Arr(vec![]))]);
    let out: Vec<i64> = render(
        "{{ input.xs.fold(input.empty, (acc, x) -> acc.concat([x * 10])) }}",
        input,
    );
    assert_eq!(out, vec![10, 20, 30]);
}

#[test]
fn complex_output_with_methods_and_this() {
    let src = r#"
        let prices = input.line_items.map(item -> item.price)
        {
            "count":    {{ input.line_items.len() }},
            "total":    {{ prices.fold(0, (acc, p) -> acc + p) }},
            "doubled":  {{ this.total * 2 }},
            "first":    {{ input.line_items.first().name }}
        }
    "#;
    let input = Value::obj([(
        "line_items",
        Value::Arr(vec![
            Value::obj([
                ("name", Value::Str("widget".into())),
                ("price", Value::Decimal("9.99".parse().unwrap())),
            ]),
            Value::obj([
                ("name", Value::Str("gadget".into())),
                ("price", Value::Decimal("4.50".parse().unwrap())),
            ]),
        ]),
    )]);
    #[derive(serde::Deserialize)]
    struct Out {
        count: i64,
        total: Decimal,
        doubled: Decimal,
        first: String,
    }
    let out: Out = render(src, input);
    assert_eq!(out.count, 2);
    assert_eq!(out.total, "14.49".parse::<Decimal>().unwrap());
    assert_eq!(out.doubled, "28.98".parse::<Decimal>().unwrap());
    assert_eq!(out.first, "widget");
}
