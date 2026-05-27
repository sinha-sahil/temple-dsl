use serde::Deserialize;
use temple_dsl::{Template, Value};

fn render<T: serde::de::DeserializeOwned>(src: &str, input: Value) -> T {
    let template = Template::compile(src).expect("compile");
    template.render::<T>(input).expect("render")
}

#[test]
fn simple_let() {
    let src = r#"
        let x = input.value
        { "out": {{ x }} }
    "#;
    #[derive(Deserialize)]
    struct Out {
        out: i64,
    }
    let out: Out = render(src, Value::obj([("value", Value::Int(7))]));
    assert_eq!(out.out, 7);
}

#[test]
fn multiple_lets() {
    let src = r#"
        let a = input.a
        let b = input.b
        { "sum": {{ a + b }} }
    "#;
    #[derive(Deserialize)]
    struct Out {
        sum: i64,
    }
    let out: Out = render(
        src,
        Value::obj([("a", Value::Int(5)), ("b", Value::Int(3))]),
    );
    assert_eq!(out.sum, 8);
}

#[test]
fn let_references_earlier_let() {
    let src = r#"
        let base = input.x
        let doubled = base * 2
        let plus_one = doubled + 1
        { "n": {{ plus_one }} }
    "#;
    #[derive(Deserialize)]
    struct Out {
        n: i64,
    }
    let out: Out = render(src, Value::obj([("x", Value::Int(10))]));
    assert_eq!(out.n, 21);
}

#[test]
fn let_with_path_into_object() {
    let src = r#"
        let user = input.profile.user
        { "name": {{ user.name }}, "email": {{ user.email }} }
    "#;
    #[derive(Deserialize)]
    struct Out {
        name: String,
        email: String,
    }
    let input = Value::obj([(
        "profile",
        Value::obj([(
            "user",
            Value::obj([
                ("name", Value::Str("Ada".into())),
                ("email", Value::Str("ada@example.com".into())),
            ]),
        )]),
    )]);
    let out: Out = render(src, input);
    assert_eq!(out.name, "Ada");
    assert_eq!(out.email, "ada@example.com");
}

#[test]
fn let_forward_reference_errors() {
    let src = r#"
        let a = b
        let b = 1
        { "x": {{ a }} }
    "#;
    let err = Template::compile(src).expect_err("expected forward-ref error");
    assert!(format!("{err:?}").contains("unknown identifier 'b'"));
}

#[test]
fn let_duplicate_errors() {
    let src = r#"
        let a = 1
        let a = 2
        { "x": {{ a }} }
    "#;
    let err = Template::compile(src).expect_err("expected duplicate error");
    assert!(format!("{err:?}").contains("already defined"));
}

#[test]
fn let_reserved_name_errors() {
    let src = r#"
        let input = 1
        { "x": {{ input }} }
    "#;
    let err = Template::compile(src).expect_err("expected reserved error");
    assert!(format!("{err:?}").contains("reserved"));
}

#[test]
fn let_cannot_reference_this() {
    let src = r#"
        let x = this.y
        { "y": 1 }
    "#;
    let err = Template::compile(src).expect_err("expected this-in-let error");
    assert!(format!("{err:?}").contains("this"));
}

#[test]
fn this_basic_reference() {
    let src = r#"{
        "a": {{ input.x }},
        "b": {{ this.a + 1 }}
    }"#;
    #[derive(Deserialize)]
    struct Out {
        a: i64,
        b: i64,
    }
    let out: Out = render(src, Value::obj([("x", Value::Int(10))]));
    assert_eq!(out.a, 10);
    assert_eq!(out.b, 11);
}

#[test]
fn this_reordering_via_topo_sort() {
    let src = r#"{
        "label":   {{ this.is_admin ? "admin" : "user" }},
        "is_admin": {{ input.role == "admin" }}
    }"#;
    #[derive(Deserialize)]
    struct Out {
        label: String,
        is_admin: bool,
    }
    let out: Out = render(src, Value::obj([("role", Value::Str("admin".into()))]));
    assert_eq!(out.label, "admin");
    assert!(out.is_admin);
}

#[test]
fn this_output_preserves_declared_order() {
    use indexmap::IndexMap;
    let src = r#"{
        "z": {{ this.a }},
        "a": {{ input.x }},
        "m": {{ this.a + 1 }}
    }"#;
    let out: IndexMap<String, i64> = render(src, Value::obj([("x", Value::Int(5))]));
    let keys: Vec<&str> = out.keys().map(std::string::String::as_str).collect();
    assert_eq!(keys, vec!["z", "a", "m"]);
    assert_eq!(out["a"], 5);
    assert_eq!(out["z"], 5);
    assert_eq!(out["m"], 6);
}

#[test]
fn this_cycle_errors() {
    let src = r#"{
        "a": {{ this.b }},
        "b": {{ this.a }}
    }"#;
    let err = Template::compile(src).expect_err("expected cycle error");
    assert!(format!("{err:?}").contains("cycle"));
}

#[test]
fn this_self_reference_errors() {
    let src = r#"{ "a": {{ this.a }} }"#;
    let err = Template::compile(src).expect_err("expected self-cycle error");
    assert!(format!("{err:?}").contains("cycle"));
}

#[test]
fn this_unknown_key_errors() {
    let src = r#"{ "a": {{ this.missing }} }"#;
    let err = Template::compile(src).expect_err("expected unknown-key error");
    assert!(format!("{err:?}").contains("unknown output key"));
}

#[test]
fn this_outside_object_output_errors() {
    let src = r#"[{{ this.a }}]"#;
    let err = Template::compile(src).expect_err("expected this-outside-object error");
    assert!(format!("{err:?}").contains("'this'"));
}

#[test]
fn unknown_identifier_errors() {
    let src = r#"{ "x": {{ foo }} }"#;
    let err = Template::compile(src).expect_err("expected unknown ident error");
    assert!(format!("{err:?}").contains("unknown identifier 'foo'"));
}

#[test]
fn let_and_this_together() {
    let src = r#"
        let user = input.user
        {
            "name":     {{ user.name }},
            "is_admin": {{ user.role == "admin" }},
            "label":    {{ this.is_admin ? "admin" : "user" }},
            "greeting": {{ when {
                this.is_admin: "Welcome back, admin",
                else: "Hello"
            } }}
        }
    "#;
    #[derive(Deserialize, Debug)]
    struct Out {
        name: String,
        is_admin: bool,
        label: String,
        greeting: String,
    }
    let input = Value::obj([(
        "user",
        Value::obj([
            ("name", Value::Str("Ada".into())),
            ("role", Value::Str("admin".into())),
        ]),
    )]);
    let out: Out = render(src, input);
    assert_eq!(out.name, "Ada");
    assert!(out.is_admin);
    assert_eq!(out.label, "admin");
    assert_eq!(out.greeting, "Welcome back, admin");
}

#[test]
fn let_with_array_output_no_this() {
    let src = r#"
        let n = input.n
        [{{ n }}, {{ n * 2 }}, {{ n * 3 }}]
    "#;
    let out: Vec<i64> = render(src, Value::obj([("n", Value::Int(7))]));
    assert_eq!(out, vec![7, 14, 21]);
}

#[test]
fn let_nested_path_through_let_var() {
    let src = r#"
        let address = input.profile.address
        let city = address.city
        { "city": {{ city }}, "country": {{ address.country }} }
    "#;
    #[derive(Deserialize)]
    struct Out {
        city: String,
        country: String,
    }
    let input = Value::obj([(
        "profile",
        Value::obj([(
            "address",
            Value::obj([
                ("city", Value::Str("London".into())),
                ("country", Value::Str("UK".into())),
            ]),
        )]),
    )]);
    let out: Out = render(src, input);
    assert_eq!(out.city, "London");
    assert_eq!(out.country, "UK");
}

#[test]
fn this_chained_dependencies() {
    let src = r#"{
        "final": {{ this.middle + 1 }},
        "middle": {{ this.base * 2 }},
        "base": {{ input.x }}
    }"#;
    #[derive(Deserialize)]
    struct Out {
        base: i64,
        middle: i64,
        #[serde(rename = "final")]
        final_: i64,
    }
    let out: Out = render(src, Value::obj([("x", Value::Int(5))]));
    assert_eq!(out.base, 5);
    assert_eq!(out.middle, 10);
    assert_eq!(out.final_, 11);
}

#[test]
fn this_with_optional_access() {
    let src = r#"{
        "raw": {{ input.value }},
        "label": {{ this.raw ?? "default" }}
    }"#;
    #[derive(Deserialize)]
    struct Out {
        raw: Option<String>,
        label: String,
    }
    let out: Out = render(src, Value::obj([("value", Value::Null)]));
    assert!(out.raw.is_none());
    assert_eq!(out.label, "default");
}
