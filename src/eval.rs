use crate::error::{RenderError, Span};
use crate::parse::{BinOp, Expr, ExprKind, OutKind, OutNode, PathSegment, UnOp};
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
        ExprKind::Path { segments } => evaluate_path(segments, input, lets, this),
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
    segments: &[PathSegment],
    input: &Value,
    lets: &Scope,
    this: &Scope,
) -> Result<Value, RenderError> {
    let root = segments.first().ok_or(RenderError::TypeMismatch {
        expected: "path",
        got: "empty".to_string(),
        span: Span::new(0, 0),
    })?;
    let root_name = root.name.as_str();

    let (current, mut walked, start_idx): (&Value, Vec<String>, usize) = match root_name {
        "input" => (input, vec!["input".to_string()], 1),
        "this" => {
            let key_seg = segments.get(1).ok_or(RenderError::TypeMismatch {
                expected: "'this' followed by '.field'",
                got: "bare 'this'".to_string(),
                span: root.span,
            })?;
            let v = this.get(&key_seg.name).ok_or(RenderError::MissingPath {
                path: format!("this.{}", key_seg.name),
                key: Some(key_seg.name.to_string()),
                span: key_seg.span,
            })?;
            (v, vec!["this".to_string(), key_seg.name.to_string()], 2)
        }
        _ => {
            let v = lets.get(&root.name).ok_or(RenderError::TypeMismatch {
                expected: "known identifier",
                got: root_name.to_string(),
                span: root.span,
            })?;
            (v, vec![root_name.to_string()], 1)
        }
    };

    let mut current = current;
    for segment in &segments[start_idx..] {
        walked.push(segment.name.to_string());

        if segment.optional && matches!(current, Value::Null) {
            return Ok(Value::Null);
        }

        match current {
            Value::Obj(obj) => match obj.get(&segment.name) {
                Some(value) => current = value,
                None if segment.optional => return Ok(Value::Null),
                None => {
                    return Err(RenderError::MissingPath {
                        path: walked.join("."),
                        key: Some(segment.name.to_string()),
                        span: segment.span,
                    });
                }
            },
            other => {
                return Err(RenderError::TypeMismatch {
                    expected: "object",
                    got: other.kind().to_string(),
                    span: segment.span,
                });
            }
        }
    }

    Ok(current.clone())
}
