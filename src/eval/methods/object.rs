use super::{Call, Method};
use crate::common::error::RenderError;
use crate::common::value::Value;
use crate::eval::budget::Meter;
use indexmap::IndexMap;
use smol_str::SmolStr;

pub(super) fn call<M: Meter>(
    object: &IndexMap<SmolStr, Value>,
    call: &Call<M>,
) -> Result<Value, M::Err> {
    let n = object.len() as u64;
    let run = |cost, method: Method<IndexMap<SmolStr, Value>, M>| {
        call.charge(cost)?;
        method(object, call)
    };
    match call.name.as_str() {
        "keys" => run(n, keys),
        "values" => run(0, values),
        "entries" => run(n, entries),
        "has" => run(0, has),
        "get" => run(0, get),
        "merge" => run(0, merge),
        _ => Err(RenderError::unknown_method(call.name, "object", call.span).into()),
    }
}

fn keys<M: Meter>(obj: &IndexMap<SmolStr, Value>, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    Ok(Value::Arr(
        obj.keys().map(|k| Value::Str(k.clone())).collect(),
    ))
}

fn values<M: Meter>(obj: &IndexMap<SmolStr, Value>, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    Ok(Value::Arr(call.copy_all(obj.values())?.collect()))
}

fn entries<M: Meter>(obj: &IndexMap<SmolStr, Value>, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    let mut entries = Vec::with_capacity(obj.len());
    for (key, value) in obj {
        let value = call.copy(value)?;
        entries.push(Value::obj([
            ("key", Value::Str(key.clone())),
            ("value", value),
        ]));
    }
    Ok(Value::Arr(entries))
}

fn has<M: Meter>(obj: &IndexMap<SmolStr, Value>, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let key = call.string_arg(0)?;
    Ok(Value::Bool(obj.contains_key(key.as_str())))
}

/// Lenient read: null when the key is missing (pairs with `??`).
fn get<M: Meter>(obj: &IndexMap<SmolStr, Value>, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let key = call.string_arg(0)?;
    match obj.get(key.as_str()) {
        Some(value) => call.copy(value),
        None => Ok(Value::Null),
    }
}

fn merge<M: Meter>(obj: &IndexMap<SmolStr, Value>, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    match call.arg(0)?.into_obj() {
        Ok(extra) => {
            let mut merged = call.copy_object(obj)?;
            merged.extend(extra);
            Ok(Value::Obj(merged))
        }
        Err(other) => Err(RenderError::type_mismatch("object", other.kind(), call.span).into()),
    }
}
