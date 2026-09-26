//! Canonical formatter: idempotent, and its output parses back to the same
//! tree.

use super::ast::{
    BinOp, Expr, ExprKind, InterpPart, Lit, LitKey, Module, OutKind, OutNode, PathSegment, UnOp,
};

const INDENT: &str = "  ";

pub(crate) fn format_module(module: &Module) -> String {
    let mut out = String::new();
    for binding in &module.lets {
        out.push_str("let ");
        out.push_str(&binding.name);
        out.push_str(" = ");
        out.push_str(&format_expr(&binding.expr));
        out.push('\n');
    }
    if !module.lets.is_empty() {
        out.push('\n');
    }
    format_output(&module.output, 0, &mut out);
    out.push('\n');
    out
}

fn format_output(node: &OutNode, depth: usize, out: &mut String) {
    match &node.kind {
        OutKind::Literal(literal) => out.push_str(&format_literal(literal)),
        OutKind::Hole(expr) => {
            out.push_str("{{ ");
            out.push_str(&format_expr(expr));
            out.push_str(" }}");
        }
        OutKind::Interp(parts) => out.push_str(&format_interp(parts)),
        OutKind::Object(fields) => {
            if fields.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (i, field) in fields.iter().enumerate() {
                indent(out, depth + 1);
                out.push_str(&quote_double(&field.key));
                if field.optional {
                    out.push('?');
                }
                out.push_str(": ");
                format_output(&field.value, depth + 1, out);
                out.push_str(if i + 1 < fields.len() { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push('}');
        }
        OutKind::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                indent(out, depth + 1);
                format_output(item, depth + 1, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push(']');
        }
    }
}

fn format_interp(parts: &[InterpPart]) -> String {
    let mut out = String::from("\"");
    for part in parts {
        match part {
            InterpPart::Text(text) => out.push_str(&escape_interp_text(text)),
            InterpPart::Hole(expr) => {
                out.push_str("{{ ");
                out.push_str(&format_expr(expr));
                out.push_str(" }}");
            }
        }
    }
    out.push('"');
    out
}

fn format_expr(expr: &Expr) -> String {
    match &expr.kind {
        ExprKind::Literal(literal) => format_literal(literal),
        ExprKind::Path { root, segments, .. } => {
            let mut out = root.to_string();
            for segment in segments {
                out.push_str(&format_segment(segment));
            }
            out
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let own = op.precedence();
            let right_assoc = matches!(op, BinOp::Coalesce);
            let left_prec = precedence(lhs);
            let right_prec = precedence(rhs);
            let left = wrap(
                format_expr(lhs),
                if right_assoc {
                    left_prec <= own
                } else {
                    left_prec < own
                },
            );
            let right = wrap(
                format_expr(rhs),
                if right_assoc {
                    right_prec < own
                } else {
                    right_prec <= own
                },
            );
            format!("{left} {} {right}", op.symbol())
        }
        ExprKind::Unary { op, operand } => {
            let inner = wrap(format_expr(operand), precedence(operand) < UNARY_PREC);
            let symbol = match op {
                UnOp::Neg => "-",
                UnOp::Not => "!",
            };
            format!("{symbol}{inner}")
        }
        ExprKind::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            let cond = wrap(format_expr(cond), precedence(cond) <= TERNARY_PREC);
            format!(
                "{cond} ? {} : {}",
                format_expr(then_branch),
                format_expr(else_branch)
            )
        }
        ExprKind::When { branches, fallback } => {
            let mut parts: Vec<String> = branches
                .iter()
                .map(|branch| {
                    format!(
                        "{}: {}",
                        format_expr(&branch.cond),
                        format_expr(&branch.result)
                    )
                })
                .collect();
            if let Some(fallback) = fallback {
                parts.push(format!("else: {}", format_expr(fallback)));
            }
            format!("when {{ {} }}", parts.join(", "))
        }
        ExprKind::Lambda { params, body } => {
            let body = format_expr(body);
            if params.len() == 1 {
                format!("{} -> {body}", params[0].name)
            } else {
                let names: Vec<&str> = params.iter().map(|param| param.name.as_str()).collect();
                format!("({}) -> {body}", names.join(", "))
            }
        }
        ExprKind::Let {
            name, value, body, ..
        } => format!(
            "let {name} = {} in {}",
            format_expr(value),
            format_expr(body)
        ),
        ExprKind::ArrayLit(items) => {
            let items: Vec<String> = items.iter().map(format_expr).collect();
            format!("[{}]", items.join(", "))
        }
        ExprKind::ObjectLit(entries) => {
            if entries.is_empty() {
                return "{}".to_string();
            }
            let parts: Vec<String> = entries
                .iter()
                .map(|entry| {
                    let key = match &entry.key {
                        LitKey::Static(key) => quote_double(key),
                        LitKey::Computed(key_expr) => format!("[{}]", format_expr(key_expr)),
                    };
                    let optional = if entry.optional { "?" } else { "" };
                    format!("{key}{optional}: {}", format_expr(&entry.value))
                })
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
        ExprKind::FuncCall { name, args, .. } => {
            let args: Vec<String> = args.iter().map(format_expr).collect();
            format!("{name}({})", args.join(", "))
        }
        ExprKind::Access { base, segments } => {
            // A numeric literal must be parenthesized before a `.` segment —
            // `5.foo` lexes as a malformed number, `(5).foo` re-parses cleanly.
            let needs_parens = precedence(base) < ATOM_PREC
                || matches!(&base.kind, ExprKind::Literal(Lit::Int(_) | Lit::Decimal(_)));
            let mut out = wrap(format_expr(base), needs_parens);
            for segment in segments {
                out.push_str(&format_segment(segment));
            }
            out
        }
    }
}

fn format_segment(segment: &PathSegment) -> String {
    match segment {
        PathSegment::Field { name, optional, .. } => {
            if *optional {
                format!("?.{name}")
            } else {
                format!(".{name}")
            }
        }
        PathSegment::Method { name, args, .. } => {
            let args: Vec<String> = args.iter().map(format_expr).collect();
            format!(".{name}({})", args.join(", "))
        }
        PathSegment::Index { expr, .. } => format!("[{}]", format_expr(expr)),
    }
}

fn format_literal(literal: &Lit) -> String {
    match literal {
        Lit::Null => "null".to_string(),
        Lit::Bool(value) => value.to_string(),
        Lit::Int(value) => value.to_string(),
        Lit::Decimal(value) => value.to_string(),
        Lit::Str(text) => format!("'{}'", escape(text, '\'')),
    }
}

// Precedence of the non-binary nodes, on the scale of `BinOp::precedence`.
const TERNARY_PREC: u8 = 1;
const UNARY_PREC: u8 = 9;
const ATOM_PREC: u8 = 10;

fn precedence(expr: &Expr) -> u8 {
    match &expr.kind {
        ExprKind::Ternary { .. } => TERNARY_PREC,
        ExprKind::Binary { op, .. } => op.precedence(),
        ExprKind::Unary { .. } => UNARY_PREC,
        ExprKind::Lambda { .. } | ExprKind::Let { .. } => 0,
        _ => ATOM_PREC,
    }
}

fn wrap(text: String, needs_parens: bool) -> String {
    if needs_parens {
        format!("({text})")
    } else {
        text
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str(INDENT);
    }
}

fn quote_double(text: &str) -> String {
    format!("\"{}\"", escape(text, '"'))
}

/// The inverse of `unescape`, for printing a string literal inside `quote`s.
fn escape(text: &str, quote: char) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// `escape` for interpolated text; also escapes `{` so it can't open a hole.
fn escape_interp_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '{' => out.push_str("\\{"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}
