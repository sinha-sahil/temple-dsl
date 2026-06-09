//! Built-in functions (`abs`, `round`, `min`, `upper`, …) and the scalar
//! stringification shared with interpolation.

use super::ops::compare;
use crate::error::{RenderError, Span};
use crate::value::Value;
use rust_decimal::Decimal;
use smol_str::SmolStr;
use std::cmp::Ordering;

pub(crate) const BUILTINS: &[&str] = &[
    "abs",
    "round",
    "floor",
    "ceil",
    "min",
    "max",
    "upper",
    "lower",
    "trim",
    "to_string",
    "concat",
    "to_number",
    "type_of",
    "is_null",
    "is_bool",
    "is_number",
    "is_string",
    "is_array",
    "is_object",
    "json_encode",
    "url_encode",
    "base64",
];

pub fn is_known_function(name: &str) -> bool {
    BUILTINS.contains(&name)
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
            // round takes 1 or 2 args; report against the nearer bound.
            if args.is_empty() || args.len() > 2 {
                return Err(RenderError::ArityMismatch {
                    method: "round".to_string(),
                    expected: if args.len() > 2 { 2 } else { 1 },
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
        "concat" => {
            require_at_least("concat", args.len(), 1, span)?;
            let mut s = String::new();
            for a in args {
                s.push_str(&scalar_to_string(a, span)?);
            }
            Ok(Value::Str(s.into()))
        }
        "to_number" => {
            require_func_arity("to_number", args.len(), 1, span)?;
            match &args[0] {
                Value::Int(_) | Value::Decimal(_) => Ok(args[0].clone()),
                Value::Str(s) => parse_number_str(s, span),
                v => Err(type_err("string or number", v, span)),
            }
        }
        "type_of" => {
            require_func_arity("type_of", args.len(), 1, span)?;
            Ok(Value::Str(args[0].kind().into()))
        }
        "is_null" | "is_bool" | "is_number" | "is_string" | "is_array" | "is_object" => {
            require_func_arity(name, args.len(), 1, span)?;
            let v = &args[0];
            // Every predicate is named explicitly — a new is_* added to the
            // outer arm without a row here is an error, not silently is_object.
            let yes = match name.as_str() {
                "is_null" => matches!(v, Value::Null),
                "is_bool" => matches!(v, Value::Bool(_)),
                "is_number" => matches!(v, Value::Int(_) | Value::Decimal(_)),
                "is_string" => matches!(v, Value::Str(_)),
                "is_array" => matches!(v, Value::Arr(_)),
                "is_object" => matches!(v, Value::Obj(_)),
                _ => {
                    return Err(RenderError::UnknownMethod {
                        method: name.to_string(),
                        on_type: "<function>".to_string(),
                        span,
                    });
                }
            };
            Ok(Value::Bool(yes))
        }
        "json_encode" => {
            require_func_arity("json_encode", args.len(), 1, span)?;
            let mut out = String::new();
            write_json(&args[0], 0, span, &mut out)?;
            Ok(Value::Str(out.into()))
        }
        "url_encode" => {
            require_func_arity("url_encode", args.len(), 1, span)?;
            match &args[0] {
                Value::Str(s) => Ok(Value::Str(url_encode_str(s).into())),
                v => Err(type_err("string", v, span)),
            }
        }
        "base64" => {
            require_func_arity("base64", args.len(), 1, span)?;
            match &args[0] {
                Value::Str(s) => Ok(Value::Str(base64_encode(s.as_bytes()).into())),
                v => Err(type_err("string", v, span)),
            }
        }
        _ => Err(RenderError::UnknownMethod {
            method: name.to_string(),
            on_type: "<function>".to_string(),
            span,
        }),
    }
}

/// Shared by the `min`/`max` builtins and the `.min()`/`.max()` array methods,
/// so the two forms always agree. All-string inputs compare lexicographically;
/// numeric inputs use numeric comparison with Decimal promotion of the winner.
pub(super) fn fold_compare(
    args: &[Value],
    span: Span,
    name: &str,
    keep_when: Ordering,
) -> Result<Value, RenderError> {
    require_at_least(name, args.len(), 1, span)?;
    if args.iter().all(|v| matches!(v, Value::Str(_))) {
        let mut best = &args[0];
        for arg in &args[1..] {
            if let (Value::Str(a), Value::Str(b)) = (arg, best) {
                if a.cmp(b) == keep_when {
                    best = arg;
                }
            }
        }
        return Ok(best.clone());
    }
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

pub(super) fn type_err(expected: &'static str, got: &Value, span: Span) -> RenderError {
    RenderError::TypeMismatch {
        expected,
        got: got.kind().to_string(),
        span,
    }
}

fn parse_number_str(s: &str, span: Span) -> Result<Value, RenderError> {
    let t = s.trim();
    if let Ok(i) = t.parse::<i64>() {
        return Ok(Value::Int(i));
    }
    if let Ok(d) = t.parse::<Decimal>() {
        return Ok(Value::Decimal(d));
    }
    Err(RenderError::TypeMismatch {
        expected: "numeric string",
        got: format!("'{s}'"),
        span,
    })
}

/// Serialize a value to a JSON string. Depth-bounded so a pathological value
/// can't overflow the stack (upholding the no-panic guarantee).
fn write_json(v: &Value, depth: usize, span: Span, out: &mut String) -> Result<(), RenderError> {
    if depth > 256 {
        return Err(RenderError::ValueTooDeep { limit: 256, span });
    }
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Decimal(d) => out.push_str(&d.to_string()),
        Value::Str(s) => json_string(s, out),
        Value::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(x, depth + 1, span, out)?;
            }
            out.push(']');
        }
        Value::Obj(o) => {
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_string(k, out);
                out.push(':');
                write_json(x, depth + 1, span, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";
const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";

fn json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                let b = c as u32 as u8;
                out.push_str("\\u00");
                out.push(HEX_LOWER[(b >> 4) as usize] as char);
                out.push(HEX_LOWER[(b & 0xF) as usize] as char);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn url_encode_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push(HEX_UPPER[(b >> 4) as usize] as char);
                out.push(HEX_UPPER[(b & 0xF) as usize] as char);
            }
        }
    }
    out
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
