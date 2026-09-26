mod array;
mod object;
mod string;

use crate::common::error::{RenderError, Span};
use crate::common::value::Value;
use crate::eval::budget::Meter;
use crate::eval::ops::require_bool;
use crate::eval::scope::Scope;
use crate::eval::{evaluate_expr, Context};
use crate::syntax::ast::{Expr, ExprKind, LambdaParam};
use indexmap::IndexMap;
use smol_str::SmolStr;
use std::iter::Cloned;

/// Each dispatcher pairs a method with what it scans up front; a lambda pays
/// as it runs, a copy as it is made.
type Method<R, M> = fn(&R, &Call<M>) -> Result<Value, <M as Meter>::Err>;

pub(super) struct Call<'c, M: Meter> {
    pub name: &'c SmolStr,
    pub args: &'c [Expr],
    pub span: Span,
    pub ctx: &'c Context<'c, M>,
    pub scope: &'c Scope<'c>,
}

impl<'c, M: Meter> Call<'c, M> {
    fn arity(&self, expected: usize) -> Result<(), RenderError> {
        if self.args.len() != expected {
            return Err(RenderError::ArityMismatch {
                method: self.name.to_string(),
                expected,
                got: self.args.len(),
                span: self.span,
            });
        }
        Ok(())
    }

    fn charge(&self, units: u64) -> Result<(), M::Err> {
        self.ctx.meter.charge(units, self.span)
    }

    fn copy(&self, value: &Value) -> Result<Value, M::Err> {
        self.ctx.meter.charge_copy(value, self.span)?;
        Ok(value.clone())
    }

    fn copy_all<'v, I>(&self, values: I) -> Result<Cloned<I::IntoIter>, M::Err>
    where
        I: IntoIterator<Item = &'v Value>,
        I::IntoIter: ExactSizeIterator + Clone,
    {
        let values = values.into_iter();
        self.charge_copies(values.clone())?;
        Ok(values.cloned())
    }

    fn copy_object(
        &self,
        object: &IndexMap<SmolStr, Value>,
    ) -> Result<IndexMap<SmolStr, Value>, M::Err> {
        self.charge_copies(object.values())?;
        Ok(object.clone())
    }

    /// One unit per value plus its size. Charging walks every value first, so
    /// one too deep to clone is refused before the clone recurses.
    fn charge_copies<'v>(
        &self,
        values: impl ExactSizeIterator<Item = &'v Value>,
    ) -> Result<(), M::Err> {
        self.charge(values.len() as u64)?;
        for value in values {
            self.ctx.meter.charge_copy(value, self.span)?;
        }
        Ok(())
    }

    fn arg(&self, i: usize) -> Result<Value, M::Err> {
        evaluate_expr(&self.args[i], self.ctx, self.scope)
    }

    /// The string is moved out, not copied.
    fn string_arg(&self, i: usize) -> Result<SmolStr, M::Err> {
        let mut value = self.arg(i)?;
        match &mut value {
            Value::Str(s) => Ok(std::mem::take(s)),
            other => {
                Err(RenderError::type_mismatch("string", other.kind(), self.args[i].span).into())
            }
        }
    }

    fn int_arg(&self, i: usize) -> Result<i64, M::Err> {
        match self.arg(i)? {
            Value::Int(n) => Ok(n),
            other => {
                Err(RenderError::type_mismatch("integer", other.kind(), self.args[i].span).into())
            }
        }
    }

    fn lambda(&self, i: usize, arity: usize) -> Result<Lambda<'c>, RenderError> {
        let lambda = &self.args[i];
        let ExprKind::Lambda { params, body } = &lambda.kind else {
            return Err(RenderError::LambdaExpected { span: lambda.span });
        };
        if params.len() != arity {
            return Err(RenderError::ArityMismatch {
                method: "lambda".to_string(),
                expected: arity,
                got: params.len(),
                span: lambda.span,
            });
        }
        Ok(Lambda {
            params,
            body,
            span: lambda.span,
        })
    }
}

struct Lambda<'c> {
    params: &'c [LambdaParam],
    body: &'c Expr,
    span: Span,
}

impl Lambda<'_> {
    fn run1<M: Meter>(&self, a: &Value, call: &Call<M>) -> Result<Value, M::Err> {
        let scope = call.scope.layered(&self.params[0].name, a);
        evaluate_expr(self.body, call.ctx, &scope)
    }

    fn run2<M: Meter>(&self, a: &Value, b: &Value, call: &Call<M>) -> Result<Value, M::Err> {
        let outer = call.scope.layered(&self.params[0].name, a);
        let scope = outer.layered(&self.params[1].name, b);
        evaluate_expr(self.body, call.ctx, &scope)
    }

    fn test<M: Meter>(&self, a: &Value, call: &Call<M>) -> Result<bool, M::Err> {
        let verdict = self.run1(a, call)?;
        Ok(require_bool(&verdict, self.span)?)
    }
}

pub(super) fn call_method<M: Meter>(receiver: &Value, call: &Call<M>) -> Result<Value, M::Err> {
    match receiver {
        Value::Str(text) => string::call(text, call),
        Value::Arr(items) => array::call(items, call),
        Value::Obj(object) => object::call(object, call),
        other => Err(RenderError::unknown_method(call.name, other.kind(), call.span).into()),
    }
}

fn slice_bounds(start: i64, end: i64, len: usize) -> (usize, usize) {
    let len = len as i64;
    let from = start.clamp(0, len);
    let to = end.clamp(from, len);
    (from as usize, to as usize)
}
