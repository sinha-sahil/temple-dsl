//! Canonical pretty-printer over the parsed AST — milestone 8.
//!
//! The layout is deterministic and idempotent: `format(format(src)) ==
//! format(src)`, and the output always re-parses to the same AST. The output
//! *structure* (the `let` preamble and output object/array tree) is laid out
//! multiline with two-space indents; *expressions* inside holes are rendered
//! inline with canonical spacing, parenthesized exactly enough to preserve the
//! parse tree.

use crate::parse::{
    escape, escape_interp_text, BinOp, Expr, ExprKind, InterpPart, Lit, LitKey, Module, OutKind,
    OutNode, PathSegment, UnOp,
};

const INDENT: &str = "  ";

pub fn format_module(module: &Module) -> String {
    let mut out = String::new();
    for binding in &module.lets {
        out.push_str("let ");
        out.push_str(&binding.name);
        out.push_str(" = ");
        out.push_str(&fmt_expr(&binding.expr));
        out.push('\n');
    }
    if !module.lets.is_empty() {
        out.push('\n');
    }
    fmt_out(&module.output, 0, &mut out);
    out.push('\n');
    out
}

fn fmt_out(node: &OutNode, depth: usize, out: &mut String) {
    match &node.kind {
        OutKind::Literal(lit) => out.push_str(&fmt_lit(lit)),
        OutKind::Hole(expr) => {
            out.push_str("{{ ");
            out.push_str(&fmt_expr(expr));
            out.push_str(" }}");
        }
        OutKind::Interp(parts) => out.push_str(&fmt_interp(parts)),
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
                fmt_out(&field.value, depth + 1, out);
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
                fmt_out(item, depth + 1, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push(']');
        }
    }
}

fn fmt_interp(parts: &[InterpPart]) -> String {
    let mut s = String::from("\"");
    for part in parts {
        match part {
            InterpPart::Text(t) => s.push_str(&escape_interp_text(t)),
            InterpPart::Hole(expr) => {
                s.push_str("{{ ");
                s.push_str(&fmt_expr(expr));
                s.push_str(" }}");
            }
        }
    }
    s.push('"');
    s
}

fn fmt_expr(expr: &Expr) -> String {
    match &expr.kind {
        ExprKind::Literal(lit) => fmt_lit(lit),
        ExprKind::Path { root, segments, .. } => {
            let mut s = root.to_string();
            for seg in segments {
                s.push_str(&fmt_segment(seg));
            }
            s
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let my = bin_prec(*op);
            let right_assoc = matches!(op, BinOp::Coalesce);
            let lp = prec(lhs);
            let rp = prec(rhs);
            let l = wrap(fmt_expr(lhs), if right_assoc { lp <= my } else { lp < my });
            let r = wrap(fmt_expr(rhs), if right_assoc { rp < my } else { rp <= my });
            format!("{l} {} {r}", bin_op_str(*op))
        }
        ExprKind::Unary { op, operand } => {
            let inner = wrap(fmt_expr(operand), prec(operand) < UNARY_PREC);
            let op_str = match op {
                UnOp::Neg => "-",
                UnOp::Not => "!",
            };
            format!("{op_str}{inner}")
        }
        ExprKind::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            let c = wrap(fmt_expr(cond), prec(cond) <= TERNARY_PREC);
            format!(
                "{c} ? {} : {}",
                fmt_expr(then_branch),
                fmt_expr(else_branch)
            )
        }
        ExprKind::When { branches, fallback } => {
            let mut parts: Vec<String> = branches
                .iter()
                .map(|b| format!("{}: {}", fmt_expr(&b.cond), fmt_expr(&b.result)))
                .collect();
            if let Some(fb) = fallback {
                parts.push(format!("else: {}", fmt_expr(fb)));
            }
            format!("when {{ {} }}", parts.join(", "))
        }
        ExprKind::Lambda { params, body } => {
            let body = fmt_expr(body);
            if params.len() == 1 {
                format!("{} -> {body}", params[0].name)
            } else {
                let ps: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
                format!("({}) -> {body}", ps.join(", "))
            }
        }
        ExprKind::Let {
            name, value, body, ..
        } => format!("let {name} = {} in {}", fmt_expr(value), fmt_expr(body)),
        ExprKind::ArrayLit(items) => {
            let items: Vec<String> = items.iter().map(fmt_expr).collect();
            format!("[{}]", items.join(", "))
        }
        ExprKind::ObjectLit(entries) => {
            if entries.is_empty() {
                return "{}".to_string();
            }
            let parts: Vec<String> = entries
                .iter()
                .map(|e| {
                    let key = match &e.key {
                        LitKey::Static(k) => quote_double(k),
                        LitKey::Computed(ke) => format!("[{}]", fmt_expr(ke)),
                    };
                    let opt = if e.optional { "?" } else { "" };
                    format!("{key}{opt}: {}", fmt_expr(&e.value))
                })
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
        ExprKind::FuncCall { name, args, .. } => {
            let args: Vec<String> = args.iter().map(fmt_expr).collect();
            format!("{name}({})", args.join(", "))
        }
        ExprKind::Access { base, segments } => {
            // A numeric literal must be parenthesized before a `.` segment —
            // `5.foo` lexes as a malformed number, `(5).foo` re-parses cleanly.
            let needs_parens = prec(base) < ATOM_PREC
                || matches!(&base.kind, ExprKind::Literal(Lit::Int(_) | Lit::Decimal(_)));
            let mut s = wrap(fmt_expr(base), needs_parens);
            for seg in segments {
                s.push_str(&fmt_segment(seg));
            }
            s
        }
    }
}

fn fmt_segment(seg: &PathSegment) -> String {
    match seg {
        PathSegment::Field { name, optional, .. } => {
            if *optional {
                format!("?.{name}")
            } else {
                format!(".{name}")
            }
        }
        PathSegment::Method { name, args, .. } => {
            let args: Vec<String> = args.iter().map(fmt_expr).collect();
            format!(".{name}({})", args.join(", "))
        }
        PathSegment::Index { expr, .. } => format!("[{}]", fmt_expr(expr)),
    }
}

fn fmt_lit(lit: &Lit) -> String {
    match lit {
        Lit::Null => "null".to_string(),
        Lit::Bool(b) => b.to_string(),
        Lit::Int(n) => n.to_string(),
        Lit::Decimal(d) => d.to_string(),
        Lit::Str(s) => format!("'{}'", escape(s, '\'')),
    }
}

// Precedence ladder, mirroring the parser. Atoms bind tightest; ternary loosest.
const TERNARY_PREC: u8 = 1;
const UNARY_PREC: u8 = 9;
const ATOM_PREC: u8 = 10;

fn bin_prec(op: BinOp) -> u8 {
    match op {
        BinOp::Coalesce => 2,
        BinOp::Or => 3,
        BinOp::And => 4,
        BinOp::Eq | BinOp::Ne => 5,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 6,
        BinOp::Add | BinOp::Sub => 7,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 8,
    }
}

fn prec(expr: &Expr) -> u8 {
    match &expr.kind {
        ExprKind::Ternary { .. } => TERNARY_PREC,
        ExprKind::Binary { op, .. } => bin_prec(*op),
        ExprKind::Unary { .. } => UNARY_PREC,
        ExprKind::Lambda { .. } | ExprKind::Let { .. } => 0,
        _ => ATOM_PREC,
    }
}

fn bin_op_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::Coalesce => "??",
    }
}

fn wrap(s: String, needs_parens: bool) -> String {
    if needs_parens {
        format!("({s})")
    } else {
        s
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str(INDENT);
    }
}

fn quote_double(s: &str) -> String {
    format!("\"{}\"", escape(s, '"'))
}
