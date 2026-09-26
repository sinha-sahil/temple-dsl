use super::scan::{escaped, ItemEnd};
use super::{Parsed, Parser};
use crate::common::error::{CompileError, Span};
use crate::syntax::ast::{Expr, ExprKind, InterpPart, Lit, ObjField, OutKind, OutNode};
use smol_str::SmolStr;

impl<'a> Parser<'a> {
    pub(super) fn parse_output(&mut self) -> Result<Parsed<OutNode>, CompileError> {
        self.nested(Self::parse_output_inner)
    }

    fn parse_output_inner(&mut self) -> Result<Parsed<OutNode>, CompileError> {
        self.skip_ws();
        let start = self.pos;
        if self.matches("{{") {
            return self.parse_hole();
        }
        let literal = match self.peek() {
            Some(b'{') => return self.parse_object(),
            Some(b'[') => return self.parse_array(),
            Some(b'"') => return self.parse_interp_string(start),
            Some(b'\'') => Lit::Str(self.parse_quoted_string(b'\'')?),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number()?,
            Some(byte) if byte.is_ascii_alphabetic() => self.parse_keyword_value()?,
            Some(_) => {
                return Err(CompileError::Syntax {
                    message: format!("unexpected `{}`", self.peek_char().unwrap_or('?')),
                    span: self.cur_char_span(),
                })
            }
            None => {
                return Err(CompileError::Syntax {
                    message: "unexpected end of input".into(),
                    span: Span::new(self.pos, self.pos),
                })
            }
        };
        Ok(Parsed::leaf(OutNode {
            kind: OutKind::Literal(literal),
            span: Span::new(start, self.pos),
        }))
    }

    fn parse_object(&mut self) -> Result<Parsed<OutNode>, CompileError> {
        let start = self.pos;
        self.expect(b'{')?;
        let fields = self.parse_items(b'}', start, Self::parse_object_field);
        let span = Span::new(start, self.pos);
        let node = OutNode {
            kind: OutKind::Object(fields.node),
            span,
        };
        self.above(node, fields.depth, span)
    }

    fn parse_object_field(&mut self) -> Result<Parsed<ObjField>, CompileError> {
        self.skip_ws();
        let key = self.parse_quoted_string(b'"')?;
        self.skip_ws();
        let optional = self.peek() == Some(b'?');
        if optional {
            self.pos += 1;
            self.skip_ws();
        }
        self.expect(b':')?;
        self.skip_ws();
        let value = self.parse_output()?;
        Ok(Parsed {
            node: ObjField {
                key,
                value: value.node,
                optional,
            },
            depth: value.depth,
        })
    }

    fn parse_array(&mut self) -> Result<Parsed<OutNode>, CompileError> {
        let start = self.pos;
        self.expect(b'[')?;
        let items = self.parse_items(b']', start, Self::parse_output);
        let span = Span::new(start, self.pos);
        let node = OutNode {
            kind: OutKind::Array(items.node),
            span,
        };
        self.above(node, items.depth, span)
    }

    fn parse_items<T>(
        &mut self,
        closer: u8,
        start: usize,
        mut parse_item: impl FnMut(&mut Self) -> Result<Parsed<T>, CompileError>,
    ) -> Parsed<Vec<T>> {
        let mut items = Vec::new();
        let mut deepest = 0;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(byte) if byte == closer => {
                    self.pos += 1;
                    break;
                }
                None => {
                    self.errors.push(CompileError::Syntax {
                        message: format!("unterminated, expected `{}`", closer as char),
                        span: Span::new(start, self.pos),
                    });
                    break;
                }
                _ => {}
            }
            match parse_item(self) {
                Ok(item) => {
                    items.push(item.node);
                    deepest = deepest.max(item.depth);
                }
                Err(error) => {
                    self.errors.push(error);
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
        Parsed {
            node: items,
            depth: deepest,
        }
    }

    fn after_item(&mut self, closer: u8) -> ItemEnd {
        self.skip_ws();
        match self.peek() {
            Some(b',') => {
                self.pos += 1;
                ItemEnd::More
            }
            Some(byte) if byte == closer => {
                self.pos += 1;
                ItemEnd::Closed
            }
            None => {
                self.errors.push(CompileError::Syntax {
                    message: format!("expected `{}`", closer as char),
                    span: Span::new(self.pos, self.pos),
                });
                ItemEnd::Closed
            }
            Some(_) if self.newline_behind() => ItemEnd::More,
            Some(_) => {
                self.errors.push(CompileError::Syntax {
                    message: format!(
                        "expected `,` or `{}`, found `{}`",
                        closer as char,
                        self.peek_char().unwrap_or('?')
                    ),
                    span: self.cur_char_span(),
                });
                self.recover_item(closer)
            }
        }
    }

    fn recover_item(&mut self, closer: u8) -> ItemEnd {
        if self.skip_to_item_boundary() {
            self.pos += 1;
            ItemEnd::More
        } else {
            if self.peek() == Some(closer) {
                self.pos += 1;
            }
            ItemEnd::Closed
        }
    }

    /// Skip a bad item to the next top-level `,` or line break (true, cursor on
    /// it) or to the closer or end (false).
    fn skip_to_item_boundary(&mut self) -> bool {
        let mut depth: i32 = 0;
        while let Some(byte) = self.peek() {
            match byte {
                // a `{{ … }}` hole is skipped whole, so its `}}` is never
                // mistaken for the collection's closing bracket
                b'{' if self.peek_at(1) == Some(b'{') => {
                    self.pos += 2;
                    self.skip_to_hole_close();
                }
                b'"' | b'\'' => self.skip_string_lenient(byte),
                // stop before the comment's line break, which is a boundary
                b'#' => {
                    while self.peek().is_some_and(|byte| byte != b'\n') {
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
        while let Some(byte) = self.peek() {
            self.pos += 1;
            if byte == b'\\' {
                if self.peek().is_some() {
                    self.pos += 1;
                }
            } else if byte == quote {
                break;
            }
        }
    }

    fn parse_interp_string(&mut self, start: usize) -> Result<Parsed<OutNode>, CompileError> {
        self.expect(b'"')?;
        let mut parts: Vec<InterpPart> = Vec::new();
        let mut text: Vec<u8> = Vec::new();
        let mut deepest = 0;
        let flush = |text: &mut Vec<u8>, parts: &mut Vec<InterpPart>| {
            if !text.is_empty() {
                let text = String::from_utf8(std::mem::take(text)).unwrap_or_default();
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
                        Some(next) => {
                            match escaped(next) {
                                Some(byte) => text.push(byte),
                                None => text.extend_from_slice(&[b'\\', next]),
                            }
                            self.pos += 1;
                        }
                        None => text.push(b'\\'),
                    }
                }
                Some(b'{') if self.peek_at(1) == Some(b'{') => {
                    flush(&mut text, &mut parts);
                    self.pos += 2;
                    self.skip_ws();
                    let hole = self.parse_expr()?;
                    self.skip_ws();
                    self.expect_str("}}")?;
                    parts.push(InterpPart::Hole(hole.node));
                    deepest = deepest.max(hole.depth);
                }
                Some(byte) => {
                    text.push(byte);
                    self.pos += 1;
                }
            }
        }
        flush(&mut text, &mut parts);
        let span = Span::new(start, self.pos);
        if parts.iter().any(|part| matches!(part, InterpPart::Hole(_))) {
            let node = OutNode {
                kind: OutKind::Interp(parts),
                span,
            };
            return self.above(node, deepest, span);
        }
        let mut whole = String::new();
        for part in &parts {
            if let InterpPart::Text(text) = part {
                whole.push_str(text);
            }
        }
        Ok(Parsed::leaf(OutNode {
            kind: OutKind::Literal(Lit::Str(SmolStr::new(whole))),
            span,
        }))
    }

    fn parse_hole(&mut self) -> Result<Parsed<OutNode>, CompileError> {
        let start = self.pos;
        self.expect_str("{{")?;
        self.skip_ws();
        // A bad hole recovers here, at its own `}}`. Bubbling up would leave
        // the cursor before the `}}`, which the surrounding object or array
        // would take for its own closing bracket.
        match self.parse_expr() {
            Ok(hole) => {
                self.skip_ws();
                if let Err(error) = self.expect_str("}}") {
                    self.errors.push(error);
                    self.skip_to_hole_close();
                }
                let span = Span::new(start, self.pos);
                let node = OutNode {
                    kind: OutKind::Hole(hole.node),
                    span,
                };
                self.above(node, hole.depth, span)
            }
            Err(error) => {
                self.errors.push(error);
                self.skip_to_hole_close();
                let span = Span::new(start, self.pos);
                let placeholder = Expr {
                    kind: ExprKind::Literal(Lit::Null),
                    span,
                };
                let node = OutNode {
                    kind: OutKind::Hole(placeholder),
                    span,
                };
                self.above(node, 1, span)
            }
        }
    }

    /// Skip to the hole's closing `}}`; the cursor must be past its `{{`.
    fn skip_to_hole_close(&mut self) {
        let mut depth: i32 = 0;
        while let Some(byte) = self.peek() {
            match byte {
                b'"' | b'\'' => self.skip_string_lenient(byte),
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
}
