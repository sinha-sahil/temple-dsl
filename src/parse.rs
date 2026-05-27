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
    Path { segments: Vec<PathSegment> },
}

#[derive(Debug, Clone)]
pub struct PathSegment {
    pub name: SmolStr,
    pub span: Span,
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
        let start = self.pos;
        match self.peek() {
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
                    "input" => {
                        let mut segments = vec![PathSegment {
                            name: ident,
                            span: ident_span,
                        }];
                        loop {
                            self.skip_ws();
                            if self.peek() != Some(b'.') {
                                break;
                            }
                            self.pos += 1;
                            self.skip_ws();
                            let seg_start = self.pos;
                            let name = self.read_ident();
                            if name.is_empty() {
                                return Err(CompileError::Syntax {
                                    message: "expected identifier after '.'".into(),
                                    span: Span::new(self.pos, self.pos + 1),
                                });
                            }
                            segments.push(PathSegment {
                                name,
                                span: Span::new(seg_start, self.pos),
                            });
                        }
                        Ok(Expr {
                            kind: ExprKind::Path { segments },
                            span: Span::new(start, self.pos),
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
            // Only a decimal point if a digit follows (else it's a path `.field`).
            if self.src.get(self.pos + 1).is_some_and(|b| b.is_ascii_digit()) {
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
