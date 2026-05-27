use crate::error::{CompileError, RenderError};
use crate::parse::{self, OutNode};
use crate::value::Value;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone)]
pub struct Template {
    output: OutNode,
}

impl Template {
    pub fn compile(src: &str) -> Result<Self, Vec<CompileError>> {
        let output = parse::parse(src)?;
        Ok(Template { output })
    }

    pub fn render<T>(&self, input: impl Into<Value>) -> Result<T, RenderError>
    where
        T: DeserializeOwned,
    {
        let input = input.into();
        let output_value = crate::eval::evaluate(&self.output, &input)?;
        T::deserialize(&output_value).map_err(|e| RenderError::Deserialize(e.to_string()))
    }
}
