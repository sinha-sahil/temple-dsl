//! End-to-end lifecycle against a real Postgres: compile -> to_bytes -> store in
//! a BYTEA column -> reload -> from_bytes -> render. Exercises the whole language
//! and the failure/recovery paths (corrupt blob, version mismatch).
//!
//! Run:  DATABASE_URL=postgres://user@localhost/db cargo run --example postgres_e2e --features pg
#![allow(dead_code)]

use postgres::{Client, NoTls};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::process::ExitCode;
use temple_dsl::{LoadError, Template, Value};

fn dec(s: &str) -> Value {
    Value::Decimal(s.parse().unwrap())
}

/// Build the nested input the kitchen-sink template expects.
fn kitchen_input(tier: Value, note: Value) -> Value {
    Value::obj([
        (
            "user",
            Value::obj([
                ("name", Value::Str("ada".into())),
                ("tier", tier),
                ("age", Value::Int(36)),
                ("note", note),
            ]),
        ),
        (
            "items",
            Value::Arr(vec![
                Value::obj([
                    ("sku", Value::Str("widget".into())),
                    ("qty", Value::Int(6)),
                    ("price", dec("9.99")),
                    ("in_stock", Value::Bool(true)),
                ]),
                Value::obj([
                    ("sku", Value::Str("gizmo".into())),
                    ("qty", Value::Int(1)),
                    ("price", dec("19.99")),
                    ("in_stock", Value::Bool(true)),
                ]),
            ]),
        ),
    ])
}

const KITCHEN_SINK: &str = r#"
    let user = input.user
    let items = input.items
    {
        "name":         {{ upper(user.name) }},
        "tier":         {{ user.tier ?? "standard" }},
        "is_adult":     {{ user.age >= 18 }},
        "greeting":     "Hi {{ user.name }}, you have {{ items.length() }} item(s)",
        "lines":        {{ items.map(it -> { "sku": upper(it.sku), "amount": round(it.qty * it.price, 2) }) }},
        "subtotal":     {{ items.map(it -> it.qty * it.price).sum() }},
        "expensive":    {{ items.filter(it -> it.price >= 10).map(it -> it.sku) }},
        "any_bulk":     {{ items.any(it -> it.qty >= 5) }},
        "all_in_stock": {{ items.all(it -> it.in_stock) }},
        "first_sku":    {{ items.first().sku }},
        "max_price":    {{ items.fold(0, (acc, it) -> max(acc, it.price)) }},
        "discount":     {{ when {
                            this.subtotal >= 100: 0.10,
                            this.subtotal >= 50:  0.05,
                            else:                 0.0
                        } }},
        "total":        {{ round(this.subtotal * (1 - this.discount), 2) }},
        "label":        {{ this.is_adult ? "adult" : "minor" }},
        "note":         {{ user.note?.text ?? "no note" }}
    }
"#;

/// (id, source, input) — every entry must render identically before and after a
/// Postgres blob round-trip, which is the core serialize+deserialize assertion.
fn corpus() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        (
            "kitchen",
            KITCHEN_SINK,
            kitchen_input(Value::Null, Value::Null),
        ),
        (
            "operators",
            r#"{ "v": {{ (input.a + input.b) * 2 - input.c / 4 }}, "cmp": {{ input.a < input.b && !input.flag }} }"#,
            Value::obj([
                ("a", Value::Int(3)),
                ("b", Value::Int(5)),
                ("c", Value::Int(8)),
                ("flag", Value::Bool(false)),
            ]),
        ),
        (
            "grade",
            r#"{ "g": {{ when { input.s >= 90: "A", input.s >= 80: "B", else: "C" } }}, "pass": {{ input.s >= 60 ? true : false }} }"#,
            Value::obj([("s", Value::Int(85))]),
        ),
        (
            "safe-access",
            r#"{ "email": {{ input.contact?.email ?? "none" }}, "city": {{ input.addr?.city ?? "unknown" }} }"#,
            Value::obj([
                ("contact", Value::Null),
                ("addr", Value::obj([("city", Value::Str("London".into()))])),
            ]),
        ),
        (
            "collections",
            r#"{
                "doubled": {{ input.xs.map(x -> x * 2) }},
                "evens":   {{ input.xs.filter(x -> x / 2 * 2 == x) }},
                "sum":     {{ input.xs.sum() }},
                "any3":    {{ input.xs.any(x -> x > 3) }},
                "all_pos": {{ input.xs.all(x -> x > 0) }},
                "len":     {{ input.xs.len() }},
                "first":   {{ input.xs.first() }},
                "last":    {{ input.xs.last() }},
                "joined":  {{ input.xs.concat([99, 100]) }},
                "at2":     {{ input.xs[2] }}
            }"#,
            Value::obj([("xs", Value::Arr((1..=5).map(Value::Int).collect()))]),
        ),
        (
            "interp",
            r#""{{ input.who }} owes {{ input.amt }} on {{ input.day }}""#,
            Value::obj([
                ("who", Value::Str("Bob".into())),
                ("amt", dec("12.50")),
                ("day", Value::Str("Mon".into())),
            ]),
        ),
        (
            "array-output",
            r#"[{{ input.a }}, {{ input.b * 2 }}, {{ input.a + input.b }}]"#,
            Value::obj([("a", Value::Int(10)), ("b", Value::Int(20))]),
        ),
        (
            "builtins",
            r#"{
                "abs": {{ abs(-7) }}, "round": {{ round(3.14159, 2) }},
                "floor": {{ floor(3.9) }}, "ceil": {{ ceil(3.1) }},
                "min": {{ min(5, 2, 8) }}, "max": {{ max(5, 2, 8) }},
                "upper": {{ upper("hi") }}, "lower": {{ lower("HI") }},
                "trim": {{ trim("  x  ") }}, "to_string": {{ to_string(42) }},
                "len": {{ len(input.s) }}
            }"#,
            Value::obj([("s", Value::Str("hello".into()))]),
        ),
        (
            "decimals",
            r#"{ "sub": {{ input.p * input.q }}, "tax": {{ round(input.p * input.q * 0.0825, 2) }}, "exact": {{ 0.1 + 0.2 }} }"#,
            Value::obj([("p", dec("19.99")), ("q", Value::Int(3))]),
        ),
    ]
}

fn store(
    client: &mut Client,
    id: &str,
    source: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let blob = Template::compile(source)
        .map_err(|e| format!("compile {id}: {e:?}"))?
        .to_bytes();
    client.execute(
        "INSERT INTO temple_e2e (id, source, version, compiled) VALUES ($1, $2, $3, $4)
         ON CONFLICT (id) DO UPDATE SET source = $2, version = $3, compiled = $4",
        &[&id, &source, &1i32, &blob],
    )?;
    Ok(blob)
}

fn load(client: &mut Client, id: &str) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    let row = client.query_one(
        "SELECT source, compiled FROM temple_e2e WHERE id = $1",
        &[&id],
    )?;
    Ok((row.get(0), row.get(1)))
}

fn main() -> ExitCode {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "set DATABASE_URL, e.g. DATABASE_URL=postgres://postgres@localhost:5432/postgres"
        );
        return ExitCode::from(2);
    };
    match run(&url) {
        Ok(n) => {
            println!("\n✓ all {n} end-to-end checks passed");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("\n✗ FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(url: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let mut client = Client::connect(url, NoTls)?;
    client.batch_execute(
        "DROP TABLE IF EXISTS temple_e2e;
         CREATE TABLE temple_e2e (
             id       TEXT  PRIMARY KEY,
             source   TEXT  NOT NULL,
             version  INT   NOT NULL,
             compiled BYTEA NOT NULL
         );",
    )?;
    let mut checks = 0usize;

    // 1) Round-trip identity across the whole language: rendering the freshly
    //    compiled template must equal rendering the one reloaded from Postgres.
    println!("== round-trip identity (compile -> to_bytes -> BYTEA -> from_bytes -> render) ==");
    for (id, src, input) in corpus() {
        let fresh = Template::compile(src).map_err(|e| format!("{id}: {e:?}"))?;
        store(&mut client, id, src)?;
        let (_, blob) = load(&mut client, id)?;
        let reloaded = Template::from_bytes(&blob)?;

        let a = fresh.render_value(input.clone())?;
        let b = reloaded.render_value(input)?;
        if a != b {
            return Err(format!(
                "{id}: render diverged after blob round-trip\n fresh={a:?}\n reloaded={b:?}"
            )
            .into());
        }
        println!("  ✓ {id:<13} blob {} B, identical render", blob.len());
        checks += 1;
    }

    // 2) Typed deserialize from a reloaded template (the serde path).
    println!("== typed deserialize from a reloaded blob ==");
    {
        #[derive(Deserialize, PartialEq, Debug)]
        struct Line {
            sku: String,
            amount: Decimal,
        }
        #[derive(Deserialize, PartialEq, Debug)]
        struct Receipt {
            name: String,
            tier: String,
            subtotal: Decimal,
            total: Decimal,
            label: String,
            any_bulk: bool,
            all_in_stock: bool,
            greeting: String,
            lines: Vec<Line>,
            expensive: Vec<String>,
            note: String,
        }
        let (_, blob) = load(&mut client, "kitchen")?;
        let t = Template::from_bytes(&blob)?;
        let r: Receipt = t.render(kitchen_input(Value::Str("gold".into()), Value::Null))?;
        assert_eq!(r.name, "ADA");
        assert_eq!(r.tier, "gold");
        assert_eq!(r.subtotal, "79.93".parse::<Decimal>().unwrap()); // 6*9.99 + 1*19.99
        assert_eq!(r.total, "75.93".parse::<Decimal>().unwrap()); // 5% off (>=50)
        assert_eq!(r.label, "adult");
        assert!(r.any_bulk && r.all_in_stock);
        assert_eq!(r.greeting, "Hi ada, you have 2 item(s)");
        assert_eq!(r.expensive, vec!["gizmo".to_string()]);
        assert_eq!(r.note, "no note");
        assert_eq!(
            r.lines,
            vec![
                Line {
                    sku: "WIDGET".into(),
                    amount: "59.94".parse().unwrap()
                },
                Line {
                    sku: "GIZMO".into(),
                    amount: "19.99".parse().unwrap()
                },
            ]
        );
        println!("  ✓ kitchen reloaded -> Receipt struct, all fields exact (subtotal=79.93, total=75.93)");
        checks += 1;
    }

    // 3) The ?? / ?. coalescing branches when input fields are present vs null.
    println!("== optional/coalesce branches via reloaded blob ==");
    {
        #[derive(Deserialize)]
        struct R {
            tier: String,
            note: String,
        }
        let (_, blob) = load(&mut client, "kitchen")?;
        let t = Template::from_bytes(&blob)?;
        let present: R = t.render(kitchen_input(
            Value::Str("silver".into()),
            Value::obj([("text", Value::Str("VIP".into()))]),
        ))?;
        let absent: R = t.render(kitchen_input(Value::Null, Value::Null))?;
        assert_eq!(present.tier, "silver");
        assert_eq!(present.note, "VIP");
        assert_eq!(absent.tier, "standard"); // ?? fallback
        assert_eq!(absent.note, "no note"); // ?. -> null -> ?? fallback
        println!("  ✓ present vs null inputs take the right ??/?. branches");
        checks += 1;
    }

    // 4) render_value: the dynamic Value comes back directly from a reloaded blob.
    println!("== render_value (dynamic output) from a reloaded blob ==");
    {
        let (_, blob) = load(&mut client, "array-output")?;
        let t = Template::from_bytes(&blob)?;
        let v = t.render_value(Value::obj([("a", Value::Int(10)), ("b", Value::Int(20))]))?;
        assert_eq!(
            v,
            Value::Arr(vec![Value::Int(10), Value::Int(40), Value::Int(30)])
        );
        println!("  ✓ render_value -> {v:?}");
        checks += 1;
    }

    // 5) Failure & recovery paths.
    println!("== failure & recovery ==");
    {
        // compile error surfaces (and is never stored).
        assert!(Template::compile("{{ ").is_err());
        println!("  ✓ malformed template -> compile error");
        checks += 1;

        // render error: a missing required path is an Err, not a panic.
        let (_, blob) = load(&mut client, "operators")?;
        let t = Template::from_bytes(&blob)?;
        assert!(t.render_value(Value::obj([("a", Value::Int(1))])).is_err());
        println!("  ✓ missing input field -> render error (no panic)");
        checks += 1;

        // Truncated blob in storage (partial write) -> Err(Corrupt) -> recover by
        // recompiling the stored source. (Note: CBOR carries no checksum, so a lone
        // bit flip in the payload may still decode; truncation is reliably caught,
        // and the post-decode resolve() catches structural corruption.)
        let (source, mut blob) = load(&mut client, "kitchen")?;
        blob.truncate(blob.len() * 3 / 4); // header intact, CBOR payload cut short
        match Template::from_bytes(&blob) {
            Err(LoadError::Corrupt(_)) => {
                let recovered =
                    Template::compile(&source).map_err(|e| format!("recompile: {e:?}"))?;
                let _ = recovered.render_value(kitchen_input(Value::Null, Value::Null))?;
                println!(
                    "  ✓ truncated BYTEA -> LoadError::Corrupt -> recompiled from stored source"
                );
                checks += 1;
            }
            other => return Err(format!("expected Corrupt, got {other:?}").into()),
        }

        // version-tagged blob from a future format -> IncompatibleVersion -> recompile.
        let (source, mut blob) = load(&mut client, "decimals")?;
        blob[4] = 0xFF; // bump the LE version word
        match Template::from_bytes(&blob) {
            Err(LoadError::IncompatibleVersion { .. }) => {
                let _ = Template::compile(&source).map_err(|e| format!("recompile: {e:?}"))?;
                println!(
                    "  ✓ version-mismatched blob -> IncompatibleVersion -> recompiled from source"
                );
                checks += 1;
            }
            other => return Err(format!("expected IncompatibleVersion, got {other:?}").into()),
        }
    }

    // 6) Determinism: the same source stored twice yields the identical blob.
    println!("== determinism ==");
    {
        let b1 = store(&mut client, "det", KITCHEN_SINK)?;
        let b2 = Template::compile(KITCHEN_SINK).unwrap().to_bytes();
        assert_eq!(b1, b2, "to_bytes must be deterministic");
        println!("  ✓ identical source -> byte-identical blob");
        checks += 1;
    }

    client.batch_execute("DROP TABLE temple_e2e;")?;
    Ok(checks)
}
