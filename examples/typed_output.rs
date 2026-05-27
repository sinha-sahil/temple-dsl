#![allow(dead_code)]

use rust_decimal::Decimal;
use serde::Deserialize;
use temple_dsl::{Template, Value};

#[derive(Deserialize, Debug)]
struct Receipt {
    customer: String,
    item_count: i64,
    subtotal: Decimal,
    paid: bool,
}

fn main() {
    let src = r#"{
        "customer":   {{ input.customer.name }},
        "item_count": {{ input.cart.item_count }},
        "subtotal":   {{ input.cart.subtotal }},
        "paid":       true
    }"#;

    let template = Template::compile(src).expect("compile");

    let input = Value::obj([
        (
            "customer",
            Value::obj([("name", Value::Str("Ada Lovelace".into()))]),
        ),
        (
            "cart",
            Value::obj([
                ("item_count", Value::Int(3)),
                (
                    "subtotal",
                    Value::Decimal("129.99".parse().expect("decimal literal")),
                ),
            ]),
        ),
    ]);

    let receipt: Receipt = template.render(input).expect("render");
    println!("{:#?}", receipt);
}
