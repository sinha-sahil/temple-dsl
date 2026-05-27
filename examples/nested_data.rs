#![allow(dead_code)]

use serde::Deserialize;
use temple_dsl::{Template, Value};

#[derive(Deserialize, Debug)]
struct Person {
    name: String,
    email: String,
    city: String,
    country: String,
}

fn main() {
    let src = r#"{
        "name":    {{ input.profile.contact.name }},
        "email":   {{ input.profile.contact.email }},
        "city":    {{ input.profile.address.city }},
        "country": {{ input.profile.address.country }}
    }"#;

    let template = Template::compile(src).expect("compile");

    let input = Value::obj([(
        "profile",
        Value::obj([
            (
                "contact",
                Value::obj([
                    ("name", Value::Str("Ada".into())),
                    ("email", Value::Str("ada@example.com".into())),
                ]),
            ),
            (
                "address",
                Value::obj([
                    ("city", Value::Str("London".into())),
                    ("country", Value::Str("UK".into())),
                ]),
            ),
        ]),
    )]);

    let person: Person = template.render(input).expect("render");
    println!("{person:#?}");
}
