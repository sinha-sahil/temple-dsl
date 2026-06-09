use crate::error::{CompileError, Span};
use crate::value::Value;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutNode {
    pub kind: OutKind,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutKind {
    Literal(Lit),
    Hole(Expr),
    Object(Vec<ObjField>),
    Array(Vec<OutNode>),
    Interp(Vec<InterpPart>),
}

/// One key/value entry of an output object. `optional` (`"key"?:`) drops the
/// key from the rendered object when its value is null.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjField {
    pub key: SmolStr,
    pub value: OutNode,
    pub optional: bool,
}

/// A scalar literal, kept distinct from `Value` so the serialized AST doesn't force
/// `Value: Deserialize` — which is what makes `render::<Value>` a compile error.
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
pub enum InterpPart {
    Text(SmolStr),
    Hole(Expr),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExprKind {
    Literal(Lit),
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
    /// Trailing segments on a non-identifier base — `f(x).method()`, `[1,2].sort()`,
    /// `(a + b).foo`. Identifier-rooted paths use `Path` instead.
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

/// One entry of an expression-position object literal. The key may be a static
/// string or a computed expression (`{ [expr]: v }`); `optional` (`"k"?:`) drops
/// the entry when its value is null.
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

pub fn parse(src: &str) -> Result<Module, Vec<CompileError>> {
    let mut p = Parser::new(src);
    let mut lets = Vec::new();
    loop {
        p.skip_ws();
        if !p.matches_ident("let") {
            break;
        }
        match p.parse_let_binding() {
            Ok(b) => lets.push(b),
            // Lets are line-oriented; recover to the next line and keep going so
            // a typo in one binding doesn't mask the rest.
            Err(e) => {
                p.errors.push(e);
                p.skip_to_line_end();
            }
        }
    }
    let output = match p.parse_output() {
        Ok(n) => n,
        Err(e) => {
            p.errors.push(e);
            return Err(p.errors);
        }
    };
    p.skip_ws();
    // Only flag trailing input on an otherwise-clean parse — after recovery the
    // leftover bytes are expected noise, not a separate error worth reporting.
    if p.pos < p.src.len() && p.errors.is_empty() {
        return Err(vec![CompileError::Syntax {
            message: format!(
                "expected end of input, found '{}'",
                p.peek_char().unwrap_or('?')
            ),
            span: p.cur_char_span(),
        }]);
    }
    if p.errors.is_empty() {
        Ok(Module { lets, output })
    } else {
        Err(p.errors)
    }
}

/// Parser nesting cap. One level can push the whole precedence chain (~11 frames),
/// so this stays well under the stack-overflow point — don't raise it casually.
const MAX_DEPTH: usize = 64;

/// What follows a collection item: more items, or the collection closed.
enum ItemEnd {
    More,
    Closed,
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    depth: usize,
    /// Recovered errors. Collection parsers (object/array) push here and resync
    /// to the next item boundary instead of aborting, so one save surfaces every
    /// independent structural error. Sub-expression parsing stays fail-fast and
    /// propagates to the nearest item boundary.
    errors: Vec<CompileError>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            depth: 0,
            errors: Vec::new(),
        }
    }

    fn too_deep(&self) -> CompileError {
        CompileError::TooDeep {
            limit: MAX_DEPTH,
            span: Span::new(self.pos, self.pos),
        }
    }

    /// Run `f` one nesting level deeper, rejecting past MAX_DEPTH. The counter
    /// unwinds on every exit path, so recursion points can't leak depth.
    fn guarded<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err(self.too_deep());
        }
        let r = f(self);
        self.depth -= 1;
        r
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_char(&self) -> Option<char> {
        let b = self.peek()?;
        let len = utf8_len(b);
        std::str::from_utf8(self.src.get(self.pos..self.pos + len)?)
            .ok()?
            .chars()
            .next()
    }

    /// Span covering exactly the codepoint at the cursor, so error spans always
    /// land on UTF-8 char boundaries (never mid-codepoint).
    fn cur_char_span(&self) -> Span {
        let w = self.peek().map_or(0, utf8_len);
        Span::new(self.pos, self.pos + w)
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else if b == b'#' {
                while let Some(c) = self.peek() {
                    self.pos += 1;
                    if c == b'\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn skip_ws_at(&self, mut p: usize) -> usize {
        while p < self.src.len() {
            let b = self.src[p];
            if b.is_ascii_whitespace() {
                p += 1;
            } else if b == b'#' {
                while p < self.src.len() && self.src[p] != b'\n' {
                    p += 1;
                }
            } else {
                break;
            }
        }
        p
    }

    fn skip_ident_at(&self, mut p: usize) -> usize {
        while p < self.src.len() && (self.src[p].is_ascii_alphanumeric() || self.src[p] == b'_') {
            p += 1;
        }
        p
    }

    fn expect(&mut self, byte: u8) -> Result<(), CompileError> {
        match self.peek() {
            Some(b) if b == byte => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(CompileError::Syntax {
                message: format!("expected '{}'", byte as char),
                span: self.cur_char_span(),
            }),
        }
    }

    fn expect_str(&mut self, s: &str) -> Result<(), CompileError> {
        if self.src[self.pos..].starts_with(s.as_bytes()) {
            self.pos += s.len();
            Ok(())
        } else {
            Err(CompileError::Syntax {
                message: format!("expected '{s}'"),
                span: Span::new(self.pos, self.pos + s.len()),
            })
        }
    }

    fn matches(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s.as_bytes())
    }

    fn matches_ident(&self, ident: &str) -> bool {
        if !self.src[self.pos..].starts_with(ident.as_bytes()) {
            return false;
        }
        let next = self.src.get(self.pos + ident.len()).copied();
        !matches!(next, Some(b) if b.is_ascii_alphanumeric() || b == b'_')
    }

    fn parse_output(&mut self) -> Result<OutNode, CompileError> {
        self.guarded(Self::parse_output_inner)
    }

    fn parse_output_inner(&mut self) -> Result<OutNode, CompileError> {
        self.skip_ws();
        let start = self.pos;

        if self.matches("{{") {
            return self.parse_hole();
        }

        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_interp_string(start),
            Some(b'\'') => {
                let v = self.parse_single_quoted_string()?;
                Ok(OutNode {
                    kind: OutKind::Literal(Lit::Str(v)),
                    span: Span::new(start, self.pos),
                })
            }
            Some(b'-') | Some(b'0'..=b'9') => {
                let v = self.parse_number()?;
                Ok(OutNode {
                    kind: OutKind::Literal(v),
                    span: Span::new(start, self.pos),
                })
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let v = self.parse_keyword_value()?;
                Ok(OutNode {
                    kind: OutKind::Literal(v),
                    span: Span::new(start, self.pos),
                })
            }
            Some(_) => Err(CompileError::Syntax {
                message: format!("unexpected '{}'", self.peek_char().unwrap_or('?')),
                span: self.cur_char_span(),
            }),
            None => Err(CompileError::Syntax {
                message: "unexpected end of input".into(),
                span: Span::new(self.pos, self.pos),
            }),
        }
    }

    fn parse_object(&mut self) -> Result<OutNode, CompileError> {
        let start = self.pos;
        self.expect(b'{')?;
        let entries = self.parse_items(b'}', start, Self::parse_object_entry);
        Ok(OutNode {
            kind: OutKind::Object(entries),
            span: Span::new(start, self.pos),
        })
    }

    /// The shared collection loop: parse items separated by `,` or a newline
    /// (DESIGN §2) until `closer`. On a bad item it records the error and
    /// resyncs to the next boundary, so independent errors report together.
    fn parse_items<T>(
        &mut self,
        closer: u8,
        start: usize,
        mut parse_item: impl FnMut(&mut Self) -> Result<T, CompileError>,
    ) -> Vec<T> {
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b) if b == closer => {
                    self.pos += 1;
                    break;
                }
                None => {
                    self.unterminated(start, closer as char);
                    break;
                }
                _ => {}
            }
            match parse_item(self) {
                Ok(v) => items.push(v),
                Err(e) => {
                    self.errors.push(e);
                    if matches!(self.recover_item(closer), ItemEnd::Closed) {
                        break;
                    }
                    continue;
                }
            }
            if matches!(self.after_item(closer), ItemEnd::Closed) {
                break;
            }
        }
        items
    }

    fn parse_object_entry(&mut self) -> Result<ObjField, CompileError> {
        self.skip_ws();
        let key = self.parse_double_quoted_string()?;
        self.skip_ws();
        let optional = self.peek() == Some(b'?');
        if optional {
            self.pos += 1;
            self.skip_ws();
        }
        self.expect(b':')?;
        self.skip_ws();
        let value = self.parse_output()?;
        Ok(ObjField {
            key,
            value,
            optional,
        })
    }

    fn parse_array(&mut self) -> Result<OutNode, CompileError> {
        let start = self.pos;
        self.expect(b'[')?;
        let items = self.parse_items(b']', start, Self::parse_output);
        Ok(OutNode {
            kind: OutKind::Array(items),
            span: Span::new(start, self.pos),
        })
    }

    /// After a failed item, skip to the next boundary and consume it.
    fn recover_item(&mut self, closer: u8) -> ItemEnd {
        if self.sync_to_item_boundary() {
            self.pos += 1; // the `,` or newline the sync stopped at
            ItemEnd::More
        } else {
            if self.peek() == Some(closer) {
                self.pos += 1;
            }
            ItemEnd::Closed
        }
    }

    /// After a good item, consume its separator — a `,`, a line break
    /// (newline-separated entries, DESIGN §2), or the closer.
    fn after_item(&mut self, closer: u8) -> ItemEnd {
        self.skip_ws();
        match self.peek() {
            Some(b',') => {
                self.pos += 1;
                ItemEnd::More
            }
            Some(b) if b == closer => {
                self.pos += 1;
                ItemEnd::Closed
            }
            None => {
                self.errors.push(CompileError::Syntax {
                    message: format!("expected '{}'", closer as char),
                    span: Span::new(self.pos, self.pos),
                });
                ItemEnd::Closed
            }
            Some(_) if self.newline_behind() => ItemEnd::More,
            Some(_) => {
                self.errors.push(CompileError::Syntax {
                    message: format!(
                        "expected ',' or '{}', found '{}'",
                        closer as char,
                        self.peek_char().unwrap_or('?')
                    ),
                    span: self.cur_char_span(),
                });
                self.recover_item(closer)
            }
        }
    }

    /// Separator inside an expression-position `[…]`/`{…}` literal: a `,`
    /// (tolerating a trailing one) or a line break. `Closed` leaves the cursor
    /// in place for the caller's closing `expect` to consume or report.
    fn expr_item_sep(&mut self, closer: u8) -> ItemEnd {
        self.skip_ws();
        if self.peek() == Some(b',') {
            self.pos += 1;
            self.skip_ws();
            if self.peek() == Some(closer) {
                return ItemEnd::Closed;
            }
            return ItemEnd::More;
        }
        if self.peek() == Some(closer) || self.peek().is_none() {
            return ItemEnd::Closed;
        }
        if self.newline_behind() {
            ItemEnd::More
        } else {
            ItemEnd::Closed
        }
    }

    /// True when the last non-whitespace byte before the cursor sits on an
    /// earlier line — entries separated by a line break instead of a comma.
    fn newline_behind(&self) -> bool {
        let mut i = self.pos;
        while i > 0 {
            let b = self.src[i - 1];
            if b == b'\n' {
                return true;
            }
            if !b.is_ascii_whitespace() {
                return false;
            }
            i -= 1;
        }
        false
    }

    fn unterminated(&mut self, start: usize, closer: char) {
        self.errors.push(CompileError::Syntax {
            message: format!("unterminated, expected '{closer}'"),
            span: Span::new(start, self.pos),
        });
    }

    /// Skip past a malformed item to the next top-level separator (`,` or
    /// newline) or the closing bracket/EOF, tracking nesting and strings so we
    /// never stop inside them. Returns `true` if it stopped at a separator
    /// (left unconsumed). Always makes forward progress.
    fn sync_to_item_boundary(&mut self) -> bool {
        let mut depth: i32 = 0;
        while let Some(b) = self.peek() {
            match b {
                // A `{{ … }}` hole is opaque to the scan — skip it whole so its
                // `}}` is never mistaken for this collection's closing bracket.
                b'{' if self.peek_at(1) == Some(b'{') => {
                    self.pos += 2;
                    self.skip_to_hole_close();
                }
                b'"' | b'\'' => self.skip_string_lenient(b),
                // Stop before the comment's newline so it counts as a boundary.
                b'#' => {
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.pos += 1;
                    }
                }
                b'{' | b'[' | b'(' => {
                    depth += 1;
                    self.pos += 1;
                }
                b'}' | b']' | b')' => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                    self.pos += 1;
                }
                b',' | b'\n' if depth == 0 => return true,
                _ => self.pos += 1,
            }
        }
        false
    }

    fn skip_string_lenient(&mut self, quote: u8) {
        self.pos += 1;
        while let Some(b) = self.peek() {
            if b == b'\\' {
                self.pos += 1;
                if self.peek().is_some() {
                    self.pos += 1;
                }
            } else if b == quote {
                self.pos += 1;
                break;
            } else {
                self.pos += 1;
            }
        }
    }

    fn skip_to_line_end(&mut self) {
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == b'\n' {
                break;
            }
        }
    }

    /// Parse an output-position double-quoted string, splitting `{{ expr }}` holes
    /// for interpolation; with no holes it collapses to a plain literal.
    fn parse_interp_string(&mut self, start: usize) -> Result<OutNode, CompileError> {
        self.expect(b'"')?;
        let mut parts: Vec<InterpPart> = Vec::new();
        let mut buf: Vec<u8> = Vec::new();
        let flush = |buf: &mut Vec<u8>, parts: &mut Vec<InterpPart>| {
            if !buf.is_empty() {
                let text = String::from_utf8(std::mem::take(buf)).unwrap_or_default();
                parts.push(InterpPart::Text(SmolStr::new(text)));
            }
        };
        loop {
            match self.peek() {
                None => {
                    return Err(CompileError::Syntax {
                        message: "unterminated string".into(),
                        span: Span::new(start, self.pos),
                    });
                }
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'n') => buf.push(b'\n'),
                        Some(b't') => buf.push(b'\t'),
                        Some(b'r') => buf.push(b'\r'),
                        Some(b'"') => buf.push(b'"'),
                        Some(b'\'') => buf.push(b'\''),
                        Some(b'\\') => buf.push(b'\\'),
                        // `\{` = literal brace, so text can hold `{{` without
                        // opening a hole (and the formatter can re-emit it).
                        Some(b'{') => buf.push(b'{'),
                        Some(other) => {
                            buf.push(b'\\');
                            buf.push(other);
                        }
                        None => {
                            buf.push(b'\\');
                            continue;
                        }
                    }
                    self.pos += 1;
                }
                Some(b'{') if self.peek_at(1) == Some(b'{') => {
                    flush(&mut buf, &mut parts);
                    self.pos += 2;
                    self.skip_ws();
                    let expr = self.parse_expr()?;
                    self.skip_ws();
                    self.expect_str("}}")?;
                    parts.push(InterpPart::Hole(expr));
                }
                Some(b) => {
                    buf.push(b);
                    self.pos += 1;
                }
            }
        }
        flush(&mut buf, &mut parts);
        let span = Span::new(start, self.pos);
        if parts.iter().any(|p| matches!(p, InterpPart::Hole(_))) {
            Ok(OutNode {
                kind: OutKind::Interp(parts),
                span,
            })
        } else {
            let mut s = String::new();
            for p in &parts {
                if let InterpPart::Text(t) = p {
                    s.push_str(t);
                }
            }
            Ok(OutNode {
                kind: OutKind::Literal(Lit::Str(SmolStr::new(s))),
                span,
            })
        }
    }

    fn parse_hole(&mut self) -> Result<OutNode, CompileError> {
        let start = self.pos;
        self.expect_str("{{")?;
        self.skip_ws();
        // A failed hole recovers *here* (skip to its own `}}`) rather than
        // bubbling up — otherwise the cursor would sit before the `}}`, which a
        // surrounding object/array would misread as its own closing bracket.
        match self.parse_expr() {
            Ok(expr) => {
                self.skip_ws();
                if let Err(e) = self.expect_str("}}") {
                    self.errors.push(e);
                    self.skip_to_hole_close();
                }
                Ok(OutNode {
                    kind: OutKind::Hole(expr),
                    span: Span::new(start, self.pos),
                })
            }
            Err(e) => {
                self.errors.push(e);
                self.skip_to_hole_close();
                let span = Span::new(start, self.pos);
                Ok(OutNode {
                    kind: OutKind::Hole(Expr {
                        kind: ExprKind::Literal(Lit::Null),
                        span,
                    }),
                    span,
                })
            }
        }
    }

    /// Scan to the hole's closing `}}` (at bracket-depth 0), skipping strings,
    /// comments, and any balanced brackets the expression contained. The cursor
    /// must already be past the opening `{{`.
    fn skip_to_hole_close(&mut self) {
        let mut depth: i32 = 0;
        while let Some(b) = self.peek() {
            match b {
                b'"' | b'\'' => self.skip_string_lenient(b),
                b'#' => self.skip_to_line_end(),
                b'}' if depth == 0 && self.peek_at(1) == Some(b'}') => {
                    self.pos += 2;
                    return;
                }
                b'{' | b'[' | b'(' => {
                    depth += 1;
                    self.pos += 1;
                }
                b'}' | b']' | b')' => {
                    depth = (depth - 1).max(0);
                    self.pos += 1;
                }
                _ => self.pos += 1,
            }
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, CompileError> {
        self.parse_let_in()
    }

    /// `let NAME = EXPR in EXPR` — a local binding usable anywhere an expression
    /// is (including `when` branches). Falls through to the operator chain when
    /// the expression doesn't start with `let`.
    fn parse_let_in(&mut self) -> Result<Expr, CompileError> {
        self.skip_ws();
        if !self.matches_ident("let") {
            return self.parse_ternary();
        }
        self.guarded(Self::parse_let_in_inner)
    }

    fn parse_let_in_inner(&mut self) -> Result<Expr, CompileError> {
        let start = self.pos;
        self.pos += "let".len();
        self.skip_ws();
        let name_start = self.pos;
        let name = self.read_ident();
        if name.is_empty() {
            return Err(CompileError::Syntax {
                message: "expected a name after 'let'".into(),
                span: Span::new(name_start, name_start + 1),
            });
        }
        let name_span = Span::new(name_start, self.pos);
        self.skip_ws();
        self.expect(b'=')?;
        self.skip_ws();
        let value = self.parse_expr()?;
        self.skip_ws();
        if !self.matches_ident("in") {
            return Err(CompileError::Syntax {
                message: "expected 'in' after the let-binding value".into(),
                span: self.cur_char_span(),
            });
        }
        self.pos += "in".len();
        self.skip_ws();
        let body = self.parse_expr()?;
        let end = body.span.end as usize;
        Ok(Expr {
            kind: ExprKind::Let {
                name,
                name_span,
                value: Box::new(value),
                body: Box::new(body),
            },
            span: Span::new(start, end),
        })
    }

    fn parse_ternary(&mut self) -> Result<Expr, CompileError> {
        let cond = self.parse_coalesce()?;
        self.skip_ws();
        if self.peek() == Some(b'?') && !self.matches("?.") && !self.matches("??") {
            let cond_start = cond.span.start;
            self.pos += 1;
            self.skip_ws();
            let then_branch = self.parse_expr()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let else_branch = self.parse_expr()?;
            let end = else_branch.span.end;
            Ok(Expr {
                kind: ExprKind::Ternary {
                    cond: Box::new(cond),
                    then_branch: Box::new(then_branch),
                    else_branch: Box::new(else_branch),
                },
                span: Span {
                    start: cond_start,
                    end,
                },
            })
        } else {
            Ok(cond)
        }
    }

    fn parse_coalesce(&mut self) -> Result<Expr, CompileError> {
        let lhs = self.parse_or()?;
        self.skip_ws();
        if self.matches("??") {
            self.pos += 2;
            self.skip_ws();
            let rhs = self.parse_coalesce()?;
            Ok(make_binary(BinOp::Coalesce, lhs, rhs))
        } else {
            Ok(lhs)
        }
    }

    fn parse_or(&mut self) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_and()?;
        loop {
            self.skip_ws();
            if !self.matches("||") {
                break;
            }
            self.pos += 2;
            self.skip_ws();
            let rhs = self.parse_and()?;
            lhs = make_binary(BinOp::Or, lhs, rhs);
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_equality()?;
        loop {
            self.skip_ws();
            if !self.matches("&&") {
                break;
            }
            self.pos += 2;
            self.skip_ws();
            let rhs = self.parse_equality()?;
            lhs = make_binary(BinOp::And, lhs, rhs);
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_comparison()?;
        loop {
            self.skip_ws();
            let op = if self.matches("==") {
                self.pos += 2;
                BinOp::Eq
            } else if self.matches("!=") {
                self.pos += 2;
                BinOp::Ne
            } else {
                break;
            };
            self.skip_ws();
            let rhs = self.parse_comparison()?;
            lhs = make_binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    fn parse_comparison(&mut self) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_additive()?;
        loop {
            self.skip_ws();
            let op = if self.matches("<=") {
                self.pos += 2;
                BinOp::Le
            } else if self.matches(">=") {
                self.pos += 2;
                BinOp::Ge
            } else if self.peek() == Some(b'<') {
                self.pos += 1;
                BinOp::Lt
            } else if self.peek() == Some(b'>') {
                self.pos += 1;
                BinOp::Gt
            } else {
                break;
            };
            self.skip_ws();
            let rhs = self.parse_additive()?;
            lhs = make_binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    fn parse_additive(&mut self) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some(b'+') => {
                    self.pos += 1;
                    BinOp::Add
                }
                Some(b'-') => {
                    self.pos += 1;
                    BinOp::Sub
                }
                _ => break,
            };
            self.skip_ws();
            let rhs = self.parse_multiplicative()?;
            lhs = make_binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_unary()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some(b'*') => {
                    self.pos += 1;
                    BinOp::Mul
                }
                Some(b'/') => {
                    self.pos += 1;
                    BinOp::Div
                }
                Some(b'%') => {
                    self.pos += 1;
                    BinOp::Mod
                }
                _ => break,
            };
            self.skip_ws();
            let rhs = self.parse_unary()?;
            lhs = make_binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, CompileError> {
        self.guarded(Self::parse_unary_inner)
    }

    fn parse_unary_inner(&mut self) -> Result<Expr, CompileError> {
        self.skip_ws();
        let start = self.pos;
        if self.peek() == Some(b'!') {
            self.pos += 1;
            self.skip_ws();
            let operand = self.parse_unary()?;
            let end = operand.span.end;
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op: UnOp::Not,
                    operand: Box::new(operand),
                },
                span: Span {
                    start: start as u32,
                    end,
                },
            });
        }
        if self.peek() == Some(b'-') {
            let next = self.peek_at(1);
            let is_number_literal = matches!(next, Some(b'0'..=b'9'))
                || (next == Some(b'.') && self.peek_at(2).is_some_and(|b| b.is_ascii_digit()));
            if !is_number_literal {
                self.pos += 1;
                self.skip_ws();
                let operand = self.parse_unary()?;
                let end = operand.span.end;
                return Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnOp::Neg,
                        operand: Box::new(operand),
                    },
                    span: Span {
                        start: start as u32,
                        end,
                    },
                });
            }
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, CompileError> {
        self.skip_ws();
        let start = self.pos;
        let base = self.parse_primary_inner(start)?;
        self.with_postfix(base, start)
    }

    /// Attach trailing `.field` / `?.field` / `.method()` / `[idx]` segments to a
    /// non-identifier base. (Identifier roots build `Path` directly, consuming
    /// their own segments, so this is a no-op for them.)
    fn with_postfix(&mut self, base: Expr, start: usize) -> Result<Expr, CompileError> {
        let segments = self.parse_path_segments()?;
        if segments.is_empty() {
            Ok(base)
        } else {
            Ok(Expr {
                kind: ExprKind::Access {
                    base: Box::new(base),
                    segments,
                },
                span: Span::new(start, self.pos),
            })
        }
    }

    fn parse_primary_inner(&mut self, start: usize) -> Result<Expr, CompileError> {
        match self.peek() {
            Some(b'(') => {
                self.pos += 1;
                self.skip_ws();
                let inner = self.parse_expr()?;
                self.skip_ws();
                self.expect(b')')?;
                Ok(Expr {
                    kind: inner.kind,
                    span: Span::new(start, self.pos),
                })
            }
            Some(b'\'') => {
                let s = self.parse_single_quoted_string()?;
                Ok(Expr {
                    kind: ExprKind::Literal(Lit::Str(s)),
                    span: Span::new(start, self.pos),
                })
            }
            Some(b'"') => {
                let s = self.parse_double_quoted_string()?;
                Ok(Expr {
                    kind: ExprKind::Literal(Lit::Str(s)),
                    span: Span::new(start, self.pos),
                })
            }
            Some(b'-') | Some(b'0'..=b'9') => {
                let v = self.parse_number()?;
                Ok(Expr {
                    kind: ExprKind::Literal(v),
                    span: Span::new(start, self.pos),
                })
            }
            Some(b'[') => {
                self.pos += 1;
                let mut items: Vec<Expr> = Vec::new();
                self.skip_ws();
                if self.peek() != Some(b']') {
                    loop {
                        self.skip_ws();
                        items.push(self.parse_expr()?);
                        if matches!(self.expr_item_sep(b']'), ItemEnd::Closed) {
                            break;
                        }
                    }
                }
                self.expect(b']')?;
                Ok(Expr {
                    kind: ExprKind::ArrayLit(items),
                    span: Span::new(start, self.pos),
                })
            }
            Some(b'{') => {
                self.pos += 1;
                let mut entries: Vec<ObjEntry> = Vec::new();
                self.skip_ws();
                if self.peek() != Some(b'}') {
                    loop {
                        self.skip_ws();
                        let key = if self.peek() == Some(b'[') {
                            self.pos += 1;
                            self.skip_ws();
                            let ke = self.parse_expr()?;
                            self.skip_ws();
                            self.expect(b']')?;
                            LitKey::Computed(ke)
                        } else {
                            LitKey::Static(self.parse_double_quoted_string()?)
                        };
                        self.skip_ws();
                        let optional = self.peek() == Some(b'?');
                        if optional {
                            self.pos += 1;
                            self.skip_ws();
                        }
                        self.expect(b':')?;
                        self.skip_ws();
                        let value = self.parse_expr()?;
                        entries.push(ObjEntry {
                            key,
                            value,
                            optional,
                        });
                        if matches!(self.expr_item_sep(b'}'), ItemEnd::Closed) {
                            break;
                        }
                    }
                }
                self.expect(b'}')?;
                Ok(Expr {
                    kind: ExprKind::ObjectLit(entries),
                    span: Span::new(start, self.pos),
                })
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let ident_start = self.pos;
                let ident = self.read_ident();
                let ident_span = Span::new(ident_start, self.pos);
                match ident.as_str() {
                    "true" => Ok(Expr {
                        kind: ExprKind::Literal(Lit::Bool(true)),
                        span: ident_span,
                    }),
                    "false" => Ok(Expr {
                        kind: ExprKind::Literal(Lit::Bool(false)),
                        span: ident_span,
                    }),
                    "null" => Ok(Expr {
                        kind: ExprKind::Literal(Lit::Null),
                        span: ident_span,
                    }),
                    "when" => self.parse_when_body(ident_start),
                    _ => {
                        if self.peek() == Some(b'(') {
                            let args = self.parse_method_args()?;
                            Ok(Expr {
                                kind: ExprKind::FuncCall {
                                    name: ident,
                                    name_span: ident_span,
                                    args,
                                },
                                span: Span::new(ident_start, self.pos),
                            })
                        } else {
                            let segments = self.parse_path_segments()?;
                            Ok(Expr {
                                kind: ExprKind::Path {
                                    root: ident,
                                    root_span: ident_span,
                                    segments,
                                },
                                span: Span::new(ident_start, self.pos),
                            })
                        }
                    }
                }
            }
            Some(_) => Err(CompileError::Syntax {
                message: format!(
                    "unexpected '{}' in expression",
                    self.peek_char().unwrap_or('?')
                ),
                span: self.cur_char_span(),
            }),
            None => Err(CompileError::Syntax {
                message: "expected expression, found end of input".into(),
                span: Span::new(self.pos, self.pos),
            }),
        }
    }

    fn parse_path_segments(&mut self) -> Result<Vec<PathSegment>, CompileError> {
        let mut segments: Vec<PathSegment> = Vec::new();
        loop {
            if self.peek() == Some(b'[') {
                let istart = self.pos;
                self.pos += 1;
                self.skip_ws();
                let inner = self.parse_expr()?;
                self.skip_ws();
                self.expect(b']')?;
                let ispan = Span::new(istart, self.pos);
                segments.push(PathSegment::Index {
                    expr: Box::new(inner),
                    span: ispan,
                });
                continue;
            }
            let saved = self.pos;
            self.skip_ws();
            if self.matches("?.")
                && self
                    .peek_at(2)
                    .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            {
                self.pos += 2;
                let nstart = self.pos;
                let name = self.read_ident();
                let nspan = Span::new(nstart, self.pos);
                segments.push(PathSegment::Field {
                    name,
                    span: nspan,
                    optional: true,
                });
                continue;
            }
            if self.peek() == Some(b'.')
                && self
                    .peek_at(1)
                    .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            {
                self.pos += 1;
                let nstart = self.pos;
                let name = self.read_ident();
                let nspan = Span::new(nstart, self.pos);
                if self.peek() == Some(b'(') {
                    let args = self.parse_method_args()?;
                    segments.push(PathSegment::Method {
                        name,
                        span: nspan,
                        args,
                    });
                } else {
                    segments.push(PathSegment::Field {
                        name,
                        span: nspan,
                        optional: false,
                    });
                }
                continue;
            }
            self.pos = saved;
            break;
        }
        Ok(segments)
    }

    fn parse_method_args(&mut self) -> Result<Vec<Expr>, CompileError> {
        self.expect(b'(')?;
        let mut args = Vec::new();
        self.skip_ws();
        if self.peek() != Some(b')') {
            loop {
                self.skip_ws();
                args.push(self.parse_method_arg()?);
                self.skip_ws();
                if self.peek() == Some(b',') {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b')') {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(b')')?;
        Ok(args)
    }

    fn parse_method_arg(&mut self) -> Result<Expr, CompileError> {
        if self.is_lambda_start() {
            self.parse_lambda()
        } else {
            self.parse_expr()
        }
    }

    fn is_lambda_start(&self) -> bool {
        let mut p = self.skip_ws_at(self.pos);
        if p >= self.src.len() {
            return false;
        }
        if self.src[p] == b'(' {
            p = self.skip_ws_at(p + 1);
            if p < self.src.len() && self.src[p] == b')' {
                p = self.skip_ws_at(p + 1);
                return self.src.get(p) == Some(&b'-') && self.src.get(p + 1) == Some(&b'>');
            }
            loop {
                if p >= self.src.len() {
                    return false;
                }
                let s = p;
                p = self.skip_ident_at(p);
                if p == s {
                    return false;
                }
                p = self.skip_ws_at(p);
                if p >= self.src.len() {
                    return false;
                }
                if self.src[p] == b',' {
                    p = self.skip_ws_at(p + 1);
                    continue;
                }
                if self.src[p] == b')' {
                    p = self.skip_ws_at(p + 1);
                    return self.src.get(p) == Some(&b'-') && self.src.get(p + 1) == Some(&b'>');
                }
                return false;
            }
        }
        if self.src[p].is_ascii_alphabetic() || self.src[p] == b'_' {
            let s = p;
            p = self.skip_ident_at(p);
            if p == s {
                return false;
            }
            p = self.skip_ws_at(p);
            return self.src.get(p) == Some(&b'-') && self.src.get(p + 1) == Some(&b'>');
        }
        false
    }

    fn parse_lambda(&mut self) -> Result<Expr, CompileError> {
        let start = self.pos;
        self.skip_ws();
        let params: Vec<LambdaParam> = if self.peek() == Some(b'(') {
            self.pos += 1;
            let mut ps: Vec<LambdaParam> = Vec::new();
            self.skip_ws();
            if self.peek() != Some(b')') {
                loop {
                    self.skip_ws();
                    let pstart = self.pos;
                    let name = self.read_ident();
                    if name.is_empty() {
                        return Err(CompileError::Syntax {
                            message: "expected lambda parameter name".into(),
                            span: Span::new(pstart, pstart + 1),
                        });
                    }
                    ps.push(LambdaParam {
                        name,
                        span: Span::new(pstart, self.pos),
                    });
                    self.skip_ws();
                    if self.peek() == Some(b',') {
                        self.pos += 1;
                        continue;
                    }
                    break;
                }
            }
            self.expect(b')')?;
            ps
        } else {
            let pstart = self.pos;
            let name = self.read_ident();
            if name.is_empty() {
                return Err(CompileError::Syntax {
                    message: "expected lambda parameter".into(),
                    span: Span::new(pstart, pstart + 1),
                });
            }
            vec![LambdaParam {
                name,
                span: Span::new(pstart, self.pos),
            }]
        };
        self.skip_ws();
        self.expect_str("->")?;
        self.skip_ws();
        let body = self.parse_expr()?;
        let end = body.span.end as usize;
        Ok(Expr {
            kind: ExprKind::Lambda {
                params,
                body: Box::new(body),
            },
            span: Span::new(start, end),
        })
    }

    fn parse_let_binding(&mut self) -> Result<LetBinding, CompileError> {
        let start = self.pos;
        self.pos += "let".len();
        self.skip_ws();
        let name_start = self.pos;
        let name = self.read_ident();
        if name.is_empty() {
            return Err(CompileError::Syntax {
                message: "expected name after 'let'".into(),
                span: Span::new(name_start, name_start + 1),
            });
        }
        self.skip_ws();
        self.expect(b'=')?;
        self.skip_ws();
        let expr = self.parse_expr()?;
        Ok(LetBinding {
            name,
            expr,
            span: Span::new(start, self.pos),
        })
    }

    fn parse_when_body(&mut self, when_start: usize) -> Result<Expr, CompileError> {
        self.skip_ws();
        self.expect(b'{')?;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            let end = self.pos + 1;
            return Err(CompileError::Syntax {
                message: "`when` table must have at least one branch or `else`".into(),
                span: Span::new(when_start, end),
            });
        }
        let mut branches: Vec<WhenBranch> = Vec::new();
        let mut fallback: Option<Box<Expr>> = None;
        loop {
            self.skip_ws();
            if self.matches_ident("else") {
                let else_start = self.pos;
                self.pos += "else".len();
                if fallback.is_some() {
                    return Err(CompileError::Syntax {
                        message: "`when` already has an `else` branch".into(),
                        span: Span::new(else_start, self.pos),
                    });
                }
                self.skip_ws();
                self.expect(b':')?;
                self.skip_ws();
                let result = self.parse_expr()?;
                fallback = Some(Box::new(result));
            } else {
                let cond = self.parse_expr()?;
                self.skip_ws();
                self.expect(b':')?;
                self.skip_ws();
                let result = self.parse_expr()?;
                branches.push(WhenBranch { cond, result });
            }
            self.skip_ws();
            if self.peek() == Some(b',') {
                self.pos += 1;
                self.skip_ws();
                if self.peek() == Some(b'}') {
                    self.pos += 1;
                    break;
                }
                continue;
            }
            if self.peek() == Some(b'}') {
                self.pos += 1;
                break;
            }
            // Branches may also be separated by a line break (DESIGN §2).
            if self.peek().is_some() && self.newline_behind() {
                continue;
            }
            return Err(CompileError::Syntax {
                message: format!(
                    "expected ',' or '}}' in `when` table, found '{}'",
                    self.peek_char().unwrap_or('?')
                ),
                span: self.cur_char_span(),
            });
        }
        let end = self.pos;
        Ok(Expr {
            kind: ExprKind::When { branches, fallback },
            span: Span::new(when_start, end),
        })
    }

    fn read_ident(&mut self) -> SmolStr {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        SmolStr::new(s)
    }

    fn parse_double_quoted_string(&mut self) -> Result<SmolStr, CompileError> {
        let start = self.pos;
        self.expect(b'"')?;
        let content_start = self.pos;
        while let Some(b) = self.peek() {
            if b == b'"' {
                break;
            }
            if b == b'\\' {
                self.pos += 1;
                if self.peek().is_some() {
                    self.pos += 1;
                }
            } else {
                self.pos += 1;
            }
        }
        let content_end = self.pos;
        self.expect(b'"').map_err(|_| CompileError::Syntax {
            message: "unterminated string".into(),
            span: Span::new(start, self.pos),
        })?;
        let raw = std::str::from_utf8(&self.src[content_start..content_end]).unwrap_or("");
        Ok(SmolStr::new(unescape(raw)))
    }

    fn parse_single_quoted_string(&mut self) -> Result<SmolStr, CompileError> {
        let start = self.pos;
        self.expect(b'\'')?;
        let content_start = self.pos;
        while let Some(b) = self.peek() {
            if b == b'\'' {
                break;
            }
            if b == b'\\' {
                self.pos += 1;
                if self.peek().is_some() {
                    self.pos += 1;
                }
            } else {
                self.pos += 1;
            }
        }
        let content_end = self.pos;
        self.expect(b'\'').map_err(|_| CompileError::Syntax {
            message: "unterminated string".into(),
            span: Span::new(start, self.pos),
        })?;
        let raw = std::str::from_utf8(&self.src[content_start..content_end]).unwrap_or("");
        Ok(SmolStr::new(unescape(raw)))
    }

    fn parse_number(&mut self) -> Result<Lit, CompileError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let mut is_decimal = false;
        if self.peek() == Some(b'.') {
            if self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
                is_decimal = true;
                self.pos += 1;
                while let Some(b) = self.peek() {
                    if b.is_ascii_digit() {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            } else {
                return Err(CompileError::Syntax {
                    message: "malformed number: expected a digit after '.'".into(),
                    span: Span::new(start, self.pos + 1),
                });
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        if is_decimal {
            text.parse::<Decimal>()
                .map(Lit::Decimal)
                .map_err(|e| CompileError::Syntax {
                    message: format!("invalid decimal '{text}': {e}"),
                    span: Span::new(start, self.pos),
                })
        } else {
            text.parse::<i64>()
                .map(Lit::Int)
                .map_err(|e| CompileError::Syntax {
                    message: format!("invalid integer '{text}': {e}"),
                    span: Span::new(start, self.pos),
                })
        }
    }

    fn parse_keyword_value(&mut self) -> Result<Lit, CompileError> {
        let start = self.pos;
        let ident = self.read_ident();
        let span = Span::new(start, self.pos);
        match ident.as_str() {
            "true" => Ok(Lit::Bool(true)),
            "false" => Ok(Lit::Bool(false)),
            "null" => Ok(Lit::Null),
            other => Err(CompileError::Syntax {
                message: format!("unknown identifier '{other}'"),
                span,
            }),
        }
    }
}

/// Byte length of the UTF-8 codepoint led by `b`; a stray byte counts as 1.
fn utf8_len(b: u8) -> usize {
    if b < 0xC0 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

fn make_binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    let span = Span {
        start: lhs.span.start,
        end: rhs.span.end,
    };
    Expr {
        kind: ExprKind::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        span,
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                Some('{') => out.push('{'),
                Some('\\') | None => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Inverse of `unescape`, kept beside it so the escape table has one home.
/// Used by the formatter for string literals and object keys.
pub(crate) fn escape(s: &str, quote: char) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
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

/// `escape` for interpolation text: additionally escapes `{` so the re-emitted
/// text can never form a `{{` hole opener.
pub(crate) fn escape_interp_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
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
