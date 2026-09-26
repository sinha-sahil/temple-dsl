//! The evaluator recurses once per tree level, and a debug build gives every
//! local its own stack slot for the whole call, so functions on that path
//! stay small and split by kind; `#[inline(never)]` keeps a big one out.
//! `#[inline]` sits only where the benchmarks show it matters.

pub(crate) mod budget;
pub(crate) mod builtins;
mod methods;
mod ops;
mod path;
pub(crate) mod scope;

use crate::common::error::{RenderError, Span};
use crate::common::value::Value;
use crate::syntax::ast::{
    Expr, ExprKind, InterpPart, LitKey, Module, ObjEntry, ObjField, OutKind, OutNode, WhenBranch,
};
use budget::{Meter, Unmetered};
use indexmap::IndexMap;
use rustc_hash::FxHashMap;
use scope::{Scope, Vars};
use smol_str::SmolStr;
use std::borrow::Cow;

#[derive(Debug, Clone)]
pub(crate) struct FunctionDef {
    pub params: Vec<SmolStr>,
    pub body: Expr,
}

pub(crate) type FunctionTable = FxHashMap<SmolStr, FunctionDef>;

pub(crate) fn run_template(
    module: &Module,
    order: &[usize],
    input: &Value,
) -> Result<Value, RenderError> {
    let empty = Vars::default();
    let ctx = Context::new(input, &empty, &empty, None, &Unmetered);
    let mut lets = Vars::with_capacity_and_hasher(module.lets.len(), Default::default());
    for binding in &module.lets {
        // each `let` sees the ones above it
        let value = evaluate_expr(&binding.expr, &ctx, &Scope::base(&lets))?;
        lets.insert(binding.name.clone(), value);
    }
    render_output(&module.output, order, &ctx, &Scope::base(&lets))
}

pub(crate) fn run_expr<M: Meter>(
    expr: &Expr,
    input: &Value,
    vars: &Vars,
    functions: Option<&FunctionTable>,
    meter: &M,
) -> Result<Value, M::Err> {
    let no_this = Vars::default();
    let ctx = Context::new(input, &no_this, vars, functions, meter);
    evaluate_expr(expr, &ctx, &Scope::base(vars))
}

pub(crate) fn run_output<M: Meter>(
    node: &OutNode,
    order: &[usize],
    input: &Value,
    vars: &Vars,
    functions: Option<&FunctionTable>,
    meter: &M,
) -> Result<Value, M::Err> {
    let no_this = Vars::default();
    let ctx = Context::new(input, &no_this, vars, functions, meter);
    render_output(node, order, &ctx, &Scope::base(vars))
}

struct Context<'a, M: Meter> {
    input: &'a Value,
    /// The sibling fields already rendered, for `this.key`.
    this: &'a Vars,
    functions: Option<&'a FunctionTable>,
    globals: &'a Vars,
    meter: &'a M,
    depth: u32,
}

impl<'a, M: Meter> Context<'a, M> {
    fn new(
        input: &'a Value,
        this: &'a Vars,
        globals: &'a Vars,
        functions: Option<&'a FunctionTable>,
        meter: &'a M,
    ) -> Self {
        Context {
            input,
            this,
            functions,
            globals,
            meter,
            depth: 0,
        }
    }

    fn with_this<'b>(&'b self, this: &'b Vars) -> Context<'b, M> {
        Context { this, ..*self }
    }

    #[inline(always)]
    fn deeper(&self) -> Context<'a, M> {
        Context {
            depth: self.depth + 1,
            ..*self
        }
    }
}

/// Object fields are filled in `order` so each `this.key` sees its
/// sibling; keys still come out in source order.
fn render_output<M: Meter>(
    output: &OutNode,
    order: &[usize],
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let OutKind::Object(fields) = &output.kind else {
        return evaluate(output, ctx, scope);
    };
    let mut this = Vars::with_capacity_and_hasher(fields.len(), Default::default());
    for &position in order {
        let field = &fields[position];
        let value = evaluate(&field.value, &ctx.with_this(&this), scope)?;
        this.insert(field.key.clone(), value);
    }
    let mut result = IndexMap::with_capacity(fields.len());
    for field in fields {
        if let Some(value) = this.remove(&field.key) {
            if field.optional && matches!(value, Value::Null) {
                continue;
            }
            result.insert(field.key.clone(), value);
        }
    }
    Ok(Value::Obj(result))
}

fn evaluate<M: Meter>(node: &OutNode, ctx: &Context<M>, scope: &Scope) -> Result<Value, M::Err> {
    match &node.kind {
        OutKind::Literal(literal) => Ok(literal.to_value()),
        OutKind::Hole(expr) => evaluate_expr(expr, ctx, scope),
        OutKind::Object(fields) => output_object(fields, ctx, scope),
        OutKind::Array(items) => output_array(items, ctx, scope),
        OutKind::Interp(parts) => output_interp(parts, ctx, scope),
    }
}

fn output_object<M: Meter>(
    fields: &[ObjField],
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let mut object = IndexMap::with_capacity(fields.len());
    for field in fields {
        let value = evaluate(&field.value, ctx, scope)?;
        if field.optional && matches!(value, Value::Null) {
            continue;
        }
        object.insert(field.key.clone(), value);
    }
    Ok(Value::Obj(object))
}

fn output_array<M: Meter>(
    items: &[OutNode],
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let mut array = Vec::with_capacity(items.len());
    for item in items {
        array.push(evaluate(item, ctx, scope)?);
    }
    Ok(Value::Arr(array))
}

fn output_interp<M: Meter>(
    parts: &[InterpPart],
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let mut text = String::new();
    for part in parts {
        match part {
            InterpPart::Text(literal) => text.push_str(literal),
            InterpPart::Hole(expr) => {
                let value = evaluate_expr(expr, ctx, scope)?;
                text.push_str(&builtins::scalar_to_string(&value, expr.span)?);
            }
        }
    }
    Ok(Value::Str(text.into()))
}

fn evaluate_expr<M: Meter>(expr: &Expr, ctx: &Context<M>, scope: &Scope) -> Result<Value, M::Err> {
    ctx.meter.charge(1, expr.span)?;
    if !M::LIMITED {
        return evaluate_kind(expr, ctx, scope);
    }
    ctx.meter.enter(ctx.depth, expr.span)?;
    evaluate_kind(expr, &ctx.deeper(), scope)
}

#[inline(always)]
fn evaluate_kind<M: Meter>(expr: &Expr, ctx: &Context<M>, scope: &Scope) -> Result<Value, M::Err> {
    match &expr.kind {
        ExprKind::Literal(literal) => Ok(literal.to_value()),
        ExprKind::Path {
            root,
            root_span,
            segments,
        } => path::evaluate_path(root, *root_span, segments, ctx, scope),
        ExprKind::Binary { op, lhs, rhs } => {
            ops::evaluate_binary(*op, lhs, rhs, expr.span, ctx, scope)
        }
        ExprKind::Unary { op, operand } => ops::evaluate_unary(*op, operand, expr.span, ctx, scope),
        ExprKind::Ternary {
            cond,
            then_branch,
            else_branch,
        } => evaluate_ternary(cond, then_branch, else_branch, ctx, scope),
        ExprKind::When { branches, fallback } => {
            evaluate_when(branches, fallback.as_deref(), expr.span, ctx, scope)
        }
        ExprKind::Lambda { .. } => Err(RenderError::type_mismatch(
            "value",
            "lambda (only valid as a method argument)",
            expr.span,
        )
        .into()),
        ExprKind::Let {
            name, value, body, ..
        } => evaluate_let(name, value, body, ctx, scope),
        ExprKind::ArrayLit(items) => evaluate_array(items, ctx, scope),
        ExprKind::ObjectLit(entries) => evaluate_object(entries, ctx, scope),
        ExprKind::FuncCall {
            name,
            name_span,
            args,
        } => evaluate_call(name, *name_span, args, ctx, scope),
        ExprKind::Access { base, segments } => {
            let base_value = evaluate_expr(base, ctx, scope)?;
            path::evaluate_access(base_value, base.span, segments, ctx, scope)
        }
    }
}

fn evaluate_ternary<M: Meter>(
    cond: &Expr,
    then_branch: &Expr,
    else_branch: &Expr,
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let condition = evaluate_expr(cond, ctx, scope)?;
    if ops::require_bool(&condition, cond.span)? {
        evaluate_expr(then_branch, ctx, scope)
    } else {
        evaluate_expr(else_branch, ctx, scope)
    }
}

fn evaluate_when<M: Meter>(
    branches: &[WhenBranch],
    fallback: Option<&Expr>,
    span: Span,
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    for branch in branches {
        let condition = evaluate_expr(&branch.cond, ctx, scope)?;
        if ops::require_bool(&condition, branch.cond.span)? {
            return evaluate_expr(&branch.result, ctx, scope);
        }
    }
    match fallback {
        Some(fallback) => evaluate_expr(fallback, ctx, scope),
        None => Err(RenderError::WhenNoMatch { span }.into()),
    }
}

fn evaluate_let<M: Meter>(
    name: &SmolStr,
    value: &Expr,
    body: &Expr,
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let bound = evaluate_expr(value, ctx, scope)?;
    evaluate_expr(body, ctx, &scope.layered(name, &bound))
}

fn evaluate_array<M: Meter>(
    items: &[Expr],
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let mut array = Vec::with_capacity(items.len());
    for item in items {
        array.push(evaluate_expr(item, ctx, scope)?);
    }
    Ok(Value::Arr(array))
}

fn evaluate_object<M: Meter>(
    entries: &[ObjEntry],
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    let mut object = IndexMap::with_capacity(entries.len());
    for entry in entries {
        let (key, key_span) = match &entry.key {
            LitKey::Static(key) => (key.clone(), entry.value.span),
            LitKey::Computed(key_expr) => match &evaluate_expr(key_expr, ctx, scope)? {
                Value::Str(key) => (key.clone(), key_expr.span),
                other => {
                    return Err(RenderError::TypeMismatch {
                        expected: "string object key",
                        got: other.kind().to_string(),
                        span: key_expr.span,
                    }
                    .into());
                }
            },
        };
        let value = evaluate_expr(&entry.value, ctx, scope)?;
        if entry.optional && matches!(value, Value::Null) {
            continue;
        }
        // two written keys can't collide (the checker refuses that), so a
        // collision here involves a computed key and is an error, not an
        // overwrite
        if object.insert(key.clone(), value).is_some() {
            return Err(RenderError::DuplicateKey {
                key: key.to_string(),
                span: key_span,
            }
            .into());
        }
    }
    Ok(Value::Obj(object))
}

#[inline(never)]
fn evaluate_call<M: Meter>(
    name: &SmolStr,
    name_span: Span,
    args: &[Expr],
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    // a host function only reads its arguments, so paths are passed borrowed
    if let Some(function) = ctx.functions.and_then(|table| table.get(name)) {
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(ops::operand(arg, ctx, scope)?);
        }
        return call_host_function(function, name, name_span, &values, ctx);
    }
    let mut values = Vec::with_capacity(args.len());
    for arg in args {
        values.push(evaluate_expr(arg, ctx, scope)?);
    }
    ctx.meter.charge(args_weight(&values), name_span)?;
    Ok(builtins::call_builtin(name, &values, name_span)?)
}

/// A host function body sees globals and its parameters, never the caller's
/// locals.
fn call_host_function<M: Meter>(
    function: &FunctionDef,
    name: &SmolStr,
    span: Span,
    args: &[Cow<Value>],
    ctx: &Context<M>,
) -> Result<Value, M::Err> {
    if args.len() != function.params.len() {
        return Err(RenderError::ArityMismatch {
            method: name.to_string(),
            expected: function.params.len(),
            got: args.len(),
            span,
        }
        .into());
    }
    let scope = Scope::with_params(ctx.globals, &function.params, args);
    evaluate_expr(&function.body, ctx, &scope)
}

fn args_weight(args: &[Value]) -> u64 {
    args.iter()
        .map(|arg| match arg {
            Value::Arr(items) => items.len() as u64,
            Value::Obj(object) => object.len() as u64,
            Value::Str(text) => (text.len() / 64) as u64,
            _ => 0,
        })
        .sum()
}
