use super::scan::ItemEnd;
use super::{Parsed, Parser};
use crate::common::error::{CompileError, Span};
use crate::syntax::ast::{
    Expr, ExprKind, LambdaParam, Lit, LitKey, ObjEntry, PathSegment, WhenBranch,
};

impl<'a> Parser<'a> {
    pub(super) fn parse_primary(&mut self) -> Result<Parsed<Expr>, CompileError> {
        self.skip_ws();
        let start = self.pos;
        let base = self.parse_primary_inner(start)?;
        self.with_postfix(base, start)
    }

    fn with_postfix(
        &mut self,
        base: Parsed<Expr>,
        start: usize,
    ) -> Result<Parsed<Expr>, CompileError> {
        let segments = self.parse_path_segments()?;
        if segments.node.is_empty() {
            return Ok(base);
        }
        let span = Span::new(start, self.pos);
        let expr = Expr {
            kind: ExprKind::Access {
                base: Box::new(base.node),
                segments: segments.node,
            },
            span,
        };
        self.above(expr, base.depth.max(segments.depth), span)
    }

    fn parse_primary_inner(&mut self, start: usize) -> Result<Parsed<Expr>, CompileError> {
        let literal = match self.peek() {
            Some(b'(') => {
                self.pos += 1;
                self.skip_ws();
                let inner = self.parse_expr()?;
                self.skip_ws();
                self.expect(b')')?;
                return Ok(Parsed {
                    node: Expr {
                        kind: inner.node.kind,
                        span: Span::new(start, self.pos),
                    },
                    depth: inner.depth,
                });
            }
            Some(quote @ (b'\'' | b'"')) => Lit::Str(self.parse_quoted_string(quote)?),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number()?,
            Some(b'[') => return self.parse_array_literal(start),
            Some(b'{') => return self.parse_object_literal(start),
            Some(byte) if byte.is_ascii_alphabetic() => return self.parse_word(start),
            Some(_) => {
                return Err(CompileError::Syntax {
                    message: format!(
                        "unexpected `{}` in expression",
                        self.peek_char().unwrap_or('?')
                    ),
                    span: self.cur_char_span(),
                })
            }
            None => {
                return Err(CompileError::Syntax {
                    message: "expected expression, found end of input".into(),
                    span: Span::new(self.pos, self.pos),
                })
            }
        };
        Ok(Parsed::leaf(Expr {
            kind: ExprKind::Literal(literal),
            span: Span::new(start, self.pos),
        }))
    }

    fn parse_array_literal(&mut self, start: usize) -> Result<Parsed<Expr>, CompileError> {
        self.pos += 1;
        let mut items: Vec<Expr> = Vec::new();
        let mut deepest = 0;
        self.skip_ws();
        if self.peek() != Some(b']') {
            loop {
                self.skip_ws();
                let item = self.parse_expr()?;
                items.push(item.node);
                deepest = deepest.max(item.depth);
                if matches!(self.expr_item_sep(b']'), ItemEnd::Closed) {
                    break;
                }
            }
        }
        self.expect(b']')?;
        let span = Span::new(start, self.pos);
        let expr = Expr {
            kind: ExprKind::ArrayLit(items),
            span,
        };
        self.above(expr, deepest, span)
    }

    fn parse_object_literal(&mut self, start: usize) -> Result<Parsed<Expr>, CompileError> {
        self.pos += 1;
        let mut entries: Vec<ObjEntry> = Vec::new();
        let mut deepest = 0;
        self.skip_ws();
        if self.peek() != Some(b'}') {
            loop {
                self.skip_ws();
                let key = if self.peek() == Some(b'[') {
                    self.pos += 1;
                    self.skip_ws();
                    let key_expr = self.parse_expr()?;
                    self.skip_ws();
                    self.expect(b']')?;
                    deepest = deepest.max(key_expr.depth);
                    LitKey::Computed(key_expr.node)
                } else {
                    LitKey::Static(self.parse_quoted_string(b'"')?)
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
                deepest = deepest.max(value.depth);
                entries.push(ObjEntry {
                    key,
                    value: value.node,
                    optional,
                });
                if matches!(self.expr_item_sep(b'}'), ItemEnd::Closed) {
                    break;
                }
            }
        }
        self.expect(b'}')?;
        let span = Span::new(start, self.pos);
        let expr = Expr {
            kind: ExprKind::ObjectLit(entries),
            span,
        };
        self.above(expr, deepest, span)
    }

    /// `Closed` leaves the closer to the caller.
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

    fn parse_word(&mut self, start: usize) -> Result<Parsed<Expr>, CompileError> {
        let ident = self.read_ident();
        let ident_span = Span::new(start, self.pos);
        let literal = match ident.as_str() {
            "true" => Some(Lit::Bool(true)),
            "false" => Some(Lit::Bool(false)),
            "null" => Some(Lit::Null),
            _ => None,
        };
        if let Some(literal) = literal {
            return Ok(Parsed::leaf(Expr {
                kind: ExprKind::Literal(literal),
                span: ident_span,
            }));
        }
        if ident == "when" {
            return self.parse_when(start);
        }
        if self.peek() == Some(b'(') {
            let args = self.parse_args()?;
            let span = Span::new(start, self.pos);
            let expr = Expr {
                kind: ExprKind::FuncCall {
                    name: ident,
                    name_span: ident_span,
                    args: args.node,
                },
                span,
            };
            return self.above(expr, args.depth, span);
        }
        let segments = self.parse_path_segments()?;
        let span = Span::new(start, self.pos);
        let expr = Expr {
            kind: ExprKind::Path {
                root: ident,
                root_span: ident_span,
                segments: segments.node,
            },
            span,
        };
        self.above(expr, segments.depth, span)
    }

    /// Depth 0 when no expression is inside.
    fn parse_path_segments(&mut self) -> Result<Parsed<Vec<PathSegment>>, CompileError> {
        let mut segments: Vec<PathSegment> = Vec::new();
        let mut deepest = 0;
        loop {
            if self.peek() == Some(b'[') {
                let index_start = self.pos;
                self.pos += 1;
                self.skip_ws();
                let index = self.parse_expr()?;
                self.skip_ws();
                self.expect(b']')?;
                segments.push(PathSegment::Index {
                    expr: Box::new(index.node),
                    span: Span::new(index_start, self.pos),
                });
                deepest = deepest.max(index.depth);
                continue;
            }
            let before_dot = self.pos;
            self.skip_ws();
            let optional = self.matches("?.");
            let dot_len = if optional { 2 } else { 1 };
            let is_field = (optional || self.peek() == Some(b'.'))
                && self
                    .peek_at(dot_len)
                    .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_');
            if !is_field {
                self.pos = before_dot;
                break;
            }
            self.pos += dot_len;
            let name_start = self.pos;
            let name = self.read_ident();
            let span = Span::new(name_start, self.pos);
            if !optional && self.peek() == Some(b'(') {
                let args = self.parse_args()?;
                segments.push(PathSegment::Method {
                    name,
                    span,
                    args: args.node,
                });
                deepest = deepest.max(args.depth);
            } else {
                segments.push(PathSegment::Field {
                    name,
                    span,
                    optional,
                });
            }
        }
        Ok(Parsed {
            node: segments,
            depth: deepest,
        })
    }

    fn parse_args(&mut self) -> Result<Parsed<Vec<Expr>>, CompileError> {
        self.expect(b'(')?;
        let mut args = Vec::new();
        let mut deepest = 0;
        self.skip_ws();
        if self.peek() != Some(b')') {
            loop {
                self.skip_ws();
                let arg = self.parse_arg()?;
                args.push(arg.node);
                deepest = deepest.max(arg.depth);
                self.skip_ws();
                if self.peek() != Some(b',') {
                    break;
                }
                self.pos += 1;
                self.skip_ws();
                if self.peek() == Some(b')') {
                    break;
                }
            }
        }
        self.expect(b')')?;
        Ok(Parsed {
            node: args,
            depth: deepest,
        })
    }

    fn parse_arg(&mut self) -> Result<Parsed<Expr>, CompileError> {
        if self.lambda_follows() {
            self.parse_lambda()
        } else {
            self.parse_expr()
        }
    }

    fn lambda_follows(&self) -> bool {
        let mut at = self.skip_ws_at(self.pos);
        let Some(&first) = self.src.get(at) else {
            return false;
        };
        if first == b'(' {
            at = self.skip_ws_at(at + 1);
            if self.src.get(at) == Some(&b')') {
                return self.arrow_at(self.skip_ws_at(at + 1));
            }
            loop {
                let ident_end = self.skip_ident_at(at);
                if ident_end == at {
                    return false;
                }
                at = self.skip_ws_at(ident_end);
                match self.src.get(at) {
                    Some(b',') => at = self.skip_ws_at(at + 1),
                    Some(b')') => return self.arrow_at(self.skip_ws_at(at + 1)),
                    _ => return false,
                }
            }
        }
        if first.is_ascii_alphabetic() || first == b'_' {
            let ident_end = self.skip_ident_at(at);
            return self.arrow_at(self.skip_ws_at(ident_end));
        }
        false
    }

    fn arrow_at(&self, at: usize) -> bool {
        self.src.get(at) == Some(&b'-') && self.src.get(at + 1) == Some(&b'>')
    }

    fn parse_lambda(&mut self) -> Result<Parsed<Expr>, CompileError> {
        let start = self.pos;
        self.skip_ws();
        let params = if self.peek() == Some(b'(') {
            self.pos += 1;
            let mut params: Vec<LambdaParam> = Vec::new();
            self.skip_ws();
            if self.peek() != Some(b')') {
                loop {
                    self.skip_ws();
                    params.push(self.parse_lambda_param("expected lambda parameter name")?);
                    self.skip_ws();
                    if self.peek() != Some(b',') {
                        break;
                    }
                    self.pos += 1;
                }
            }
            self.expect(b')')?;
            params
        } else {
            vec![self.parse_lambda_param("expected lambda parameter")?]
        };
        self.skip_ws();
        self.expect_str("->")?;
        self.skip_ws();
        let body = self.parse_expr()?;
        let span = Span::new(start, body.node.span.range().end);
        let expr = Expr {
            kind: ExprKind::Lambda {
                params,
                body: Box::new(body.node),
            },
            span,
        };
        self.above(expr, body.depth, span)
    }

    fn parse_lambda_param(&mut self, missing: &str) -> Result<LambdaParam, CompileError> {
        let start = self.pos;
        let name = self.read_ident();
        if name.is_empty() {
            return Err(CompileError::Syntax {
                message: missing.into(),
                span: Span::new(start, start + 1),
            });
        }
        Ok(LambdaParam {
            name,
            span: Span::new(start, self.pos),
        })
    }

    fn parse_when(&mut self, when_start: usize) -> Result<Parsed<Expr>, CompileError> {
        self.skip_ws();
        self.expect(b'{')?;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            return Err(CompileError::Syntax {
                message: "`when` table must have at least one branch or `else`".into(),
                span: Span::new(when_start, self.pos + 1),
            });
        }
        let mut branches: Vec<WhenBranch> = Vec::new();
        let mut fallback: Option<Box<Expr>> = None;
        let mut deepest = 0;
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
                fallback = Some(Box::new(result.node));
                deepest = deepest.max(result.depth);
            } else {
                let cond = self.parse_expr()?;
                self.skip_ws();
                self.expect(b':')?;
                self.skip_ws();
                let result = self.parse_expr()?;
                branches.push(WhenBranch {
                    cond: cond.node,
                    result: result.node,
                });
                deepest = deepest.max(cond.depth).max(result.depth);
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
            // branches may also be separated by a line break
            if self.peek().is_some() && self.newline_behind() {
                continue;
            }
            return Err(CompileError::Syntax {
                message: format!(
                    "expected `,` or `}}` in `when` table, found `{}`",
                    self.peek_char().unwrap_or('?')
                ),
                span: self.cur_char_span(),
            });
        }
        let span = Span::new(when_start, self.pos);
        let expr = Expr {
            kind: ExprKind::When { branches, fallback },
            span,
        };
        self.above(expr, deepest, span)
    }
}
