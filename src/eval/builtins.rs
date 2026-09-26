use super::ops::compare;
use crate::common::error::{RenderError, Span};
use crate::common::limits::{MAX_DECIMAL_SCALE, MAX_JSON_DEPTH};
use crate::common::value::Value;
use rust_decimal::Decimal;
use std::cmp::Ordering;

#[derive(Clone, Copy)]
enum Arity {
    Exactly(usize),
    Between(usize, usize),
    AtLeast(usize),
}

impl Arity {
    fn accepts(self, given: usize) -> bool {
        match self {
            Arity::Exactly(count) => given == count,
            Arity::Between(low, high) => (low..=high).contains(&given),
            Arity::AtLeast(low) => given >= low,
        }
    }

    /// The allowed count nearest to `given`, for `RenderError::ArityMismatch`.
    fn nearest(self, given: usize) -> usize {
        match self {
            Arity::Exactly(count) => count,
            Arity::Between(low, high) => given.clamp(low, high),
            Arity::AtLeast(low) => given.max(low),
        }
    }
}

type Builtin = fn(&[Value], Span) -> Result<Value, RenderError>;

/// The arity is checked before the function runs, so each one may index its
/// arguments directly.
const BUILTINS: &[(&str, Arity, Builtin)] = &[
    ("abs", Arity::Exactly(1), abs),
    ("round", Arity::Between(1, 2), round),
    ("floor", Arity::Exactly(1), floor),
    ("ceil", Arity::Exactly(1), ceil),
    ("min", Arity::AtLeast(1), min),
    ("max", Arity::AtLeast(1), max),
    ("upper", Arity::Exactly(1), upper),
    ("lower", Arity::Exactly(1), lower),
    ("trim", Arity::Exactly(1), trim),
    ("to_string", Arity::Exactly(1), to_string),
    ("concat", Arity::AtLeast(1), concat),
    ("to_number", Arity::Exactly(1), to_number),
    ("type_of", Arity::Exactly(1), type_of),
    ("is_null", Arity::Exactly(1), is_null),
    ("is_bool", Arity::Exactly(1), is_bool),
    ("is_number", Arity::Exactly(1), is_number),
    ("is_string", Arity::Exactly(1), is_string),
    ("is_array", Arity::Exactly(1), is_array),
    ("is_object", Arity::Exactly(1), is_object),
    ("json_encode", Arity::Exactly(1), json_encode),
    ("url_encode", Arity::Exactly(1), url_encode),
    ("base64", Arity::Exactly(1), base64),
];

pub(crate) fn is_builtin_function(name: &str) -> bool {
    builtin_names().any(|builtin| builtin == name)
}

pub(crate) fn builtin_names() -> impl Iterator<Item = &'static str> {
    BUILTINS.iter().map(|&(name, _, _)| name)
}

pub(super) fn call_builtin(name: &str, args: &[Value], span: Span) -> Result<Value, RenderError> {
    let Some(&(_, arity, call)) = BUILTINS.iter().find(|(builtin, _, _)| *builtin == name) else {
        return Err(RenderError::unknown_method(name, "<function>", span));
    };
    if !arity.accepts(args.len()) {
        return Err(RenderError::ArityMismatch {
            method: name.to_string(),
            expected: arity.nearest(args.len()),
            got: args.len(),
            span,
        });
    }
    call(args, span)
}

fn abs(args: &[Value], span: Span) -> Result<Value, RenderError> {
    match &args[0] {
        Value::Int(n) => n
            .checked_abs()
            .map(Value::Int)
            .ok_or(RenderError::ArithmeticOverflow { span }),
        Value::Decimal(d) => Ok(Value::Decimal(d.abs())),
        other => Err(RenderError::type_mismatch("number", other.kind(), span)),
    }
}

fn round(args: &[Value], span: Span) -> Result<Value, RenderError> {
    let places: u32 = match args.get(1) {
        None => 0,
        Some(Value::Int(n)) if *n >= 0 => (*n).min(i64::from(MAX_DECIMAL_SCALE)) as u32,
        Some(other) => {
            return Err(RenderError::type_mismatch(
                "non-negative integer",
                other.kind(),
                span,
            ))
        }
    };
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Decimal(d) => Ok(Value::Decimal(d.round_dp(places))),
        other => Err(RenderError::type_mismatch("number", other.kind(), span)),
    }
}

fn floor(args: &[Value], span: Span) -> Result<Value, RenderError> {
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Decimal(d) => Ok(Value::Decimal(d.floor())),
        other => Err(RenderError::type_mismatch("number", other.kind(), span)),
    }
}

fn ceil(args: &[Value], span: Span) -> Result<Value, RenderError> {
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Decimal(d) => Ok(Value::Decimal(d.ceil())),
        other => Err(RenderError::type_mismatch("number", other.kind(), span)),
    }
}

fn min(args: &[Value], span: Span) -> Result<Value, RenderError> {
    fold_compare(args, span, Ordering::Less)
}

fn max(args: &[Value], span: Span) -> Result<Value, RenderError> {
    fold_compare(args, span, Ordering::Greater)
}

fn upper(args: &[Value], span: Span) -> Result<Value, RenderError> {
    map_text(args, span, str::to_uppercase)
}

fn lower(args: &[Value], span: Span) -> Result<Value, RenderError> {
    map_text(args, span, str::to_lowercase)
}

fn trim(args: &[Value], span: Span) -> Result<Value, RenderError> {
    map_text(args, span, |text| text.trim().to_string())
}

fn map_text(args: &[Value], span: Span, f: impl Fn(&str) -> String) -> Result<Value, RenderError> {
    match &args[0] {
        Value::Str(text) => Ok(Value::Str(f(text).into())),
        other => Err(RenderError::type_mismatch("string", other.kind(), span)),
    }
}

fn to_string(args: &[Value], span: Span) -> Result<Value, RenderError> {
    if let Value::Str(text) = &args[0] {
        return Ok(Value::Str(text.clone()));
    }
    Ok(Value::Str(scalar_to_string(&args[0], span)?.into()))
}

fn concat(args: &[Value], span: Span) -> Result<Value, RenderError> {
    let mut out = String::new();
    for arg in args {
        out.push_str(&scalar_to_string(arg, span)?);
    }
    Ok(Value::Str(out.into()))
}

fn to_number(args: &[Value], span: Span) -> Result<Value, RenderError> {
    match &args[0] {
        Value::Int(_) | Value::Decimal(_) => Ok(args[0].clone()),
        Value::Str(text) => parse_number_str(text, span),
        other => Err(RenderError::type_mismatch(
            "string or number",
            other.kind(),
            span,
        )),
    }
}

fn type_of(args: &[Value], _span: Span) -> Result<Value, RenderError> {
    Ok(Value::Str(args[0].kind().into()))
}

fn is_null(args: &[Value], _span: Span) -> Result<Value, RenderError> {
    Ok(Value::Bool(matches!(args[0], Value::Null)))
}

fn is_bool(args: &[Value], _span: Span) -> Result<Value, RenderError> {
    Ok(Value::Bool(matches!(args[0], Value::Bool(_))))
}

fn is_number(args: &[Value], _span: Span) -> Result<Value, RenderError> {
    Ok(Value::Bool(matches!(
        args[0],
        Value::Int(_) | Value::Decimal(_)
    )))
}

fn is_string(args: &[Value], _span: Span) -> Result<Value, RenderError> {
    Ok(Value::Bool(matches!(args[0], Value::Str(_))))
}

fn is_array(args: &[Value], _span: Span) -> Result<Value, RenderError> {
    Ok(Value::Bool(matches!(args[0], Value::Arr(_))))
}

fn is_object(args: &[Value], _span: Span) -> Result<Value, RenderError> {
    Ok(Value::Bool(matches!(args[0], Value::Obj(_))))
}

fn json_encode(args: &[Value], span: Span) -> Result<Value, RenderError> {
    let mut out = String::new();
    write_json(&args[0], 0, span, &mut out)?;
    Ok(Value::Str(out.into()))
}

fn url_encode(args: &[Value], span: Span) -> Result<Value, RenderError> {
    map_text(args, span, percent_encode)
}

fn base64(args: &[Value], span: Span) -> Result<Value, RenderError> {
    map_text(args, span, |text| base64_encode(text.as_bytes()))
}

/// Smallest or largest of `args`, shared with the array methods so both forms
/// agree. All strings compare as text; otherwise numbers, decimal if any arg is.
pub(super) fn fold_compare(
    args: &[Value],
    span: Span,
    keep_when: Ordering,
) -> Result<Value, RenderError> {
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

fn parse_number_str(s: &str, span: Span) -> Result<Value, RenderError> {
    let text = s.trim();
    if let Ok(i) = text.parse::<i64>() {
        return Ok(Value::Int(i));
    }
    if let Ok(d) = text.parse::<Decimal>() {
        return Ok(Value::Decimal(d));
    }
    Err(RenderError::TypeMismatch {
        expected: "numeric string",
        got: format!("'{s}'"),
        span,
    })
}

fn write_json(v: &Value, depth: usize, span: Span, out: &mut String) -> Result<(), RenderError> {
    if depth > MAX_JSON_DEPTH {
        return Err(RenderError::ValueTooDeep {
            limit: MAX_JSON_DEPTH,
            span,
        });
    }
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Decimal(d) => out.push_str(&d.to_string()),
        Value::Str(s) => write_json_string(s, out),
        Value::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(item, depth + 1, span, out)?;
            }
            out.push(']');
        }
        Value::Obj(obj) => {
            out.push('{');
            for (i, (key, value)) in obj.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(key, out);
                out.push(':');
                write_json(value, depth + 1, span, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";
const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";

fn write_json_string(s: &str, out: &mut String) {
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

fn percent_encode(s: &str) -> String {
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
