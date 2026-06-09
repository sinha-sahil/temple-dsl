//! Operators — binary, unary, and the numeric/equality/comparison helpers.

use super::{evaluate_expr, Scope, Scopes};
use crate::error::{RenderError, Span};
use crate::parse::{BinOp, Expr, UnOp};
use crate::value::Value;
use rust_decimal::Decimal;
use std::cmp::Ordering;

pub(super) fn evaluate_binary(
    op: BinOp,
    lhs: &Expr,
    rhs: &Expr,
    span: Span,
    input: &Value,
    lets: &Scopes,
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
        BinOp::Mod => rem(&l, &r, span),
        BinOp::Eq => Ok(Value::Bool(values_equal(&l, &r))),
        BinOp::Ne => Ok(Value::Bool(!values_equal(&l, &r))),
        BinOp::Lt => compare(&l, &r, span).map(|c| Value::Bool(c == Ordering::Less)),
        BinOp::Le => compare(&l, &r, span).map(|c| Value::Bool(c != Ordering::Greater)),
        BinOp::Gt => compare(&l, &r, span).map(|c| Value::Bool(c == Ordering::Greater)),
        BinOp::Ge => compare(&l, &r, span).map(|c| Value::Bool(c != Ordering::Less)),
        BinOp::And | BinOp::Or | BinOp::Coalesce => unreachable!(),
    }
}

pub(super) fn evaluate_unary(
    op: UnOp,
    operand: &Expr,
    span: Span,
    input: &Value,
    lets: &Scopes,
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

pub(super) fn arith<I, D>(
    l: &Value,
    r: &Value,
    span: Span,
    int_op: I,
    dec_op: D,
) -> Result<Value, RenderError>
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

fn rem(l: &Value, r: &Value, span: Span) -> Result<Value, RenderError> {
    // Int % Int stays an Int; any Decimal operand promotes the result.
    if let (Value::Int(a), Value::Int(b)) = (l, r) {
        if *b == 0 {
            return Err(RenderError::DivideByZero { span });
        }
        return a
            .checked_rem(*b)
            .map(Value::Int)
            .ok_or(RenderError::ArithmeticOverflow { span });
    }
    let (a, b) = match (l, r) {
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
    a.checked_rem(b)
        .map(Value::Decimal)
        .ok_or(RenderError::ArithmeticOverflow { span })
}

pub(super) fn require_bool(v: &Value, span: Span) -> Result<bool, RenderError> {
    match v {
        Value::Bool(b) => Ok(*b),
        other => Err(RenderError::TypeMismatch {
            expected: "bool",
            got: other.kind().to_string(),
            span,
        }),
    }
}

pub(super) fn values_equal(l: &Value, r: &Value) -> bool {
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

pub(super) fn compare(l: &Value, r: &Value, span: Span) -> Result<Ordering, RenderError> {
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
