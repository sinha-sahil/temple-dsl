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

    /// 1-based (line, column) of this span's start within `src`. Column counts
    /// characters, not bytes, so it lines up under multi-byte source.
    pub fn line_col(self, src: &str) -> (usize, usize) {
        let off = floor_char_boundary(src, self.start as usize);
        let mut line = 1usize;
        let mut line_start = 0usize;
        for (i, b) in src.as_bytes()[..off].iter().enumerate() {
            if *b == b'\n' {
                line += 1;
                line_start = i + 1;
            }
        }
        let col = src[line_start..off].chars().count() + 1;
        (line, col)
    }
}

#[derive(Debug, Clone)]
pub enum CompileError {
    Syntax { message: String, span: Span },
    TooDeep { limit: usize, span: Span },
    TooLarge { bytes: usize, limit: usize },
}

impl CompileError {
    /// The source span this error points at, if any (`TooLarge` has none).
    pub fn span(&self) -> Option<Span> {
        match self {
            CompileError::Syntax { span, .. } | CompileError::TooDeep { span, .. } => Some(*span),
            CompileError::TooLarge { .. } => None,
        }
    }

    /// Render this error against its source as an underlined snippet:
    ///
    /// ```text
    /// error: unknown identifier 'inputt'
    ///  --> 2:13
    ///   |
    /// 2 |   "id": {{ inputt.id }}
    ///   |          ^^^^^^
    /// ```
    pub fn report(&self, src: &str) -> String {
        let Some(span) = self.span() else {
            return format!("error: {}", self.headline());
        };
        let (line, col) = span.line_col(src);
        let line_text = nth_line(src, line);
        let num = line.to_string();
        let pad = " ".repeat(num.len());
        let carets = "^".repeat(caret_width(src, span));
        let caret_indent = " ".repeat(col.saturating_sub(1));
        format!(
            "error: {msg}\n{pad} --> {line}:{col}\n{pad} |\n{num} | {line_text}\n{pad} | {caret_indent}{carets}",
            msg = self.headline(),
        )
    }

    /// Render every error in a batch as snippets, separated by blank lines.
    pub fn report_all(src: &str, errors: &[CompileError]) -> String {
        errors
            .iter()
            .map(|e| e.report(src))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The human message without the byte-offset prefix that `Display` adds —
    /// the snippet shows the location, so the headline stays clean.
    fn headline(&self) -> String {
        match self {
            CompileError::Syntax { message, .. } => message.clone(),
            CompileError::TooDeep { limit, .. } => {
                format!("template nests too deeply (limit {limit} levels)")
            }
            CompileError::TooLarge { bytes, limit } => {
                format!("template is too large: {bytes} bytes exceeds the limit of {limit}")
            }
        }
    }
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
    DuplicateKey {
        key: String,
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
        // One wording source: `headline` carries the message, Display adds the
        // location prefix where one exists.
        match (self, self.span()) {
            (CompileError::Syntax { .. }, Some(span)) => {
                write!(
                    f,
                    "syntax error at {}..{}: {}",
                    span.start,
                    span.end,
                    self.headline()
                )
            }
            (_, Some(span)) => {
                write!(f, "{} at {}..{}", self.headline(), span.start, span.end)
            }
            (_, None) => write!(f, "{}", self.headline()),
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
            RenderError::DuplicateKey { key, .. } => {
                write!(f, "duplicate object key `{key}`")
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

fn nth_line(src: &str, line: usize) -> &str {
    src.lines().nth(line.saturating_sub(1)).unwrap_or("")
}

/// Caret width for a span, in characters, capped to its first line so the
/// underline never spills past the snippet's single source line.
fn caret_width(src: &str, span: Span) -> usize {
    let start = floor_char_boundary(src, span.start as usize);
    let end = floor_char_boundary(src, span.end as usize).max(start);
    let slice = &src[start..end];
    let first_line = slice.split('\n').next().unwrap_or(slice);
    first_line.chars().count().max(1)
}

/// Largest char boundary `<= i`, so slicing never splits a codepoint even if a
/// span is malformed.
fn floor_char_boundary(src: &str, mut i: usize) -> usize {
    if i >= src.len() {
        return src.len();
    }
    while i > 0 && !src.is_char_boundary(i) {
        i -= 1;
    }
    i
}
