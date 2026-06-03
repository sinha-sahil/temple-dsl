//! Built-in functions (`abs`, `round`, `min`, `upper`, …) and the scalar
//! stringification shared with interpolation.

use super::ops::compare;
use crate::error::{RenderError, Span};
use crate::value::Value;
use rust_decimal::Decimal;
use smol_str::SmolStr;
use std::cmp::Ordering;

pub fn is_known_function(name: &str) -> bool {
    matches!(
        name,
        "abs"
            | "round"
            | "floor"
            | "ceil"
            | "min"
            | "max"
            | "upper"
            | "lower"
            | "trim"
            | "to_string"
            | "len"
    )
}

pub(super) fn call_func(name: &SmolStr, args: &[Value], span: Span) -> Result<Value, RenderError> {
    match name.as_str() {
        "abs" => {
            require_func_arity("abs", args.len(), 1, span)?;
            match &args[0] {
                Value::Int(n) => n
                    .checked_abs()
                    .map(Value::Int)
                    .ok_or(RenderError::ArithmeticOverflow { span }),
                Value::Decimal(d) => Ok(Value::Decimal(d.abs())),
                v => Err(type_err("number", v, span)),
            }
        }
        "round" => {
            if args.is_empty() || args.len() > 2 {
                return Err(RenderError::ArityMismatch {
                    method: "round".to_string(),
                    expected: 1,
                    got: args.len(),
                    span,
                });
            }
            let dp: u32 = if args.len() == 2 {
                match &args[1] {
                    // Clamp to rust_decimal's max scale, avoiding a wrapping `as u32`.
                    Value::Int(n) if *n >= 0 => (*n).min(28) as u32,
                    v => return Err(type_err("non-negative integer", v, span)),
                }
            } else {
                0
            };
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(*n)),
                Value::Decimal(d) => Ok(Value::Decimal(d.round_dp(dp))),
                v => Err(type_err("number", v, span)),
            }
        }
        "floor" => {
            require_func_arity("floor", args.len(), 1, span)?;
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(*n)),
                Value::Decimal(d) => Ok(Value::Decimal(d.floor())),
                v => Err(type_err("number", v, span)),
            }
        }
        "ceil" => {
            require_func_arity("ceil", args.len(), 1, span)?;
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(*n)),
                Value::Decimal(d) => Ok(Value::Decimal(d.ceil())),
                v => Err(type_err("number", v, span)),
            }
        }
        "min" => fold_compare(args, span, "min", Ordering::Less),
        "max" => fold_compare(args, span, "max", Ordering::Greater),
        "upper" => {
            require_func_arity("upper", args.len(), 1, span)?;
            match &args[0] {
                Value::Str(s) => Ok(Value::Str(s.to_uppercase().into())),
                v => Err(type_err("string", v, span)),
            }
        }
        "lower" => {
            require_func_arity("lower", args.len(), 1, span)?;
            match &args[0] {
                Value::Str(s) => Ok(Value::Str(s.to_lowercase().into())),
                v => Err(type_err("string", v, span)),
            }
        }
        "trim" => {
            require_func_arity("trim", args.len(), 1, span)?;
            match &args[0] {
                Value::Str(s) => Ok(Value::Str(s.trim().into())),
                v => Err(type_err("string", v, span)),
            }
        }
        "to_string" => {
            require_func_arity("to_string", args.len(), 1, span)?;
            if let Value::Str(s) = &args[0] {
                return Ok(Value::Str(s.clone()));
            }
            Ok(Value::Str(scalar_to_string(&args[0], span)?.into()))
        }
        "len" => {
            require_func_arity("len", args.len(), 1, span)?;
            match &args[0] {
                Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
                Value::Arr(a) => Ok(Value::Int(a.len() as i64)),
                v => Err(type_err("string or array", v, span)),
            }
        }
        _ => Err(RenderError::UnknownMethod {
            method: name.to_string(),
            on_type: "<function>".to_string(),
            span,
        }),
    }
}

fn fold_compare(
    args: &[Value],
    span: Span,
    name: &str,
    keep_when: Ordering,
) -> Result<Value, RenderError> {
    require_at_least(name, args.len(), 1, span)?;
    let any_decimal = args.iter().any(|v| matches!(v, Value::Decimal(_)));
    let mut best = args[0].clone();
    for arg in &args[1..] {
        if compare(arg, &best, span)? == keep_when {
            best = arg.clone();
        }
    }
    // Promote the winner to Decimal if any arg was, matching arithmetic's mixing rule.
    if any_decimal {
        if let Value::Int(n) = best {
            return Ok(Value::Decimal(Decimal::from(n)));
        }
    }
    Ok(best)
}

pub(super) fn scalar_to_string(v: &Value, span: Span) -> Result<String, RenderError> {
    Ok(match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Decimal(d) => d.to_string(),
        Value::Str(s) => s.to_string(),
        other => {
            return Err(RenderError::TypeMismatch {
                expected: "scalar (string interpolation and to_string require a non-collection)",
                got: other.kind().to_string(),
                span,
            });
        }
    })
}

fn require_func_arity(
    name: &str,
    got: usize,
    expected: usize,
    span: Span,
) -> Result<(), RenderError> {
    if got != expected {
        return Err(RenderError::ArityMismatch {
            method: name.to_string(),
            expected,
            got,
            span,
        });
    }
    Ok(())
}

fn require_at_least(name: &str, got: usize, minimum: usize, span: Span) -> Result<(), RenderError> {
    if got < minimum {
        return Err(RenderError::ArityMismatch {
            method: name.to_string(),
            expected: minimum,
            got,
            span,
        });
    }
    Ok(())
}

fn type_err(expected: &'static str, got: &Value, span: Span) -> RenderError {
    RenderError::TypeMismatch {
        expected,
        got: got.kind().to_string(),
        span,
    }
}
