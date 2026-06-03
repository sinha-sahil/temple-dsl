#![allow(dead_code)]

use serde::Deserialize;
use temple_dsl::{Template, Value};

#[derive(Deserialize, Debug)]
struct Receipt {
    customer: String,
    total: rust_decimal::Decimal,
}

fn main() {
    let src = r#"
        let items = input.items
        {
            "customer": {{ input.customer }},
            "total":    {{ items.map(it -> it.qty * it.price).sum() }}
        }
    "#;

    let compiled = Template::compile(src).expect("compile");
    let blob: Vec<u8> = compiled.to_bytes();
    println!("compiled blob: {} bytes", blob.len());

    let template = Template::from_bytes(&blob).expect("load");

    let input = Value::obj([
        ("customer", Value::Str("Ada".into())),
        (
            "items",
            Value::Arr(vec![
                Value::obj([
                    ("qty", Value::Int(3)),
                    ("price", Value::Decimal("9.99".parse().unwrap())),
                ]),
                Value::obj([
                    ("qty", Value::Int(1)),
                    ("price", Value::Decimal("4.50".parse().unwrap())),
                ]),
            ]),
        ),
    ]);

    let receipt: Receipt = template.render(input).expect("render");
    println!("{receipt:?}");
}
