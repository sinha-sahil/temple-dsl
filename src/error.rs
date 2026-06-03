use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }

    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

#[derive(Debug, Clone)]
pub enum CompileError {
    Syntax { message: String, span: Span },
    TooDeep { limit: usize, span: Span },
    TooLarge { bytes: usize, limit: usize },
}

#[derive(Debug, Clone)]
pub enum RenderError {
    MissingPath {
        path: String,
        key: Option<String>,
        span: Span,
    },
    TypeMismatch {
        expected: &'static str,
        got: String,
        span: Span,
    },
    DivideByZero {
        span: Span,
    },
    ArithmeticOverflow {
        span: Span,
    },
    WhenNoMatch {
        span: Span,
    },
    UnknownMethod {
        method: String,
        on_type: String,
        span: Span,
    },
    ArityMismatch {
        method: String,
        expected: usize,
        got: usize,
        span: Span,
    },
    IndexOutOfBounds {
        index: i64,
        length: usize,
        span: Span,
    },
    NotIndexable {
        got: String,
        span: Span,
    },
    LambdaExpected {
        span: Span,
    },
    ValueTooDeep {
        limit: usize,
        span: Span,
    },
    Deserialize(String),
}

#[derive(Debug, Clone)]
pub enum LoadError {
    Corrupt(String),
    IncompatibleVersion { found: u32, expected: u32 },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::Syntax { message, span } => {
                write!(
                    f,
                    "syntax error at {}..{}: {}",
                    span.start, span.end, message
                )
            }
            CompileError::TooDeep { limit, span } => {
                write!(
                    f,
                    "template nests too deeply at {}..{}: exceeds the limit of {} levels",
                    span.start, span.end, limit
                )
            }
            CompileError::TooLarge { bytes, limit } => {
                write!(
                    f,
                    "template is too large: {bytes} bytes exceeds the limit of {limit} bytes"
                )
            }
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::MissingPath { path, key, .. } => match key {
                Some(k) => write!(f, "missing field `{k}` while resolving `{path}`"),
                None => write!(f, "missing path `{path}`"),
            },
            RenderError::TypeMismatch { expected, got, .. } => {
                write!(f, "type mismatch: expected {expected}, got {got}")
            }
            RenderError::DivideByZero { .. } => write!(f, "divide by zero"),
            RenderError::ArithmeticOverflow { .. } => write!(f, "arithmetic overflow"),
            RenderError::WhenNoMatch { .. } => {
                write!(f, "no `when` branch matched and no `else` provided")
            }
            RenderError::UnknownMethod {
                method, on_type, ..
            } => {
                write!(f, "no method `{method}` on {on_type}")
            }
            RenderError::ArityMismatch {
                method,
                expected,
                got,
                ..
            } => write!(
                f,
                "method `{method}` expects {expected} argument(s), got {got}"
            ),
            RenderError::IndexOutOfBounds { index, length, .. } => {
                write!(
                    f,
                    "index {index} out of bounds for array of length {length}"
                )
            }
            RenderError::NotIndexable { got, .. } => write!(f, "cannot index into {got}"),
            RenderError::LambdaExpected { .. } => write!(f, "expected a lambda"),
            RenderError::ValueTooDeep { limit, .. } => {
                write!(f, "value nests deeper than the limit of {limit} levels")
            }
            RenderError::Deserialize(msg) => write!(f, "deserialize error: {msg}"),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Corrupt(s) => write!(f, "corrupt blob: {s}"),
            LoadError::IncompatibleVersion { found, expected } => {
                write!(
                    f,
                    "incompatible blob version: found {found}, expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for CompileError {}
impl std::error::Error for RenderError {}
impl std::error::Error for LoadError {}
