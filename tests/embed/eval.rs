use super::*;

#[test]
fn units_share_one_env() {
    let names = Names::new().vars(["limit", "order"]);
    let order = obj(&[("total", Value::Int(120))]);
    let env = Env::new().set("limit", 100).set("order", order);
    let budget = Budget::new(10_000);
    let over = ExprUnit::compile("order.total > limit", &names).unwrap();
    let diff = ExprUnit::compile("order.total - limit", &names).unwrap();
    assert_eq!(
        over.eval(&Value::Null, &env, &budget).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        diff.eval(&Value::Null, &env, &budget).unwrap(),
        Value::Int(20)
    );
    assert!(budget.used() > 0);
}

#[test]
fn unbound_host_names_fail_at_evaluation() {
    let names = Names::new().var("x");
    let unit = ExprUnit::compile("x + 1", &names).unwrap();
    let err = unit
        .eval(&Value::Null, &Env::new(), &Budget::unlimited())
        .unwrap_err();
    assert!(matches!(err, EvalError::Render(_)), "{err:?}");
}

#[test]
fn template_units_render_with_bindings() {
    let names = Names::new().vars(["amount"]);
    let t = TemplateUnit::compile(
        "{ \"status\": \"INPROGRESS\", \"do\": [ { \"type\": \"refund\", \"amount\": {{ amount }} } ] }",
        &names,
    )
    .unwrap();
    let out = t
        .render(
            &Value::Null,
            &Env::new().set("amount", 49),
            &Budget::unlimited(),
        )
        .unwrap();
    let expected = obj(&[
        ("status", Value::Str("INPROGRESS".into())),
        (
            "do",
            arr(vec![obj(&[
                ("type", Value::Str("refund".into())),
                ("amount", Value::Int(49)),
            ])]),
        ),
    ]);
    assert_eq!(out, expected);
}

#[test]
fn version_constants_are_public() {
    assert_eq!(temple_dsl::BLOB_VERSION, 1);
    assert!(!temple_dsl::VERSION.is_empty());
}

#[test]
fn lambda_scoping_is_unchanged() {
    let names = Names::new().var("x");
    let env = Env::new().set("x", 100);
    let cases = [
        (
            "[1, 2].map(x -> x + 1)",
            arr(vec![Value::Int(2), Value::Int(3)]),
        ),
        (
            "[1, 2].map(y -> x + y)",
            arr(vec![Value::Int(101), Value::Int(102)]),
        ),
        (
            "[[1, 2], [3]].map(r -> r.map(x -> x * 10))",
            arr(vec![
                arr(vec![Value::Int(10), Value::Int(20)]),
                arr(vec![Value::Int(30)]),
            ]),
        ),
        ("[1, 2, 3].fold(0, (acc, x) -> acc + x)", Value::Int(6)),
        (
            "[1, 2].map(y -> let x = y * 2 in x)",
            arr(vec![Value::Int(2), Value::Int(4)]),
        ),
        (
            "[3, 1, 2].sort_by(x -> -x)",
            arr(vec![Value::Int(3), Value::Int(2), Value::Int(1)]),
        ),
        ("x", Value::Int(100)),
    ];
    for (src, expected) in cases {
        let u = ExprUnit::compile(src, &names).unwrap();
        assert_eq!(
            u.eval(&Value::Null, &env, &Budget::unlimited()).unwrap(),
            expected,
            "{src}"
        );
    }
}
