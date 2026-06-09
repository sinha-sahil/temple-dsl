//! The tree-walking evaluator. This root holds the two dispatch entry points
//! (`evaluate` over output nodes, `evaluate_expr` over expressions) and the
//! scope types; the per-domain machinery lives in the submodules below.

use crate::error::RenderError;
use crate::parse::{Expr, ExprKind, InterpPart, LitKey, OutKind, OutNode};
use crate::value::Value;
use indexmap::IndexMap;
use smol_str::SmolStr;
use std::collections::HashMap;

mod functions;
mod methods;
mod ops;
mod path;

pub use functions::is_known_function;
pub(crate) use functions::BUILTINS;

pub type Scope = HashMap<SmolStr, Value>;

/// The let-bindings visible to an expression: a base map (the preamble or a
/// lambda's scope) plus a chain of `let … in …` overlays. Layering makes each
/// let-in binding O(1) — no per-evaluation clone of the whole map, which would
/// otherwise repeat for every element inside `map`/`filter`/`fold`.
pub struct Scopes<'a> {
    map: &'a Scope,
    layer: Option<(&'a SmolStr, &'a Value, &'a Scopes<'a>)>,
}

impl<'a> Scopes<'a> {
    pub fn base(map: &'a Scope) -> Self {
        Scopes { map, layer: None }
    }

    fn layered<'b>(&'b self, name: &'b SmolStr, value: &'b Value) -> Scopes<'b> {
        Scopes {
            map: self.map,
            layer: Some((name, value, self)),
        }
    }

    /// Innermost binding wins (let-in shadows outer let-ins and the base map).
    fn get(&self, name: &str) -> Option<&Value> {
        let mut cur = self;
        loop {
            match cur.layer {
                Some((n, v, parent)) => {
                    if n.as_str() == name {
                        return Some(v);
                    }
                    cur = parent;
                }
                None => return cur.map.get(name),
            }
        }
    }

    /// Flatten to an owned map — done once per collection walk to seed a
    /// lambda's scope, never per element.
    fn materialize(&self) -> Scope {
        let mut chain = Vec::new();
        let mut cur = self;
        while let Some((n, v, parent)) = cur.layer {
            chain.push((n, v));
            cur = parent;
        }
        let mut map = cur.map.clone();
        // outermost layer first, so inner shadows overwrite
        for (n, v) in chain.into_iter().rev() {
            map.insert(n.clone(), v.clone());
        }
        map
    }
}

pub fn evaluate(
    node: &OutNode,
    input: &Value,
    lets: &Scopes,
    this: &Scope,
) -> Result<Value, RenderError> {
    match &node.kind {
        OutKind::Literal(lit) => Ok(lit.to_value()),
        OutKind::Hole(expr) => evaluate_expr(expr, input, lets, this),
        OutKind::Object(fields) => {
            let mut obj = IndexMap::with_capacity(fields.len());
            for field in fields {
                let v = evaluate(&field.value, input, lets, this)?;
                if field.optional && matches!(v, Value::Null) {
                    continue;
                }
                obj.insert(field.key.clone(), v);
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
                        s.push_str(&functions::scalar_to_string(&v, expr.span)?);
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
    lets: &Scopes,
    this: &Scope,
) -> Result<Value, RenderError> {
    match &expr.kind {
        ExprKind::Literal(lit) => Ok(lit.to_value()),
        ExprKind::Path {
            root,
            root_span,
            segments,
        } => path::evaluate_path(root, *root_span, segments, input, lets, this),
        ExprKind::Binary { op, lhs, rhs } => {
            ops::evaluate_binary(*op, lhs, rhs, expr.span, input, lets, this)
        }
        ExprKind::Unary { op, operand } => {
            ops::evaluate_unary(*op, operand, expr.span, input, lets, this)
        }
        ExprKind::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            let c = evaluate_expr(cond, input, lets, this)?;
            let b = ops::require_bool(&c, cond.span)?;
            if b {
                evaluate_expr(then_branch, input, lets, this)
            } else {
                evaluate_expr(else_branch, input, lets, this)
            }
        }
        ExprKind::When { branches, fallback } => {
            for branch in branches {
                let c = evaluate_expr(&branch.cond, input, lets, this)?;
                let b = ops::require_bool(&c, branch.cond.span)?;
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
        ExprKind::Let {
            name, value, body, ..
        } => {
            let v = evaluate_expr(value, input, lets, this)?;
            let scope = lets.layered(name, &v);
            evaluate_expr(body, input, &scope, this)
        }
        ExprKind::ArrayLit(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(evaluate_expr(item, input, lets, this)?);
            }
            Ok(Value::Arr(out))
        }
        ExprKind::ObjectLit(entries) => {
            let mut obj = IndexMap::with_capacity(entries.len());
            for entry in entries {
                let (key, key_span) = match &entry.key {
                    LitKey::Static(k) => (k.clone(), entry.value.span),
                    LitKey::Computed(e) => match &evaluate_expr(e, input, lets, this)? {
                        Value::Str(s) => (s.clone(), e.span),
                        other => {
                            return Err(RenderError::TypeMismatch {
                                expected: "string object key",
                                got: other.kind().to_string(),
                                span: e.span,
                            });
                        }
                    },
                };
                let v = evaluate_expr(&entry.value, input, lets, this)?;
                if entry.optional && matches!(v, Value::Null) {
                    continue;
                }
                // Static/static collisions are compile errors, so any collision
                // here involves a computed key — a defined render error, never a
                // silent overwrite.
                if obj.insert(key.clone(), v).is_some() {
                    return Err(RenderError::DuplicateKey {
                        key: key.to_string(),
                        span: key_span,
                    });
                }
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
            functions::call_func(name, &values, *name_span)
        }
        ExprKind::Access { base, segments } => {
            let b = evaluate_expr(base, input, lets, this)?;
            path::evaluate_access(b, base.span, segments, input, lets, this)
        }
    }
}
