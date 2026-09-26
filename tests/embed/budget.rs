use super::*;

#[test]
fn nested_iteration_is_counted_in_full() {
    let names = Names::new();
    let unit = ExprUnit::compile(
        "input.xs.map(a -> input.xs.map(b -> a * b).sum()).sum()",
        &names,
    )
    .unwrap();
    let xs: Vec<Value> = (0..200).map(Value::Int).collect();
    let input = obj(&[("xs", arr(xs))]);
    let budget = Budget::new(10_000);
    let err = unit.eval(&input, &Env::new(), &budget).unwrap_err();
    assert!(err.is_budget_exceeded(), "{err:?}");
    let big = Budget::new(10_000_000);
    unit.eval(&input, &Env::new(), &big).unwrap();
    assert!(big.used() > 40_000, "used {}", big.used());
}

#[test]
fn quadratic_builtins_are_charged() {
    let unit = ExprUnit::compile("input.xs.unique().len()", &Names::new()).unwrap();
    let xs: Vec<Value> = (0..2_000).map(Value::Int).collect();
    let input = obj(&[("xs", arr(xs))]);
    let err = unit
        .eval(&input, &Env::new(), &Budget::new(100_000))
        .unwrap_err();
    assert!(err.is_budget_exceeded());
}

#[test]
fn budget_error_points_at_the_expression_and_reports() {
    let src = "input.xs.map(x -> x + 1)";
    let unit = ExprUnit::compile(src, &Names::new()).unwrap();
    let input = obj(&[("xs", arr((0..100).map(Value::Int).collect()))]);
    let err = unit
        .eval(&input, &Env::new(), &Budget::new(20))
        .unwrap_err();
    let EvalError::BudgetExceeded { limit, .. } = err else {
        panic!("{err:?}")
    };
    assert_eq!(limit, 20);
    assert!(err.report(src).contains("budget of 20 steps exceeded"));
}

#[test]
fn budget_accounting() {
    let b = Budget::new(5);
    assert_eq!(b.remaining(), 5);
    let unit = ExprUnit::compile("1 + 2", &Names::new()).unwrap();
    unit.eval(&Value::Null, &Env::new(), &b).unwrap();
    assert_eq!(b.used(), 3);
    assert_eq!(b.remaining(), 2);
    b.reset();
    assert_eq!(b.used(), 0);
    assert_eq!(Budget::unlimited().limit(), u64::MAX);
}

fn used(src: &str, input: &Value) -> Result<u64, EvalError> {
    let unit = ExprUnit::compile(src, &Names::new()).unwrap();
    let budget = Budget::unlimited();
    unit.eval(input, &Env::new(), &budget)?;
    Ok(budget.used())
}

#[test]
fn copying_data_is_charged_by_its_size() {
    let big = Value::Arr(
        (0..1000)
            .map(|i| Value::obj([("k", Value::Int(i))]))
            .collect(),
    );
    let input = Value::obj([("big", big), ("n", Value::Int(1))]);
    // a scalar read costs one step, as before. Reading the list walks its
    // 2000 elements (1000 objects of one field) to check their depth, and
    // copying it costs one step per element plus one per 64 bytes of text,
    // keys included (1000 one-byte keys)
    assert_eq!(used("input.n", &input).unwrap(), 1);
    assert_eq!(
        used("input.big", &input).unwrap(),
        1 + 2000 + 2000 + 1000 / 64
    );
    // a method reads its receiver in place: nothing walked, nothing copied
    assert!(used("input.big.len()", &input).unwrap() < 10);
    // but comparing reads the whole value, every time
    assert!(used("input.big != null", &input).unwrap() > 2000);
}

#[test]
fn copying_the_input_over_and_over_runs_out_of_budget() {
    let big = Value::Arr((0..1000).map(Value::Int).collect());
    let xs = Value::Arr((0..1000).map(Value::Int).collect());
    let input = Value::obj([("big", big), ("xs", xs)]);
    let unit = ExprUnit::compile("input.xs.map(i -> input.big).len()", &Names::new()).unwrap();
    let err = unit
        .eval(&input, &Env::new(), &Budget::new(100_000))
        .unwrap_err();
    assert!(err.is_budget_exceeded(), "{err}");
}

#[test]
fn text_is_charged_by_the_bytes_copied_or_built() {
    let input = Value::obj([
        ("short", Value::from("abc")),
        ("long", Value::from("x".repeat(6400).as_str())),
        ("a", Value::from("a".repeat(1000).as_str())),
        ("wide", Value::from("y".repeat(10_000).as_str())),
    ]);
    assert_eq!(used("input.short", &input).unwrap(), 1);
    assert_eq!(used("input.long", &input).unwrap(), 1 + 100);
    // replacing each of 1000 letters with 10,000 builds 10 MB: charged first
    let unit = ExprUnit::compile("input.a.replace('a', input.wide)", &Names::new()).unwrap();
    let err = unit
        .eval(&input, &Env::new(), &Budget::new(100_000))
        .unwrap_err();
    assert!(err.is_budget_exceeded(), "{err}");
    // splitting into letters is one step per part
    assert!(used("input.a.split('')", &input).unwrap() >= 1000);
    // joining many copies of one shared string pays for every copy: 1000
    // copies of 10 KB, then 10 MB of text built, about 315,000 steps
    let join = "input.a.split('').map(c -> input.wide).join('')";
    let unit = ExprUnit::compile(join, &Names::new()).unwrap();
    let r = unit.eval(&input, &Env::new(), &Budget::new(200_000));
    assert!(
        matches!(&r, Err(e) if e.is_budget_exceeded()),
        "not refused"
    );
    let budget = Budget::new(1_000_000);
    assert!(unit.eval(&input, &Env::new(), &budget).is_ok());
    assert!(budget.used() > 300_000, "{}", budget.used());
}

#[test]
fn reading_a_large_value_over_and_over_runs_out_of_budget() {
    // each element compares the whole list with null: quadratic work
    let xs = Value::Arr((0..20_000).map(Value::Int).collect());
    let input = Value::obj([("xs", xs)]);
    let unit = ExprUnit::compile("input.xs.all(x -> input.xs != null)", &Names::new()).unwrap();
    let started = std::time::Instant::now();
    let err = unit
        .eval(&input, &Env::new(), &Budget::new(2_000_000))
        .unwrap_err();
    assert!(err.is_budget_exceeded(), "{err}");
    assert!(started.elapsed().as_secs() < 5);
}

#[test]
fn deep_host_data_is_refused_before_it_is_copied_or_compared() {
    on_small_stack(|| {
        let mut deep = Value::Int(1);
        for _ in 0..100_000 {
            deep = Value::Arr(vec![deep]);
        }
        let input = Value::obj([("xs", Value::Arr(vec![deep, Value::Int(2)]))]);
        for src in [
            "input.xs.unique()",
            "input.xs.reverse()",
            "input.xs.filter(x -> true)",
            "input.xs.first()",
            "input.xs.sort_by(x -> 1)",
        ] {
            let unit = ExprUnit::compile(src, &Names::new()).unwrap();
            let err = unit
                .eval(&input, &Env::new(), &Budget::unlimited())
                .unwrap_err();
            assert!(err.to_string().contains("deep"), "{src}: {err}");
        }
        // a method that copies nothing still works
        let unit = ExprUnit::compile("input.xs.len()", &Names::new()).unwrap();
        assert_eq!(
            unit.eval(&input, &Env::new(), &Budget::unlimited())
                .unwrap(),
            Value::Int(2)
        );
    });
}
