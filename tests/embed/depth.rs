use super::*;

fn chain(n: usize) -> String {
    format!("1{}", " + 1".repeat(n))
}

#[test]
fn deep_units_are_refused_not_a_crash() {
    on_small_stack(|| {
        let deep = [
            chain(200_000),
            format!("null{}", " ?? null".repeat(200_000)),
            format!("{}1", "true ? 1 : ".repeat(200_000)),
            format!("true{}", " && true".repeat(200_000)),
            format!("{} +", chain(200_000)),
            format!("[{}]", chain(200_000)),
        ];
        for src in &deep {
            let errors = ExprUnit::parse_at(src, 0).unwrap_err();
            assert!(
                errors.iter().any(|e| matches!(
                    e,
                    CompileError::TooDeep { .. } | CompileError::Syntax { .. }
                )),
                "{:?}",
                errors[0]
            );
        }
        let template = format!("{{ \"a\": {{{{ {} }}}} }}", chain(200_000));
        assert!(TemplateUnit::parse_at(&template, 0).is_err());
    });
}

#[test]
fn the_deepest_accepted_unit_runs_every_pass_on_a_small_stack() {
    on_small_stack(|| {
        let mut n = 1;
        while ExprUnit::parse_at(&chain(n + 1), 0).is_ok() {
            n += 1;
        }
        assert!(n >= 90, "only {n} operators in a row are accepted");
        let unit = ExprUnit::compile(&chain(n), &Names::new()).unwrap();
        let _ = unit.reads();
        let copy = unit.clone();
        let v = copy.eval(&Value::Null, &Env::new(), &Budget::unlimited());
        assert_eq!(v.unwrap(), Value::Int(n as i64 + 1));
        assert!(matches!(
            ExprUnit::parse_at(&chain(n + 1), 0).unwrap_err()[0],
            CompileError::TooDeep { .. }
        ));
    });
}

#[test]
fn nested_lambdas_at_the_limit_evaluate_on_a_small_stack() {
    on_small_stack(|| {
        // xs.map(a0 -> a0.map(a1 -> ... ak.len()))
        let mut deepest = None;
        for k in 1..64 {
            let mut body = format!("a{}.len()", k - 1);
            for i in (0..k).rev() {
                let recv = if i == 0 {
                    "input.xs".to_string()
                } else {
                    format!("a{}", i - 1)
                };
                body = format!("{recv}.map(a{i} -> {body})");
            }
            match ExprUnit::compile(&body, &Names::new()) {
                Ok(u) => deepest = Some((k, u)),
                Err(_) => break,
            }
        }
        let (k, unit) = deepest.unwrap();
        let mut data = Value::Arr(vec![Value::Int(1)]);
        for _ in 0..=k {
            data = Value::Arr(vec![data]);
        }
        let input = Value::obj([("xs", data)]);
        let _ = unit.eval(&input, &Env::new(), &Budget::unlimited());
    });
}

#[test]
fn helpers_cannot_nest_evaluation_past_the_limit() {
    on_small_stack(|| {
        // f0 calls f1 at the bottom of a long chain, f1 calls f2, and so on.
        let n = 60;
        let names = (0..n).fold(Names::new().var("x"), |acc, i| {
            acc.function(format!("f{i}"), 1)
        });
        let defs = (0..n).map(|i| {
            let call = if i + 1 < n {
                format!("f{}(x)", i + 1)
            } else {
                "x".to_string()
            };
            let body = format!("{call}{}", " + 1".repeat(80));
            let unit = ExprUnit::compile(&body, &names.clone()).unwrap();
            (format!("f{i}"), vec!["x"], unit)
        });
        let funcs = Functions::build(defs.collect::<Vec<_>>()).unwrap();
        let env = Env::with_functions(&funcs).set("x", 1);
        let unit = ExprUnit::compile("f0(x)", &names).unwrap();
        match unit.eval(&Value::Null, &env, &Budget::unlimited()) {
            Err(EvalError::TooDeep { limit, .. }) => assert!(limit >= 100),
            other => panic!("{other:?}"),
        }
        // one helper that deep still works
        let short = ExprUnit::compile("f59(x)", &names).unwrap();
        assert_eq!(
            short
                .eval(&Value::Null, &env, &Budget::unlimited())
                .unwrap(),
            Value::Int(81)
        );
    });
}

#[test]
fn helpers_full_of_nested_lambdas_stop_cleanly_on_a_small_stack() {
    on_small_stack(|| {
        // the deepest lambda nesting a unit accepts, over the helper's argument
        let nested = |k: usize, tail: &str| {
            let mut body = format!("{tail} + a{}.len()", k - 1);
            for i in (0..k).rev() {
                let recv = if i == 0 {
                    "x".to_string()
                } else {
                    format!("a{}", i - 1)
                };
                body = format!("{recv}.map(a{i} -> {body})");
            }
            format!("{body}.len()")
        };
        let n = 60;
        let names = (0..n).fold(Names::new().var("x"), |acc, i| {
            acc.function(format!("f{i}"), 1)
        });
        let mut k = 1;
        while ExprUnit::compile(&nested(k + 1, "f1(x)"), &names).is_ok() {
            k += 1;
        }
        let defs: Vec<_> = (0..n)
            .map(|i| {
                let tail = if i + 1 < n {
                    format!("f{}(x)", i + 1)
                } else {
                    "0".into()
                };
                let unit = ExprUnit::compile(&nested(k, &tail), &names).unwrap();
                (format!("f{i}"), vec!["x"], unit)
            })
            .collect();
        let funcs = Functions::build(defs).unwrap();
        let mut data = Value::Arr(vec![Value::Int(1)]);
        for _ in 0..=k {
            data = Value::Arr(vec![data]);
        }
        let env = Env::with_functions(&funcs).set("x", data);
        let unit = ExprUnit::compile("f0(x)", &names).unwrap();
        match unit.eval(&Value::Null, &env, &Budget::unlimited()) {
            Err(EvalError::TooDeep { .. }) => {}
            other => panic!("{other:?}"),
        }
    });
}

#[test]
fn the_depth_limit_counts_the_real_tree() {
    // a 61-term chain whose last operand is a 40-term chain is 62 levels
    // deep: the inner chain hangs one level below the outer one's top
    let inner = format!("(b{})", " + b".repeat(39));
    let src = format!("a{} + {inner}", " + a".repeat(59));
    let names = Names::new().vars(["a", "b"]);
    assert!(ExprUnit::compile(&src, &names).is_ok());
    // 100 operators in one chain is 101 levels: refused
    let long = format!("a{}", " + a".repeat(100));
    let err = ExprUnit::compile(&long, &names).unwrap_err();
    assert!(
        matches!(err[0], CompileError::TooDeep { limit: 100, .. }),
        "{err:?}"
    );
    assert!(ExprUnit::compile(&format!("a{}", " + a".repeat(99)), &names).is_ok());
}
