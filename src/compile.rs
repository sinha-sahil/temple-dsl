use crate::error::{CompileError, RenderError, Span};
use crate::eval::{self, Scope};
use crate::parse::{self, Expr, ExprKind, LetBinding, Module, OutKind, OutNode};
use crate::value::Value;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};

const RESERVED: &[&str] = &[
    "input", "this", "let", "when", "else", "true", "false", "null",
];

#[derive(Debug, Clone)]
pub struct Template {
    module: Module,
    output_order: Vec<usize>,
}

impl Template {
    pub fn compile(src: &str) -> Result<Self, Vec<CompileError>> {
        let module = parse::parse(src)?;
        let output_order = resolve(&module)?;
        Ok(Template {
            module,
            output_order,
        })
    }

    pub fn render<T>(&self, input: impl Into<Value>) -> Result<T, RenderError>
    where
        T: DeserializeOwned,
    {
        let input = input.into();
        let empty_this: Scope = HashMap::new();

        let mut lets: Scope = HashMap::with_capacity(self.module.lets.len());
        for binding in &self.module.lets {
            let v = eval::evaluate_expr(&binding.expr, &input, &lets, &empty_this)?;
            lets.insert(binding.name.clone(), v);
        }

        let output_value = match &self.module.output.kind {
            OutKind::Object(fields) => {
                let mut this: Scope = HashMap::with_capacity(fields.len());
                for &idx in &self.output_order {
                    let (key, node) = &fields[idx];
                    let v = eval::evaluate(node, &input, &lets, &this)?;
                    this.insert(key.clone(), v);
                }
                let mut result = IndexMap::with_capacity(fields.len());
                for (key, _) in fields {
                    if let Some(v) = this.remove(key) {
                        result.insert(key.clone(), v);
                    }
                }
                Value::Obj(result)
            }
            _ => eval::evaluate(&self.module.output, &input, &lets, &empty_this)?,
        };

        T::deserialize(&output_value).map_err(|e| RenderError::Deserialize(e.to_string()))
    }
}

fn resolve(module: &Module) -> Result<Vec<usize>, Vec<CompileError>> {
    let mut errors: Vec<CompileError> = Vec::new();

    let mut known_lets: HashSet<SmolStr> = HashSet::new();
    for binding in &module.lets {
        validate_let_name(binding, &known_lets, &mut errors);
        validate_expr(&binding.expr, &known_lets, None, &mut errors);
        if !RESERVED.contains(&binding.name.as_str()) {
            known_lets.insert(binding.name.clone());
        }
    }

    let output_keys: Option<HashSet<SmolStr>> = match &module.output.kind {
        OutKind::Object(fields) => Some(fields.iter().map(|(k, _)| k.clone()).collect()),
        _ => None,
    };
    validate_out_node(
        &module.output,
        &known_lets,
        output_keys.as_ref(),
        &mut errors,
    );

    let order = match &module.output.kind {
        OutKind::Object(fields) => match topo_sort_fields(fields) {
            Ok(o) => o,
            Err(e) => {
                errors.push(e);
                Vec::new()
            }
        },
        _ => Vec::new(),
    };

    if errors.is_empty() {
        Ok(order)
    } else {
        Err(errors)
    }
}

fn validate_let_name(
    binding: &LetBinding,
    known_lets: &HashSet<SmolStr>,
    errors: &mut Vec<CompileError>,
) {
    if RESERVED.contains(&binding.name.as_str()) {
        errors.push(CompileError::Syntax {
            message: format!("'{}' is a reserved name", binding.name),
            span: binding.span,
        });
    } else if known_lets.contains(&binding.name) {
        errors.push(CompileError::Syntax {
            message: format!("variable '{}' is already defined", binding.name),
            span: binding.span,
        });
    }
}

fn validate_expr(
    expr: &Expr,
    lets: &HashSet<SmolStr>,
    output_keys: Option<&HashSet<SmolStr>>,
    errors: &mut Vec<CompileError>,
) {
    match &expr.kind {
        ExprKind::Literal(_) => {}
        ExprKind::Path { segments } => {
            let Some(root) = segments.first() else {
                return;
            };
            let name = root.name.as_str();
            match name {
                "input" => {}
                "this" => match output_keys {
                    None => errors.push(CompileError::Syntax {
                        message: "'this' is only valid inside an object output".into(),
                        span: root.span,
                    }),
                    Some(keys) => match segments.get(1) {
                        None => errors.push(CompileError::Syntax {
                            message: "'this' must be followed by '.<field>'".into(),
                            span: root.span,
                        }),
                        Some(key_seg) => {
                            if !keys.contains(&key_seg.name) {
                                errors.push(CompileError::Syntax {
                                    message: format!("unknown output key 'this.{}'", key_seg.name),
                                    span: key_seg.span,
                                });
                            }
                        }
                    },
                },
                _ => {
                    if !lets.contains(&root.name) {
                        errors.push(CompileError::Syntax {
                            message: format!("unknown identifier '{name}'"),
                            span: root.span,
                        });
                    }
                }
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            validate_expr(lhs, lets, output_keys, errors);
            validate_expr(rhs, lets, output_keys, errors);
        }
        ExprKind::Unary { operand, .. } => {
            validate_expr(operand, lets, output_keys, errors);
        }
        ExprKind::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            validate_expr(cond, lets, output_keys, errors);
            validate_expr(then_branch, lets, output_keys, errors);
            validate_expr(else_branch, lets, output_keys, errors);
        }
        ExprKind::When { branches, fallback } => {
            for b in branches {
                validate_expr(&b.cond, lets, output_keys, errors);
                validate_expr(&b.result, lets, output_keys, errors);
            }
            if let Some(fb) = fallback {
                validate_expr(fb, lets, output_keys, errors);
            }
        }
    }
}

fn validate_out_node(
    node: &OutNode,
    lets: &HashSet<SmolStr>,
    output_keys: Option<&HashSet<SmolStr>>,
    errors: &mut Vec<CompileError>,
) {
    match &node.kind {
        OutKind::Literal(_) => {}
        OutKind::Hole(expr) => validate_expr(expr, lets, output_keys, errors),
        OutKind::Object(fields) => {
            for (_, child) in fields {
                validate_out_node(child, lets, output_keys, errors);
            }
        }
        OutKind::Array(items) => {
            for item in items {
                validate_out_node(item, lets, output_keys, errors);
            }
        }
    }
}

fn topo_sort_fields(fields: &[(SmolStr, OutNode)]) -> Result<Vec<usize>, CompileError> {
    let key_to_idx: HashMap<&str, usize> = fields
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (k.as_str(), i))
        .collect();

    let mut deps: Vec<HashSet<usize>> = vec![HashSet::new(); fields.len()];
    for (i, (_, node)) in fields.iter().enumerate() {
        let mut this_refs: HashSet<SmolStr> = HashSet::new();
        collect_this_refs_in_out(node, &mut this_refs);
        for r in &this_refs {
            if let Some(&j) = key_to_idx.get(r.as_str()) {
                deps[i].insert(j);
            }
        }
    }

    let mut in_degree: Vec<usize> = deps.iter().map(std::collections::HashSet::len).collect();
    let mut order: Vec<usize> = Vec::with_capacity(fields.len());
    let mut queue: Vec<usize> = (0..fields.len()).filter(|&i| in_degree[i] == 0).collect();

    while let Some(i) = queue.pop() {
        order.push(i);
        for (j, d) in deps.iter().enumerate() {
            if d.contains(&i) {
                in_degree[j] -= 1;
                if in_degree[j] == 0 {
                    queue.push(j);
                }
            }
        }
    }

    if order.len() != fields.len() {
        let stuck: Vec<&str> = (0..fields.len())
            .filter(|&i| !order.contains(&i))
            .map(|i| fields[i].0.as_str())
            .collect();
        let span = stuck
            .first()
            .and_then(|name| key_to_idx.get(name))
            .map_or(Span::new(0, 0), |&i| fields[i].1.span);
        return Err(CompileError::Syntax {
            message: format!("cycle in `this` references involving: {}", stuck.join(", ")),
            span,
        });
    }

    Ok(order)
}

fn collect_this_refs_in_out(node: &OutNode, refs: &mut HashSet<SmolStr>) {
    match &node.kind {
        OutKind::Literal(_) => {}
        OutKind::Hole(expr) => collect_this_refs_in_expr(expr, refs),
        OutKind::Object(fields) => {
            for (_, c) in fields {
                collect_this_refs_in_out(c, refs);
            }
        }
        OutKind::Array(items) => {
            for item in items {
                collect_this_refs_in_out(item, refs);
            }
        }
    }
}

fn collect_this_refs_in_expr(expr: &Expr, refs: &mut HashSet<SmolStr>) {
    match &expr.kind {
        ExprKind::Literal(_) => {}
        ExprKind::Path { segments } => {
            if segments.first().map(|s| s.name.as_str()) == Some("this") {
                if let Some(key_seg) = segments.get(1) {
                    refs.insert(key_seg.name.clone());
                }
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            collect_this_refs_in_expr(lhs, refs);
            collect_this_refs_in_expr(rhs, refs);
        }
        ExprKind::Unary { operand, .. } => collect_this_refs_in_expr(operand, refs),
        ExprKind::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_this_refs_in_expr(cond, refs);
            collect_this_refs_in_expr(then_branch, refs);
            collect_this_refs_in_expr(else_branch, refs);
        }
        ExprKind::When { branches, fallback } => {
            for b in branches {
                collect_this_refs_in_expr(&b.cond, refs);
                collect_this_refs_in_expr(&b.result, refs);
            }
            if let Some(fb) = fallback {
                collect_this_refs_in_expr(fb, refs);
            }
        }
    }
}
