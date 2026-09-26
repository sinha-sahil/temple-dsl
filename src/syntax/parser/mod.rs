//! Recursive-descent parser over bytes. Two limits keep every later pass
//! safe: recursion stops at `MAX_NESTING`, and no tree deeper than
//! `MAX_TREE_DEPTH` (`MAX_UNIT_DEPTH` for an embedded unit) is built. Every
//! parse fn returns its node's depth (`Parsed`), so nothing is walked to
//! measure it.

mod expr;
mod output;
mod primary;
mod scan;

use crate::common::error::{CompileError, Span};
use crate::common::limits::{MAX_NESTING, MAX_TREE_DEPTH, MAX_UNIT_DEPTH};
use crate::syntax::ast::{Expr, Module, OutNode};

pub(crate) struct ParsedUnit<T> {
    pub node: T,
    /// Byte offset just past the unit's last token.
    pub end: usize,
}

pub(crate) fn parse_module(src: &str) -> Result<Module, Vec<CompileError>> {
    let mut parser = Parser::new(src);
    let mut lets = Vec::new();
    loop {
        parser.skip_ws();
        if !parser.matches_ident("let") {
            break;
        }
        match parser.parse_let_binding() {
            Ok(binding) => lets.push(binding),
            // Lets are line-oriented: skip to the next line and keep going, so
            // one bad line doesn't hide the errors after it.
            Err(error) => {
                parser.errors.push(error);
                parser.skip_to_line_end();
            }
        }
    }
    let output = match parser.parse_output() {
        Ok(parsed) => parsed.node,
        Err(error) => {
            parser.errors.push(error);
            return Err(parser.errors);
        }
    };
    parser.skip_ws();
    // Trailing text is only reported on an otherwise clean parse: after a
    // recovery it is expected leftovers, not a separate mistake.
    if parser.pos < parser.src.len() && parser.errors.is_empty() {
        return Err(vec![CompileError::Syntax {
            message: format!(
                "expected end of input, found `{}`",
                parser.peek_char().unwrap_or('?')
            ),
            span: parser.cur_char_span(),
        }]);
    }
    if parser.errors.is_empty() {
        Ok(Module { lets, output })
    } else {
        Err(parser.errors)
    }
}

pub(crate) fn parse_expr_at(
    src: &str,
    start: usize,
) -> Result<ParsedUnit<Expr>, Vec<CompileError>> {
    parse_unit_at(src, start, Parser::parse_expr)
}

pub(crate) fn parse_output_at(
    src: &str,
    start: usize,
) -> Result<ParsedUnit<OutNode>, Vec<CompileError>> {
    parse_unit_at(src, start, Parser::parse_output)
}

pub(crate) fn expect_end(src: &str, end: usize, what: &str) -> Result<(), Vec<CompileError>> {
    let rest = scan::skip_trivia(src.as_bytes(), end);
    if rest < src.len() {
        return Err(vec![CompileError::Syntax {
            message: format!("unexpected text after the {what}"),
            span: Span::new(rest, src.len()),
        }]);
    }
    Ok(())
}

fn parse_unit_at<'a, T>(
    src: &'a str,
    start: usize,
    parse: impl FnOnce(&mut Parser<'a>) -> Result<Parsed<T>, CompileError>,
) -> Result<ParsedUnit<T>, Vec<CompileError>> {
    let mut parser = Parser::at(src, start)?;
    match parse(&mut parser) {
        Ok(parsed) if parser.errors.is_empty() => Ok(ParsedUnit {
            node: parsed.node,
            end: parser.unit_end(),
        }),
        Ok(_) => Err(parser.errors),
        Err(error) => {
            parser.errors.push(error);
            Err(parser.errors)
        }
    }
}

/// A parsed node and its depth (a leaf is 1).
struct Parsed<T> {
    node: T,
    depth: usize,
}

impl<T> Parsed<T> {
    fn leaf(node: T) -> Self {
        Parsed { node, depth: 1 }
    }
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    nesting: usize,
    /// Links of the `?:` and `??` chains being parsed.
    links: usize,
    max_depth: usize,
    /// Errors recovered from: collections skip a bad item and go on;
    /// expressions stop at the first error.
    errors: Vec<CompileError>,
    /// End of the last token, and of the whitespace after it: where an
    /// embedded unit really ends.
    tok_end: usize,
    ws_end: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            nesting: 0,
            links: 0,
            max_depth: MAX_TREE_DEPTH,
            errors: Vec::new(),
            tok_end: 0,
            ws_end: 0,
        }
    }

    fn at(src: &'a str, start: usize) -> Result<Self, Vec<CompileError>> {
        if start > src.len() || !src.is_char_boundary(start) {
            return Err(vec![CompileError::Syntax {
                message: "start offset is outside the source or inside a character".into(),
                span: Span::new(start.min(src.len()), start.min(src.len())),
            }]);
        }
        let mut parser = Parser::new(src);
        parser.max_depth = MAX_UNIT_DEPTH;
        parser.pos = start;
        parser.tok_end = start;
        parser.ws_end = start;
        parser.skip_ws();
        Ok(parser)
    }

    fn unit_end(&self) -> usize {
        if self.pos == self.ws_end {
            self.tok_end
        } else {
            self.pos
        }
    }

    fn nested<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        self.nesting += 1;
        if self.nesting > MAX_NESTING {
            self.nesting -= 1;
            return Err(CompileError::TooDeep {
                limit: MAX_NESTING,
                span: Span::new(self.pos, self.pos),
            });
        }
        let result = parse(self);
        self.nesting -= 1;
        result
    }

    /// One level above the deepest child (`0` for none); errors past
    /// `max_depth`.
    fn above<T>(
        &self,
        node: T,
        deepest_child: usize,
        span: Span,
    ) -> Result<Parsed<T>, CompileError> {
        let depth = deepest_child + 1;
        if depth > self.max_depth {
            return Err(CompileError::TooDeep {
                limit: self.max_depth,
                span,
            });
        }
        Ok(Parsed { node, depth })
    }

    /// Run `parse` for the next link of a `?:` or `??` chain. A link is a few
    /// light frames and one tree level, so chains are bounded by the tree
    /// depth; 0.3.0 never counted them as nesting.
    fn linked<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        self.links += 1;
        if self.links > self.max_depth {
            self.links -= 1;
            return Err(CompileError::TooDeep {
                limit: self.max_depth,
                span: Span::new(self.pos, self.pos),
            });
        }
        let result = parse(self);
        self.links -= 1;
        result
    }
}
