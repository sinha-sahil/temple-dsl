use crate::error::{CompileError, Span};
use crate::value::Value;
use rust_decimal::Decimal;
use smol_str::SmolStr;

#[derive(Debug, Clone)]
pub struct OutNode {
    pub kind: OutKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum OutKind {
    Literal(Value),
    Hole(Expr),
    Object(Vec<(SmolStr, OutNode)>),
    Array(Vec<OutNode>),
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Literal(Value),
    Path {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub struct WhenBranch {
    pub cond: Expr,
    pub result: Expr,
}

#[derive(Debug, Clone)]
pub struct PathSegment {
    pub name: SmolStr,
    pub span: Span,
    pub optional: bool,
}

pub fn parse(src: &str) -> Result<OutNode, Vec<CompileError>> {
    let mut p = Parser::new(src);
    p.skip_ws();
    let node = match p.parse_output() {
        Ok(n) => n,
        Err(e) => return Err(vec![e]),
    };
    p.skip_ws();
    if p.pos < p.src.len() {
        return Err(vec![CompileError::Syntax {
            message: format!(
                "expected end of input, found '{}'",
                p.peek_char().unwrap_or('?')
            ),
            span: Span::new(p.pos, p.pos + 1),
        }]);
    }
    Ok(node)
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_char(&self) -> Option<char> {
        self.peek().map(|b| b as char)
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

    fn expect(&mut self, byte: u8) -> Result<(), CompileError> {
        match self.peek() {
            Some(b) if b == byte => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(CompileError::Syntax {
                message: format!("expected '{}'", byte as char),
                span: Span::new(self.pos, self.pos.saturating_add(1)),
            }),
        }
    }

    fn expect_str(&mut self, s: &str) -> Result<(), CompileError> {
        if self.src[self.pos..].starts_with(s.as_bytes()) {
            self.pos += s.len();
            Ok(())
        } else {
            Err(CompileError::Syntax {
                message: format!("expected '{}'", s),
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
        self.skip_ws();
        let start = self.pos;

        if self.matches("{{") {
            return self.parse_hole();
        }

        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => {
                let v = self.parse_double_quoted_string()?;
                Ok(OutNode {
                    kind: OutKind::Literal(Value::Str(v)),
                    span: Span::new(start, self.pos),
                })
            }
            Some(b'\'') => {
                let v = self.parse_single_quoted_string()?;
                Ok(OutNode {
                    kind: OutKind::Literal(Value::Str(v)),
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
            Some(c) => Err(CompileError::Syntax {
                message: format!("unexpected '{}'", c as char),
                span: Span::new(self.pos, self.pos + 1),
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
        self.skip_ws();
        let mut entries: Vec<(SmolStr, OutNode)> = Vec::new();
        if self.peek() != Some(b'}') {
            loop {
                self.skip_ws();
                let key = self.parse_double_quoted_string()?;
                self.skip_ws();
                self.expect(b':')?;
                self.skip_ws();
                let value = self.parse_output()?;
                entries.push((key, value));
                self.skip_ws();
                if self.peek() == Some(b',') {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(b'}')?;
        Ok(OutNode {
            kind: OutKind::Object(entries),
            span: Span::new(start, self.pos),
        })
    }

    fn parse_array(&mut self) -> Result<OutNode, CompileError> {
        let start = self.pos;
        self.expect(b'[')?;
        self.skip_ws();
        let mut items: Vec<OutNode> = Vec::new();
        if self.peek() != Some(b']') {
            loop {
                self.skip_ws();
                items.push(self.parse_output()?);
                self.skip_ws();
                if self.peek() == Some(b',') {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(b']')?;
        Ok(OutNode {
            kind: OutKind::Array(items),
            span: Span::new(start, self.pos),
        })
    }

    fn parse_hole(&mut self) -> Result<OutNode, CompileError> {
        let start = self.pos;
        self.expect_str("{{")?;
        self.skip_ws();
        let expr = self.parse_expr()?;
        self.skip_ws();
        self.expect_str("}}")?;
        Ok(OutNode {
            kind: OutKind::Hole(expr),
            span: Span::new(start, self.pos),
        })
    }

    fn parse_expr(&mut self) -> Result<Expr, CompileError> {
        self.parse_ternary()
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
                _ => break,
            };
            self.skip_ws();
            let rhs = self.parse_unary()?;
            lhs = make_binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, CompileError> {
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
                || (next == Some(b'.')
                    && self.peek_at(2).is_some_and(|b| b.is_ascii_digit()));
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
                    kind: ExprKind::Literal(Value::Str(s)),
                    span: Span::new(start, self.pos),
                })
            }
            Some(b'"') => {
                let s = self.parse_double_quoted_string()?;
                Ok(Expr {
                    kind: ExprKind::Literal(Value::Str(s)),
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
            Some(c) if c.is_ascii_alphabetic() => {
                let ident_start = self.pos;
                let ident = self.read_ident();
                let ident_span = Span::new(ident_start, self.pos);
                match ident.as_str() {
                    "true" => Ok(Expr {
                        kind: ExprKind::Literal(Value::Bool(true)),
                        span: ident_span,
                    }),
                    "false" => Ok(Expr {
                        kind: ExprKind::Literal(Value::Bool(false)),
                        span: ident_span,
                    }),
                    "null" => Ok(Expr {
                        kind: ExprKind::Literal(Value::Null),
                        span: ident_span,
                    }),
                    "when" => self.parse_when_body(ident_start),
                    "input" => {
                        let mut segments = vec![PathSegment {
                            name: ident,
                            span: ident_span,
                            optional: false,
                        }];
                        loop {
                            self.skip_ws();
                            let (advance, optional) = if self.matches("?.")
                                && self
                                    .peek_at(2)
                                    .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                            {
                                (2, true)
                            } else if self.peek() == Some(b'.')
                                && self
                                    .peek_at(1)
                                    .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                            {
                                (1, false)
                            } else {
                                break;
                            };
                            self.pos += advance;
                            let seg_start = self.pos;
                            let name = self.read_ident();
                            segments.push(PathSegment {
                                name,
                                span: Span::new(seg_start, self.pos),
                                optional,
                            });
                        }
                        Ok(Expr {
                            kind: ExprKind::Path { segments },
                            span: Span::new(ident_start, self.pos),
                        })
                    }
                    other => Err(CompileError::Syntax {
                        message: format!("unknown identifier '{}'", other),
                        span: ident_span,
                    }),
                }
            }
            Some(c) => Err(CompileError::Syntax {
                message: format!("unexpected '{}' in expression", c as char),
                span: Span::new(self.pos, self.pos + 1),
            }),
            None => Err(CompileError::Syntax {
                message: "expected expression, found end of input".into(),
                span: Span::new(self.pos, self.pos),
            }),
        }
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
            return Err(CompileError::Syntax {
                message: format!(
                    "expected ',' or '}}' in `when` table, found '{}'",
                    self.peek_char().unwrap_or('?')
                ),
                span: Span::new(self.pos, self.pos + 1),
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

    fn parse_number(&mut self) -> Result<Value, CompileError> {
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
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        if is_decimal {
            text.parse::<Decimal>()
                .map(Value::Decimal)
                .map_err(|e| CompileError::Syntax {
                    message: format!("invalid decimal '{}': {}", text, e),
                    span: Span::new(start, self.pos),
                })
        } else {
            text.parse::<i64>()
                .map(Value::Int)
                .map_err(|e| CompileError::Syntax {
                    message: format!("invalid integer '{}': {}", text, e),
                    span: Span::new(start, self.pos),
                })
        }
    }

    fn parse_keyword_value(&mut self) -> Result<Value, CompileError> {
        let start = self.pos;
        let ident = self.read_ident();
        let span = Span::new(start, self.pos);
        match ident.as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            "null" => Ok(Value::Null),
            other => Err(CompileError::Syntax {
                message: format!("unknown identifier '{}'", other),
                span,
            }),
        }
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
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
