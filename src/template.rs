use crate::check;
use crate::common::error::{CompileError, LoadError, RenderError};
use crate::common::limits::MAX_SOURCE_BYTES;
use crate::common::value::Value;
use crate::eval;
use crate::syntax::ast::Module;
use crate::syntax::{blob, format, parser};
use serde::de::DeserializeOwned;

/// A compiled template.
///
/// ```
/// use temple_dsl::{Template, Value};
///
/// let template = Template::compile(r#"{ "greeting": {{ upper(input.name) }} }"#).unwrap();
/// let out = template.render_value(Value::obj([("name", "ada")])).unwrap();
/// assert_eq!(out, Value::obj([("greeting", "ADA")]));
/// ```
#[derive(Debug, Clone)]
pub struct Template {
    module: Module,
    output_order: Vec<usize>,
}

impl Template {
    /// Parse and check `src`.
    pub fn compile(src: &str) -> Result<Self, Vec<CompileError>> {
        check_size(src)?;
        let module = parser::parse_module(src)?;
        let output_order = check::check_module(&module)?;
        Ok(Template {
            module,
            output_order,
        })
    }

    /// Whether `src` compiles; cheap enough to run on every keystroke.
    pub fn validate(src: &str) -> Result<(), Vec<CompileError>> {
        Self::compile(src).map(|_| ())
    }

    /// `src` in the canonical layout. Parses only, without name checks, so
    /// unfinished templates format too.
    pub fn format(src: &str) -> Result<String, Vec<CompileError>> {
        check_size(src)?;
        let module = parser::parse_module(src)?;
        Ok(format::format_module(&module))
    }

    /// The compiled template as bytes; [`from_bytes`](Self::from_bytes) loads
    /// it back without parsing.
    pub fn to_bytes(&self) -> Vec<u8> {
        blob::encode(&self.module)
    }

    /// Load a template saved with [`to_bytes`](Self::to_bytes). The tree is
    /// checked again, so a damaged blob fails here, not at render time.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LoadError> {
        let module = blob::decode(bytes)?;
        let output_order = check::check_module(&module).map_err(|errors| {
            LoadError::Corrupt(format!(
                "blob failed validation ({} error(s))",
                errors.len()
            ))
        })?;
        Ok(Template {
            module,
            output_order,
        })
    }

    /// Render into your own type. For a [`Value`], use
    /// [`render_value`](Self::render_value): `render::<Value>` is a compile
    /// error on purpose.
    pub fn render<T>(&self, input: impl Into<Value>) -> Result<T, RenderError>
    where
        T: DeserializeOwned,
    {
        self.render_ref(&input.into())
    }

    /// [`render`](Self::render) without taking the input, for many renders
    /// against one value.
    pub fn render_ref<T>(&self, input: &Value) -> Result<T, RenderError>
    where
        T: DeserializeOwned,
    {
        let output = self.render_value_ref(input)?;
        T::deserialize(&output).map_err(|e| RenderError::Deserialize(e.to_string()))
    }

    /// Render to a [`Value`].
    pub fn render_value(&self, input: impl Into<Value>) -> Result<Value, RenderError> {
        self.render_value_ref(&input.into())
    }

    /// [`render_value`](Self::render_value) without taking the input.
    pub fn render_value_ref(&self, input: &Value) -> Result<Value, RenderError> {
        eval::run_template(&self.module, &self.output_order, input)
    }
}

fn check_size(src: &str) -> Result<(), Vec<CompileError>> {
    if src.len() > MAX_SOURCE_BYTES {
        return Err(vec![CompileError::TooLarge {
            bytes: src.len(),
            limit: MAX_SOURCE_BYTES,
        }]);
    }
    Ok(())
}
