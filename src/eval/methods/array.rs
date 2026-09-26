use super::{slice_bounds, Call, Method};
use crate::common::error::{RenderError, Span};
use crate::common::value::Value;
use crate::eval::budget::Meter;
use crate::eval::builtins::{fold_compare, scalar_to_string};
use crate::eval::ops::{arith, compare, values_equal};
use rust_decimal::Decimal;
use std::cmp::Ordering;

pub(super) fn call<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    let n = items.len() as u64;
    let run = |cost, method: Method<[Value], M>| {
        call.charge(cost)?;
        method(items, call)
    };
    match call.name.as_str() {
        "len" => run(0, len),
        "first" => run(0, first),
        "last" => run(0, last),
        "sum" => run(n, sum),
        "avg" => run(n, avg),
        "min" => run(n, min),
        "max" => run(n, max),
        "contains" => run(n, contains),
        "index_of" => run(n, index_of),
        "find" => run(0, find),
        "count" => run(0, count),
        "any" => run(0, any),
        "all" => run(0, all),
        "map" => run(0, map),
        "filter" => run(0, filter),
        "fold" => run(0, fold),
        "flat_map" => run(0, flat_map),
        "concat" => run(0, concat),
        "reverse" => run(0, reverse),
        "unique" => run(0, unique),
        "sort" => run(sort_cost(n), sort),
        "sort_by" => run(sort_cost(n), sort_by),
        "flatten" => run(n, flatten),
        "take" => run(0, take),
        "drop" => run(0, drop),
        "slice" => run(0, slice),
        "join" => run(n, join),
        _ => Err(RenderError::unknown_method(call.name, "array", call.span).into()),
    }
}

fn len<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    Ok(Value::Int(items.len() as i64))
}

fn first<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    match items.first() {
        Some(v) => call.copy(v),
        None => Ok(Value::Null),
    }
}

fn last<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    match items.last() {
        Some(v) => call.copy(v),
        None => Ok(Value::Null),
    }
}

fn sum<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    Ok(add_all(items, call.span)?)
}

fn avg<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    if items.is_empty() {
        return Err(RenderError::type_mismatch("non-empty array", "empty array", call.span).into());
    }
    let total = match add_all(items, call.span)? {
        Value::Int(n) => Decimal::from(n),
        Value::Decimal(d) => d,
        other => return Err(RenderError::type_mismatch("number", other.kind(), call.span).into()),
    };
    total
        .checked_div(Decimal::from(items.len() as i64))
        .map(Value::Decimal)
        .ok_or_else(|| RenderError::ArithmeticOverflow { span: call.span }.into())
}

fn min<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    min_max(items, call, Ordering::Less)
}

fn max<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    min_max(items, call, Ordering::Greater)
}

fn min_max<M: Meter>(
    items: &[Value],
    call: &Call<M>,
    keep_when: Ordering,
) -> Result<Value, M::Err> {
    call.arity(0)?;
    if items.is_empty() {
        return Err(RenderError::type_mismatch("non-empty array", "empty array", call.span).into());
    }
    Ok(fold_compare(items, call.span, keep_when)?)
}

fn contains<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let needle = call.arg(0)?;
    Ok(Value::Bool(items.iter().any(|e| values_equal(e, &needle))))
}

fn index_of<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let needle = call.arg(0)?;
    let position = items.iter().position(|e| values_equal(e, &needle));
    Ok(Value::Int(position.map_or(-1, |i| i as i64)))
}

fn find<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let test = call.lambda(0, 1)?;
    for item in items {
        if test.test(item, call)? {
            return call.copy(item);
        }
    }
    Ok(Value::Null)
}

fn count<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let test = call.lambda(0, 1)?;
    let mut count = 0i64;
    for item in items {
        if test.test(item, call)? {
            count += 1;
        }
    }
    Ok(Value::Int(count))
}

fn any<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let test = call.lambda(0, 1)?;
    for item in items {
        if test.test(item, call)? {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn all<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let test = call.lambda(0, 1)?;
    for item in items {
        if !test.test(item, call)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn map<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let mapper = call.lambda(0, 1)?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(mapper.run1(item, call)?);
    }
    Ok(Value::Arr(out))
}

fn filter<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let test = call.lambda(0, 1)?;
    let mut out = Vec::new();
    for item in items {
        if test.test(item, call)? {
            out.push(call.copy(item)?);
        }
    }
    Ok(Value::Arr(out))
}

fn fold<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(2)?;
    let mut acc = call.arg(0)?;
    let step = call.lambda(1, 2)?;
    for item in items {
        acc = step.run2(&acc, item, call)?;
    }
    Ok(acc)
}

fn flat_map<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let mapper = call.lambda(0, 1)?;
    let mut out = Vec::new();
    for item in items {
        match mapper.run1(item, call)?.into_arr() {
            Ok(mut inner) => out.append(&mut inner),
            Err(other) => {
                return Err(RenderError::type_mismatch(
                    "array from flat_map lambda",
                    other.kind(),
                    call.span,
                )
                .into())
            }
        }
    }
    Ok(Value::Arr(out))
}

fn concat<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    match call.arg(0)?.into_arr() {
        Ok(mut tail) => {
            let mut out: Vec<Value> = call.copy_all(items)?.collect();
            out.append(&mut tail);
            Ok(Value::Arr(out))
        }
        Err(other) => Err(RenderError::type_mismatch("array", other.kind(), call.span).into()),
    }
}

fn reverse<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    Ok(Value::Arr(call.copy_all(items)?.rev().collect()))
}

fn unique<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    let mut out: Vec<Value> = Vec::new();
    for item in items {
        // each item is compared with every one kept so far
        call.charge(out.len() as u64 + 1)?;
        if !out.iter().any(|kept| values_equal(kept, item)) {
            out.push(call.copy(item)?);
        }
    }
    Ok(Value::Arr(out))
}

fn sort<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    let mut out: Vec<Value> = call.copy_all(items)?.collect();
    check_sortable(out.iter(), call.span)?;
    out.sort_by(compare_sort_keys);
    Ok(Value::Arr(out))
}

fn sort_by<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let key_of = call.lambda(0, 1)?;
    let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(items.len());
    for item in items {
        let key = key_of.run1(item, call)?;
        keyed.push((key, call.copy(item)?));
    }
    check_sortable(keyed.iter().map(|(key, _)| key), call.span)?;
    keyed.sort_by(|(a, _), (b, _)| compare_sort_keys(a, b));
    Ok(Value::Arr(
        keyed.into_iter().map(|(_, item)| item).collect(),
    ))
}

/// Sorting n items compares about n·log₂n pairs.
fn sort_cost(n: u64) -> u64 {
    n.saturating_mul(64 - n.leading_zeros() as u64)
}

fn flatten<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(0)?;
    let mut out = Vec::new();
    for item in items {
        match item {
            Value::Arr(inner) => out.extend(call.copy_all(inner)?),
            other => {
                return Err(
                    RenderError::type_mismatch("array of arrays", other.kind(), call.span).into(),
                )
            }
        }
    }
    Ok(Value::Arr(out))
}

fn take<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let n = call.int_arg(0)?.max(0) as usize;
    Ok(Value::Arr(
        call.copy_all(&items[..n.min(items.len())])?.collect(),
    ))
}

fn drop<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let n = call.int_arg(0)?.max(0) as usize;
    Ok(Value::Arr(
        call.copy_all(&items[n.min(items.len())..])?.collect(),
    ))
}

fn slice<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(2)?;
    let (from, to) = slice_bounds(call.int_arg(0)?, call.int_arg(1)?, items.len());
    Ok(Value::Arr(call.copy_all(&items[from..to])?.collect()))
}

fn join<M: Meter>(items: &[Value], call: &Call<M>) -> Result<Value, M::Err> {
    call.arity(1)?;
    let separator = call.string_arg(0)?;
    // items can repeat one long string, so the text can far outgrow the
    // list: charge it before building
    let text_len: usize = items
        .iter()
        .map(|item| match item {
            Value::Str(s) => s.len(),
            _ => 24,
        })
        .sum::<usize>()
        .saturating_add(
            separator
                .len()
                .saturating_mul(items.len().saturating_sub(1)),
        );
    call.charge((text_len / 64) as u64)?;
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(&separator);
        }
        match item {
            Value::Str(s) => out.push_str(s),
            other => out.push_str(&scalar_to_string(other, call.span)?),
        }
    }
    Ok(Value::Str(out.into()))
}

fn add_all(items: &[Value], span: Span) -> Result<Value, RenderError> {
    let mut total = Value::Int(0);
    for item in items {
        total = arith(&total, item, span, i64::checked_add, Decimal::checked_add)?;
    }
    Ok(total)
}

fn check_sortable<'v>(
    keys: impl Iterator<Item = &'v Value>,
    span: Span,
) -> Result<(), RenderError> {
    let mut saw_number = false;
    let mut saw_string = false;
    for key in keys {
        match key {
            Value::Int(_) | Value::Decimal(_) => saw_number = true,
            Value::Str(_) => saw_string = true,
            other => {
                return Err(RenderError::type_mismatch(
                    "sortable items (numbers or strings)",
                    other.kind(),
                    span,
                ))
            }
        }
    }
    if saw_number && saw_string {
        return Err(RenderError::TypeMismatch {
            expected: "comparable items (all numbers or all strings)",
            got: "mixed numbers and strings".to_string(),
            span,
        });
    }
    Ok(())
}

/// Compare sort keys that `check_sortable` has already vetted.
fn compare_sort_keys(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => x.cmp(y),
        _ => compare(a, b, Span::new(0, 0)).unwrap_or(Ordering::Equal),
    }
}
