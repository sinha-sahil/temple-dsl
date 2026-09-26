use super::{Parsed, Parser};
use crate::common::error::{CompileError, Span};
use crate::syntax::ast::{BinOp, Expr, ExprKind, LetBinding, UnOp};
use smol_str::SmolStr;

impl<'a> Parser<'a> {
    pub(super) fn parse_expr(&mut self) -> Result<Parsed<Expr>, CompileError> {
        self.skip_ws();
        if self.matches_ident("let") {
            self.nested(Self::parse_let_in)
        } else {
            self.parse_ternary()
        }
    }

    fn parse_let_in(&mut self) -> Result<Parsed<Expr>, CompileError> {
        let start = self.pos;
        let (name, name_span) = self.parse_let_head()?;
        let value = self.parse_expr()?;
        self.skip_ws();
        if !self.matches_ident("in") {
            return Err(CompileError::Syntax {
                message: "expected `in` after the let-binding value".into(),
                span: self.cur_char_span(),
            });
        }
        self.pos += "in".len();
        self.skip_ws();
        let body = self.parse_expr()?;
        let span = Span::new(start, body.node.span.range().end);
        let expr = Expr {
            kind: ExprKind::Let {
                name,
                name_span,
                value: Box::new(value.node),
                body: Box::new(body.node),
            },
            span,
        };
        self.above(expr, value.depth.max(body.depth), span)
    }

    fn parse_ternary(&mut self) -> Result<Parsed<Expr>, CompileError> {
        let cond = self.parse_coalesce()?;
        self.skip_ws();
        if self.peek() != Some(b'?') || self.matches("?.") || self.matches("??") {
            return Ok(cond);
        }
        self.pos += 1;
        self.skip_ws();
        let then_branch = self.linked(Self::parse_expr)?;
        self.skip_ws();
        self.expect(b':')?;
        self.skip_ws();
        let else_branch = self.linked(Self::parse_expr)?;
        let span = cond.node.span.to(else_branch.node.span);
        let deepest = cond.depth.max(then_branch.depth).max(else_branch.depth);
        let expr = Expr {
            kind: ExprKind::Ternary {
                cond: Box::new(cond.node),
                then_branch: Box::new(then_branch.node),
                else_branch: Box::new(else_branch.node),
            },
            span,
        };
        self.above(expr, deepest, span)
    }

    /// `a ?? b ?? c` groups to the right: `a ?? (b ?? c)`.
    fn parse_coalesce(&mut self) -> Result<Parsed<Expr>, CompileError> {
        let lhs = self.parse_binary(0)?;
        self.skip_ws();
        if !self.matches("??") {
            return Ok(lhs);
        }
        self.pos += 2;
        self.skip_ws();
        let rhs = self.linked(Self::parse_coalesce)?;
        let deepest = lhs.depth.max(rhs.depth);
        let expr = make_binary(BinOp::Coalesce, lhs.node, rhs.node);
        let span = expr.span;
        self.above(expr, deepest, span)
    }

    /// Precedence climbing from `min_precedence`. A long chain builds a deep
    /// tree without recursing; `Parsed` depth still refuses it past the limit.
    fn parse_binary(&mut self, min_precedence: u8) -> Result<Parsed<Expr>, CompileError> {
        let mut lhs = self.parse_unary()?;
        loop {
            self.skip_ws();
            let Some(op) = self.binary_operator() else {
                break;
            };
            if op.precedence() < min_precedence {
                break;
            }
            self.pos += op.symbol().len();
            self.skip_ws();
            let rhs = self.parse_binary(op.precedence() + 1)?;
            let deepest = lhs.depth.max(rhs.depth);
            let expr = make_binary(op, lhs.node, rhs.node);
            let span = expr.span;
            lhs = self.above(expr, deepest, span)?;
        }
        Ok(lhs)
    }

    fn binary_operator(&self) -> Option<BinOp> {
        BinOp::CLIMBING
            .into_iter()
            .find(|op| self.matches(op.symbol()))
    }

    fn parse_unary(&mut self) -> Result<Parsed<Expr>, CompileError> {
        self.nested(Self::parse_unary_inner)
    }

    fn parse_unary_inner(&mut self) -> Result<Parsed<Expr>, CompileError> {
        self.skip_ws();
        let start = self.pos;
        let op = match self.peek() {
            Some(b'!') => Some(UnOp::Not),
            // `-` starts a negative number literal, not a negation, when a
            // digit follows it
            Some(b'-') if !self.number_follows_minus() => Some(UnOp::Neg),
            _ => None,
        };
        let Some(op) = op else {
            return self.parse_primary();
        };
        self.pos += 1;
        self.skip_ws();
        let operand = self.parse_unary()?;
        let span = Span::new(start, operand.node.span.range().end);
        let expr = Expr {
            kind: ExprKind::Unary {
                op,
                operand: Box::new(operand.node),
            },
            span,
        };
        self.above(expr, operand.depth, span)
    }

    fn number_follows_minus(&self) -> bool {
        match self.peek_at(1) {
            Some(b'0'..=b'9') => true,
            Some(b'.') => self.peek_at(2).is_some_and(|byte| byte.is_ascii_digit()),
            _ => false,
        }
    }

    pub(super) fn parse_let_binding(&mut self) -> Result<LetBinding, CompileError> {
        let start = self.pos;
        let (name, _) = self.parse_let_head()?;
        let expr = self.parse_expr()?;
        Ok(LetBinding {
            name,
            expr: expr.node,
            span: Span::new(start, self.pos),
        })
    }

    /// Expects the cursor on `let`.
    fn parse_let_head(&mut self) -> Result<(SmolStr, Span), CompileError> {
        self.pos += "let".len();
        self.skip_ws();
        let name_start = self.pos;
        let name = self.read_ident();
        if name.is_empty() {
            return Err(CompileError::Syntax {
                message: "expected a name after `let`".into(),
                span: Span::new(name_start, name_start + 1),
            });
        }
        let name_span = Span::new(name_start, self.pos);
        self.skip_ws();
        self.expect(b'=')?;
        self.skip_ws();
        Ok((name, name_span))
    }
}

fn make_binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    let span = lhs.span.to(rhs.span);
    Expr {
        kind: ExprKind::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        span,
    }
}
