use crate::error::{RenderError, Span};
use crate::parse::{Expr, ExprKind, OutKind, OutNode, PathSegment};
use crate::value::Value;
use indexmap::IndexMap;

pub fn evaluate(node: &OutNode, input: &Value) -> Result<Value, RenderError> {
    match &node.kind {
        OutKind::Literal(v) => Ok(v.clone()),
        OutKind::Hole(expr) => evaluate_expr(expr, input),
        OutKind::Object(fields) => {
            let mut obj = IndexMap::with_capacity(fields.len());
            for (key, value_node) in fields {
                obj.insert(key.clone(), evaluate(value_node, input)?);
            }
            Ok(Value::Obj(obj))
        }
        OutKind::Array(items) => {
            let mut arr = Vec::with_capacity(items.len());
            for item in items {
                arr.push(evaluate(item, input)?);
            }
            Ok(Value::Arr(arr))
        }
    }
}

fn evaluate_expr(expr: &Expr, input: &Value) -> Result<Value, RenderError> {
    match &expr.kind {
        ExprKind::Literal(v) => Ok(v.clone()),
        ExprKind::Path { segments } => evaluate_path(segments, input),
    }
}

fn evaluate_path(segments: &[PathSegment], input: &Value) -> Result<Value, RenderError> {
    let root_name = segments
        .first()
        .map(|s| s.name.as_str())
        .unwrap_or("<empty>");
    let root_span = segments
        .first()
        .map(|s| s.span)
        .unwrap_or(Span::new(0, 0));
    if root_name != "input" {
        return Err(RenderError::TypeMismatch {
            expected: "path rooted at 'input'",
            got: root_name.to_string(),
            span: root_span,
        });
    }

    let mut current = input;
    let mut walked: Vec<&str> = vec!["input"];
    for segment in &segments[1..] {
        walked.push(segment.name.as_str());
        match current {
            Value::Obj(obj) => match obj.get(&segment.name) {
                Some(value) => current = value,
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
