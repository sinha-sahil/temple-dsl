use crate::error::{RenderError, Span};
use crate::parse::{
    BinOp, Expr, ExprKind, InterpPart, LambdaParam, OutKind, OutNode, PathSegment, UnOp,
};
use crate::value::Value;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use smol_str::SmolStr;
use std::cmp::Ordering;
use std::collections::HashMap;

pub type Scope = HashMap<SmolStr, Value>;

pub fn evaluate(
    node: &OutNode,
    input: &Value,
    lets: &Scope,
    this: &Scope,
) -> Result<Value, RenderError> {
    match &node.kind {
        OutKind::Literal(v) => Ok(v.clone()),
        OutKind::Hole(expr) => evaluate_expr(expr, input, lets, this),
        OutKind::Object(fields) => {
            let mut obj = IndexMap::with_capacity(fields.len());
            for (key, value_node) in fields {
                obj.insert(key.clone(), evaluate(value_node, input, lets, this)?);
            }
            Ok(Value::Obj(obj))
        }
        OutKind::Array(items) => {
            let mut arr = Vec::with_capacity(items.len());
            for item in items {
                arr.push(evaluate(item, input, lets, this)?);
            }
            Ok(Value::Arr(arr))
        }
        OutKind::Interp(parts) => {
            let mut s = String::new();
            for part in parts {
                match part {
                    InterpPart::Text(t) => s.push_str(t),
                    InterpPart::Hole(expr) => {
                        let v = evaluate_expr(expr, input, lets, this)?;
                        s.push_str(&scalar_to_string(&v, expr.span)?);
                    }
                }
            }
            Ok(Value::Str(s.into()))
        }
    }
}

pub fn evaluate_expr(
    expr: &Expr,
    input: &Value,
    lets: &Scope,
    this: &Scope,
) -> Result<Value, RenderError> {
    match &expr.kind {
        ExprKind::Literal(v) => Ok(v.clone()),
        ExprKind::Path {
            root,
            root_span,
            segments,
        } => evaluate_path(root, *root_span, segments, input, lets, this),
        ExprKind::Binary { op, lhs, rhs } => {
            evaluate_binary(*op, lhs, rhs, expr.span, input, lets, this)
        }
        ExprKind::Unary { op, operand } => {
            evaluate_unary(*op, operand, expr.span, input, lets, this)
        }
        ExprKind::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            let c = evaluate_expr(cond, input, lets, this)?;
            let b = require_bool(&c, cond.span)?;
            if b {
                evaluate_expr(then_branch, input, lets, this)
            } else {
                evaluate_expr(else_branch, input, lets, this)
            }
        }
        ExprKind::When { branches, fallback } => {
            for branch in branches {
                let c = evaluate_expr(&branch.cond, input, lets, this)?;
                let b = require_bool(&c, branch.cond.span)?;
                if b {
                    return evaluate_expr(&branch.result, input, lets, this);
                }
            }
            match fallback {
                Some(fb) => evaluate_expr(fb, input, lets, this),
                None => Err(RenderError::WhenNoMatch { span: expr.span }),
            }
        }
        ExprKind::Lambda { .. } => Err(RenderError::TypeMismatch {
            expected: "value",
            got: "lambda (only valid as a method argument)".to_string(),
            span: expr.span,
        }),
        ExprKind::ArrayLit(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(evaluate_expr(item, input, lets, this)?);
            }
            Ok(Value::Arr(out))
        }
        ExprKind::ObjectLit(entries) => {
            let mut obj = IndexMap::with_capacity(entries.len());
            for (k, e) in entries {
                obj.insert(k.clone(), evaluate_expr(e, input, lets, this)?);
            }
            Ok(Value::Obj(obj))
        }
        ExprKind::FuncCall {
            name,
            name_span,
            args,
        } => {
            let mut values = Vec::with_capacity(args.len());
            for a in args {
                values.push(evaluate_expr(a, input, lets, this)?);
            }
            call_func(name, &values, *name_span)
        }
    }
}

fn evaluate_binary(
    op: BinOp,
    lhs: &Expr,
    rhs: &Expr,
    span: Span,
    input: &Value,
    lets: &Scope,
    this: &Scope,
) -> Result<Value, RenderError> {
    match op {
        BinOp::And => {
            let l = evaluate_expr(lhs, input, lets, this)?;
            let lb = require_bool(&l, lhs.span)?;
            if !lb {
                return Ok(Value::Bool(false));
            }
            let r = evaluate_expr(rhs, input, lets, this)?;
            let rb = require_bool(&r, rhs.span)?;
            return Ok(Value::Bool(rb));
        }
        BinOp::Or => {
            let l = evaluate_expr(lhs, input, lets, this)?;
            let lb = require_bool(&l, lhs.span)?;
            if lb {
                return Ok(Value::Bool(true));
            }
            let r = evaluate_expr(rhs, input, lets, this)?;
            let rb = require_bool(&r, rhs.span)?;
            return Ok(Value::Bool(rb));
        }
        BinOp::Coalesce => {
            let l = evaluate_expr(lhs, input, lets, this)?;
            if matches!(l, Value::Null) {
                return evaluate_expr(rhs, input, lets, this);
            }
            return Ok(l);
        }
        _ => {}
    }

    let l = evaluate_expr(lhs, input, lets, this)?;
    let r = evaluate_expr(rhs, input, lets, this)?;

    match op {
        BinOp::Add => arith(&l, &r, span, i64::checked_add, Decimal::checked_add),
        BinOp::Sub => arith(&l, &r, span, i64::checked_sub, Decimal::checked_sub),
        BinOp::Mul => arith(&l, &r, span, i64::checked_mul, Decimal::checked_mul),
        BinOp::Div => div(&l, &r, span),
        BinOp::Eq => Ok(Value::Bool(values_equal(&l, &r))),
        BinOp::Ne => Ok(Value::Bool(!values_equal(&l, &r))),
        BinOp::Lt => compare(&l, &r, span).map(|c| Value::Bool(c == Ordering::Less)),
        BinOp::Le => compare(&l, &r, span).map(|c| Value::Bool(c != Ordering::Greater)),
        BinOp::Gt => compare(&l, &r, span).map(|c| Value::Bool(c == Ordering::Greater)),
        BinOp::Ge => compare(&l, &r, span).map(|c| Value::Bool(c != Ordering::Less)),
        BinOp::And | BinOp::Or | BinOp::Coalesce => unreachable!(),
    }
}

fn evaluate_unary(
    op: UnOp,
    operand: &Expr,
    span: Span,
    input: &Value,
    lets: &Scope,
    this: &Scope,
) -> Result<Value, RenderError> {
    let v = evaluate_expr(operand, input, lets, this)?;
    match op {
        UnOp::Neg => match v {
            Value::Int(n) => n
                .checked_neg()
                .map(Value::Int)
                .ok_or(RenderError::ArithmeticOverflow { span }),
            Value::Decimal(d) => Ok(Value::Decimal(-d)),
            other => Err(RenderError::TypeMismatch {
                expected: "number",
                got: other.kind().to_string(),
                span,
            }),
        },
        UnOp::Not => {
            let b = require_bool(&v, span)?;
            Ok(Value::Bool(!b))
        }
    }
}

fn arith<I, D>(l: &Value, r: &Value, span: Span, int_op: I, dec_op: D) -> Result<Value, RenderError>
where
    I: Fn(i64, i64) -> Option<i64>,
    D: Fn(Decimal, Decimal) -> Option<Decimal>,
{
    let checked = |opt: Option<Decimal>| {
        opt.map(Value::Decimal)
            .ok_or(RenderError::ArithmeticOverflow { span })
    };
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => int_op(*a, *b)
            .map(Value::Int)
            .ok_or(RenderError::ArithmeticOverflow { span }),
        (Value::Decimal(a), Value::Decimal(b)) => checked(dec_op(*a, *b)),
        (Value::Int(a), Value::Decimal(b)) => checked(dec_op(Decimal::from(*a), *b)),
        (Value::Decimal(a), Value::Int(b)) => checked(dec_op(*a, Decimal::from(*b))),
        _ => Err(RenderError::TypeMismatch {
            expected: "number",
            got: format!("{} and {}", l.kind(), r.kind()),
            span,
        }),
    }
}

fn div(l: &Value, r: &Value, span: Span) -> Result<Value, RenderError> {
    let (a, b) = match (l, r) {
        (Value::Int(a), Value::Int(b)) => (Decimal::from(*a), Decimal::from(*b)),
        (Value::Decimal(a), Value::Decimal(b)) => (*a, *b),
        (Value::Int(a), Value::Decimal(b)) => (Decimal::from(*a), *b),
        (Value::Decimal(a), Value::Int(b)) => (*a, Decimal::from(*b)),
        _ => {
            return Err(RenderError::TypeMismatch {
                expected: "number",
                got: format!("{} and {}", l.kind(), r.kind()),
                span,
            });
        }
    };
    if b.is_zero() {
        return Err(RenderError::DivideByZero { span });
    }
    a.checked_div(b)
        .map(Value::Decimal)
        .ok_or(RenderError::ArithmeticOverflow { span })
}

fn require_bool(v: &Value, span: Span) -> Result<bool, RenderError> {
    match v {
        Value::Bool(b) => Ok(*b),
        other => Err(RenderError::TypeMismatch {
            expected: "bool",
            got: other.kind().to_string(),
            span,
        }),
    }
}

fn values_equal(l: &Value, r: &Value) -> bool {
    match (l, r) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Decimal(a), Value::Decimal(b)) => a == b,
        (Value::Int(a), Value::Decimal(b)) => Decimal::from(*a) == *b,
        (Value::Decimal(a), Value::Int(b)) => *a == Decimal::from(*b),
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::Arr(a), Value::Arr(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| values_equal(x, y))
        }
        (Value::Obj(a), Value::Obj(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|bv| values_equal(v, bv)))
        }
        _ => false,
    }
}

fn compare(l: &Value, r: &Value, span: Span) -> Result<Ordering, RenderError> {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => Ok(a.cmp(b)),
        (Value::Decimal(a), Value::Decimal(b)) => Ok(a.cmp(b)),
        (Value::Int(a), Value::Decimal(b)) => Ok(Decimal::from(*a).cmp(b)),
        (Value::Decimal(a), Value::Int(b)) => Ok(a.cmp(&Decimal::from(*b))),
        _ => Err(RenderError::TypeMismatch {
            expected: "comparable numbers",
            got: format!("{} and {}", l.kind(), r.kind()),
            span,
        }),
    }
}

/// A path cursor that stays a borrow into `input`/`lets`/`this` until a method
/// (or owned index) forces ownership — so `input.a.b.c` never clones the whole
/// input, only the final value once at the end.
enum Cursor<'a> {
    Borrowed(&'a Value),
    Owned(Value),
}

impl Cursor<'_> {
    fn as_ref(&self) -> &Value {
        match self {
            Cursor::Borrowed(v) => v,
            Cursor::Owned(v) => v,
        }
    }

    fn into_value(self) -> Value {
        match self {
            Cursor::Borrowed(v) => v.clone(),
            Cursor::Owned(v) => v,
        }
    }
}

fn array_index(idx: &Value, len: usize, span: Span) -> Result<usize, RenderError> {
    match idx {
        Value::Int(i) => {
            if *i < 0 || *i as usize >= len {
                Err(RenderError::IndexOutOfBounds {
                    index: *i,
                    length: len,
                    span,
                })
            } else {
                Ok(*i as usize)
            }
        }
        other => Err(RenderError::TypeMismatch {
            expected: "integer index",
            got: other.kind().to_string(),
            span,
        }),
    }
}

fn evaluate_path(
    root: &SmolStr,
    root_span: Span,
    segments: &[PathSegment],
    input: &Value,
    lets: &Scope,
    this: &Scope,
) -> Result<Value, RenderError> {
    let (mut current, start_idx): (Cursor, usize) = match root.as_str() {
        "input" => (Cursor::Borrowed(input), 0),
        "this" => match segments.first() {
            Some(PathSegment::Field { name, span, .. }) => match this.get(name) {
                Some(v) => (Cursor::Borrowed(v), 1),
                None => {
                    return Err(RenderError::MissingPath {
                        path: format!("this.{name}"),
                        key: Some(name.to_string()),
                        span: *span,
                    });
                }
            },
            _ => {
                return Err(RenderError::TypeMismatch {
                    expected: "'this' followed by '.field'",
                    got: "bare 'this'".to_string(),
                    span: root_span,
                });
            }
        },
        _ => match lets.get(root) {
            Some(v) => (Cursor::Borrowed(v), 0),
            None => {
                return Err(RenderError::TypeMismatch {
                    expected: "known identifier",
                    got: root.to_string(),
                    span: root_span,
                });
            }
        },
    };

    for segment in &segments[start_idx..] {
        match segment {
            PathSegment::Field {
                name,
                span,
                optional,
            } => {
                if *optional && matches!(current.as_ref(), Value::Null) {
                    return Ok(Value::Null);
                }
                current = match current {
                    Cursor::Borrowed(Value::Obj(obj)) => match obj.get(name) {
                        Some(v) => Cursor::Borrowed(v),
                        None if *optional => return Ok(Value::Null),
                        None => return Err(missing_field(name, *span)),
                    },
                    Cursor::Owned(Value::Obj(mut obj)) => match obj.swap_remove(name) {
                        Some(v) => Cursor::Owned(v),
                        None if *optional => return Ok(Value::Null),
                        None => return Err(missing_field(name, *span)),
                    },
                    other => {
                        return Err(RenderError::TypeMismatch {
                            expected: "object",
                            got: other.as_ref().kind().to_string(),
                            span: *span,
                        });
                    }
                };
            }
            PathSegment::Method {
                name, span, args, ..
            } => {
                let result = call_method(current.as_ref(), name, args, *span, input, lets, this)?;
                current = Cursor::Owned(result);
            }
            PathSegment::Index { expr, span } => {
                let idx = evaluate_expr(expr, input, lets, this)?;
                current = match current {
                    Cursor::Borrowed(Value::Arr(a)) => {
                        let u = array_index(&idx, a.len(), *span)?;
                        Cursor::Borrowed(&a[u])
                    }
                    Cursor::Owned(Value::Arr(mut a)) => {
                        let u = array_index(&idx, a.len(), *span)?;
                        Cursor::Owned(a.swap_remove(u))
                    }
                    other => {
                        return Err(RenderError::NotIndexable {
                            got: other.as_ref().kind().to_string(),
                            span: *span,
                        });
                    }
                };
            }
        }
    }

    Ok(current.into_value())
}

fn missing_field(name: &SmolStr, span: Span) -> RenderError {
    RenderError::MissingPath {
        path: name.to_string(),
        key: Some(name.to_string()),
        span,
    }
}

fn call_method(
    receiver: &Value,
    method: &SmolStr,
    args: &[Expr],
    span: Span,
    input: &Value,
    lets: &Scope,
    this: &Scope,
) -> Result<Value, RenderError> {
    match (receiver, method.as_str()) {
        (Value::Arr(arr), "length" | "len") => {
            check_arity(method, args, 0, span)?;
            Ok(Value::Int(arr.len() as i64))
        }
        (Value::Str(s), "length" | "len") => {
            check_arity(method, args, 0, span)?;
            Ok(Value::Int(s.chars().count() as i64))
        }
        (Value::Arr(arr), "sum") => {
            check_arity(method, args, 0, span)?;
            let mut acc = Value::Int(0);
            for elem in arr {
                acc = arith(&acc, elem, span, i64::checked_add, Decimal::checked_add)?;
            }
            Ok(acc)
        }
        (Value::Arr(arr), "any") => {
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
        (Value::Arr(arr), "all") => {
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
        (Value::Arr(arr), "first") => {
            check_arity(method, args, 0, span)?;
            Ok(arr.first().cloned().unwrap_or(Value::Null))
        }
        (Value::Arr(arr), "last") => {
            check_arity(method, args, 0, span)?;
            Ok(arr.last().cloned().unwrap_or(Value::Null))
        }
        (Value::Arr(a), "concat") => {
            check_arity(method, args, 1, span)?;
            let other = evaluate_expr(&args[0], input, lets, this)?;
            match other {
                Value::Arr(b) => {
                    let mut result = a.clone();
                    result.extend(b);
                    Ok(Value::Arr(result))
                }
                v => Err(RenderError::TypeMismatch {
                    expected: "array",
                    got: v.kind().to_string(),
                    span,
                }),
            }
        }
        (Value::Arr(arr), "map") => {
            check_arity(method, args, 1, span)?;
            let mut ctx = lambda_ctx(&args[0], 1, lets)?;
            let mut result = Vec::with_capacity(arr.len());
            for elem in arr {
                result.push(ctx.run(std::slice::from_ref(elem), input, this)?);
            }
            Ok(Value::Arr(result))
        }
        (Value::Arr(arr), "filter") => {
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
        (Value::Arr(arr), "fold") => {
            check_arity(method, args, 2, span)?;
            let mut acc = evaluate_expr(&args[0], input, lets, this)?;
            let mut ctx = lambda_ctx(&args[1], 2, lets)?;
            for elem in arr {
                acc = ctx.run(&[acc.clone(), elem.clone()], input, this)?;
            }
            Ok(acc)
        }
        (other, _) => Err(RenderError::UnknownMethod {
            method: method.to_string(),
            on_type: other.kind().to_string(),
            span,
        }),
    }
}

/// A lambda bound for a single collection walk. The enclosing `lets` scope is
/// cloned once here; each element only overwrites the parameter bindings,
/// turning a per-element scope clone into a per-loop one.
struct LambdaCtx<'a> {
    params: &'a [LambdaParam],
    body: &'a Expr,
    scope: Scope,
}

fn lambda_ctx<'a>(
    lambda: &'a Expr,
    arity: usize,
    lets: &Scope,
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
        scope: lets.clone(),
    })
}

impl LambdaCtx<'_> {
    fn run(&mut self, args: &[Value], input: &Value, this: &Scope) -> Result<Value, RenderError> {
        for (param, value) in self.params.iter().zip(args) {
            self.scope.insert(param.name.clone(), value.clone());
        }
        evaluate_expr(self.body, input, &self.scope, this)
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

fn call_func(name: &SmolStr, args: &[Value], span: Span) -> Result<Value, RenderError> {
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

fn scalar_to_string(v: &Value, span: Span) -> Result<String, RenderError> {
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
