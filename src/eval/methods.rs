//! The method stdlib, split by receiver type — `str_method`, `arr_method`,
//! `obj_method` — plus the lambda binding that backs the iterating methods.

use super::functions::{fold_compare, scalar_to_string, type_err};
use super::ops::{arith, compare, require_bool, values_equal};
use super::{evaluate_expr, Scope, Scopes};
use crate::error::{RenderError, Span};
use crate::parse::{Expr, ExprKind, LambdaParam};
use crate::value::Value;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use smol_str::SmolStr;
use std::cmp::Ordering;

pub(super) fn call_method(
    receiver: &Value,
    method: &SmolStr,
    args: &[Expr],
    span: Span,
    input: &Value,
    lets: &Scopes,
    this: &Scope,
) -> Result<Value, RenderError> {
    match receiver {
        Value::Str(s) => str_method(s, method, args, span, input, lets, this),
        Value::Arr(a) => arr_method(a, method, args, span, input, lets, this),
        Value::Obj(o) => obj_method(o, method, args, span, input, lets, this),
        other => Err(unknown_method(method, other.kind(), span)),
    }
}

fn str_method(
    s: &SmolStr,
    method: &SmolStr,
    args: &[Expr],
    span: Span,
    input: &Value,
    lets: &Scopes,
    this: &Scope,
) -> Result<Value, RenderError> {
    match method.as_str() {
        "len" => {
            check_arity(method, args, 0, span)?;
            Ok(Value::Int(s.chars().count() as i64))
        }
        "contains" => {
            check_arity(method, args, 1, span)?;
            let sub = string_arg(&args[0], input, lets, this)?;
            Ok(Value::Bool(s.contains(sub.as_str())))
        }
        "starts_with" => {
            check_arity(method, args, 1, span)?;
            let p = string_arg(&args[0], input, lets, this)?;
            Ok(Value::Bool(s.starts_with(p.as_str())))
        }
        "ends_with" => {
            check_arity(method, args, 1, span)?;
            let p = string_arg(&args[0], input, lets, this)?;
            Ok(Value::Bool(s.ends_with(p.as_str())))
        }
        "replace" => {
            check_arity(method, args, 2, span)?;
            let from = string_arg(&args[0], input, lets, this)?;
            let to = string_arg(&args[1], input, lets, this)?;
            Ok(Value::Str(s.replace(from.as_str(), to.as_str()).into()))
        }
        "split" => {
            check_arity(method, args, 1, span)?;
            let sep = string_arg(&args[0], input, lets, this)?;
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars()
                    .map(|c| Value::Str(c.to_string().into()))
                    .collect()
            } else {
                s.split(sep.as_str())
                    .map(|p| Value::Str(p.into()))
                    .collect()
            };
            Ok(Value::Arr(parts))
        }
        "slice" => {
            check_arity(method, args, 2, span)?;
            let chars: Vec<char> = s.chars().collect();
            let (a, b) = slice_bounds(
                int_arg(&args[0], input, lets, this)?,
                int_arg(&args[1], input, lets, this)?,
                chars.len(),
            );
            Ok(Value::Str(chars[a..b].iter().collect::<String>().into()))
        }
        "index_of" => {
            check_arity(method, args, 1, span)?;
            let sub = string_arg(&args[0], input, lets, this)?;
            Ok(Value::Int(match s.find(sub.as_str()) {
                Some(byte) => s[..byte].chars().count() as i64,
                None => -1,
            }))
        }
        _ => Err(unknown_method(method, "string", span)),
    }
}

fn arr_method(
    arr: &[Value],
    method: &SmolStr,
    args: &[Expr],
    span: Span,
    input: &Value,
    lets: &Scopes,
    this: &Scope,
) -> Result<Value, RenderError> {
    match method.as_str() {
        "len" => {
            check_arity(method, args, 0, span)?;
            Ok(Value::Int(arr.len() as i64))
        }
        "first" => {
            check_arity(method, args, 0, span)?;
            Ok(arr.first().cloned().unwrap_or(Value::Null))
        }
        "last" => {
            check_arity(method, args, 0, span)?;
            Ok(arr.last().cloned().unwrap_or(Value::Null))
        }

        // aggregates
        "sum" => {
            check_arity(method, args, 0, span)?;
            sum_values(arr, span)
        }
        "avg" => {
            check_arity(method, args, 0, span)?;
            if arr.is_empty() {
                return Err(empty_array(span));
            }
            let total = match sum_values(arr, span)? {
                Value::Int(n) => Decimal::from(n),
                Value::Decimal(d) => d,
                _ => unreachable!("sum of numbers is a number"),
            };
            total
                .checked_div(Decimal::from(arr.len() as i64))
                .map(Value::Decimal)
                .ok_or(RenderError::ArithmeticOverflow { span })
        }
        // min/max share fold_compare with the min()/max() builtins, so the
        // method and function forms always agree (promotion, string support).
        "min" => {
            check_arity(method, args, 0, span)?;
            if arr.is_empty() {
                return Err(empty_array(span));
            }
            fold_compare(arr, span, "min", Ordering::Less)
        }
        "max" => {
            check_arity(method, args, 0, span)?;
            if arr.is_empty() {
                return Err(empty_array(span));
            }
            fold_compare(arr, span, "max", Ordering::Greater)
        }

        // search & membership
        "contains" => {
            check_arity(method, args, 1, span)?;
            let needle = evaluate_expr(&args[0], input, lets, this)?;
            Ok(Value::Bool(arr.iter().any(|e| values_equal(e, &needle))))
        }
        "index_of" => {
            check_arity(method, args, 1, span)?;
            let needle = evaluate_expr(&args[0], input, lets, this)?;
            let idx = arr.iter().position(|e| values_equal(e, &needle));
            Ok(Value::Int(idx.map_or(-1, |i| i as i64)))
        }
        "find" => {
            check_arity(method, args, 1, span)?;
            let mut ctx = lambda_ctx(&args[0], 1, lets)?;
            for elem in arr {
                let keep = ctx.run(std::slice::from_ref(elem), input, this)?;
                if require_bool(&keep, args[0].span)? {
                    return Ok(elem.clone());
                }
            }
            Ok(Value::Null)
        }
        "count" => {
            check_arity(method, args, 1, span)?;
            let mut ctx = lambda_ctx(&args[0], 1, lets)?;
            let mut n = 0i64;
            for elem in arr {
                let keep = ctx.run(std::slice::from_ref(elem), input, this)?;
                if require_bool(&keep, args[0].span)? {
                    n += 1;
                }
            }
            Ok(Value::Int(n))
        }
        "any" => {
            check_arity(method, args, 1, span)?;
            let mut ctx = lambda_ctx(&args[0], 1, lets)?;
            for elem in arr {
                let v = ctx.run(std::slice::from_ref(elem), input, this)?;
                if require_bool(&v, args[0].span)? {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        "all" => {
            check_arity(method, args, 1, span)?;
            let mut ctx = lambda_ctx(&args[0], 1, lets)?;
            for elem in arr {
                let v = ctx.run(std::slice::from_ref(elem), input, this)?;
                if !require_bool(&v, args[0].span)? {
                    return Ok(Value::Bool(false));
                }
            }
            Ok(Value::Bool(true))
        }

        // iteration
        "map" => {
            check_arity(method, args, 1, span)?;
            let mut ctx = lambda_ctx(&args[0], 1, lets)?;
            let mut result = Vec::with_capacity(arr.len());
            for elem in arr {
                result.push(ctx.run(std::slice::from_ref(elem), input, this)?);
            }
            Ok(Value::Arr(result))
        }
        "filter" => {
            check_arity(method, args, 1, span)?;
            let mut ctx = lambda_ctx(&args[0], 1, lets)?;
            let mut result = Vec::new();
            for elem in arr {
                let keep = ctx.run(std::slice::from_ref(elem), input, this)?;
                if require_bool(&keep, args[0].span)? {
                    result.push(elem.clone());
                }
            }
            Ok(Value::Arr(result))
        }
        "fold" => {
            check_arity(method, args, 2, span)?;
            let mut acc = evaluate_expr(&args[0], input, lets, this)?;
            let mut ctx = lambda_ctx(&args[1], 2, lets)?;
            for elem in arr {
                acc = ctx.run(&[acc.clone(), elem.clone()], input, this)?;
            }
            Ok(acc)
        }
        "flat_map" => {
            check_arity(method, args, 1, span)?;
            let mut ctx = lambda_ctx(&args[0], 1, lets)?;
            let mut out = Vec::new();
            for e in arr {
                // `append` drains the lambda's array result via &mut — no move out of Drop.
                match &mut ctx.run(std::slice::from_ref(e), input, this)? {
                    Value::Arr(inner) => out.append(inner),
                    other => return Err(type_err("array from flat_map lambda", other, span)),
                }
            }
            Ok(Value::Arr(out))
        }

        // transforms
        "concat" => {
            check_arity(method, args, 1, span)?;
            let mut other = evaluate_expr(&args[0], input, lets, this)?;
            match &mut other {
                // append drains the other array via &mut — no move out of a Drop type.
                Value::Arr(b) => {
                    let mut result = arr.to_vec();
                    result.append(b);
                    Ok(Value::Arr(result))
                }
                v => Err(type_err("array", v, span)),
            }
        }
        "reverse" => {
            check_arity(method, args, 0, span)?;
            let mut v = arr.to_vec();
            v.reverse();
            Ok(Value::Arr(v))
        }
        "unique" => {
            check_arity(method, args, 0, span)?;
            let mut out: Vec<Value> = Vec::new();
            for e in arr {
                if !out.iter().any(|x| values_equal(x, e)) {
                    out.push(e.clone());
                }
            }
            Ok(Value::Arr(out))
        }
        "sort" => {
            check_arity(method, args, 0, span)?;
            let mut v = arr.to_vec();
            check_sortable(v.iter(), span)?;
            v.sort_by(cmp_sort_keys);
            Ok(Value::Arr(v))
        }
        "sort_by" => {
            check_arity(method, args, 1, span)?;
            let mut ctx = lambda_ctx(&args[0], 1, lets)?;
            let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(arr.len());
            for e in arr {
                let key = ctx.run(std::slice::from_ref(e), input, this)?;
                keyed.push((key, e.clone()));
            }
            check_sortable(keyed.iter().map(|(k, _)| k), span)?;
            keyed.sort_by(|(a, _), (b, _)| cmp_sort_keys(a, b));
            Ok(Value::Arr(keyed.into_iter().map(|(_, v)| v).collect()))
        }
        "flatten" => {
            check_arity(method, args, 0, span)?;
            let mut out = Vec::new();
            for e in arr {
                match e {
                    Value::Arr(inner) => out.extend(inner.iter().cloned()),
                    other => return Err(type_err("array of arrays", other, span)),
                }
            }
            Ok(Value::Arr(out))
        }
        "take" => {
            check_arity(method, args, 1, span)?;
            let n = int_arg(&args[0], input, lets, this)?.max(0) as usize;
            Ok(Value::Arr(arr.iter().take(n).cloned().collect()))
        }
        "drop" => {
            check_arity(method, args, 1, span)?;
            let n = int_arg(&args[0], input, lets, this)?.max(0) as usize;
            Ok(Value::Arr(arr.iter().skip(n).cloned().collect()))
        }
        "slice" => {
            check_arity(method, args, 2, span)?;
            let (a, b) = slice_bounds(
                int_arg(&args[0], input, lets, this)?,
                int_arg(&args[1], input, lets, this)?,
                arr.len(),
            );
            Ok(Value::Arr(arr[a..b].to_vec()))
        }
        "join" => {
            check_arity(method, args, 1, span)?;
            let sep = string_arg(&args[0], input, lets, this)?;
            let mut out = String::new();
            for (i, e) in arr.iter().enumerate() {
                if i > 0 {
                    out.push_str(&sep);
                }
                match e {
                    Value::Str(s) => out.push_str(s),
                    other => out.push_str(&scalar_to_string(other, span)?),
                }
            }
            Ok(Value::Str(out.into()))
        }
        _ => Err(unknown_method(method, "array", span)),
    }
}

fn obj_method(
    obj: &IndexMap<SmolStr, Value>,
    method: &SmolStr,
    args: &[Expr],
    span: Span,
    input: &Value,
    lets: &Scopes,
    this: &Scope,
) -> Result<Value, RenderError> {
    match method.as_str() {
        "keys" => {
            check_arity(method, args, 0, span)?;
            Ok(Value::Arr(
                obj.keys().map(|k| Value::Str(k.clone())).collect(),
            ))
        }
        "values" => {
            check_arity(method, args, 0, span)?;
            Ok(Value::Arr(obj.values().cloned().collect()))
        }
        "entries" => {
            check_arity(method, args, 0, span)?;
            let entries = obj
                .iter()
                .map(|(k, v)| {
                    let mut e = IndexMap::with_capacity(2);
                    e.insert(SmolStr::new("key"), Value::Str(k.clone()));
                    e.insert(SmolStr::new("value"), v.clone());
                    Value::Obj(e)
                })
                .collect();
            Ok(Value::Arr(entries))
        }
        "has" => {
            check_arity(method, args, 1, span)?;
            let k = string_arg(&args[0], input, lets, this)?;
            Ok(Value::Bool(obj.contains_key(k.as_str())))
        }
        "get" => {
            // Lenient computed-key read: null when absent (pairs with `?? default`).
            check_arity(method, args, 1, span)?;
            let k = string_arg(&args[0], input, lets, this)?;
            Ok(obj.get(k.as_str()).cloned().unwrap_or(Value::Null))
        }
        "merge" => {
            check_arity(method, args, 1, span)?;
            // `take` drains the other map through &mut — no move out of a Drop type.
            let mut other = evaluate_expr(&args[0], input, lets, this)?;
            match &mut other {
                Value::Obj(o2) => {
                    let mut merged = obj.clone();
                    for (k, v) in std::mem::take(o2) {
                        merged.insert(k, v);
                    }
                    Ok(Value::Obj(merged))
                }
                v => Err(type_err("object", v, span)),
            }
        }
        _ => Err(unknown_method(method, "object", span)),
    }
}

fn unknown_method(method: &SmolStr, on_type: &str, span: Span) -> RenderError {
    RenderError::UnknownMethod {
        method: method.to_string(),
        on_type: on_type.to_string(),
        span,
    }
}

/// A lambda bound for a single collection walk. The enclosing scope is
/// flattened once here; each element only overwrites the parameter bindings,
/// so the per-element cost is two inserts, not a scope clone.
struct LambdaCtx<'a> {
    params: &'a [LambdaParam],
    body: &'a Expr,
    scope: Scope,
}

fn lambda_ctx<'a>(
    lambda: &'a Expr,
    arity: usize,
    lets: &Scopes,
) -> Result<LambdaCtx<'a>, RenderError> {
    let (params, body) = match &lambda.kind {
        ExprKind::Lambda { params, body } => (params.as_slice(), body.as_ref()),
        _ => return Err(RenderError::LambdaExpected { span: lambda.span }),
    };
    if params.len() != arity {
        return Err(RenderError::ArityMismatch {
            method: "lambda".to_string(),
            expected: arity,
            got: params.len(),
            span: lambda.span,
        });
    }
    Ok(LambdaCtx {
        params,
        body,
        scope: lets.materialize(),
    })
}

impl LambdaCtx<'_> {
    fn run(&mut self, args: &[Value], input: &Value, this: &Scope) -> Result<Value, RenderError> {
        for (param, value) in self.params.iter().zip(args) {
            self.scope.insert(param.name.clone(), value.clone());
        }
        evaluate_expr(self.body, input, &Scopes::base(&self.scope), this)
    }
}

fn check_arity(
    method: &SmolStr,
    args: &[Expr],
    expected: usize,
    span: Span,
) -> Result<(), RenderError> {
    if args.len() != expected {
        return Err(RenderError::ArityMismatch {
            method: method.to_string(),
            expected,
            got: args.len(),
            span,
        });
    }
    Ok(())
}

/// Evaluate an argument and require a string; moves the value out (no copy).
fn string_arg(
    arg: &Expr,
    input: &Value,
    lets: &Scopes,
    this: &Scope,
) -> Result<SmolStr, RenderError> {
    let mut v = evaluate_expr(arg, input, lets, this)?;
    match &mut v {
        Value::Str(s) => Ok(std::mem::take(s)),
        other => Err(type_err("string", other, arg.span)),
    }
}

/// Evaluate an argument and require an integer.
fn int_arg(arg: &Expr, input: &Value, lets: &Scopes, this: &Scope) -> Result<i64, RenderError> {
    match evaluate_expr(arg, input, lets, this)? {
        Value::Int(n) => Ok(n),
        other => Err(type_err("integer", &other, arg.span)),
    }
}

/// Clamp a `[start, end)` window (which may be negative or past the end) to a
/// valid, non-inverted `[a, b)` over `len` items.
fn slice_bounds(start: i64, end: i64, len: usize) -> (usize, usize) {
    let len = len as i64;
    let a = start.clamp(0, len);
    let b = end.clamp(a, len);
    (a as usize, b as usize)
}

fn empty_array(span: Span) -> RenderError {
    RenderError::TypeMismatch {
        expected: "non-empty array",
        got: "empty array".to_string(),
        span,
    }
}

fn sum_values(arr: &[Value], span: Span) -> Result<Value, RenderError> {
    let mut acc = Value::Int(0);
    for elem in arr {
        acc = arith(&acc, elem, span, i64::checked_add, Decimal::checked_add)?;
    }
    Ok(acc)
}

/// Sort keys must be all numbers or all strings.
fn check_sortable<'v>(
    keys: impl Iterator<Item = &'v Value>,
    span: Span,
) -> Result<(), RenderError> {
    let mut saw_num = false;
    let mut saw_str = false;
    for k in keys {
        match k {
            Value::Int(_) | Value::Decimal(_) => saw_num = true,
            Value::Str(_) => saw_str = true,
            other => return Err(type_err("sortable items (numbers or strings)", other, span)),
        }
    }
    if saw_num && saw_str {
        return Err(RenderError::TypeMismatch {
            expected: "comparable items (all numbers or all strings)",
            got: "mixed numbers and strings".to_string(),
            span,
        });
    }
    Ok(())
}

/// Compare pre-checked sort keys (same kind throughout, per `check_sortable`).
fn cmp_sort_keys(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => x.cmp(y),
        _ => compare(a, b, Span::new(0, 0)).unwrap_or(Ordering::Equal),
    }
}
