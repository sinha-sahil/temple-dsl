use super::Parser;
use crate::common::error::{CompileError, Span};
use crate::syntax::ast::Lit;
use rust_decimal::Decimal;
use smol_str::SmolStr;

pub(super) enum ItemEnd {
    More,
    Closed,
}

impl<'a> Parser<'a> {
    pub(super) fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    pub(super) fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    pub(super) fn peek_char(&self) -> Option<char> {
        let first = self.peek()?;
        let len = utf8_len(first);
        std::str::from_utf8(self.src.get(self.pos..self.pos + len)?)
            .ok()?
            .chars()
            .next()
    }

    /// Span of the char at the cursor, so error spans never split a char.
    pub(super) fn cur_char_span(&self) -> Span {
        let width = self.peek().map_or(0, utf8_len);
        Span::new(self.pos, self.pos + width)
    }

    pub(super) fn skip_ws(&mut self) {
        if self.pos > self.ws_end {
            self.tok_end = self.pos;
        }
        self.pos = self.skip_ws_at(self.pos);
        self.ws_end = self.pos;
    }

    pub(super) fn skip_ws_at(&self, from: usize) -> usize {
        skip_trivia(self.src, from)
    }

    pub(super) fn skip_ident_at(&self, from: usize) -> usize {
        let mut at = from;
        while at < self.src.len() && (self.src[at].is_ascii_alphanumeric() || self.src[at] == b'_')
        {
            at += 1;
        }
        at
    }

    pub(super) fn skip_to_line_end(&mut self) {
        while let Some(byte) = self.peek() {
            self.pos += 1;
            if byte == b'\n' {
                break;
            }
        }
    }

    pub(super) fn matches(&self, text: &str) -> bool {
        self.src[self.pos..].starts_with(text.as_bytes())
    }

    /// `ident` at the cursor, and not merely the start of a longer word.
    pub(super) fn matches_ident(&self, ident: &str) -> bool {
        if !self.matches(ident) {
            return false;
        }
        let next = self.src.get(self.pos + ident.len()).copied();
        !matches!(next, Some(byte) if byte.is_ascii_alphanumeric() || byte == b'_')
    }

    pub(super) fn expect(&mut self, byte: u8) -> Result<(), CompileError> {
        match self.peek() {
            Some(found) if found == byte => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(CompileError::Syntax {
                message: format!("expected `{}`", byte as char),
                span: self.cur_char_span(),
            }),
        }
    }

    pub(super) fn expect_str(&mut self, text: &str) -> Result<(), CompileError> {
        if self.matches(text) {
            self.pos += text.len();
            Ok(())
        } else {
            Err(CompileError::Syntax {
                message: format!("expected `{text}`"),
                span: Span::new(self.pos, self.pos + text.len()),
            })
        }
    }

    /// Whether a line break separates the cursor from the previous token (it
    /// can stand in for a comma).
    pub(super) fn newline_behind(&self) -> bool {
        let mut at = self.pos;
        while at > 0 {
            let byte = self.src[at - 1];
            if byte == b'\n' {
                return true;
            }
            if !byte.is_ascii_whitespace() {
                return false;
            }
            at -= 1;
        }
        false
    }

    pub(super) fn read_ident(&mut self) -> SmolStr {
        let start = self.pos;
        self.pos = self.skip_ident_at(self.pos);
        // identifiers are ASCII, so this slice is always valid text
        SmolStr::new(std::str::from_utf8(&self.src[start..self.pos]).unwrap_or(""))
    }

    pub(super) fn parse_quoted_string(&mut self, quote: u8) -> Result<SmolStr, CompileError> {
        let start = self.pos;
        self.expect(quote)?;
        let content_start = self.pos;
        while let Some(byte) = self.peek() {
            if byte == quote {
                break;
            }
            self.pos += 1;
            if byte == b'\\' && self.peek().is_some() {
                self.pos += 1;
            }
        }
        let content_end = self.pos;
        self.expect(quote).map_err(|_| CompileError::Syntax {
            message: "unterminated string".into(),
            span: Span::new(start, self.pos),
        })?;
        let raw = std::str::from_utf8(&self.src[content_start..content_end]).unwrap_or("");
        Ok(SmolStr::new(unescape(raw)))
    }

    pub(super) fn parse_number(&mut self) -> Result<Lit, CompileError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        self.skip_digits();
        let mut is_decimal = false;
        if self.peek() == Some(b'.') {
            if !self.peek_at(1).is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(CompileError::Syntax {
                    message: "malformed number: expected a digit after `.`".into(),
                    span: Span::new(start, self.pos + 1),
                });
            }
            is_decimal = true;
            self.pos += 1;
            self.skip_digits();
        }
        // digits, `-` and `.` only, so this slice is always valid text
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        let span = Span::new(start, self.pos);
        if is_decimal {
            text.parse::<Decimal>()
                .map(Lit::Decimal)
                .map_err(|error| CompileError::Syntax {
                    message: format!("invalid decimal `{text}`: {error}"),
                    span,
                })
        } else {
            text.parse::<i64>()
                .map(Lit::Int)
                .map_err(|error| CompileError::Syntax {
                    message: format!("invalid integer `{text}`: {error}"),
                    span,
                })
        }
    }

    fn skip_digits(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    pub(super) fn parse_keyword_value(&mut self) -> Result<Lit, CompileError> {
        let start = self.pos;
        let ident = self.read_ident();
        match ident.as_str() {
            "true" => Ok(Lit::Bool(true)),
            "false" => Ok(Lit::Bool(false)),
            "null" => Ok(Lit::Null),
            other => Err(CompileError::Syntax {
                message: format!("unknown identifier `{other}`"),
                span: Span::new(start, self.pos),
            }),
        }
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0xBF => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

pub(super) fn skip_trivia(src: &[u8], from: usize) -> usize {
    let mut at = from;
    while at < src.len() {
        if src[at].is_ascii_whitespace() {
            at += 1;
        } else if src[at] == b'#' {
            while at < src.len() && src[at] != b'\n' {
                at += 1;
            }
        } else {
            break;
        }
    }
    at
}

/// `\{` lets text hold `{{`; an unknown escape stays as written.
pub(super) fn escaped(c: u8) -> Option<u8> {
    match c {
        b'n' => Some(b'\n'),
        b't' => Some(b'\t'),
        b'r' => Some(b'\r'),
        b'"' | b'\'' | b'{' | b'\\' => Some(c),
        _ => None,
    }
}

fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            None => out.push('\\'),
            Some(next) => match u8::try_from(next).ok().and_then(escaped) {
                Some(byte) => out.push(char::from(byte)),
                None => {
                    out.push('\\');
                    out.push(next);
                }
            },
        }
    }
    out
}
