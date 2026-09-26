use crate::check::name_problem;
use crate::common::error::{CompileError, Span};
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};

/// The variables and functions a unit may use besides `input` and `this`.
#[derive(Debug, Clone, Default)]
pub struct Names {
    pub(super) variables: HashSet<SmolStr>,
    pub(super) functions: HashMap<SmolStr, usize>,
    /// Problems with the names themselves, found as they were added.
    problems: Vec<String>,
}

impl Names {
    /// No names.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allow a variable.
    pub fn var(mut self, name: impl Into<SmolStr>) -> Self {
        self.add_var(name);
        self
    }

    /// Allow several variables.
    pub fn vars<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        for name in names {
            self.add_var(name);
        }
        self
    }

    /// Allow a host function taking `arity` arguments.
    pub fn function(mut self, name: impl Into<SmolStr>, arity: usize) -> Self {
        self.add_function(name, arity);
        self
    }

    /// Allow a variable, in place.
    pub fn add_var(&mut self, name: impl Into<SmolStr>) {
        let name = name.into();
        if self.variables.contains(&name) {
            return;
        }
        if let Some(problem) = name_problem(&name, false) {
            self.problems.push(problem);
        } else if self.functions.contains_key(&name) {
            self.problems.push(both(&name));
        }
        self.variables.insert(name);
    }

    /// Allow a function, in place; redefining it changes its arity.
    pub fn add_function(&mut self, name: impl Into<SmolStr>, arity: usize) {
        let name = name.into();
        if !self.functions.contains_key(&name) {
            if let Some(problem) = name_problem(&name, true) {
                self.problems.push(problem);
            } else if self.variables.contains(&name) {
                self.problems.push(both(&name));
            }
        }
        self.functions.insert(name, arity);
    }

    /// Whether `name` is an allowed variable.
    pub fn has_var(&self, name: &str) -> bool {
        self.variables.contains(name)
    }

    /// How many arguments the function `name` takes, if it is allowed.
    pub fn function_arity(&self, name: &str) -> Option<usize> {
        self.functions.get(name).copied()
    }

    pub(super) fn checked<T>(
        &self,
        span: Span,
        checked: Result<T, Vec<CompileError>>,
    ) -> Result<T, Vec<CompileError>> {
        let mut errors: Vec<CompileError> = self
            .problems
            .iter()
            .map(|message| CompileError::Syntax {
                message: message.clone(),
                span,
            })
            .collect();
        match checked {
            Ok(value) if errors.is_empty() => Ok(value),
            Ok(_) => Err(errors),
            Err(mut more) => {
                errors.append(&mut more);
                Err(errors)
            }
        }
    }
}

fn both(name: &str) -> String {
    format!("`{name}` is both a variable and a function")
}
