mod budget;
mod depth;
mod eval;
mod functions;
mod names;
mod parse;
mod queries;

use temple_dsl::{
    Budget, CompileError, Env, EvalError, ExprUnit, Functions, InputRead, Names, TemplateUnit,
    Value,
};

fn obj(pairs: &[(&str, Value)]) -> Value {
    Value::obj(pairs.iter().map(|(k, v)| (*k, v.clone())))
}

fn arr(items: Vec<Value>) -> Value {
    Value::Arr(items)
}

fn msg(errs: &[CompileError]) -> String {
    errs.iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run `f` on a thread with the 2 MB stack Rust gives spawned threads.
#[cfg(not(target_family = "wasm"))]
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// WebAssembly has no threads; its main stack is smaller still.
#[cfg(target_family = "wasm")]
fn on_small_stack<T>(f: impl FnOnce() -> T) -> T {
    f()
}
