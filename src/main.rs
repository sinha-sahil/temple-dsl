use std::{env, fs, process::ExitCode};
use temple_dsl::{CompileError, Template};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("fmt") if args.len() == 3 => run_fmt(&args[2]),
        Some(_) if args.len() == 3 => run_render(&args[1], &args[2]),
        _ => {
            eprintln!(
                "usage:\n  temple <template.temple> <input.json>   render against a JSON input\n  temple fmt <template.temple>             print the canonical formatting"
            );
            ExitCode::from(2)
        }
    }
}

fn run_render(template_path: &str, input_path: &str) -> ExitCode {
    let src = match fs::read_to_string(template_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read template: {e}");
            return ExitCode::from(1);
        }
    };
    let input_text = match fs::read_to_string(input_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read input: {e}");
            return ExitCode::from(1);
        }
    };

    let template = match Template::compile(&src) {
        Ok(t) => t,
        Err(errs) => {
            eprintln!("{}", CompileError::report_all(&src, &errs));
            return ExitCode::from(1);
        }
    };

    let input: serde_json::Value = match serde_json::from_str(&input_text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("parse input json: {e}");
            return ExitCode::from(1);
        }
    };

    // serde_json::Value flows straight in via Into<Value>; converting the
    // result back emits decimals as exact JSON number tokens.
    let output = match template.render_value(input) {
        Ok(v) => serde_json::Value::from(v),
        Err(e) => {
            eprintln!("render error: {e}");
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

fn run_fmt(template_path: &str) -> ExitCode {
    let src = match fs::read_to_string(template_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read template: {e}");
            return ExitCode::from(1);
        }
    };
    match Template::format(&src) {
        Ok(formatted) => {
            print!("{formatted}");
            ExitCode::SUCCESS
        }
        Err(errs) => {
            eprintln!("{}", CompileError::report_all(&src, &errs));
            ExitCode::from(1)
        }
    }
}
