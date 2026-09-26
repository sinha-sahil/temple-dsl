use super::*;

#[test]
fn host_names_are_accepted_and_typos_get_hints() {
    let names = Names::new().vars(["windowDays", "order"]);
    assert!(ExprUnit::compile("order.age > windowDays", &names).is_ok());
    let errs = ExprUnit::compile("windowDay > 1", &names).unwrap_err();
    assert!(
        msg(&errs).contains("did you mean `windowDays`"),
        "{}",
        msg(&errs)
    );
}

#[test]
fn reserved_and_clashing_names_are_rejected() {
    for bad in [
        Names::new().var("input"),
        Names::new().var("this"),
        Names::new().function("round", 1),
        Names::new().function("when", 0),
        Names::new().var("f").function("f", 1),
    ] {
        assert!(ExprUnit::compile("1", &bad).is_err(), "{bad:?}");
    }
}

#[test]
fn host_functions_are_checked_for_arity_and_suggested() {
    let names = Names::new().function("open", 1);
    assert!(ExprUnit::compile("open(1) > 0", &names).is_ok());
    let errs = ExprUnit::compile("open(1, 2)", &names).unwrap_err();
    assert!(
        msg(&errs).contains("takes 1 argument, got 2"),
        "{}",
        msg(&errs)
    );
    let errs = ExprUnit::compile("opne(1)", &names).unwrap_err();
    assert!(msg(&errs).contains("did you mean `open`"), "{}", msg(&errs));
}

#[test]
fn this_is_only_allowed_in_object_templates() {
    assert!(ExprUnit::compile("this.a", &Names::new()).is_err());
    let t = TemplateUnit::compile("{ \"a\": 1, \"b\": {{ this.a + 1 }} }", &Names::new()).unwrap();
    let out = t
        .render(&Value::Null, &Env::new(), &Budget::unlimited())
        .unwrap();
    assert_eq!(out, obj(&[("a", Value::Int(1)), ("b", Value::Int(2))]));
}

#[test]
fn a_name_is_not_both_a_variable_and_a_function() {
    let mut names = Names::new().var("a");
    names.add_function("a", 1);
    names.add_function("a", 2); // redefining changes only the arity
    let errors = ExprUnit::compile("1", &names).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(msg(&errors).contains("both a variable and a function"));
    assert_eq!(names.function_arity("a"), Some(2));
}

#[test]
fn only_the_first_hundred_unknown_names_in_a_unit_get_hints() {
    let names = Names::new().var("apple");
    let src = format!("[{}]", vec!["appel"; 150].join(", "));
    let errors = ExprUnit::compile(&src, &names).unwrap_err();
    assert_eq!(errors.len(), 150);
    let hinted = errors
        .iter()
        .filter(|e| e.to_string().contains("did you mean `apple`"))
        .count();
    assert_eq!(hinted, 100);
}
