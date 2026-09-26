use super::budget::Meter;
use super::methods::{call_method, Call};
use super::scope::Scope;
use super::{evaluate_expr, Context};
use crate::common::error::{RenderError, Span};
use crate::common::limits::MAX_VALUE_DEPTH;
use crate::common::value::Value;
use crate::syntax::ast::PathSegment;
use indexmap::IndexMap;
use smol_str::SmolStr;
use std::borrow::Cow;

/// Objects this small are scanned instead of hashed; hashing every lookup
/// measured about 40% slower on `render_lambdas`.
const SCAN_LIMIT: usize = 8;

enum Cursor<'a> {
    Borrowed(&'a Value),
    Owned(Value),
}

impl<'a> Cursor<'a> {
    fn as_ref(&self) -> &Value {
        match self {
            Cursor::Borrowed(v) => v,
            Cursor::Owned(v) => v,
        }
    }

    fn into_cow(self) -> Cow<'a, Value> {
        match self {
            Cursor::Borrowed(v) => Cow::Borrowed(v),
            Cursor::Owned(v) => Cow::Owned(v),
        }
    }
}

pub(super) fn evaluate_path<M: Meter>(
    root: &SmolStr,
    root_span: Span,
    segments: &[PathSegment],
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    match evaluate_path_cow(root, root_span, segments, ctx, scope)? {
        Cow::Borrowed(v) => {
            ctx.meter.charge_copy(v, root_span)?;
            Ok(v.clone())
        }
        Cow::Owned(v) => Ok(v),
    }
}

pub(super) fn evaluate_path_cow<'r, M: Meter>(
    root: &SmolStr,
    root_span: Span,
    segments: &[PathSegment],
    ctx: &'r Context<M>,
    scope: &'r Scope,
) -> Result<Cow<'r, Value>, M::Err> {
    let (start, first_segment) = match root.as_str() {
        "input" => (Cursor::Borrowed(ctx.input), 0),
        "this" => match segments.first() {
            Some(PathSegment::Field { name, span, .. }) => match ctx.this.get(name) {
                Some(v) => (Cursor::Borrowed(v), 1),
                None => {
                    return Err(RenderError::MissingPath {
                        path: format!("this.{name}"),
                        key: Some(name.to_string()),
                        span: *span,
                    }
                    .into());
                }
            },
            _ => {
                return Err(RenderError::TypeMismatch {
                    expected: "`this` followed by `.field`",
                    got: "bare `this`".to_string(),
                    span: root_span,
                }
                .into());
            }
        },
        _ => match scope.get(root) {
            Some(v) => (Cursor::Borrowed(v), 0),
            None => {
                return Err(RenderError::TypeMismatch {
                    expected: "known identifier",
                    got: root.to_string(),
                    span: root_span,
                }
                .into());
            }
        },
    };
    walk_segments(start, root_span, &segments[first_segment..], ctx, scope).map(Cursor::into_cow)
}

pub(super) fn evaluate_access<M: Meter>(
    base: Value,
    base_span: Span,
    segments: &[PathSegment],
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Value, M::Err> {
    walk_segments(Cursor::Owned(base), base_span, segments, ctx, scope)
        .map(|cursor| cursor.into_cow().into_owned())
}

fn walk_segments<'r, M: Meter>(
    mut current: Cursor<'r>,
    root_span: Span,
    segments: &[PathSegment],
    ctx: &Context<M>,
    scope: &Scope,
) -> Result<Cursor<'r>, M::Err> {
    for segment in segments {
        current = match segment {
            PathSegment::Field {
                name,
                span,
                optional,
            } => match field_step(current, name, *span, *optional)? {
                Some(next) => next,
                None => return Ok(Cursor::Owned(Value::Null)),
            },
            PathSegment::Method { name, span, args } => {
                let call = Call {
                    name,
                    args,
                    span: *span,
                    ctx,
                    scope,
                };
                Cursor::Owned(call_method(current.as_ref(), &call)?)
            }
            PathSegment::Index { expr, span } => {
                let index = evaluate_expr(expr, ctx, scope)?;
                index_step(current, &index, *span)?
            }
        };
    }

    // refuse too-deep values before anything clones or compares them
    let (too_deep, values_seen) = probe(current.as_ref());
    if too_deep {
        return Err(RenderError::ValueTooDeep {
            limit: MAX_VALUE_DEPTH,
            span: root_span,
        }
        .into());
    }
    // under a budget, reading a large borrowed value is work even without a
    // copy; values a method built were charged as they were built
    if M::LIMITED && values_seen > 1 && matches!(current, Cursor::Borrowed(_)) {
        ctx.meter.charge(values_seen - 1, root_span)?;
    }
    Ok(current)
}

/// `None` means the path ends here with null.
#[inline(always)]
fn field_step<'r>(
    current: Cursor<'r>,
    name: &SmolStr,
    span: Span,
    optional: bool,
) -> Result<Option<Cursor<'r>>, RenderError> {
    if optional && matches!(current.as_ref(), Value::Null) {
        return Ok(None);
    }
    let next = match current {
        Cursor::Borrowed(value) => match value {
            Value::Obj(obj) => field(obj, name).map(Cursor::Borrowed),
            other => return Err(RenderError::type_mismatch("object", other.kind(), span)),
        },
        Cursor::Owned(mut value) => match &mut value {
            Value::Obj(obj) => take_field(obj, name).map(Cursor::Owned),
            other => return Err(RenderError::type_mismatch("object", other.kind(), span)),
        },
    };
    match next {
        Some(cursor) => Ok(Some(cursor)),
        None if optional => Ok(None),
        None => Err(RenderError::missing_field(name, span)),
    }
}

#[inline]
fn index_step<'r>(
    current: Cursor<'r>,
    index: &Value,
    span: Span,
) -> Result<Cursor<'r>, RenderError> {
    Ok(match current {
        Cursor::Borrowed(value) => match value {
            Value::Arr(items) => Cursor::Borrowed(&items[array_index(index, items.len(), span)?]),
            Value::Obj(obj) => match index {
                Value::Str(key) => match field(obj, key) {
                    Some(v) => Cursor::Borrowed(v),
                    None => return Err(RenderError::missing_field(key, span)),
                },
                other => {
                    return Err(RenderError::type_mismatch(
                        "string key for object index",
                        other.kind(),
                        span,
                    ))
                }
            },
            other => return Err(RenderError::not_indexable(other.kind(), span)),
        },
        Cursor::Owned(mut value) => match &mut value {
            Value::Arr(items) => {
                let i = array_index(index, items.len(), span)?;
                Cursor::Owned(items.swap_remove(i))
            }
            Value::Obj(obj) => match index {
                Value::Str(key) => match take_field(obj, key) {
                    Some(v) => Cursor::Owned(v),
                    None => return Err(RenderError::missing_field(key, span)),
                },
                other => {
                    return Err(RenderError::type_mismatch(
                        "string key for object index",
                        other.kind(),
                        span,
                    ))
                }
            },
            other => return Err(RenderError::not_indexable(other.kind(), span)),
        },
    })
}

#[inline]
fn field<'v>(obj: &'v IndexMap<SmolStr, Value>, name: &str) -> Option<&'v Value> {
    if obj.len() <= SCAN_LIMIT {
        obj.iter()
            .find(|(key, _)| key.as_str() == name)
            .map(|(_, value)| value)
    } else {
        obj.get(name)
    }
}

#[inline]
fn take_field(obj: &mut IndexMap<SmolStr, Value>, name: &str) -> Option<Value> {
    if obj.len() <= SCAN_LIMIT {
        let position = obj.keys().position(|key| key.as_str() == name)?;
        obj.swap_remove_index(position).map(|(_, value)| value)
    } else {
        obj.swap_remove(name)
    }
}

fn array_index(index: &Value, len: usize, span: Span) -> Result<usize, RenderError> {
    match index {
        Value::Int(i) if *i >= 0 && (*i as usize) < len => Ok(*i as usize),
        Value::Int(i) => Err(RenderError::IndexOutOfBounds {
            index: *i,
            length: len,
            span,
        }),
        other => Err(RenderError::TypeMismatch {
            expected: "integer index",
            got: other.kind().to_string(),
            span,
        }),
    }
}

/// Whether `root` is too deep, and how many values were seen. Recursion is
/// bounded by `MAX_VALUE_DEPTH`.
#[inline]
fn probe(root: &Value) -> (bool, u64) {
    if !matches!(root, Value::Arr(_) | Value::Obj(_)) {
        return (false, 1);
    }
    fn walk(v: &Value, depth: usize, seen: &mut u64) -> bool {
        *seen += 1;
        if depth > MAX_VALUE_DEPTH {
            return true;
        }
        match v {
            Value::Arr(items) => items.iter().any(|c| walk(c, depth + 1, seen)),
            Value::Obj(obj) => obj.values().any(|c| walk(c, depth + 1, seen)),
            _ => false,
        }
    }
    let mut seen = 0;
    let too_deep = walk(root, 1, &mut seen);
    (too_deep, seen)
}
