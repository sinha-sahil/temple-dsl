#![allow(dead_code)]

use serde::Deserialize;
use temple_dsl::{Template, Value};

const TEMPLATE: &str = r#"{
    "id":        {{ input.data.user.identifiers.uuid }},
    "email":     {{ input.data.user.contact.email_primary }},
    "full_name": {{ input.data.user.profile.display_name }},
    "country":   {{ input.data.user.location.address.country_code }},
    "city":      {{ input.data.user.location.address.city }},
    "tier":      {{ input.data.subscription.plan.tier_label }},
    "active":    {{ input.data.subscription.status.is_active }},
    "addresses": [
        {
            "type":    "billing",
            "street":  {{ input.data.user.billing.street }},
            "city":    {{ input.data.user.billing.city }},
            "country": {{ input.data.user.billing.country }}
        },
        {
            "type":    "shipping",
            "street":  {{ input.data.user.shipping.street }},
            "city":    {{ input.data.user.shipping.city }},
            "country": {{ input.data.user.shipping.country }}
        }
    ]
}"#;

#[derive(Deserialize, Debug)]
struct UserCanonical {
    id: String,
    email: String,
    full_name: String,
    country: String,
    city: String,
    tier: String,
    active: bool,
    addresses: Vec<Address>,
}

#[derive(Deserialize, Debug)]
struct Address {
    #[serde(rename = "type")]
    kind: String,
    street: String,
    city: String,
    country: String,
}

fn external_response() -> Value {
    Value::obj([(
        "data",
        Value::obj([
            (
                "user",
                Value::obj([
                    (
                        "identifiers",
                        Value::obj([("uuid", Value::Str("u_8a92f3".into()))]),
                    ),
                    (
                        "contact",
                        Value::obj([("email_primary", Value::Str("ada@example.com".into()))]),
                    ),
                    (
                        "profile",
                        Value::obj([("display_name", Value::Str("Ada Lovelace".into()))]),
                    ),
                    (
                        "location",
                        Value::obj([(
                            "address",
                            Value::obj([
                                ("country_code", Value::Str("UK".into())),
                                ("city", Value::Str("London".into())),
                            ]),
                        )]),
                    ),
                    (
                        "billing",
                        Value::obj([
                            ("street", Value::Str("1 Lovelace Lane".into())),
                            ("city", Value::Str("London".into())),
                            ("country", Value::Str("UK".into())),
                        ]),
                    ),
                    (
                        "shipping",
                        Value::obj([
                            ("street", Value::Str("221B Baker St".into())),
                            ("city", Value::Str("London".into())),
                            ("country", Value::Str("UK".into())),
                        ]),
                    ),
                ]),
            ),
            (
                "subscription",
                Value::obj([
                    (
                        "plan",
                        Value::obj([("tier_label", Value::Str("gold".into()))]),
                    ),
                    ("status", Value::obj([("is_active", Value::Bool(true))])),
                ]),
            ),
        ]),
    )])
}

fn main() {
    let template = Template::compile(TEMPLATE).expect("compile");
    let response = external_response();
    let user: UserCanonical = template.render(response).expect("render");
    println!("{:#?}", user);
}
