use super::budget::Meter;
use super::scope::Scope;
use super::{evaluate_expr, path, Context};
use crate::common::error::{RenderError, Span};
use crate::common::value::Value;
use crate::syntax::ast::{BinOp, Expr, ExprKind, UnOp};
use rust_decimal::Decimal;
use std::borrow::Cow;
use std::cmp::Ordering;

pub(super) fn evaluate_binary<M: Meter>(
    op: BinOp,
    lhs: &Expr,
    rhs: &Expr,
    span: Span,
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    // short-circuiting operators decide whether to evaluate the right side
    match op {
        BinOp::And => evaluate_and(lhs, rhs, ctx, scope),
        BinOp::Or => evaluate_or(lhs, rhs, ctx, scope),
        BinOp::Coalesce => evaluate_coalesce(lhs, rhs, ctx, scope),
        _ => evaluate_strict(op, lhs, rhs, span, ctx, scope),
    }
}

fn evaluate_and<M: Meter>(
    lhs: &Expr,
    rhs: &Expr,
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let left = evaluate_expr(lhs, ctx, scope)?;
    if !require_bool(&left, lhs.span)? {
        return Ok(Value::Bool(false));
    }
    let right = evaluate_expr(rhs, ctx, scope)?;
    Ok(Value::Bool(require_bool(&right, rhs.span)?))
}

fn evaluate_or<M: Meter>(
    lhs: &Expr,
    rhs: &Expr,
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let left = evaluate_expr(lhs, ctx, scope)?;
    if require_bool(&left, lhs.span)? {
        return Ok(Value::Bool(true));
    }
    let right = evaluate_expr(rhs, ctx, scope)?;
    Ok(Value::Bool(require_bool(&right, rhs.span)?))
}

fn evaluate_coalesce<M: Meter>(
    lhs: &Expr,
    rhs: &Expr,
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let left = evaluate_expr(lhs, ctx, scope)?;
    if matches!(left, Value::Null) {
        return evaluate_expr(rhs, ctx, scope);
    }
    Ok(left)
}

fn evaluate_strict<M: Meter>(
    op: BinOp,
    lhs: &Expr,
    rhs: &Expr,
    span: Span,
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let left = operand(lhs, ctx, scope)?;
    let right = operand(rhs, ctx, scope)?;
    Ok(apply_binary(op, &left, &right, span)?)
}

/// Evaluate a read-only operand; paths stay borrowed, so `a.id == b.id`
/// copies nothing.
pub(super) fn operand<'r, M: Meter>(
    expr: &Expr,
    ctx: &'r Context<M>,
    scope: &'r Scope,
) -> Result<Cow<'r, Value>, M::Err> {
    match &expr.kind {
        ExprKind::Path {
            root,
            root_span,
            segments,
        } => {
            ctx.meter.charge(1, expr.span)?;
            path::evaluate_path_cow(root, *root_span, segments, ctx, scope)
        }
        _ => evaluate_expr(expr, ctx, scope).map(Cow::Owned),
    }
}

fn apply_binary(op: BinOp, left: &Value, right: &Value, span: Span) -> Result<Value, RenderError> {
    match op {
        BinOp::Add => arith(left, right, span, i64::checked_add, Decimal::checked_add),
        BinOp::Sub => arith(left, right, span, i64::checked_sub, Decimal::checked_sub),
        BinOp::Mul => arith(left, right, span, i64::checked_mul, Decimal::checked_mul),
        BinOp::Div => div(left, right, span),
        BinOp::Mod => rem(left, right, span),
        BinOp::Eq => Ok(Value::Bool(values_equal(left, right))),
        BinOp::Ne => Ok(Value::Bool(!values_equal(left, right))),
        BinOp::Lt => compare(left, right, span).map(|order| Value::Bool(order == Ordering::Less)),
        BinOp::Le => {
            compare(left, right, span).map(|order| Value::Bool(order != Ordering::Greater))
        }
        BinOp::Gt => {
            compare(left, right, span).map(|order| Value::Bool(order == Ordering::Greater))
        }
        BinOp::Ge => compare(left, right, span).map(|order| Value::Bool(order != Ordering::Less)),
        BinOp::And => Ok(Value::Bool(
            require_bool(left, span)? && require_bool(right, span)?,
        )),
        BinOp::Or => Ok(Value::Bool(
            require_bool(left, span)? || require_bool(right, span)?,
        )),
        BinOp::Coalesce => Ok(if matches!(left, Value::Null) {
            right.clone()
        } else {
            left.clone()
        }),
    }
}

pub(super) fn evaluate_unary<M: Meter>(
    op: UnOp,
    operand: &Expr,
    span: Span,
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let value = evaluate_expr(operand, ctx, scope)?;
    Ok(apply_unary(op, value, span)?)
}

fn apply_unary(op: UnOp, value: Value, span: Span) -> Result<Value, RenderError> {
    match op {
        UnOp::Neg => match value {
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
        UnOp::Not => Ok(Value::Bool(!require_bool(&value, span)?)),
    }
}

/// `+`, `-`, `*`: integers stay integers; any decimal makes a decimal.
pub(super) fn arith<I, D>(
    left: &Value,
    right: &Value,
    span: Span,
    int_op: I,
    decimal_op: D,
) -> Result<Value, RenderError>
where
    I: Fn(i64, i64) -> Option<i64>,
    D: Fn(Decimal, Decimal) -> Option<Decimal>,
{
    let checked = |result: Option<Decimal>| {
        result
            .map(Value::Decimal)
            .ok_or(RenderError::ArithmeticOverflow { span })
    };
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => int_op(*a, *b)
            .map(Value::Int)
            .ok_or(RenderError::ArithmeticOverflow { span }),
        (Value::Decimal(a), Value::Decimal(b)) => checked(decimal_op(*a, *b)),
        (Value::Int(a), Value::Decimal(b)) => checked(decimal_op(Decimal::from(*a), *b)),
        (Value::Decimal(a), Value::Int(b)) => checked(decimal_op(*a, Decimal::from(*b))),
        _ => Err(RenderError::type_mismatch(
            "number",
            format!("{} and {}", left.kind(), right.kind()),
            span,
        )),
    }
}

/// `/` always produces a decimal, so `7 / 2` is `3.5`.
fn div(left: &Value, right: &Value, span: Span) -> Result<Value, RenderError> {
    let (dividend, divisor) = as_decimals(left, right, span)?;
    if divisor.is_zero() {
        return Err(RenderError::DivideByZero { span });
    }
    dividend
        .checked_div(divisor)
        .map(Value::Decimal)
        .ok_or(RenderError::ArithmeticOverflow { span })
}

fn rem(left: &Value, right: &Value, span: Span) -> Result<Value, RenderError> {
    if let (Value::Int(dividend), Value::Int(divisor)) = (left, right) {
        if *divisor == 0 {
            return Err(RenderError::DivideByZero { span });
        }
        return dividend
            .checked_rem(*divisor)
            .map(Value::Int)
            .ok_or(RenderError::ArithmeticOverflow { span });
    }
    let (dividend, divisor) = as_decimals(left, right, span)?;
    if divisor.is_zero() {
        return Err(RenderError::DivideByZero { span });
    }
    dividend
        .checked_rem(divisor)
        .map(Value::Decimal)
        .ok_or(RenderError::ArithmeticOverflow { span })
}

fn as_decimals(left: &Value, right: &Value, span: Span) -> Result<(Decimal, Decimal), RenderError> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok((Decimal::from(*a), Decimal::from(*b))),
        (Value::Decimal(a), Value::Decimal(b)) => Ok((*a, *b)),
        (Value::Int(a), Value::Decimal(b)) => Ok((Decimal::from(*a), *b)),
        (Value::Decimal(a), Value::Int(b)) => Ok((*a, Decimal::from(*b))),
        _ => Err(RenderError::type_mismatch(
            "number",
            format!("{} and {}", left.kind(), right.kind()),
            span,
        )),
    }
}

pub(super) fn require_bool(value: &Value, span: Span) -> Result<bool, RenderError> {
    match value {
        Value::Bool(b) => Ok(*b),
        other => Err(RenderError::TypeMismatch {
            expected: "bool",
            got: other.kind().to_string(),
            span,
        }),
    }
}

/// `==`: numbers by value across int and decimal; collections element-wise.
pub(super) fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
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
                    .all(|(key, value)| b.get(key).is_some_and(|other| values_equal(value, other)))
        }
        _ => false,
    }
}

/// `<`, `<=`, `>`, `>=`: numbers only.
pub(super) fn compare(left: &Value, right: &Value, span: Span) -> Result<Ordering, RenderError> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok(a.cmp(b)),
        (Value::Decimal(a), Value::Decimal(b)) => Ok(a.cmp(b)),
        (Value::Int(a), Value::Decimal(b)) => Ok(Decimal::from(*a).cmp(b)),
        (Value::Decimal(a), Value::Int(b)) => Ok(a.cmp(&Decimal::from(*b))),
        _ => Err(RenderError::TypeMismatch {
            expected: "comparable numbers",
            got: format!("{} and {}", left.kind(), right.kind()),
            span,
        }),
    }
}
