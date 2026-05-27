use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
                write!(f, "syntax error at {}..{}: {}", span.start, span.end, message)
            }
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::MissingPath { path, key, .. } => match key {
                Some(k) => write!(f, "missing field `{}` while resolving `{}`", k, path),
                None => write!(f, "missing path `{}`", path),
            },
            RenderError::TypeMismatch { expected, got, .. } => {
                write!(f, "type mismatch: expected {}, got {}", expected, got)
            }
            RenderError::Deserialize(msg) => write!(f, "deserialize error: {}", msg),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Corrupt(s) => write!(f, "corrupt blob: {}", s),
            LoadError::IncompatibleVersion { found, expected } => {
                write!(
                    f,
                    "incompatible blob version: found {}, expected {}",
                    found, expected
                )
            }
        }
    }
}

impl std::error::Error for CompileError {}
impl std::error::Error for RenderError {}
impl std::error::Error for LoadError {}
