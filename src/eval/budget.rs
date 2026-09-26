use crate::common::error::{EvalError, RenderError, Span};
use crate::common::limits::{MAX_EVAL_DEPTH, MAX_VALUE_DEPTH};
use crate::common::value::Value;
use std::cell::Cell;

/// Where evaluation reports work. [`Unmetered`] compiles to nothing; the
/// embedding API's `Budget` counts and refuses.
pub(crate) trait Meter {
    type Err: From<RenderError>;
    /// Whether this meter can refuse; when false, limit bookkeeping is skipped.
    const LIMITED: bool;
    fn charge(&self, units: u64, span: Span) -> Result<(), Self::Err>;
    fn charge_copy(&self, value: &Value, span: Span) -> Result<(), Self::Err>;
    fn enter(&self, depth: u32, span: Span) -> Result<(), Self::Err>;
}

pub(crate) struct Unmetered;

impl Meter for Unmetered {
    type Err = RenderError;
    const LIMITED: bool = false;

    #[inline(always)]
    fn charge(&self, _units: u64, _span: Span) -> Result<(), RenderError> {
        Ok(())
    }

    #[inline(always)]
    fn charge_copy(&self, _value: &Value, _span: Span) -> Result<(), RenderError> {
        Ok(())
    }

    #[inline(always)]
    fn enter(&self, _depth: u32, _span: Span) -> Result<(), RenderError> {
        Ok(())
    }
}

/// A work limit shared by every unit evaluated with it: one unit per
/// expression and per element a method or built-in touches; copies cost one
/// per element and one per 64 bytes of text.
#[derive(Debug, Clone)]
pub struct Budget {
    limit: u64,
    used: Cell<u64>,
}

impl Budget {
    /// A budget of `limit` units.
    pub fn new(limit: u64) -> Self {
        Budget {
            limit,
            used: Cell::new(0),
        }
    }

    /// A budget that never runs out.
    pub fn unlimited() -> Self {
        Budget::new(u64::MAX)
    }

    /// The limit this budget was made with.
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Units charged so far.
    pub fn used(&self) -> u64 {
        self.used.get()
    }

    /// Units left before the budget runs out.
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used.get())
    }

    /// Start counting from zero again.
    pub fn reset(&self) {
        self.used.set(0);
    }
}

impl Meter for Budget {
    type Err = EvalError;
    const LIMITED: bool = true;

    #[inline]
    fn charge(&self, units: u64, span: Span) -> Result<(), EvalError> {
        let used = self.used.get().saturating_add(units);
        if used > self.limit {
            return Err(EvalError::BudgetExceeded {
                limit: self.limit,
                span,
            });
        }
        self.used.set(used);
        Ok(())
    }

    fn charge_copy(&self, value: &Value, span: Span) -> Result<(), EvalError> {
        let cost = match value {
            Value::Str(s) => (s.len() / 64) as u64,
            // counting stops past what is left, so refusing a huge value is cheap
            Value::Arr(_) | Value::Obj(_) => {
                match copy_cost(value, self.remaining().saturating_add(1)) {
                    Some(cost) => cost,
                    // copying and comparing both recurse, so refuse before either
                    None => {
                        return Err(EvalError::Render(RenderError::ValueTooDeep {
                            limit: MAX_VALUE_DEPTH,
                            span,
                        }))
                    }
                }
            }
            _ => 0,
        };
        if cost == 0 {
            return Ok(());
        }
        self.charge(cost, span)
    }

    #[inline]
    fn enter(&self, depth: u32, span: Span) -> Result<(), EvalError> {
        if depth >= MAX_EVAL_DEPTH {
            return Err(EvalError::TooDeep {
                limit: MAX_EVAL_DEPTH,
                span,
            });
        }
        Ok(())
    }
}

/// `None` past [`MAX_VALUE_DEPTH`]; stops counting at `cap`.
fn copy_cost(value: &Value, cap: u64) -> Option<u64> {
    let mut elements = 0u64;
    let mut bytes = 0u64;
    let mut pending: Vec<(&Value, usize)> = Vec::new();
    let mut next = Some((value, 1));
    while let Some((value, depth)) = next.take().or_else(|| pending.pop()) {
        if depth > MAX_VALUE_DEPTH {
            return None;
        }
        match value {
            Value::Arr(items) => {
                elements += items.len() as u64;
                for item in items {
                    match item {
                        Value::Str(s) => bytes += s.len() as u64,
                        Value::Arr(_) | Value::Obj(_) => pending.push((item, depth + 1)),
                        _ => {}
                    }
                }
            }
            Value::Obj(obj) => {
                elements += obj.len() as u64;
                for (key, item) in obj {
                    bytes += key.len() as u64;
                    match item {
                        Value::Str(s) => bytes += s.len() as u64,
                        Value::Arr(_) | Value::Obj(_) => pending.push((item, depth + 1)),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if elements + bytes / 64 > cap {
            break;
        }
    }
    Some(elements + bytes / 64)
}
