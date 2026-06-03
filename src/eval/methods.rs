//! Collection methods (`.map`/`.filter`/`.fold`/`.sum`/…) and the lambda
//! binding that backs the iterating ones.

use super::ops::{arith, require_bool};
use super::{evaluate_expr, Scope};
use crate::error::{RenderError, Span};
use crate::parse::{Expr, ExprKind, LambdaParam};
use crate::value::Value;
use rust_decimal::Decimal;
use smol_str::SmolStr;

pub(super) fn call_method(
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
            let mut other = evaluate_expr(&args[0], input, lets, this)?;
            match &mut other {
                // append drains the other array via &mut — no move out of a Drop type.
                Value::Arr(b) => {
                    let mut result = a.clone();
                    result.append(b);
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
