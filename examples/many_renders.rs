#![allow(dead_code)]

use serde::Deserialize;
use temple_dsl::{Template, Value};

#[derive(Deserialize, Debug)]
struct Event {
    user: String,
    action: String,
    target: String,
}

fn main() {
    let src = r#"{
        "user":   {{ input.user }},
        "action": {{ input.action }},
        "target": {{ input.target }}
    }"#;

    // Compile once.
    let template = Template::compile(src).expect("compile");

    // Render against many inputs.
    let events = [
        ("ada", "viewed", "dashboard"),
        ("babbage", "edited", "report-42"),
        ("turing", "deleted", "post-7"),
    ];

    for (user, action, target) in events {
        let input = Value::obj([
            ("user", Value::Str(user.into())),
            ("action", Value::Str(action.into())),
            ("target", Value::Str(target.into())),
        ]);
        let event: Event = template.render(input).expect("render");
        println!("{:?}", event);
    }
}
