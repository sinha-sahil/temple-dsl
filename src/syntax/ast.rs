//! Serialized into blobs: changing a type here changes the blob format, so
//! bump `BLOB_VERSION`.

use crate::common::error::Span;
use crate::common::value::Value;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Module {
    pub lets: Vec<LetBinding>,
    pub output: OutNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetBinding {
    pub name: SmolStr,
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutNode {
    pub kind: OutKind,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutKind {
    Literal(Lit),
    /// `{{ expr }}` on its own: the hole's value is the node's value.
    Hole(Expr),
    Object(Vec<ObjField>),
    Array(Vec<OutNode>),
    /// `"text {{ expr }} more text"`.
    Interp(Vec<InterpPart>),
}

/// `"key"?:` sets `optional`: the field is left out when null.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjField {
    pub key: SmolStr,
    pub value: OutNode,
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InterpPart {
    Text(SmolStr),
    Hole(Expr),
}

/// Kept apart from [`Value`] so the tree serializes without making `Value` a
/// serde type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Lit {
    Null,
    Bool(bool),
    Int(i64),
    Decimal(Decimal),
    Str(SmolStr),
}

impl Lit {
    pub fn to_value(&self) -> Value {
        match self {
            Lit::Null => Value::Null,
            Lit::Bool(b) => Value::Bool(*b),
            Lit::Int(n) => Value::Int(*n),
            Lit::Decimal(d) => Value::Decimal(*d),
            Lit::Str(s) => Value::Str(s.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExprKind {
    Literal(Lit),
    /// `root.a.b[i].method()`, where `root` is `input`, `this` or a name.
    Path {
        root: SmolStr,
        root_span: Span,
        segments: Vec<PathSegment>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: UnOp,
        operand: Box<Expr>,
    },
    Ternary {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    When {
        branches: Vec<WhenBranch>,
        fallback: Option<Box<Expr>>,
    },
    /// Only valid as a method argument.
    Lambda {
        params: Vec<LambdaParam>,
        body: Box<Expr>,
    },
    Let {
        name: SmolStr,
        name_span: Span,
        value: Box<Expr>,
        body: Box<Expr>,
    },
    ArrayLit(Vec<Expr>),
    ObjectLit(Vec<ObjEntry>),
    /// Segments after something that isn't a name: `f(x).method()`,
    /// `[1, 2].sort()`, `(a + b).foo`.
    Access {
        base: Box<Expr>,
        segments: Vec<PathSegment>,
    },
    FuncCall {
        name: SmolStr,
        name_span: Span,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjEntry {
    pub key: LitKey,
    pub value: Expr,
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LitKey {
    Static(SmolStr),
    Computed(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Coalesce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhenBranch {
    pub cond: Expr,
    pub result: Expr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LambdaParam {
    pub name: SmolStr,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PathSegment {
    /// `?.name` when `optional`: null if missing.
    Field {
        name: SmolStr,
        span: Span,
        optional: bool,
    },
    Method {
        name: SmolStr,
        span: Span,
        args: Vec<Expr>,
    },
    Index {
        expr: Box<Expr>,
        span: Span,
    },
}

impl BinOp {
    /// Operators the parser climbs, in matching order: longer spellings first,
    /// so `<=` isn't read as `<`. `??` is parsed apart (it groups right).
    pub const CLIMBING: [BinOp; 13] = [
        BinOp::Or,
        BinOp::And,
        BinOp::Eq,
        BinOp::Ne,
        BinOp::Le,
        BinOp::Ge,
        BinOp::Lt,
        BinOp::Gt,
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::Div,
        BinOp::Mod,
    ];

    pub fn symbol(self) -> &'static str {
        match self {
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

    /// Higher binds tighter.
    pub fn precedence(self) -> u8 {
        match self {
            BinOp::Coalesce => 2,
            BinOp::Or => 3,
            BinOp::And => 4,
            BinOp::Eq | BinOp::Ne => 5,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 6,
            BinOp::Add | BinOp::Sub => 7,
            BinOp::Mul | BinOp::Div | BinOp::Mod => 8,
        }
    }
}

/// Either kind of tree node, so one walk covers both.
#[derive(Clone, Copy)]
pub enum Node<'a> {
    Out(&'a OutNode),
    Expr(&'a Expr),
}

impl<'a> Node<'a> {
    pub fn for_each_child(self, f: &mut dyn FnMut(Node<'a>)) {
        match self {
            Node::Out(node) => match &node.kind {
                OutKind::Literal(_) => {}
                OutKind::Hole(expr) => f(Node::Expr(expr)),
                OutKind::Object(fields) => {
                    for field in fields {
                        f(Node::Out(&field.value));
                    }
                }
                OutKind::Array(items) => {
                    for item in items {
                        f(Node::Out(item));
                    }
                }
                OutKind::Interp(parts) => {
                    for part in parts {
                        if let InterpPart::Hole(expr) = part {
                            f(Node::Expr(expr));
                        }
                    }
                }
            },
            Node::Expr(expr) => expr.for_each_child(&mut |child| f(Node::Expr(child))),
        }
    }
}

impl Expr {
    pub fn for_each_child<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        match &self.kind {
            ExprKind::Literal(_) => {}
            ExprKind::Path { segments, .. } => for_each_segment_expr(segments, f),
            ExprKind::Binary { lhs, rhs, .. } => {
                f(lhs);
                f(rhs);
            }
            ExprKind::Unary { operand, .. } => f(operand),
            ExprKind::Ternary {
                cond,
                then_branch,
                else_branch,
            } => {
                f(cond);
                f(then_branch);
                f(else_branch);
            }
            ExprKind::When { branches, fallback } => {
                for branch in branches {
                    f(&branch.cond);
                    f(&branch.result);
                }
                if let Some(fallback) = fallback {
                    f(fallback);
                }
            }
            ExprKind::Lambda { body, .. } => f(body),
            ExprKind::Let { value, body, .. } => {
                f(value);
                f(body);
            }
            ExprKind::ArrayLit(items) => items.iter().for_each(f),
            ExprKind::ObjectLit(entries) => {
                for entry in entries {
                    if let LitKey::Computed(key) = &entry.key {
                        f(key);
                    }
                    f(&entry.value);
                }
            }
            ExprKind::Access { base, segments } => {
                f(base);
                for_each_segment_expr(segments, f);
            }
            ExprKind::FuncCall { args, .. } => args.iter().for_each(f),
        }
    }
}

fn for_each_segment_expr<'a>(segments: &'a [PathSegment], f: &mut dyn FnMut(&'a Expr)) {
    for segment in segments {
        match segment {
            PathSegment::Field { .. } => {}
            PathSegment::Method { args, .. } => args.iter().for_each(&mut *f),
            PathSegment::Index { expr, .. } => f(expr),
        }
    }
}

impl OutNode {
    pub fn for_each_expr<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        Node::Out(self).for_each_child(&mut |child| match child {
            Node::Out(node) => node.for_each_expr(f),
            Node::Expr(expr) => f(expr),
        });
    }
}
