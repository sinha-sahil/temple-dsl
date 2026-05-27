use rust_decimal::Decimal;
use smol_str::SmolStr;
use std::{env, fs, process::ExitCode};
use temple_dsl::{Template, Value};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: temple <template.temple> <input.json>");
        return ExitCode::from(2);
    }

    let src = match fs::read_to_string(&args[1]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read template: {e}");
            return ExitCode::from(1);
        }
    };
    let input_text = match fs::read_to_string(&args[2]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read input: {e}");
            return ExitCode::from(1);
        }
    };

    let template = match Template::compile(&src) {
        Ok(t) => t,
        Err(errs) => {
            for e in &errs {
                eprintln!("compile: {e}");
            }
            return ExitCode::from(1);
        }
    };

    let input_json: serde_json::Value = match serde_json::from_str(&input_text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("parse input json: {e}");
            return ExitCode::from(1);
        }
    };
    let input = json_to_value(input_json);

    let output: serde_json::Value = match template.render(input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("render: {e}");
            return ExitCode::from(1);
        }
    };

    match serde_json::to_string_pretty(&output) {
        Ok(s) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("write output: {e}");
            ExitCode::from(1)
        }
    }
}

fn json_to_value(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                f.to_string()
                    .parse::<Decimal>()
                    .map(Value::Decimal)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        serde_json::Value::String(s) => Value::Str(s.into()),
        serde_json::Value::Array(a) => Value::Arr(a.into_iter().map(json_to_value).collect()),
        serde_json::Value::Object(o) => Value::Obj(
            o.into_iter()
                .map(|(k, v)| (SmolStr::new(&k), json_to_value(v)))
                .collect(),
        ),
    }
}
