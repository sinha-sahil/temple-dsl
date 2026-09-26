//! Temple is a small language for shaping data: an input value goes in, a
//! value of the shape you asked for comes out.
//!
//! ```text
//! let total = input.items.map(i -> i.price * i.qty).sum()
//!
//! {
//!   "customer": {{ input.customer.name }},
//!   "total": {{ total }},
//!   "vip": {{ total > 1000 }}
//! }
//! ```
//!
//! # Rendering a template
//!
//! [`Template::compile`] parses and checks the source once; render it as
//! many times as you like, into a [`Value`] or straight into your own type.
//!
//! ```
//! use serde::Deserialize;
//! use temple_dsl::{Template, Value};
//!
//! #[derive(Deserialize)]
//! struct Greeting {
//!     text: String,
//! }
//!
//! let template = Template::compile(r#"{ "text": {{ concat("hi ", input.name) }} }"#).unwrap();
//! let greeting: Greeting = template.render(Value::obj([("name", "Ada")])).unwrap();
//! assert_eq!(greeting.text, "hi Ada");
//! ```
//!
//! # Embedding temple in another language
//!
//! A host parses one expression or value at a time inside its own syntax
//! ([`ExprUnit::parse_at`], [`TemplateUnit::parse_at`]), checks it against
//! the [`Names`] it will bind, and evaluates it with an [`Env`] under a
//! [`Budget`].
//!
//! ```
//! use temple_dsl::{Budget, Env, ExprUnit, Names, Value};
//!
//! let names = Names::new().var("limit");
//! let unit = ExprUnit::compile("input.age >= limit", &names).unwrap();
//! let env = Env::new().set("limit", 18);
//! let out = unit
//!     .eval(&Value::obj([("age", Value::Int(21))]), &env, &Budget::new(1_000))
//!     .unwrap();
//! assert_eq!(out, Value::Bool(true));
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod check;
mod common;
mod embed;
mod eval;
mod syntax;
mod template;

pub use common::error::{CompileError, EvalError, LoadError, RenderError, Span};
pub use common::value::Value;
pub use embed::{
    Env, ExprUnit, Functions, InputRead, Names, ParsedExpr, ParsedTemplate, TemplateUnit,
};
pub use eval::budget::Budget;
pub use syntax::blob::BLOB_VERSION;
pub use template::Template;

/// This crate's version, for hosts that record it next to stored blobs.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether `name` is reserved (`input`, `this`, `let`, …) and so can't name a
/// variable or function.
pub fn is_reserved_name(name: &str) -> bool {
    check::is_reserved(name)
}

/// Whether `name` is a built-in function, and so can't name a host function.
pub fn is_builtin_function(name: &str) -> bool {
    eval::builtins::is_builtin_function(name)
}
