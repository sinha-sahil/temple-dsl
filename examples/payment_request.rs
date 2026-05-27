#![allow(dead_code)]

use rust_decimal::Decimal;
use serde::Deserialize;
use temple_dsl::{Template, Value};

const TEMPLATE: &str = r#"{
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

#[derive(Deserialize, Debug)]
struct Request {
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

#[derive(Deserialize, Debug)]
struct Headers {
    authorization: String,
    content_type: String,
    user_agent: String,
    trace_id: String,
}

#[derive(Deserialize, Debug)]
struct Customer {
    id: String,
    email: String,
    name: String,
    tier: String,
    country: String,
}

#[derive(Deserialize, Debug)]
struct Order {
    currency: String,
    subtotal: Decimal,
    tax: Decimal,
    total: Decimal,
    discount: Decimal,
}

#[derive(Deserialize, Debug)]
struct Item {
    sku: String,
    name: String,
    qty: i64,
    price: Decimal,
}

#[derive(Deserialize, Debug)]
struct Metadata {
    source: String,
    session: String,
    timestamp: String,
}

fn build_input() -> Value {
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
                (
                    "subtotal",
                    Value::Decimal("129.99".parse().expect("decimal")),
                ),
                ("tax", Value::Decimal("10.72".parse().expect("decimal"))),
                ("total", Value::Decimal("140.71".parse().expect("decimal"))),
                ("discount", Value::Decimal("0.00".parse().expect("decimal"))),
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
                        ("price", Value::Decimal("49.99".parse().expect("decimal"))),
                    ]),
                ),
                (
                    "second",
                    Value::obj([
                        ("sku", Value::Str("gizmo-pro".into())),
                        ("name", Value::Str("Gizmo Pro".into())),
                        ("qty", Value::Int(1)),
                        ("price", Value::Decimal("30.01".parse().expect("decimal"))),
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

fn main() {
    let template = Template::compile(TEMPLATE).expect("compile");
    let input = build_input();
    let request: Request = template.render(input).expect("render");
    println!("{request:#?}");
}
