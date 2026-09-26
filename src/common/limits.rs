pub(crate) const MAX_SOURCE_BYTES: usize = 1 << 20;

/// CBOR runs about ten times the source's size.
pub(crate) const MAX_BLOB_BYTES: usize = MAX_SOURCE_BYTES * 16;

/// Parser recursion. One level can push several frames, so it stays well
/// under the stack; don't raise it casually.
pub(crate) const MAX_NESTING: usize = 64;

/// Deepest template syntax tree: every recursive pass stays inside a 2 MB
/// stack, even in a debug build.
pub(crate) const MAX_TREE_DEPTH: usize = 200;

/// Deepest embedded unit. Lower than a template's: host functions nest one
/// unit's evaluation inside another's.
pub(crate) const MAX_UNIT_DEPTH: usize = 100;

/// Deepest value a path may produce; clone, compare and deserialize recurse
/// per level.
pub(crate) const MAX_VALUE_DEPTH: usize = 128;

pub(crate) const MAX_JSON_DEPTH: usize = 256;

/// Most decimal places `round` keeps: rust_decimal's largest scale.
pub(crate) const MAX_DECIMAL_SCALE: u32 = 28;

/// Deepest nesting of evaluations under a budget: the deepest unit, plus host
/// functions called inside it.
pub(crate) const MAX_EVAL_DEPTH: u32 = MAX_UNIT_DEPTH as u32 + 50;

pub(crate) const MAX_CALL_DEPTH: usize = 64;

pub(crate) const MAX_PARAMS: usize = 64;

pub(crate) const MAX_HINTS: usize = 100;
