use super::Functions;
use crate::common::value::Value;
use crate::eval::scope::Vars;
use smol_str::SmolStr;

/// The values, and optionally the functions, a unit is evaluated with.
#[derive(Debug, Clone, Default)]
pub struct Env<'f> {
    pub(super) variables: Vars,
    pub(super) functions: Option<&'f Functions>,
}

impl Env<'static> {
    /// No variables and no functions.
    pub fn new() -> Self {
        Env {
            variables: Vars::default(),
            functions: None,
        }
    }
}

impl<'f> Env<'f> {
    /// An env whose units may call `functions`.
    pub fn with_functions(functions: &'f Functions) -> Self {
        Env {
            variables: Vars::default(),
            functions: Some(functions),
        }
    }

    /// Bind a variable, builder style.
    pub fn set(mut self, name: impl Into<SmolStr>, value: impl Into<Value>) -> Self {
        self.variables.insert(name.into(), value.into());
        self
    }

    /// Bind a variable in place, returning the value it replaced.
    pub fn insert(&mut self, name: impl Into<SmolStr>, value: impl Into<Value>) -> Option<Value> {
        self.variables.insert(name.into(), value.into())
    }
}
