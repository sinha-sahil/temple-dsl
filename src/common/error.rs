use serde::{Deserialize, Serialize};
use std::fmt;

/// A byte range `start..end` in a source string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    /// First byte.
    pub start: u32,
    /// Just past the last byte.
    pub end: u32,
}

impl Span {
    /// A span from byte `start` up to, not including, byte `end`.
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }

    /// The span as a `usize` range, for slicing the source.
    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }

    /// From the start of `self` to the end of `last`.
    pub(crate) fn to(self, last: Span) -> Span {
        Span {
            start: self.start,
            end: last.end,
        }
    }

    /// 1-based line and column of the start in `src`; columns count
    /// characters.
    pub fn line_col(self, src: &str) -> (usize, usize) {
        let location = Lines::new(src).locate(self.start as usize);
        (location.line, location.column)
    }
}

/// Why a template could not be compiled.
#[derive(Debug, Clone)]
#[allow(missing_docs)] // variant fields name themselves
pub enum CompileError {
    /// The source doesn't parse, or names something that doesn't exist.
    Syntax { message: String, span: Span },
    /// The template nests deeper than `limit` levels.
    TooDeep { limit: usize, span: Span },
    /// The source is longer than `limit` bytes.
    TooLarge { bytes: usize, limit: usize },
}

impl CompileError {
    /// The source span this error points at. `TooLarge` has none.
    pub fn span(&self) -> Option<Span> {
        match self {
            CompileError::Syntax { span, .. } | CompileError::TooDeep { span, .. } => Some(*span),
            CompileError::TooLarge { .. } => None,
        }
    }

    /// The error as an underlined snippet of `src`:
    ///
    /// ```text
    /// error: unknown identifier `inputt`
    ///  --> 2:13
    ///   |
    /// 2 |   "id": {{ inputt.id }}
    ///   |          ^^^^^^
    /// ```
    pub fn report(&self, src: &str) -> String {
        snippet(&Lines::new(src), self.span(), &self.headline())
    }

    /// Every error in `errors` as a snippet, separated by blank lines.
    pub fn report_all(src: &str, errors: &[CompileError]) -> String {
        let lines = Lines::new(src);
        errors
            .iter()
            .map(|e| snippet(&lines, e.span(), &e.headline()))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The message without its location.
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

/// Why a template could not be rendered.
#[derive(Debug, Clone)]
#[allow(missing_docs)] // variant fields name themselves
pub enum RenderError {
    /// A path reached a field that isn't there; `key` is the missing field.
    MissingPath {
        path: String,
        key: Option<String>,
        span: Span,
    },
    /// A value has the wrong type for what was done with it.
    TypeMismatch {
        expected: &'static str,
        got: String,
        span: Span,
    },
    /// Division or remainder by zero.
    DivideByZero { span: Span },
    /// A number left the range it can represent.
    ArithmeticOverflow { span: Span },
    /// No `when` branch matched and there was no `else`.
    WhenNoMatch { span: Span },
    /// A method that doesn't exist on the value's type.
    UnknownMethod {
        method: String,
        on_type: String,
        span: Span,
    },
    /// A method or function called with the wrong number of arguments.
    ArityMismatch {
        method: String,
        expected: usize,
        got: usize,
        span: Span,
    },
    /// An array index outside the array.
    IndexOutOfBounds {
        index: i64,
        length: usize,
        span: Span,
    },
    /// `[...]` on a value that isn't an array or object.
    NotIndexable { got: String, span: Span },
    /// A method that takes a lambda was given something else.
    LambdaExpected { span: Span },
    /// A value nests deeper than `limit` levels.
    ValueTooDeep { limit: usize, span: Span },
    /// An object literal produced the same key twice.
    DuplicateKey { key: String, span: Span },
    /// The rendered value could not be turned into the requested Rust type.
    Deserialize(String),
}

impl RenderError {
    /// The source span this error points at. `Deserialize` has none.
    pub fn span(&self) -> Option<Span> {
        match self {
            RenderError::MissingPath { span, .. }
            | RenderError::TypeMismatch { span, .. }
            | RenderError::DivideByZero { span }
            | RenderError::ArithmeticOverflow { span }
            | RenderError::WhenNoMatch { span }
            | RenderError::UnknownMethod { span, .. }
            | RenderError::ArityMismatch { span, .. }
            | RenderError::IndexOutOfBounds { span, .. }
            | RenderError::NotIndexable { span, .. }
            | RenderError::LambdaExpected { span }
            | RenderError::ValueTooDeep { span, .. }
            | RenderError::DuplicateKey { span, .. } => Some(*span),
            RenderError::Deserialize(_) => None,
        }
    }

    /// The error as an underlined snippet of `src`, in the same layout as
    /// [`CompileError::report`].
    pub fn report(&self, src: &str) -> String {
        snippet(&Lines::new(src), self.span(), &self.to_string())
    }

    #[cold]
    pub(crate) fn type_mismatch(
        expected: &'static str,
        got: impl Into<String>,
        span: Span,
    ) -> Self {
        RenderError::TypeMismatch {
            expected,
            got: got.into(),
            span,
        }
    }

    #[cold]
    pub(crate) fn missing_field(name: &str, span: Span) -> Self {
        RenderError::MissingPath {
            path: name.to_string(),
            key: Some(name.to_string()),
            span,
        }
    }

    #[cold]
    pub(crate) fn not_indexable(got: &str, span: Span) -> Self {
        RenderError::NotIndexable {
            got: got.to_string(),
            span,
        }
    }

    #[cold]
    pub(crate) fn unknown_method(method: &str, on_type: &str, span: Span) -> Self {
        RenderError::UnknownMethod {
            method: method.to_string(),
            on_type: on_type.to_string(),
            span,
        }
    }
}

/// Why a stored template could not be loaded.
#[derive(Debug, Clone)]
#[allow(missing_docs)] // variant fields name themselves
pub enum LoadError {
    /// The bytes are damaged or aren't a template blob.
    Corrupt(String),
    /// The blob was written with a different blob format.
    IncompatibleVersion { found: u32, expected: u32 },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.span() {
            Some(span) => write!(f, "{} at {}..{}", self.headline(), span.start, span.end),
            None => write!(f, "{}", self.headline()),
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
            } => write!(f, "no method `{method}` on {on_type}"),
            RenderError::ArityMismatch {
                method,
                expected,
                got,
                ..
            } => write!(f, "{}", arity_message(method, &arguments(*expected), *got)),
            RenderError::IndexOutOfBounds { index, length, .. } => write!(
                f,
                "index {index} out of bounds for array of length {length}"
            ),
            RenderError::NotIndexable { got, .. } => write!(f, "cannot index into {got}"),
            RenderError::LambdaExpected { .. } => write!(f, "expected a lambda"),
            RenderError::ValueTooDeep { limit, .. } => {
                write!(f, "value nests deeper than the limit of {limit} levels")
            }
            RenderError::DuplicateKey { key, .. } => write!(f, "duplicate object key `{key}`"),
            RenderError::Deserialize(msg) => write!(f, "deserialize error: {msg}"),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Corrupt(s) => write!(f, "corrupt blob: {s}"),
            LoadError::IncompatibleVersion { found, expected } => write!(
                f,
                "incompatible blob version: found {found}, expected {expected}"
            ),
        }
    }
}

/// Why evaluating a unit failed.
#[derive(Debug, Clone)]
#[allow(missing_docs)] // variant fields name themselves
pub enum EvalError {
    /// The same errors a template render can produce.
    Render(RenderError),
    /// The [`Budget`](crate::Budget) ran out while evaluating the expression at `span`.
    BudgetExceeded { limit: u64, span: Span },
    /// Evaluations nested deeper than `limit` (host functions calling host
    /// functions).
    TooDeep { limit: u32, span: Span },
}

impl EvalError {
    /// The source span this error points at, if it has one.
    pub fn span(&self) -> Option<Span> {
        match self {
            EvalError::Render(e) => e.span(),
            EvalError::BudgetExceeded { span, .. } | EvalError::TooDeep { span, .. } => Some(*span),
        }
    }

    /// The error as an underlined snippet of the source the unit came from.
    pub fn report(&self, src: &str) -> String {
        snippet(&Lines::new(src), self.span(), &self.to_string())
    }

    /// Whether this is [`EvalError::BudgetExceeded`].
    pub fn is_budget_exceeded(&self) -> bool {
        matches!(self, EvalError::BudgetExceeded { .. })
    }
}

impl From<RenderError> for EvalError {
    fn from(e: RenderError) -> Self {
        EvalError::Render(e)
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::Render(e) => e.fmt(f),
            EvalError::BudgetExceeded { limit, .. } => {
                write!(f, "evaluation budget of {limit} steps exceeded")
            }
            EvalError::TooDeep { limit, .. } => {
                write!(f, "evaluation nests more than {limit} levels deep")
            }
        }
    }
}

impl std::error::Error for EvalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EvalError::Render(e) => Some(e),
            EvalError::BudgetExceeded { .. } | EvalError::TooDeep { .. } => None,
        }
    }
}

impl std::error::Error for CompileError {}
impl std::error::Error for RenderError {}
impl std::error::Error for LoadError {}

pub(crate) fn arity_message(name: &str, takes: &str, got: usize) -> String {
    format!("`{name}` takes {takes}, got {got}")
}

pub(crate) fn arguments(count: usize) -> String {
    if count == 1 {
        "1 argument".to_string()
    } else {
        format!("{count} arguments")
    }
}

/// Line starts of a source, so placing many spans scans it once.
pub(crate) struct Lines<'a> {
    src: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub(crate) fn new(src: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            src.bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(i, _)| i + 1),
        );
        Lines { src, starts }
    }

    pub(crate) fn locate(&self, at: usize) -> Location<'a> {
        let at = floor_char_boundary(self.src, at);
        let line_index = self.starts.partition_point(|&start| start <= at) - 1;
        let start = self.starts[line_index];
        let end = self
            .starts
            .get(line_index + 1)
            .map_or(self.src.len(), |&next_start| next_start - 1);
        let text = self.src[start..end]
            .strip_suffix('\r')
            .unwrap_or(&self.src[start..end]);
        Location {
            line: line_index + 1,
            column: self.src[start..at].chars().count() + 1,
            text,
        }
    }
}

/// 1-based line and column (in characters), and the line's text.
pub(crate) struct Location<'a> {
    pub line: usize,
    pub column: usize,
    pub text: &'a str,
}

pub(crate) fn snippet(lines: &Lines, span: Option<Span>, message: &str) -> String {
    let Some(span) = span else {
        return format!("error: {message}");
    };
    let at = lines.locate(span.start as usize);
    let line_number = at.line.to_string();
    let gutter = " ".repeat(line_number.len());
    let indent = " ".repeat(at.column - 1);
    let carets = "^".repeat(caret_width(lines.src, span));
    format!(
        "error: {message}\n{gutter} --> {}:{}\n{gutter} |\n{line_number} | {}\n{gutter} | {indent}{carets}",
        at.line, at.column, at.text
    )
}

fn caret_width(src: &str, span: Span) -> usize {
    let start = floor_char_boundary(src, span.start as usize);
    let end = floor_char_boundary(src, span.end as usize).max(start);
    let first_line = src[start..end].split('\n').next().unwrap_or("");
    first_line.chars().count().max(1)
}

fn floor_char_boundary(src: &str, mut i: usize) -> usize {
    if i >= src.len() {
        return src.len();
    }
    while i > 0 && !src.is_char_boundary(i) {
        i -= 1;
    }
    i
}
