#![allow(dead_code)]

use serde::Deserialize;
use temple_dsl::{Template, Value};

#[derive(Deserialize, Debug)]
struct Greeting {
    greeting: String,
}

fn main() {
    let src = r#"{ "greeting": {{ input.name }} }"#;

    let template = Template::compile(src).expect("template should parse");
    let input = Value::obj([("name", Value::Str("world".into()))]);

    let output: Greeting = template.render(input).expect("render");
    println!("{output:?}");
}
