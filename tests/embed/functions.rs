use super::*;

fn body(src: &str, names: &Names) -> ExprUnit {
    ExprUnit::compile(src, names).unwrap_or_else(|e| panic!("{src}: {}", msg(&e)))
}

#[test]
fn host_functions_can_call_each_other() {
    let names = Names::new()
        .var("rate")
        .function("net", 1)
        .function("gross", 1)
        .var("x");
    let funcs = Functions::build([
        ("net", vec!["x"], body("x * (1 - rate)", &names)),
        ("gross", vec!["x"], body("net(x) + 1", &names)),
    ])
    .unwrap();
    let unit = ExprUnit::compile("gross(100)", &names).unwrap();
    let env = Env::with_functions(&funcs).set("rate", Value::Decimal("0.25".parse().unwrap()));
    let out = unit.eval(&Value::Null, &env, &Budget::unlimited()).unwrap();
    assert_eq!(out, Value::Decimal("76.00".parse().unwrap()));
}

#[test]
fn function_bodies_see_globals_not_the_callers_locals() {
    // `x` is a global (1) and also the caller's lambda parameter (10).
    let names = Names::new().var("x").var("y").function("f", 1);
    let funcs = Functions::build([("f", vec!["y"], body("x + y", &names))]).unwrap();
    let unit = ExprUnit::compile("[10].map(x -> f(x))", &names).unwrap();
    let env = Env::with_functions(&funcs).set("x", 1);
    let out = unit.eval(&Value::Null, &env, &Budget::unlimited()).unwrap();
    assert_eq!(out, arr(vec![Value::Int(11)]));
}

#[test]
fn recursion_is_rejected_directly_and_through_others() {
    let names = Names::new().var("n").function("a", 1).function("b", 1);
    let direct = Functions::build([("a", vec!["n"], body("a(n)", &names))]).unwrap_err();
    assert!(msg(&direct).contains("calls itself"), "{}", msg(&direct));
    let indirect = Functions::build([
        ("a", vec!["n"], body("b(n)", &names)),
        ("b", vec!["n"], body("a(n)", &names)),
    ])
    .unwrap_err();
    assert!(msg(&indirect).contains("a -> b -> a") || msg(&indirect).contains("b -> a -> b"));
}

#[test]
fn function_definitions_are_validated() {
    let names = Names::new().var("p").function("f", 1).function("g", 2);
    let cases: Vec<(&str, Vec<&str>, &str, &str)> = vec![
        ("round", vec!["p"], "p", "built-in"),
        ("input", vec!["p"], "p", "reserved"),
        ("f", vec!["input"], "1", "reserved"),
        ("f", vec!["p", "p"], "p", "appears twice"),
    ];
    for (name, params, src, expect) in cases {
        let err = Functions::build([(name, params, body(src, &names))]).unwrap_err();
        assert!(msg(&err).contains(expect), "{name}: {}", msg(&err));
    }
    // calls inside the set must match arity and exist
    let err = Functions::build([
        ("f", vec!["p"], body("g(p, p)", &names)),
        ("g", vec!["p"], body("p", &names)),
    ])
    .unwrap_err();
    assert!(
        msg(&err).contains("`g` takes 1 argument, got 2"),
        "{}",
        msg(&err)
    );
    let err = Functions::build([("f", vec!["p"], body("g(p, p)", &names))]).unwrap_err();
    assert!(msg(&err).contains("unknown function `g`"), "{}", msg(&err));
    let err = Functions::build([
        ("f", vec!["p"], body("p", &names)),
        ("f", vec!["p"], body("p", &names)),
    ])
    .unwrap_err();
    assert!(msg(&err).contains("defined twice"));
}

#[test]
fn very_deep_call_chains_are_rejected() {
    let mut names = Names::new().var("p");
    for i in 0..70 {
        names.add_function(format!("f{i}"), 1);
    }
    let defs: Vec<(String, Vec<&str>, ExprUnit)> = (0..70)
        .map(|i| {
            let src = if i == 69 {
                "p".to_string()
            } else {
                format!("f{}(p)", i + 1)
            };
            (format!("f{i}"), vec!["p"], body(&src, &names))
        })
        .collect();
    let err = Functions::build(defs).unwrap_err();
    assert!(msg(&err).contains("nest deeper than 64"), "{}", msg(&err));
}

#[test]
fn long_function_chains_and_wide_functions_are_refused_cleanly() {
    on_small_stack(|| {
        // h0 calls h1 calls h2 ... 20,000 deep: refused, without recursing
        let n = 20_000;
        let names = (0..n).fold(Names::new().var("x"), |acc, i| {
            acc.function(format!("h{i}"), 1)
        });
        let defs: Vec<_> = (0..n)
            .map(|i| {
                let body = if i + 1 < n {
                    format!("h{}(x)", i + 1)
                } else {
                    "x".into()
                };
                (
                    format!("h{i}"),
                    vec!["x"],
                    ExprUnit::compile(&body, &names).unwrap(),
                )
            })
            .collect();
        let errors = Functions::build(defs).unwrap_err();
        assert!(errors.len() <= 10, "{} errors", errors.len());
        assert!(format!("{:?}", errors[0]).contains("nest deeper than"));

        let params: Vec<String> = (0..100).map(|i| format!("p{i}")).collect();
        let body = ExprUnit::compile("p99", &Names::new().vars(params.clone())).unwrap();
        let errors = Functions::build([("wide", params, body)]).unwrap_err();
        assert!(format!("{:?}", errors[0]).contains("most a function may take"));

        // 64 parameters work, and the last one wins lookups
        let params: Vec<String> = (0..64).map(|i| format!("p{i}")).collect();
        let body = ExprUnit::compile("p0 + p63", &Names::new().vars(params.clone())).unwrap();
        let funcs = Functions::build([("wide", params, body)]).unwrap();
        let args: Vec<String> = (0..64).map(|i| i.to_string()).collect();
        let call = format!("wide({})", args.join(", "));
        let unit = ExprUnit::compile(&call, &Names::new().function("wide", 64)).unwrap();
        let env = Env::with_functions(&funcs);
        assert_eq!(
            unit.eval(&Value::Null, &env, &Budget::unlimited()).unwrap(),
            Value::Int(63)
        );
    });
}
