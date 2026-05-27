use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use rust_decimal::Decimal;
use serde::Deserialize;
use temple_dsl::{Template, Value};

// -------- small --------

const SMALL_TEMPLATE: &str = r#"{
    "id":   {{ input.id }},
    "name": {{ input.name }},
    "tag":  {{ input.tag }}
}"#;

#[derive(Deserialize)]
struct SmallOut {
    #[allow(dead_code)]
    id: i64,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    tag: String,
}

fn small_input() -> Value {
    Value::obj([
        ("id", Value::Int(42)),
        ("name", Value::Str("Ada".into())),
        ("tag", Value::Str("staff".into())),
    ])
}

// -------- big --------

const BIG_TEMPLATE: &str = r#"{
    "request_id":      {{ input.request_id }},
    "idempotency_key": {{ input.request_id }},
    "method":          "POST",
    "url":             {{ input.endpoint }},
    "headers": {
        "authorization": {{ input.auth.token }},
        "content_type":  "application/json",
        "user_agent":    {{ input.client.user_agent }},
        "trace_id":      {{ input.client.trace_id }}
    },
    "customer": {
        "id":      {{ input.customer.id }},
        "email":   {{ input.customer.email }},
        "name":    {{ input.customer.name }},
        "tier":    {{ input.customer.tier }},
        "country": {{ input.customer.country }}
    },
    "order": {
        "currency": {{ input.order.currency }},
        "subtotal": {{ input.order.subtotal }},
        "tax":      {{ input.order.tax }},
        "total":    {{ input.order.total }},
        "discount": {{ input.order.discount }}
    },
    "items": [
        {
            "sku":   {{ input.items.first.sku }},
            "name":  {{ input.items.first.name }},
            "qty":   {{ input.items.first.qty }},
            "price": {{ input.items.first.price }}
        },
        {
            "sku":   {{ input.items.second.sku }},
            "name":  {{ input.items.second.name }},
            "qty":   {{ input.items.second.qty }},
            "price": {{ input.items.second.price }}
        }
    ],
    "metadata": {
        "source":    {{ input.metadata.source }},
        "session":   {{ input.metadata.session }},
        "timestamp": {{ input.metadata.timestamp }}
    }
}"#;

#[allow(dead_code)]
#[derive(Deserialize)]
struct BigOut {
    request_id: String,
    idempotency_key: String,
    method: String,
    url: String,
    headers: Headers,
    customer: Customer,
    order: Order,
    items: Vec<Item>,
    metadata: Metadata,
}
#[allow(dead_code)]
#[derive(Deserialize)]
struct Headers {
    authorization: String,
    content_type: String,
    user_agent: String,
    trace_id: String,
}
#[allow(dead_code)]
#[derive(Deserialize)]
struct Customer {
    id: String,
    email: String,
    name: String,
    tier: String,
    country: String,
}
#[allow(dead_code)]
#[derive(Deserialize)]
struct Order {
    currency: String,
    subtotal: Decimal,
    tax: Decimal,
    total: Decimal,
    discount: Decimal,
}
#[allow(dead_code)]
#[derive(Deserialize)]
struct Item {
    sku: String,
    name: String,
    qty: i64,
    price: Decimal,
}
#[allow(dead_code)]
#[derive(Deserialize)]
struct Metadata {
    source: String,
    session: String,
    timestamp: String,
}

fn big_input() -> Value {
    Value::obj([
        ("request_id", Value::Str("req_8a92f3".into())),
        (
            "endpoint",
            Value::Str("https://api.example.com/v1/charges".into()),
        ),
        (
            "auth",
            Value::obj([("token", Value::Str("Bearer sk_live_abc123".into()))]),
        ),
        (
            "client",
            Value::obj([
                ("user_agent", Value::Str("temple/0.1".into())),
                ("trace_id", Value::Str("trace_xyz789".into())),
            ]),
        ),
        (
            "customer",
            Value::obj([
                ("id", Value::Str("cus_8842".into())),
                ("email", Value::Str("ada@example.com".into())),
                ("name", Value::Str("Ada Lovelace".into())),
                ("tier", Value::Str("gold".into())),
                ("country", Value::Str("UK".into())),
            ]),
        ),
        (
            "order",
            Value::obj([
                ("currency", Value::Str("USD".into())),
                ("subtotal", Value::Decimal("129.99".parse().unwrap())),
                ("tax", Value::Decimal("10.72".parse().unwrap())),
                ("total", Value::Decimal("140.71".parse().unwrap())),
                ("discount", Value::Decimal("0.00".parse().unwrap())),
            ]),
        ),
        (
            "items",
            Value::obj([
                (
                    "first",
                    Value::obj([
                        ("sku", Value::Str("widget-blue".into())),
                        ("name", Value::Str("Blue Widget".into())),
                        ("qty", Value::Int(2)),
                        ("price", Value::Decimal("49.99".parse().unwrap())),
                    ]),
                ),
                (
                    "second",
                    Value::obj([
                        ("sku", Value::Str("gizmo-pro".into())),
                        ("name", Value::Str("Gizmo Pro".into())),
                        ("qty", Value::Int(1)),
                        ("price", Value::Decimal("30.01".parse().unwrap())),
                    ]),
                ),
            ]),
        ),
        (
            "metadata",
            Value::obj([
                ("source", Value::Str("checkout-page".into())),
                ("session", Value::Str("sess_4f9k2l".into())),
                ("timestamp", Value::Str("2026-05-26T10:30:00Z".into())),
            ]),
        ),
    ])
}

// -------- benchmarks --------

fn benches(c: &mut Criterion) {
    c.bench_function("compile_small", |b| {
        b.iter(|| Template::compile(black_box(SMALL_TEMPLATE)).unwrap())
    });
    c.bench_function("compile_big", |b| {
        b.iter(|| Template::compile(black_box(BIG_TEMPLATE)).unwrap())
    });

    let small_t = Template::compile(SMALL_TEMPLATE).unwrap();
    let small_in = small_input();
    c.bench_function("render_small", |b| {
        b.iter_batched(
            || small_in.clone(),
            |input| {
                let _: SmallOut = small_t.render(input).unwrap();
            },
            BatchSize::SmallInput,
        )
    });

    let big_t = Template::compile(BIG_TEMPLATE).unwrap();
    let big_in = big_input();
    c.bench_function("render_big", |b| {
        b.iter_batched(
            || big_in.clone(),
            |input| {
                let _: BigOut = big_t.render(input).unwrap();
            },
            BatchSize::SmallInput,
        )
    });

    c.bench_function("compile_and_render_small", |b| {
        b.iter_batched(
            || small_in.clone(),
            |input| {
                let t = Template::compile(SMALL_TEMPLATE).unwrap();
                let _: SmallOut = t.render(input).unwrap();
            },
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(temple_benches, benches);
criterion_main!(temple_benches);
