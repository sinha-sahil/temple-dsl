mod env;
mod functions;
mod names;
mod queries;

pub use env::Env;
pub use functions::Functions;
pub use names::Names;
pub use queries::InputRead;

use crate::check;
use crate::common::error::{CompileError, EvalError, Span};
use crate::common::value::Value;
use crate::eval;
use crate::eval::budget::Budget;
use crate::syntax::ast::{Expr, OutNode};
use crate::syntax::parser;

/// An expression parsed out of a larger source, not yet checked.
#[derive(Debug, Clone)]
pub struct ParsedExpr {
    expr: Expr,
    end: usize,
}

impl ParsedExpr {
    /// The byte offset just past the expression's last token.
    pub fn end(&self) -> usize {
        self.end
    }

    /// Where the expression is in the source it was parsed from.
    pub fn span(&self) -> Span {
        self.expr.span
    }

    /// Check names and structure, giving a unit ready to evaluate.
    pub fn check(self, names: &Names) -> Result<ExprUnit, Vec<CompileError>> {
        let checked = check::check_unit_expr(&self.expr, &names.variables, &names.functions);
        names.checked(self.expr.span, checked)?;
        Ok(ExprUnit { expr: self.expr })
    }
}

/// An output value parsed out of a larger source, not yet checked.
#[derive(Debug, Clone)]
pub struct ParsedTemplate {
    node: OutNode,
    end: usize,
}

impl ParsedTemplate {
    /// The byte offset just past the value's last token.
    pub fn end(&self) -> usize {
        self.end
    }

    /// Where the value is in the source it was parsed from.
    pub fn span(&self) -> Span {
        self.node.span
    }

    /// Check names and structure, giving a unit ready to render.
    pub fn check(self, names: &Names) -> Result<TemplateUnit, Vec<CompileError>> {
        let checked = check::check_unit_output(&self.node, &names.variables, &names.functions);
        let order = names.checked(self.node.span, checked)?;
        Ok(TemplateUnit {
            node: self.node,
            order,
        })
    }
}

/// A checked expression, ready to evaluate with an [`Env`].
#[derive(Debug, Clone)]
pub struct ExprUnit {
    expr: Expr,
}

impl ExprUnit {
    /// Parse one expression at byte `start` of `src`; spans stay absolute.
    pub fn parse_at(src: &str, start: usize) -> Result<ParsedExpr, Vec<CompileError>> {
        let unit = parser::parse_expr_at(src, start)?;
        Ok(ParsedExpr {
            expr: unit.node,
            end: unit.end,
        })
    }

    /// Parse and check a string that holds exactly one expression.
    pub fn compile(src: &str, names: &Names) -> Result<ExprUnit, Vec<CompileError>> {
        let parsed = Self::parse_at(src, 0)?;
        parser::expect_end(src, parsed.end, "expression")?;
        parsed.check(names)
    }

    /// Evaluate against `input` and the bindings in `env`, charging `budget`.
    pub fn eval(&self, input: &Value, env: &Env, budget: &Budget) -> Result<Value, EvalError> {
        let functions = env.functions.map(Functions::table);
        eval::run_expr(&self.expr, input, &env.variables, functions, budget)
    }

    /// Where the expression is in the source it was parsed from.
    pub fn span(&self) -> Span {
        self.expr.span
    }

    /// Every `input` path this expression reads.
    pub fn reads(&self) -> Vec<InputRead> {
        let mut reads = Vec::new();
        queries::reads_in_expr(&self.expr, &mut reads);
        reads
    }
}

/// A checked output value, ready to render with an [`Env`].
#[derive(Debug, Clone)]
pub struct TemplateUnit {
    node: OutNode,
    order: Vec<usize>,
}

impl TemplateUnit {
    /// Parse one output value at byte `start`; same contract as
    /// [`ExprUnit::parse_at`].
    pub fn parse_at(src: &str, start: usize) -> Result<ParsedTemplate, Vec<CompileError>> {
        let unit = parser::parse_output_at(src, start)?;
        Ok(ParsedTemplate {
            node: unit.node,
            end: unit.end,
        })
    }

    /// Parse and check a string that holds exactly one output value.
    pub fn compile(src: &str, names: &Names) -> Result<TemplateUnit, Vec<CompileError>> {
        let parsed = Self::parse_at(src, 0)?;
        parser::expect_end(src, parsed.end, "value")?;
        parsed.check(names)
    }

    /// Render against `input` and the bindings in `env`, charging `budget`.
    pub fn render(&self, input: &Value, env: &Env, budget: &Budget) -> Result<Value, EvalError> {
        let functions = env.functions.map(Functions::table);
        let vars = &env.variables;
        eval::run_output(&self.node, &self.order, input, vars, functions, budget)
    }

    /// Where the value is in the source it was parsed from.
    pub fn span(&self) -> Span {
        self.node.span
    }

    /// Every `input` path this value reads.
    pub fn reads(&self) -> Vec<InputRead> {
        let mut reads = Vec::new();
        self.node
            .for_each_expr(&mut |expr| queries::reads_in_expr(expr, &mut reads));
        reads
    }
}
