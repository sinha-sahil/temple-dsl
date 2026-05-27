use serde::Deserialize;
use temple_dsl::{RenderError, Template, Value};

fn render<T: serde::de::DeserializeOwned>(src: &str, input: Value) -> T {
    let template = Template::compile(src).expect("compile");
    template.render::<T>(input).expect("render")
}

#[test]
fn optional_chain_short_circuits_on_null() {
    let src = r#"{{ input.user?.email }}"#;
    let input = Value::obj([("user", Value::Null)]);
    let out: Option<String> = render(src, input);
    assert!(out.is_none());
}

#[test]
fn optional_chain_short_circuits_on_missing() {
    let src = r#"{{ input?.missing }}"#;
    let out: Option<String> = render(src, Value::obj([("present", Value::Str("x".into()))]));
    assert!(out.is_none());
}

#[test]
fn optional_chain_passes_through_existing() {
    let src = r#"{{ input.user?.email }}"#;
    let input = Value::obj([(
        "user",
        Value::obj([("email", Value::Str("ada@example.com".into()))]),
    )]);
    let out: Option<String> = render(src, input);
    assert_eq!(out, Some("ada@example.com".to_string()));
}

#[test]
fn chained_optional_segments() {
    let src = r#"{{ input.a?.b?.c }}"#;

    let input = Value::obj([(
        "a",
        Value::obj([("b", Value::obj([("c", Value::Str("deep".into()))]))]),
    )]);
    let out: Option<String> = render(src, input);
    assert_eq!(out, Some("deep".to_string()));

    let input = Value::obj([("a", Value::obj([("b", Value::Null)]))]);
    let out: Option<String> = render(src, input);
    assert!(out.is_none());

    let input = Value::obj([("a", Value::Null)]);
    let out: Option<String> = render(src, input);
    assert!(out.is_none());
}

#[test]
fn optional_then_required_after_existing() {
    let src = r#"{{ input.user?.profile.name }}"#;
    let input = Value::obj([(
        "user",
        Value::obj([("profile", Value::obj([("name", Value::Str("Ada".into()))]))]),
    )]);
    let out: String = render(src, input);
    assert_eq!(out, "Ada");
}

#[test]
fn coalesce_returns_lhs_when_not_null() {
    let src = r#"{{ input.value ?? "fallback" }}"#;
    let input = Value::obj([("value", Value::Str("real".into()))]);
    let out: String = render(src, input);
    assert_eq!(out, "real");
}

#[test]
fn coalesce_returns_rhs_when_null() {
    let src = r#"{{ input.maybe ?? "fallback" }}"#;
    let input = Value::obj([("maybe", Value::Null)]);
    let out: String = render(src, input);
    assert_eq!(out, "fallback");
}

#[test]
fn coalesce_chain_first_non_null_wins() {
    let src = r#"{{ input.a ?? input.b ?? "last" }}"#;

    let input = Value::obj([("a", Value::Null), ("b", Value::Str("from_b".into()))]);
    let out: String = render(src, input);
    assert_eq!(out, "from_b");

    let input = Value::obj([("a", Value::Null), ("b", Value::Null)]);
    let out: String = render(src, input);
    assert_eq!(out, "last");
}

#[test]
fn coalesce_short_circuits_rhs() {
    let src = r#"{{ 1 ?? (1 / 0) }}"#;
    let out: i64 = render(src, Value::Null);
    assert_eq!(out, 1);
}

#[test]
fn optional_and_coalesce_together() {
    let src = r#"{{ input.user?.email ?? "no-email" }}"#;

    let input = Value::obj([("user", Value::Null)]);
    let out: String = render(src, input);
    assert_eq!(out, "no-email");

    let input = Value::obj([(
        "user",
        Value::obj([("email", Value::Str("ada@example.com".into()))]),
    )]);
    let out: String = render(src, input);
    assert_eq!(out, "ada@example.com");
}

#[test]
fn coalesce_does_not_replace_falsy_non_null() {
    let src = r#"{{ input.flag ?? true }}"#;
    let input = Value::obj([("flag", Value::Bool(false))]);
    let out: bool = render(src, input);
    assert!(!out);
}

#[test]
fn coalesce_inside_object() {
    #[derive(Deserialize, Debug)]
    struct Out {
        name: String,
        nickname: String,
    }
    let src = r#"{
        "name":     {{ input.name }},
        "nickname": {{ input.nickname ?? input.name }}
    }"#;
    let input = Value::obj([
        ("name", Value::Str("Ada Lovelace".into())),
        ("nickname", Value::Null),
    ]);
    let out: Out = render(src, input);
    assert_eq!(out.name, "Ada Lovelace");
    assert_eq!(out.nickname, "Ada Lovelace");
}

#[test]
fn optional_does_not_swallow_type_errors() {
    let src = r#"{{ input.n?.field }}"#;
    let input = Value::obj([("n", Value::Int(5))]);
    let template = Template::compile(src).expect("compile");
    let result = template.render::<Option<String>>(input);
    assert!(matches!(result, Err(RenderError::TypeMismatch { .. })));
}

#[test]
fn coalesce_in_when_branch() {
    let src = r#"{{ when {
        input.role == "admin": input.label ?? "Administrator",
        else: "guest"
    } }}"#;
    let input = Value::obj([("role", Value::Str("admin".into())), ("label", Value::Null)]);
    let out: String = render(src, input);
    assert_eq!(out, "Administrator");
}

#[test]
fn ternary_distinct_from_coalesce_and_optional() {
    let src = r#"{{ (input.user?.active ?? false) ? input.user.name : (input.guest_name ?? "stranger") }}"#;

    let active_user = Value::obj([(
        "user",
        Value::obj([
            ("active", Value::Bool(true)),
            ("name", Value::Str("Ada".into())),
        ]),
    )]);
    let out: String = render(src, active_user);
    assert_eq!(out, "Ada");

    let no_user = Value::obj([("user", Value::Null), ("guest_name", Value::Null)]);
    let out: String = render(src, no_user);
    assert_eq!(out, "stranger");
}
