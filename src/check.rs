use crate::common::error::{arguments, arity_message, CompileError, Span};
use crate::common::limits::MAX_HINTS;
use crate::eval::builtins::{builtin_names, is_builtin_function};
use crate::syntax::ast::{
    Expr, ExprKind, Lit, LitKey, Module, ObjField, OutKind, OutNode, PathSegment,
};
use smol_str::SmolStr;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

pub(crate) const RESERVED: &[&str] = &[
    "input", "this", "let", "in", "when", "else", "true", "false", "null",
];

pub(crate) fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

pub(crate) fn name_problem(name: &str, function: bool) -> Option<String> {
    if is_reserved(name) {
        Some(format!("`{name}` is a reserved name"))
    } else if function && is_builtin_function(name) {
        Some(format!("`{name}` is a built-in function"))
    } else {
        None
    }
}

struct Scope<'a> {
    base: &'a HashSet<SmolStr>,
    innermost: Option<Binding<'a>>,
}

struct Binding<'a> {
    name: &'a SmolStr,
    outer: &'a Scope<'a>,
}

impl<'a> Scope<'a> {
    fn new(base: &'a HashSet<SmolStr>) -> Self {
        Scope {
            base,
            innermost: None,
        }
    }

    fn bind<'b>(&'b self, name: &'b SmolStr) -> Scope<'b> {
        Scope {
            base: self.base,
            innermost: Some(Binding { name, outer: self }),
        }
    }

    fn contains(&self, name: &str) -> bool {
        let mut scope = self;
        while let Some(binding) = &scope.innermost {
            if binding.name == name {
                return true;
            }
            scope = binding.outer;
        }
        scope.base.contains(name)
    }

    fn names(&self) -> Vec<&str> {
        let mut names = Vec::new();
        let mut scope = self;
        while let Some(binding) = &scope.innermost {
            names.push(binding.name.as_str());
            scope = binding.outer;
        }
        names.extend(scope.base.iter().map(SmolStr::as_str));
        names
    }
}

fn bind_all<'s>(scope: &'s Scope<'s>, names: &[&'s SmolStr], f: &mut dyn FnMut(&Scope)) {
    match names.split_first() {
        None => f(scope),
        Some((first, rest)) => {
            let inner = scope.bind(first);
            bind_all(&inner, rest, f);
        }
    }
}

struct Checker<'a> {
    /// Keys `this.` may name; `None` outside an object output.
    output_keys: Option<&'a HashSet<SmolStr>>,
    functions: Option<&'a HashMap<SmolStr, usize>>,
    /// Each hint scans every name in scope, so only the first few unknown
    /// names get one.
    hints_left: Cell<usize>,
    errors: Vec<CompileError>,
}

impl<'a> Checker<'a> {
    fn new(
        output_keys: Option<&'a HashSet<SmolStr>>,
        functions: Option<&'a HashMap<SmolStr, usize>>,
    ) -> Self {
        Checker {
            output_keys,
            functions,
            hints_left: Cell::new(MAX_HINTS),
            errors: Vec::new(),
        }
    }

    fn error(&mut self, message: String, span: Span) {
        self.errors.push(CompileError::Syntax { message, span });
    }

    /// ` — did you mean …?` for `target`, or empty.
    fn hint<'c>(&self, target: &str, candidates: impl FnOnce() -> Vec<&'c str>) -> String {
        if self.hints_left.get() == 0 {
            return String::new();
        }
        self.hints_left.set(self.hints_left.get() - 1);
        match closest(target, candidates()) {
            Some(name) => format!(" — did you mean `{name}`?"),
            None => String::new(),
        }
    }

    fn check_output(&mut self, node: &OutNode, scope: &Scope) {
        match &node.kind {
            OutKind::Literal(_) => {}
            OutKind::Hole(expr) => self.check_expr(expr, scope),
            OutKind::Object(fields) => {
                let mut seen: HashSet<&SmolStr> = HashSet::new();
                for field in fields {
                    if !seen.insert(&field.key) {
                        self.error(
                            format!("duplicate output key `{}`", field.key),
                            field.value.span,
                        );
                    }
                    self.check_output(&field.value, scope);
                }
            }
            OutKind::Array(items) => {
                for item in items {
                    self.check_output(item, scope);
                }
            }
            OutKind::Interp(_) => node.for_each_expr(&mut |expr| self.check_expr(expr, scope)),
        }
    }

    fn check_expr(&mut self, expr: &Expr, scope: &Scope) {
        match &expr.kind {
            ExprKind::Path {
                root,
                root_span,
                segments,
            } => {
                self.check_path_root(root, *root_span, segments, scope);
                self.check_segments(segments, scope);
            }
            ExprKind::Lambda { params, body } => {
                let mut usable: Vec<&SmolStr> = Vec::new();
                for param in params {
                    match name_problem(&param.name, false) {
                        Some(problem) => self.error(problem, param.span),
                        None => usable.push(&param.name),
                    }
                }
                bind_all(scope, &usable, &mut |inner| self.check_expr(body, inner));
            }
            ExprKind::Let {
                name,
                name_span,
                value,
                body,
            } => {
                if let Some(problem) = name_problem(name, false) {
                    self.error(problem, *name_span);
                }
                self.check_expr(value, scope);
                self.check_expr(body, &scope.bind(name));
            }
            ExprKind::ObjectLit(entries) => {
                let mut seen: HashSet<&SmolStr> = HashSet::new();
                for entry in entries {
                    if let LitKey::Static(key) = &entry.key {
                        if !seen.insert(key) {
                            self.error(
                                format!("duplicate key `{key}` in object literal"),
                                entry.value.span,
                            );
                        }
                    }
                }
                expr.for_each_child(&mut |child| self.check_expr(child, scope));
            }
            ExprKind::FuncCall {
                name,
                name_span,
                args,
            } => {
                self.check_call(name, *name_span, args.len());
                expr.for_each_child(&mut |child| self.check_expr(child, scope));
            }
            ExprKind::Access { base, segments } => {
                if let ExprKind::Literal(literal) = &base.kind {
                    self.check_access_on_literal(literal, segments);
                }
                expr.for_each_child(&mut |child| self.check_expr(child, scope));
            }
            ExprKind::Literal(_)
            | ExprKind::Binary { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Ternary { .. }
            | ExprKind::When { .. }
            | ExprKind::ArrayLit(_) => {
                expr.for_each_child(&mut |child| self.check_expr(child, scope))
            }
        }
    }

    fn check_path_root(
        &mut self,
        root: &SmolStr,
        span: Span,
        segments: &[PathSegment],
        scope: &Scope,
    ) {
        match root.as_str() {
            "input" => {}
            "this" => match self.output_keys {
                None => self.error("`this` is only valid inside an object output".into(), span),
                Some(keys) => match segments.first() {
                    Some(PathSegment::Field {
                        name,
                        span: field_span,
                        ..
                    }) => {
                        if !keys.contains(name) {
                            let hint =
                                self.hint(name, || keys.iter().map(SmolStr::as_str).collect());
                            self.error(
                                format!("unknown output key `this.{name}`{hint}"),
                                *field_span,
                            );
                        }
                    }
                    _ => self.error("`this` must be followed by `.<field>`".into(), span),
                },
            },
            _ => {
                if !scope.contains(root) {
                    let hint = self.hint(root, || {
                        let mut names = vec!["input", "this"];
                        names.extend(scope.names());
                        names
                    });
                    self.error(format!("unknown identifier `{root}`{hint}"), span);
                }
            }
        }
    }

    fn check_segments(&mut self, segments: &[PathSegment], scope: &Scope) {
        for segment in segments {
            match segment {
                PathSegment::Field { .. } => {}
                PathSegment::Method { args, .. } => {
                    for arg in args {
                        self.check_expr(arg, scope);
                    }
                }
                PathSegment::Index { expr, .. } => self.check_expr(expr, scope),
            }
        }
    }

    /// Built-in arity is checked when the call runs, not here: a wrong call
    /// in a branch never taken has always compiled.
    fn check_call(&mut self, name: &SmolStr, span: Span, given: usize) {
        if let Some(&arity) = self.functions.and_then(|functions| functions.get(name)) {
            if given != arity {
                self.error(arity_message(name, &arguments(arity), given), span);
            }
            return;
        }
        if is_builtin_function(name) {
            return;
        }
        let hint = self.hint(name, || {
            let mut names: Vec<&str> = builtin_names().collect();
            if let Some(functions) = self.functions {
                names.extend(functions.keys().map(SmolStr::as_str));
            }
            names
        });
        self.error(format!("unknown function `{name}`{hint}"), span);
    }

    fn check_access_on_literal(&mut self, literal: &Lit, segments: &[PathSegment]) {
        let is_string = matches!(literal, Lit::Str(_));
        for segment in segments {
            let unsupported = match segment {
                PathSegment::Field { span, .. } => Some(("field access", *span)),
                PathSegment::Index { span, .. } => Some(("indexing", *span)),
                PathSegment::Method { span, .. } if !is_string => Some(("methods", *span)),
                PathSegment::Method { .. } => None,
            };
            if let Some((what, span)) = unsupported {
                self.error(
                    format!("a {} literal does not support {what}", lit_kind(literal)),
                    span,
                );
            }
        }
    }

    fn finish(
        mut self,
        order: Result<Vec<usize>, CompileError>,
    ) -> Result<Vec<usize>, Vec<CompileError>> {
        let order = order.unwrap_or_else(|cycle| {
            self.errors.push(cycle);
            Vec::new()
        });
        if self.errors.is_empty() {
            Ok(order)
        } else {
            Err(self.errors)
        }
    }
}

/// Returns the field order that lets every `this.x` see `x` first.
pub(crate) fn check_module(module: &Module) -> Result<Vec<usize>, Vec<CompileError>> {
    let output_keys = keys_of(&module.output);
    let mut checker = Checker::new(None, None);
    let mut known: HashSet<SmolStr> = HashSet::new();
    for binding in &module.lets {
        if let Some(problem) = name_problem(&binding.name, false) {
            checker.error(problem, binding.span);
        } else if known.contains(&binding.name) {
            checker.error(
                format!("variable `{}` is already defined", binding.name),
                binding.span,
            );
        }
        checker.check_expr(&binding.expr, &Scope::new(&known));
        if !is_reserved(&binding.name) {
            known.insert(binding.name.clone());
        }
    }
    checker.output_keys = output_keys.as_ref();
    checker.check_output(&module.output, &Scope::new(&known));
    checker.finish(output_order(&module.output))
}

pub(crate) fn check_unit_expr(
    expr: &Expr,
    names: &HashSet<SmolStr>,
    functions: &HashMap<SmolStr, usize>,
) -> Result<(), Vec<CompileError>> {
    let mut checker = Checker::new(None, Some(functions));
    checker.check_expr(expr, &Scope::new(names));
    checker.finish(Ok(Vec::new())).map(|_| ())
}

pub(crate) fn check_unit_output(
    node: &OutNode,
    names: &HashSet<SmolStr>,
    functions: &HashMap<SmolStr, usize>,
) -> Result<Vec<usize>, Vec<CompileError>> {
    let output_keys = keys_of(node);
    let mut checker = Checker::new(output_keys.as_ref(), Some(functions));
    checker.check_output(node, &Scope::new(names));
    checker.finish(output_order(node))
}

fn keys_of(node: &OutNode) -> Option<HashSet<SmolStr>> {
    match &node.kind {
        OutKind::Object(fields) => Some(fields.iter().map(|field| field.key.clone()).collect()),
        _ => None,
    }
}

fn output_order(node: &OutNode) -> Result<Vec<usize>, CompileError> {
    match &node.kind {
        OutKind::Object(fields) => sort_by_this_refs(fields),
        _ => Ok(Vec::new()),
    }
}

fn sort_by_this_refs(fields: &[ObjField]) -> Result<Vec<usize>, CompileError> {
    let position_by_key: HashMap<&str, usize> = fields
        .iter()
        .enumerate()
        .map(|(position, field)| (field.key.as_str(), position))
        .collect();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); fields.len()];
    let mut waiting_on: Vec<usize> = vec![0; fields.len()];
    for (position, field) in fields.iter().enumerate() {
        let mut refs: HashSet<SmolStr> = HashSet::new();
        field
            .value
            .for_each_expr(&mut |expr| this_refs(expr, &mut refs));
        for key in &refs {
            if let Some(&read) = position_by_key.get(key.as_str()) {
                dependents[read].push(position);
                waiting_on[position] += 1;
            }
        }
    }
    let mut ready: Vec<usize> = (0..fields.len())
        .filter(|&position| waiting_on[position] == 0)
        .collect();
    let mut order = Vec::with_capacity(fields.len());
    while let Some(position) = ready.pop() {
        order.push(position);
        for &dependent in &dependents[position] {
            waiting_on[dependent] -= 1;
            if waiting_on[dependent] == 0 {
                ready.push(dependent);
            }
        }
    }
    if order.len() == fields.len() {
        return Ok(order);
    }
    let stuck: Vec<usize> = (0..fields.len())
        .filter(|&position| waiting_on[position] > 0)
        .collect();
    let names: Vec<&str> = stuck
        .iter()
        .map(|&position| fields[position].key.as_str())
        .collect();
    Err(CompileError::Syntax {
        message: format!("cycle in `this` references involving: {}", names.join(", ")),
        span: fields[stuck[0]].value.span,
    })
}

fn this_refs(expr: &Expr, refs: &mut HashSet<SmolStr>) {
    if let ExprKind::Path { root, segments, .. } = &expr.kind {
        if root == "this" {
            if let Some(PathSegment::Field { name, .. }) = segments.first() {
                refs.insert(name.clone());
            }
        }
    }
    expr.for_each_child(&mut |child| this_refs(child, refs));
}

fn lit_kind(literal: &Lit) -> &'static str {
    match literal {
        Lit::Null => "null",
        Lit::Bool(_) => "bool",
        Lit::Int(_) => "int",
        Lit::Decimal(_) => "decimal",
        Lit::Str(_) => "string",
    }
}

/// Closest candidate within two edits, not a rewrite of a short name. Ties go
/// to the first in sort order, so hints are deterministic.
fn closest<'a>(target: &str, candidates: Vec<&'a str>) -> Option<&'a str> {
    let target_len = target.chars().count();
    let mut best: Option<(&str, usize)> = None;
    for candidate in candidates {
        // a length gap alone costs that many edits
        if candidate.chars().count().abs_diff(target_len) > 2 {
            continue;
        }
        let Some(distance) = edit_distance_within(target, candidate, 2) else {
            continue;
        };
        let better = match best {
            None => true,
            Some((best_name, best_distance)) => {
                distance < best_distance || (distance == best_distance && candidate < best_name)
            }
        };
        if better {
            best = Some((candidate, distance));
        }
    }
    best.filter(|&(_, distance)| distance > 0 && distance < target_len)
        .map(|(candidate, _)| candidate)
}

fn edit_distance_within(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, char_a) in a.iter().enumerate() {
        current[0] = i + 1;
        let mut row_min = current[0];
        for (j, char_b) in b.iter().enumerate() {
            let cost = usize::from(char_a != char_b);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
            row_min = row_min.min(current[j + 1]);
        }
        // every later row is at least this row's minimum
        if row_min > limit {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    Some(previous[b.len()]).filter(|&distance| distance <= limit)
}
