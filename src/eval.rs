use crate::error::{RenderError, Span};
use crate::parse::{BinOp, Expr, ExprKind, LambdaParam, OutKind, OutNode, PathSegment, UnOp};
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
        BinOp::Add => arith(&l, &r, span, i64::checked_add, |a, b| a + b),
        BinOp::Sub => arith(&l, &r, span, i64::checked_sub, |a, b| a - b),
        BinOp::Mul => arith(&l, &r, span, i64::checked_mul, |a, b| a * b),
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
    D: Fn(Decimal, Decimal) -> Decimal,
{
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => int_op(*a, *b)
            .map(Value::Int)
            .ok_or(RenderError::ArithmeticOverflow { span }),
        (Value::Decimal(a), Value::Decimal(b)) => Ok(Value::Decimal(dec_op(*a, *b))),
        (Value::Int(a), Value::Decimal(b)) => Ok(Value::Decimal(dec_op(Decimal::from(*a), *b))),
        (Value::Decimal(a), Value::Int(b)) => Ok(Value::Decimal(dec_op(*a, Decimal::from(*b)))),
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
    Ok(Value::Decimal(a / b))
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

fn evaluate_path(
    root: &SmolStr,
    root_span: Span,
    segments: &[PathSegment],
    input: &Value,
    lets: &Scope,
    this: &Scope,
) -> Result<Value, RenderError> {
    let (mut current, start_idx): (Value, usize) = match root.as_str() {
        "input" => (input.clone(), 0),
        "this" => match segments.first() {
            Some(PathSegment::Field { name, span, .. }) => match this.get(name) {
                Some(v) => (v.clone(), 1),
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
            Some(v) => (v.clone(), 0),
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
                if *optional && matches!(current, Value::Null) {
                    return Ok(Value::Null);
                }
                current = match current {
                    Value::Obj(obj) => match obj.get(name) {
                        Some(v) => v.clone(),
                        None if *optional => return Ok(Value::Null),
                        None => {
                            return Err(RenderError::MissingPath {
                                path: name.to_string(),
                                key: Some(name.to_string()),
                                span: *span,
                            });
                        }
                    },
                    other => {
                        return Err(RenderError::TypeMismatch {
                            expected: "object",
                            got: other.kind().to_string(),
                            span: *span,
                        });
                    }
                };
            }
            PathSegment::Method {
                name, span, args, ..
            } => {
                current = call_method(&current, name, args, *span, input, lets, this)?;
            }
            PathSegment::Index { expr, span } => {
                let idx = evaluate_expr(expr, input, lets, this)?;
                current = index_value(&current, &idx, *span)?;
            }
        }
    }

    Ok(current)
}

fn index_value(receiver: &Value, idx: &Value, span: Span) -> Result<Value, RenderError> {
    match (receiver, idx) {
        (Value::Arr(a), Value::Int(i)) => {
            if *i < 0 {
                return Err(RenderError::IndexOutOfBounds {
                    index: *i,
                    length: a.len(),
                    span,
                });
            }
            let u = *i as usize;
            if u >= a.len() {
                return Err(RenderError::IndexOutOfBounds {
                    index: *i,
                    length: a.len(),
                    span,
                });
            }
            Ok(a[u].clone())
        }
        (Value::Arr(_), other) => Err(RenderError::TypeMismatch {
            expected: "integer index",
            got: other.kind().to_string(),
            span,
        }),
        (other, _) => Err(RenderError::NotIndexable {
            got: other.kind().to_string(),
            span,
        }),
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
        (Value::Arr(arr), "length") => {
            check_arity(method, args, 0, span)?;
            Ok(Value::Int(arr.len() as i64))
        }
        (Value::Str(s), "length") => {
            check_arity(method, args, 0, span)?;
            Ok(Value::Int(s.chars().count() as i64))
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
            let lambda = &args[0];
            let mut result = Vec::with_capacity(arr.len());
            for elem in arr {
                let v = call_lambda(lambda, std::slice::from_ref(elem), input, lets, this)?;
                result.push(v);
            }
            Ok(Value::Arr(result))
        }
        (Value::Arr(arr), "filter") => {
            check_arity(method, args, 1, span)?;
            let lambda = &args[0];
            let mut result = Vec::new();
            for elem in arr {
                let v = call_lambda(lambda, std::slice::from_ref(elem), input, lets, this)?;
                if require_bool(&v, lambda.span)? {
                    result.push(elem.clone());
                }
            }
            Ok(Value::Arr(result))
        }
        (Value::Arr(arr), "fold") => {
            check_arity(method, args, 2, span)?;
            let mut acc = evaluate_expr(&args[0], input, lets, this)?;
            let lambda = &args[1];
            for elem in arr {
                acc = call_lambda(lambda, &[acc.clone(), elem.clone()], input, lets, this)?;
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

fn call_lambda(
    lambda: &Expr,
    arg_values: &[Value],
    input: &Value,
    lets: &Scope,
    this: &Scope,
) -> Result<Value, RenderError> {
    let (params, body): (&[LambdaParam], &Expr) = match &lambda.kind {
        ExprKind::Lambda { params, body } => (params, body),
        _ => return Err(RenderError::LambdaExpected { span: lambda.span }),
    };
    if params.len() != arg_values.len() {
        return Err(RenderError::ArityMismatch {
            method: "lambda".to_string(),
            expected: params.len(),
            got: arg_values.len(),
            span: lambda.span,
        });
    }
    let mut local_lets = lets.clone();
    for (param, value) in params.iter().zip(arg_values.iter()) {
        local_lets.insert(param.name.clone(), value.clone());
    }
    evaluate_expr(body, input, &local_lets, this)
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
