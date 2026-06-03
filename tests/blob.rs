//! Compiled-blob round-trip (`to_bytes` / `from_bytes`) — corruption and
//! version handling.

use rust_decimal::Decimal;
use serde::Deserialize;
use temple_dsl::{LoadError, Template, Value};

#[test]
fn round_trip_simple() {
    #[derive(Deserialize, PartialEq, Debug)]
    struct O {
        id: i64,
        name: String,
    }
    let t = Template::compile(r#"{ "id": {{ input.id }}, "name": {{ input.name }} }"#).unwrap();
    let loaded = Template::from_bytes(&t.to_bytes()).expect("load");
    let input = Value::obj([("id", Value::Int(7)), ("name", Value::Str("Ada".into()))]);
    let a: O = t.render(input.clone()).unwrap();
    let b: O = loaded.render(input).unwrap();
    assert_eq!(a, b);
    assert_eq!(
        b,
        O {
            id: 7,
            name: "Ada".into()
        }
    );
}

#[test]
fn round_trip_full_language() {
    #[derive(Deserialize, PartialEq, Debug)]
    struct Line {
        n: String,
        t: Decimal,
    }
    #[derive(Deserialize, PartialEq, Debug)]
    struct O {
        summary: String,
        lines: Vec<Line>,
        subtotal: Decimal,
        free: bool,
    }
    let src = r#"
        let items = input.items
        {
            "summary":  "{{ input.customer }} x {{ items.length() }}",
            "lines":    {{ items.map(it -> { "n": it.name, "t": it.qty * it.price }) }},
            "subtotal": {{ items.map(it -> it.qty * it.price).sum() }},
            "free":     {{ this.subtotal >= 50 }}
        }
    "#;
    let t = Template::compile(src).unwrap();
    let loaded = Template::from_bytes(&t.to_bytes()).expect("load");
    let input = Value::obj([
        ("customer", Value::Str("Ada".into())),
        (
            "items",
            Value::Arr(vec![
                Value::obj([
                    ("name", Value::Str("w".into())),
                    ("qty", Value::Int(6)),
                    ("price", Value::Decimal("9.99".parse().unwrap())),
                ]),
                Value::obj([
                    ("name", Value::Str("g".into())),
                    ("qty", Value::Int(1)),
                    ("price", Value::Decimal("4.50".parse().unwrap())),
                ]),
            ]),
        ),
    ]);
    let a: O = t.render(input.clone()).unwrap();
    let b: O = loaded.render(input).unwrap();
    assert_eq!(a, b);
    assert_eq!(b.summary, "Ada x 2");
    assert_eq!(b.subtotal, "64.44".parse::<Decimal>().unwrap());
    assert!(b.free);
}

#[test]
fn bad_signature_is_corrupt() {
    let t = Template::compile(r#"{ "x": {{ input.a }} }"#).unwrap();
    let mut bytes = t.to_bytes();
    bytes[0] ^= 0xFF;
    assert!(matches!(
        Template::from_bytes(&bytes),
        Err(LoadError::Corrupt(_))
    ));
}

#[test]
fn truncated_header_is_corrupt() {
    let t = Template::compile(r#"{ "x": {{ input.a }} }"#).unwrap();
    let bytes = t.to_bytes();
    assert!(matches!(
        Template::from_bytes(&bytes[..4]),
        Err(LoadError::Corrupt(_))
    ));
    assert!(matches!(
        Template::from_bytes(&[]),
        Err(LoadError::Corrupt(_))
    ));
}

#[test]
fn truncated_payload_is_corrupt() {
    let t = Template::compile(r#"{ "x": {{ input.a + input.b }} }"#).unwrap();
    let bytes = t.to_bytes();
    let chopped = &bytes[..bytes.len() - 3];
    assert!(matches!(
        Template::from_bytes(chopped),
        Err(LoadError::Corrupt(_))
    ));
}

#[test]
fn near_max_source_blob_still_loads() {
    // A near-cap source must produce a blob that from_bytes accepts — i.e. the
    // blob cap clears the worst-case source->CBOR expansion.
    let mut s = String::from("{\n");
    let mut i = 0usize;
    while s.len() < 1_000_000 {
        s.push_str(&format!("  \"key_{i}\": {{{{ input.v{i} * 2 + 1 }}}},\n"));
        i += 1;
    }
    s.push_str("  \"tail\": {{ input.z }}\n}");
    let t = Template::compile(&s).expect("compile near-cap template");
    let bytes = t.to_bytes();
    assert!(
        Template::from_bytes(&bytes).is_ok(),
        "blob of {} bytes from a {}-byte source must reload",
        bytes.len(),
        s.len()
    );
}

#[test]
fn wrong_version_is_incompatible() {
    let t = Template::compile(r#"{ "x": {{ input.a }} }"#).unwrap();
    let mut bytes = t.to_bytes();
    bytes[4] = 0xFF; // bump the LE version
    assert!(matches!(
        Template::from_bytes(&bytes),
        Err(LoadError::IncompatibleVersion {
            found: 255,
            expected: 1
        })
    ));
}
