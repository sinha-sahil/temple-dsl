use serde::Deserialize;
use temple_dsl::{Template, Value};

#[derive(Deserialize, Debug)]
struct Out {
    #[allow(dead_code)]
    x: String,
}

fn main() {
    let bad_template = r#"{ "x": {{ input.broken }"#;
    match Template::compile(bad_template) {
        Ok(_) => unreachable!("expected compile to fail"),
        Err(errors) => {
            println!("Compile failed with {} error(s):", errors.len());
            for e in &errors {
                println!("  • {}", e);
            }
        }
    }

    println!();

    let good_template = r#"{ "x": {{ input.missing_field }} }"#;
    let template = Template::compile(good_template).expect("compile");

    let input = Value::obj([("present", Value::Str("ok".into()))]);

    match template.render::<Out>(input) {
        Ok(_) => unreachable!("expected render to fail"),
        Err(e) => println!("Render failed: {}", e),
    }
}
