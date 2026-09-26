use crate::common::error::Span;
use crate::eval::builtins::is_builtin_function;
use crate::syntax::ast::{Expr, ExprKind, PathSegment};
use smol_str::SmolStr;

/// A read of an `input` path: `input.data.order` is `["data", "order"]`,
/// stopping at the first index or method call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)] // fields name themselves
pub struct InputRead {
    pub path: Vec<SmolStr>,
    /// Whether any step uses `?.`.
    pub optional: bool,
    pub span: Span,
}

pub(super) struct CallSite {
    pub name: SmolStr,
    pub arity: usize,
    pub span: Span,
}

fn input_read(expr: &Expr) -> Option<InputRead> {
    let ExprKind::Path { root, segments, .. } = &expr.kind else {
        return None;
    };
    if root != "input" {
        return None;
    }
    let mut read = InputRead {
        path: Vec::new(),
        optional: false,
        span: expr.span,
    };
    for segment in segments {
        let PathSegment::Field { name, optional, .. } = segment else {
            break;
        };
        read.path.push(name.clone());
        read.optional |= *optional;
    }
    Some(read)
}

pub(super) fn reads_in_expr(expr: &Expr, reads: &mut Vec<InputRead>) {
    reads.extend(input_read(expr));
    expr.for_each_child(&mut |child| reads_in_expr(child, reads));
}

pub(super) fn call_sites(expr: &Expr, calls: &mut Vec<CallSite>) {
    if let ExprKind::FuncCall {
        name,
        name_span,
        args,
    } = &expr.kind
    {
        if !is_builtin_function(name) {
            calls.push(CallSite {
                name: name.clone(),
                arity: args.len(),
                span: *name_span,
            });
        }
    }
    expr.for_each_child(&mut |child| call_sites(child, calls));
}
