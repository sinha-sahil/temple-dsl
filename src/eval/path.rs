//! Path traversal — `input.a.b[i].method()` walked over a borrow cursor, plus
//! the value-depth guard applied to whatever the path produces.

use super::methods::call_method;
use super::{evaluate_expr, Scope, Scopes};
use crate::error::{RenderError, Span};
use crate::parse::PathSegment;
use crate::value::Value;
use smol_str::SmolStr;

/// Depth cap on a materialized value, so deep nesting is a typed error rather than a
/// stack overflow in clone/`==`/deserialize. Matches serde_json's parse-recursion cap.
const MAX_VALUE_DEPTH: usize = 128;

/// Iterative (heap-stack) depth probe — never recurses, so it cannot itself
/// overflow. Walks only the given value, not the whole input.
fn value_too_deep(root: &Value) -> bool {
    let mut stack: Vec<(&Value, usize)> = vec![(root, 1)];
    while let Some((v, depth)) = stack.pop() {
        if depth > MAX_VALUE_DEPTH {
            return true;
        }
        match v {
            Value::Arr(items) => stack.extend(items.iter().map(|c| (c, depth + 1))),
            Value::Obj(obj) => stack.extend(obj.values().map(|c| (c, depth + 1))),
            _ => {}
        }
    }
    false
}

/// A path cursor that stays a borrow into `input`/`lets`/`this` until a method
/// (or owned index) forces ownership — so `input.a.b.c` never clones the whole
/// input, only the final value once at the end.
enum Cursor<'a> {
    Borrowed(&'a Value),
    Owned(Value),
}

impl Cursor<'_> {
    fn as_ref(&self) -> &Value {
        match self {
            Cursor::Borrowed(v) => v,
            Cursor::Owned(v) => v,
        }
    }

    fn into_value(self) -> Value {
        match self {
            Cursor::Borrowed(v) => v.clone(),
            Cursor::Owned(v) => v,
        }
    }
}

fn array_index(idx: &Value, len: usize, span: Span) -> Result<usize, RenderError> {
    match idx {
        Value::Int(i) => {
            if *i < 0 || *i as usize >= len {
                Err(RenderError::IndexOutOfBounds {
                    index: *i,
                    length: len,
                    span,
                })
            } else {
                Ok(*i as usize)
            }
        }
        other => Err(RenderError::TypeMismatch {
            expected: "integer index",
            got: other.kind().to_string(),
            span,
        }),
    }
}

pub(super) fn evaluate_path(
    root: &SmolStr,
    root_span: Span,
    segments: &[PathSegment],
    input: &Value,
    lets: &Scopes,
    this: &Scope,
) -> Result<Value, RenderError> {
    let (current, start_idx): (Cursor, usize) = match root.as_str() {
        "input" => (Cursor::Borrowed(input), 0),
        "this" => match segments.first() {
            Some(PathSegment::Field { name, span, .. }) => match this.get(name) {
                Some(v) => (Cursor::Borrowed(v), 1),
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
            Some(v) => (Cursor::Borrowed(v), 0),
            None => {
                return Err(RenderError::TypeMismatch {
                    expected: "known identifier",
                    got: root.to_string(),
                    span: root_span,
                });
            }
        },
    };

    walk_segments(
        current,
        root_span,
        &segments[start_idx..],
        input,
        lets,
        this,
    )
}

/// Walk trailing `.field` / `?.field` / `.method()` / `[idx]` segments against a
/// value reached through *any* expression (a literal, a call result, a paren'd
/// expression), not just an identifier root.
pub(super) fn evaluate_access(
    base: Value,
    base_span: Span,
    segments: &[PathSegment],
    input: &Value,
    lets: &Scopes,
    this: &Scope,
) -> Result<Value, RenderError> {
    walk_segments(Cursor::Owned(base), base_span, segments, input, lets, this)
}

fn walk_segments(
    mut current: Cursor,
    root_span: Span,
    segments: &[PathSegment],
    input: &Value,
    lets: &Scopes,
    this: &Scope,
) -> Result<Value, RenderError> {
    for segment in segments {
        match segment {
            PathSegment::Field {
                name,
                span,
                optional,
            } => {
                if *optional && matches!(current.as_ref(), Value::Null) {
                    return Ok(Value::Null);
                }
                current = match current {
                    Cursor::Borrowed(b) => match b {
                        Value::Obj(obj) => match obj.get(name) {
                            Some(v) => Cursor::Borrowed(v),
                            None if *optional => return Ok(Value::Null),
                            None => return Err(missing_field(name, *span)),
                        },
                        other => return Err(not_object(other, *span)),
                    },
                    Cursor::Owned(mut owned) => match &mut owned {
                        Value::Obj(obj) => match obj.swap_remove(name) {
                            Some(v) => Cursor::Owned(v),
                            None if *optional => return Ok(Value::Null),
                            None => return Err(missing_field(name, *span)),
                        },
                        other => return Err(not_object(other, *span)),
                    },
                };
            }
            PathSegment::Method {
                name, span, args, ..
            } => {
                let result = call_method(current.as_ref(), name, args, *span, input, lets, this)?;
                current = Cursor::Owned(result);
            }
            PathSegment::Index { expr, span } => {
                let idx = evaluate_expr(expr, input, lets, this)?;
                current = match current {
                    Cursor::Borrowed(b) => match b {
                        Value::Arr(a) => {
                            let u = array_index(&idx, a.len(), *span)?;
                            Cursor::Borrowed(&a[u])
                        }
                        Value::Obj(o) => match &idx {
                            Value::Str(k) => match o.get(k) {
                                Some(v) => Cursor::Borrowed(v),
                                None => return Err(missing_field(k, *span)),
                            },
                            other => return Err(not_string_key(other, *span)),
                        },
                        other => return Err(not_indexable(other, *span)),
                    },
                    Cursor::Owned(mut owned) => match &mut owned {
                        Value::Arr(a) => {
                            let u = array_index(&idx, a.len(), *span)?;
                            Cursor::Owned(a.swap_remove(u))
                        }
                        Value::Obj(o) => match &idx {
                            Value::Str(k) => match o.swap_remove(k.as_str()) {
                                Some(v) => Cursor::Owned(v),
                                None => return Err(missing_field(k, *span)),
                            },
                            other => return Err(not_string_key(other, *span)),
                        },
                        other => return Err(not_indexable(other, *span)),
                    },
                };
            }
        }
    }

    // Bound depth before this value is cloned/compared/deserialized — deep nesting
    // errors here instead of overflowing the stack downstream.
    if value_too_deep(current.as_ref()) {
        return Err(RenderError::ValueTooDeep {
            limit: MAX_VALUE_DEPTH,
            span: root_span,
        });
    }

    Ok(current.into_value())
}

fn not_object(v: &Value, span: Span) -> RenderError {
    RenderError::TypeMismatch {
        expected: "object",
        got: v.kind().to_string(),
        span,
    }
}

fn not_indexable(v: &Value, span: Span) -> RenderError {
    RenderError::NotIndexable {
        got: v.kind().to_string(),
        span,
    }
}

fn not_string_key(v: &Value, span: Span) -> RenderError {
    RenderError::TypeMismatch {
        expected: "string key for object index",
        got: v.kind().to_string(),
        span,
    }
}

fn missing_field(name: &SmolStr, span: Span) -> RenderError {
    RenderError::MissingPath {
        path: name.to_string(),
        key: Some(name.to_string()),
        span,
    }
}
