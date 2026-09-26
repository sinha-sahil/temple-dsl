use super::{slice_bounds, Call, Method};
use crate::common::error::RenderError;
use crate::common::value::Value;
use crate::eval::budget::Meter;
use smol_str::SmolStr;

pub(super) fn call<M: Meter>(text: &SmolStr, call: &Call<M>) -> Result<Value, M::Err> {
    let scan = text_cost(text);
    let run = |cost, method: Method<SmolStr, M>| {
        call.charge(cost)?;
        method(text, call)
    };
    match call.name.as_str() {
        "len" => run(scan, len),
        "contains" => run(scan, contains),
        "starts_with" => run(scan, starts_with),
        "ends_with" => run(scan, ends_with),
        "replace" => run(scan, replace),
        "split" => run(scan, split),
        "slice" => run(scan, slice),
        "index_of" => run(scan, index_of),
        _ => Err(RenderError::unknown_method(call.name, "string", call.span).into()),
    }
}

fn text_cost(s: &str) -> u64 {
    (s.len() / 64) as u64 + 1
}

fn len<M: Meter>(s: &SmolStr, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    Ok(Value::Int(s.chars().count() as i64))
}

fn contains<M: Meter>(s: &SmolStr, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let needle = call.string_arg(0)?;
    Ok(Value::Bool(s.contains(needle.as_str())))
}

fn starts_with<M: Meter>(s: &SmolStr, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let prefix = call.string_arg(0)?;
    Ok(Value::Bool(s.starts_with(prefix.as_str())))
}

fn ends_with<M: Meter>(s: &SmolStr, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let suffix = call.string_arg(0)?;
    Ok(Value::Bool(s.ends_with(suffix.as_str())))
}

fn replace<M: Meter>(s: &SmolStr, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(2)?;
    let from = call.string_arg(0)?;
    let to = call.string_arg(1)?;
    // a short `from` and a long `to` can far outgrow the text: charge first
    let hits = if from.is_empty() {
        s.chars().count() + 1
    } else {
        s.matches(from.as_str()).count()
    };
    let out_len = (s.len() - hits * from.len()).saturating_add(hits.saturating_mul(to.len()));
    call.charge((out_len / 64) as u64)?;
    Ok(Value::Str(s.replace(from.as_str(), to.as_str()).into()))
}

fn split<M: Meter>(s: &SmolStr, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let separator = call.string_arg(0)?;
    // a part per byte is many times the text's size: charge first
    let parts = if separator.is_empty() {
        s.chars().count()
    } else {
        s.matches(separator.as_str()).count() + 1
    };
    call.charge(parts as u64)?;
    let parts: Vec<Value> = if separator.is_empty() {
        s.chars()
            .map(|c| Value::Str(c.to_string().into()))
            .collect()
    } else {
        s.split(separator.as_str())
            .map(|part| Value::Str(part.into()))
            .collect()
    };
    Ok(Value::Arr(parts))
}

fn slice<M: Meter>(s: &SmolStr, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(2)?;
    let chars: Vec<char> = s.chars().collect();
    let (from, to) = slice_bounds(call.int_arg(0)?, call.int_arg(1)?, chars.len());
    Ok(Value::Str(
        chars[from..to].iter().collect::<String>().into(),
    ))
}

fn index_of<M: Meter>(s: &SmolStr, call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let needle = call.string_arg(0)?;
    Ok(Value::Int(match s.find(needle.as_str()) {
        Some(byte) => s[..byte].chars().count() as i64,
        None => -1,
    }))
}
