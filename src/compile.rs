use crate::error::{CompileError, LoadError, RenderError, Span};
use crate::eval::{self, Scope, Scopes};
use crate::parse::{
    self, Expr, ExprKind, InterpPart, LetBinding, Lit, LitKey, Module, ObjField, OutKind, OutNode,
    PathSegment,
};
use crate::value::Value;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};

const RESERVED: &[&str] = &[
    "input", "this", "let", "in", "when", "else", "true", "false", "null",
];

/// Source-size cap, rejected at compile so a pathological template never reaches render.
const MAX_SOURCE_BYTES: usize = 1 << 20; // 1 MiB

// Blob layout = signature ("TMPL") + LE version u32 + CBOR module. CBOR (not
// bincode) because rust_decimal decodes via deserialize_any, which
// non-self-describing formats reject.
const BLOB_SIGNATURE: [u8; 4] = *b"TMPL";
const BLOB_VERSION: u32 = 1;
// The CBOR AST expands ~10x over source, so the load cap must clear that worst
// case — otherwise a compilable template could produce a blob `from_bytes` rejects.
const MAX_BLOB_BYTES: usize = MAX_SOURCE_BYTES * 16;

#[derive(Debug, Clone)]
pub struct Template {
    module: Module,
    output_order: Vec<usize>,
}

impl Template {
    pub fn compile(src: &str) -> Result<Self, Vec<CompileError>> {
        if src.len() > MAX_SOURCE_BYTES {
            return Err(vec![CompileError::TooLarge {
                bytes: src.len(),
                limit: MAX_SOURCE_BYTES,
            }]);
        }
        let module = parse::parse(src)?;
        let output_order = resolve(&module)?;
        Ok(Template {
            module,
            output_order,
        })
    }

    /// Author-time check: run the full compile pipeline (parse, validate, DAG,
    /// caps) and discard the result, reporting any problems. Cheap to call from
    /// a save handler or editor integration.
    pub fn validate(src: &str) -> Result<(), Vec<CompileError>> {
        Self::compile(src).map(|_| ())
    }

    /// Re-emit `src` in Temple's canonical layout. Requires only that the source
    /// parses (not that it resolves), so an editor can format-on-type before the
    /// names are wired up. Idempotent: formatting the output yields the same string.
    pub fn format(src: &str) -> Result<String, Vec<CompileError>> {
        // Same size cap as `compile` — format is an author-time entry point too,
        // and the cap is the guard that bounds parse cost.
        if src.len() > MAX_SOURCE_BYTES {
            return Err(vec![CompileError::TooLarge {
                bytes: src.len(),
                limit: MAX_SOURCE_BYTES,
            }]);
        }
        let module = parse::parse(src)?;
        Ok(crate::format::format_module(&module))
    }

    /// Serialize the compiled template to a versioned blob for storage. The
    /// render path reloads it with [`from_bytes`](Self::from_bytes) — no reparse.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&BLOB_SIGNATURE);
        out.extend_from_slice(&BLOB_VERSION.to_le_bytes());
        ciborium::ser::into_writer(&self.module, &mut out)
            .expect("encoding a compiled module to CBOR cannot fail");
        out
    }

    /// Load a template from a [`to_bytes`](Self::to_bytes) blob — a deserialize,
    /// never a parse. Re-validates so a corrupt blob errors instead of risking
    /// a render-time panic.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LoadError> {
        if bytes.len() > MAX_BLOB_BYTES {
            return Err(LoadError::Corrupt("blob exceeds maximum size".into()));
        }
        let header = bytes
            .get(..8)
            .ok_or_else(|| LoadError::Corrupt("blob too short".into()))?;
        if header[..4] != BLOB_SIGNATURE {
            return Err(LoadError::Corrupt("not a Temple blob".into()));
        }
        let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        if version != BLOB_VERSION {
            return Err(LoadError::IncompatibleVersion {
                found: version,
                expected: BLOB_VERSION,
            });
        }
        let module: Module = ciborium::de::from_reader(&bytes[8..])
            .map_err(|e| LoadError::Corrupt(format!("malformed blob: {e}")))?;
        let output_order = resolve(&module).map_err(|errs| {
            LoadError::Corrupt(format!("blob failed validation ({} error(s))", errs.len()))
        })?;
        Ok(Template {
            module,
            output_order,
        })
    }

    /// Render into one of the caller's own types `T`. To get the dynamic
    /// [`Value`] back instead, use [`render_value`](Self::render_value) —
    /// `render::<Value>` does not compile, by design (`Value` is not a
    /// deserialize target).
    pub fn render<T>(&self, input: impl Into<Value>) -> Result<T, RenderError>
    where
        T: DeserializeOwned,
    {
        let output_value = self.eval_output(&input.into())?;
        T::deserialize(&output_value).map_err(|e| RenderError::Deserialize(e.to_string()))
    }

    /// Render to the dynamic [`Value`] directly, skipping the serde round-trip.
    pub fn render_value(&self, input: impl Into<Value>) -> Result<Value, RenderError> {
        self.eval_output(&input.into())
    }

    fn eval_output(&self, input: &Value) -> Result<Value, RenderError> {
        let empty_this: Scope = HashMap::new();

        let mut lets: Scope = HashMap::with_capacity(self.module.lets.len());
        for binding in &self.module.lets {
            let v = eval::evaluate_expr(&binding.expr, input, &Scopes::base(&lets), &empty_this)?;
            lets.insert(binding.name.clone(), v);
        }
        let scopes = Scopes::base(&lets);

        match &self.module.output.kind {
            OutKind::Object(fields) => {
                let mut this: Scope = HashMap::with_capacity(fields.len());
                for &idx in &self.output_order {
                    let field = &fields[idx];
                    let v = eval::evaluate(&field.value, input, &scopes, &this)?;
                    this.insert(field.key.clone(), v);
                }
                let mut result = IndexMap::with_capacity(fields.len());
                for field in fields {
                    if let Some(v) = this.remove(&field.key) {
                        // `"key"?:` drops the key when its value is null.
                        if field.optional && matches!(v, Value::Null) {
                            continue;
                        }
                        result.insert(field.key.clone(), v);
                    }
                }
                Ok(Value::Obj(result))
            }
            _ => eval::evaluate(&self.module.output, input, &scopes, &empty_this),
        }
    }
}

fn resolve(module: &Module) -> Result<Vec<usize>, Vec<CompileError>> {
    let mut errors: Vec<CompileError> = Vec::new();
    let empty_params: HashSet<SmolStr> = HashSet::new();

    let mut known_lets: HashSet<SmolStr> = HashSet::new();
    for binding in &module.lets {
        validate_let_name(binding, &known_lets, &mut errors);
        validate_expr(&binding.expr, &known_lets, None, &empty_params, &mut errors);
        if !RESERVED.contains(&binding.name.as_str()) {
            known_lets.insert(binding.name.clone());
        }
    }

    let output_keys: Option<HashSet<SmolStr>> = match &module.output.kind {
        OutKind::Object(fields) => Some(fields.iter().map(|f| f.key.clone()).collect()),
        _ => None,
    };
    validate_out_node(
        &module.output,
        &known_lets,
        output_keys.as_ref(),
        &empty_params,
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
    lambda_params: &HashSet<SmolStr>,
    errors: &mut Vec<CompileError>,
) {
    match &expr.kind {
        ExprKind::Literal(_) => {}
        ExprKind::Path {
            root,
            root_span,
            segments,
        } => {
            match root.as_str() {
                "input" => {}
                "this" => match output_keys {
                    None => errors.push(CompileError::Syntax {
                        message: "'this' is only valid inside an object output".into(),
                        span: *root_span,
                    }),
                    Some(keys) => match segments.first() {
                        None => errors.push(CompileError::Syntax {
                            message: "'this' must be followed by '.<field>'".into(),
                            span: *root_span,
                        }),
                        Some(PathSegment::Field { name, span, .. }) => {
                            if !keys.contains(name) {
                                let hint = did_you_mean(name, keys.iter().map(SmolStr::as_str));
                                errors.push(CompileError::Syntax {
                                    message: format!("unknown output key 'this.{name}'{hint}"),
                                    span: *span,
                                });
                            }
                        }
                        Some(_) => errors.push(CompileError::Syntax {
                            message: "'this' must be followed by '.<field>'".into(),
                            span: *root_span,
                        }),
                    },
                },
                _ => {
                    if !lets.contains(root) && !lambda_params.contains(root) {
                        let mut cands: Vec<&str> = vec!["input", "this"];
                        cands.extend(lets.iter().map(SmolStr::as_str));
                        cands.extend(lambda_params.iter().map(SmolStr::as_str));
                        let hint = did_you_mean(root, cands);
                        errors.push(CompileError::Syntax {
                            message: format!("unknown identifier '{root}'{hint}"),
                            span: *root_span,
                        });
                    }
                }
            }
            for seg in segments {
                match seg {
                    PathSegment::Field { .. } => {}
                    PathSegment::Method { args, .. } => {
                        for a in args {
                            validate_expr(a, lets, output_keys, lambda_params, errors);
                        }
                    }
                    PathSegment::Index { expr, .. } => {
                        validate_expr(expr, lets, output_keys, lambda_params, errors);
                    }
                }
            }
        }
        ExprKind::Lambda { params, body } => {
            let mut inner = lambda_params.clone();
            for p in params {
                if RESERVED.contains(&p.name.as_str()) {
                    errors.push(CompileError::Syntax {
                        message: format!("'{}' is a reserved name", p.name),
                        span: p.span,
                    });
                } else {
                    inner.insert(p.name.clone());
                }
            }
            validate_expr(body, lets, output_keys, &inner, errors);
        }
        ExprKind::Let {
            name,
            name_span,
            value,
            body,
        } => {
            if RESERVED.contains(&name.as_str()) {
                errors.push(CompileError::Syntax {
                    message: format!("'{name}' is a reserved name"),
                    span: *name_span,
                });
            }
            validate_expr(value, lets, output_keys, lambda_params, errors);
            let mut inner = lets.clone();
            inner.insert(name.clone());
            validate_expr(body, &inner, output_keys, lambda_params, errors);
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            validate_expr(lhs, lets, output_keys, lambda_params, errors);
            validate_expr(rhs, lets, output_keys, lambda_params, errors);
        }
        ExprKind::Unary { operand, .. } => {
            validate_expr(operand, lets, output_keys, lambda_params, errors);
        }
        ExprKind::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            validate_expr(cond, lets, output_keys, lambda_params, errors);
            validate_expr(then_branch, lets, output_keys, lambda_params, errors);
            validate_expr(else_branch, lets, output_keys, lambda_params, errors);
        }
        ExprKind::When { branches, fallback } => {
            for b in branches {
                validate_expr(&b.cond, lets, output_keys, lambda_params, errors);
                validate_expr(&b.result, lets, output_keys, lambda_params, errors);
            }
            if let Some(fb) = fallback {
                validate_expr(fb, lets, output_keys, lambda_params, errors);
            }
        }
        ExprKind::ArrayLit(items) => {
            for item in items {
                validate_expr(item, lets, output_keys, lambda_params, errors);
            }
        }
        ExprKind::ObjectLit(entries) => {
            let mut seen: HashSet<&SmolStr> = HashSet::new();
            for entry in entries {
                match &entry.key {
                    LitKey::Static(k) => {
                        if !seen.insert(k) {
                            errors.push(CompileError::Syntax {
                                message: format!("duplicate key '{k}' in object literal"),
                                span: entry.value.span,
                            });
                        }
                    }
                    LitKey::Computed(ke) => {
                        validate_expr(ke, lets, output_keys, lambda_params, errors);
                    }
                }
                validate_expr(&entry.value, lets, output_keys, lambda_params, errors);
            }
        }
        ExprKind::FuncCall {
            name,
            name_span,
            args,
        } => {
            if !eval::is_known_function(name) {
                let hint = did_you_mean(name, eval::BUILTINS.iter().copied());
                errors.push(CompileError::Syntax {
                    message: format!("unknown function '{name}'{hint}"),
                    span: *name_span,
                });
            }
            for a in args {
                validate_expr(a, lets, output_keys, lambda_params, errors);
            }
        }
        ExprKind::Access { base, segments } => {
            // A literal base has a statically-known type, so a nonsensical
            // access is a compile error here, not a per-render failure.
            if let ExprKind::Literal(lit) = &base.kind {
                let is_str = matches!(lit, Lit::Str(_));
                for seg in segments {
                    let (bad, what) = match seg {
                        PathSegment::Field { span, .. } => (true, ("field access", span)),
                        PathSegment::Index { span, .. } => (true, ("indexing", span)),
                        PathSegment::Method { span, .. } if !is_str => (true, ("methods", span)),
                        _ => (false, ("", &base.span)),
                    };
                    if bad {
                        errors.push(CompileError::Syntax {
                            message: format!(
                                "a {} literal does not support {}",
                                lit_kind(lit),
                                what.0
                            ),
                            span: *what.1,
                        });
                    }
                }
            }
            validate_expr(base, lets, output_keys, lambda_params, errors);
            for seg in segments {
                match seg {
                    PathSegment::Field { .. } => {}
                    PathSegment::Method { args, .. } => {
                        for a in args {
                            validate_expr(a, lets, output_keys, lambda_params, errors);
                        }
                    }
                    PathSegment::Index { expr, .. } => {
                        validate_expr(expr, lets, output_keys, lambda_params, errors);
                    }
                }
            }
        }
    }
}

fn validate_out_node(
    node: &OutNode,
    lets: &HashSet<SmolStr>,
    output_keys: Option<&HashSet<SmolStr>>,
    lambda_params: &HashSet<SmolStr>,
    errors: &mut Vec<CompileError>,
) {
    match &node.kind {
        OutKind::Literal(_) => {}
        OutKind::Hole(expr) => validate_expr(expr, lets, output_keys, lambda_params, errors),
        OutKind::Object(fields) => {
            let mut seen: HashSet<&SmolStr> = HashSet::new();
            for field in fields {
                if !seen.insert(&field.key) {
                    errors.push(CompileError::Syntax {
                        message: format!("duplicate output key '{}'", field.key),
                        span: field.value.span,
                    });
                }
                validate_out_node(&field.value, lets, output_keys, lambda_params, errors);
            }
        }
        OutKind::Array(items) => {
            for item in items {
                validate_out_node(item, lets, output_keys, lambda_params, errors);
            }
        }
        OutKind::Interp(parts) => {
            for part in parts {
                if let InterpPart::Hole(expr) = part {
                    validate_expr(expr, lets, output_keys, lambda_params, errors);
                }
            }
        }
    }
}

fn topo_sort_fields(fields: &[ObjField]) -> Result<Vec<usize>, CompileError> {
    let key_to_idx: HashMap<&str, usize> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| (f.key.as_str(), i))
        .collect();

    let mut deps: Vec<HashSet<usize>> = vec![HashSet::new(); fields.len()];
    for (i, field) in fields.iter().enumerate() {
        let mut this_refs: HashSet<SmolStr> = HashSet::new();
        collect_this_refs_in_out(&field.value, &mut this_refs);
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
            .map(|i| fields[i].key.as_str())
            .collect();
        let span = stuck
            .first()
            .and_then(|name| key_to_idx.get(name))
            .map_or(Span::new(0, 0), |&i| fields[i].value.span);
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
            for field in fields {
                collect_this_refs_in_out(&field.value, refs);
            }
        }
        OutKind::Array(items) => {
            for item in items {
                collect_this_refs_in_out(item, refs);
            }
        }
        OutKind::Interp(parts) => {
            for part in parts {
                if let InterpPart::Hole(expr) = part {
                    collect_this_refs_in_expr(expr, refs);
                }
            }
        }
    }
}

fn collect_this_refs_in_expr(expr: &Expr, refs: &mut HashSet<SmolStr>) {
    match &expr.kind {
        ExprKind::Literal(_) => {}
        ExprKind::Path { root, segments, .. } => {
            if root.as_str() == "this" {
                if let Some(PathSegment::Field { name, .. }) = segments.first() {
                    refs.insert(name.clone());
                }
            }
            for seg in segments {
                match seg {
                    PathSegment::Field { .. } => {}
                    PathSegment::Method { args, .. } => {
                        for a in args {
                            collect_this_refs_in_expr(a, refs);
                        }
                    }
                    PathSegment::Index { expr, .. } => collect_this_refs_in_expr(expr, refs),
                }
            }
        }
        ExprKind::Lambda { body, .. } => collect_this_refs_in_expr(body, refs),
        ExprKind::Let { value, body, .. } => {
            collect_this_refs_in_expr(value, refs);
            collect_this_refs_in_expr(body, refs);
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
        ExprKind::ArrayLit(items) => {
            for item in items {
                collect_this_refs_in_expr(item, refs);
            }
        }
        ExprKind::ObjectLit(entries) => {
            for entry in entries {
                if let LitKey::Computed(ke) = &entry.key {
                    collect_this_refs_in_expr(ke, refs);
                }
                collect_this_refs_in_expr(&entry.value, refs);
            }
        }
        ExprKind::FuncCall { args, .. } => {
            for a in args {
                collect_this_refs_in_expr(a, refs);
            }
        }
        ExprKind::Access { base, segments } => {
            collect_this_refs_in_expr(base, refs);
            for seg in segments {
                match seg {
                    PathSegment::Field { .. } => {}
                    PathSegment::Method { args, .. } => {
                        for a in args {
                            collect_this_refs_in_expr(a, refs);
                        }
                    }
                    PathSegment::Index { expr, .. } => collect_this_refs_in_expr(expr, refs),
                }
            }
        }
    }
}

fn lit_kind(lit: &Lit) -> &'static str {
    match lit {
        Lit::Null => "null",
        Lit::Bool(_) => "bool",
        Lit::Int(_) => "int",
        Lit::Decimal(_) => "decimal",
        Lit::Str(_) => "string",
    }
}

/// A ` — did you mean \`x\`?` suffix when `target` is a plausible typo of one of
/// `candidates`, else empty. Used to enrich unknown-name compile errors.
fn did_you_mean<'a>(target: &str, candidates: impl IntoIterator<Item = &'a str>) -> String {
    match closest(target, candidates) {
        Some(s) => format!(" — did you mean `{s}`?"),
        None => String::new(),
    }
}

fn closest<'a>(target: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for c in candidates {
        let d = edit_distance(target, c);
        match best {
            Some((_, bd)) if d >= bd => {}
            _ => best = Some((c, d)),
        }
    }
    // Suggest only for a close, non-trivial match — never for a length-1 target
    // (every other short name is "distance 1" and the suggestion would be noise).
    best.filter(|&(_, d)| d > 0 && d <= 2 && d < target.chars().count())
        .map(|(c, _)| c)
}

/// Levenshtein distance over chars, with the usual two-row DP.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}
