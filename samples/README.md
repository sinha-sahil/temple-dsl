# Temple sample templates

Each `.temple` file shows a category of supported template. For the full design
see [`../DESIGN.md`](../DESIGN.md); for the language at a glance,
[`../README.md`](../README.md).

| File | What it shows |
| ---- | ------------- |
| [`minimal.temple`](minimal.temple) | A flat object with direct path access — the simplest useful template. |
| [`http_request.temple`](http_request.temple) | An HTTP request — interpolation in the URL and headers, computed body fields. |
| [`receipt.temple`](receipt.temple) | `let` variables, `when` and ternary conditionals, `this.` self-reference, decimal-exact arithmetic. |
| [`normalize.temple`](normalize.temple) | Reshaping a third-party response into a canonical shape; `?.` / `??` for fields that may be absent. |
| [`array_output.temple`](array_output.temple) | A template whose top-level output is a (fixed-length) array. |
| [`scalar_output.temple`](scalar_output.temple) | A whole-template-is-one-expression rule predicate. |
| [`collections.temple`](collections.temple) | `map` / `filter` / `fold` over an input array — bounded iteration. |

## Reading a template

- `{{ … }}` is a **hole** — an expression. **Bare** (`"k": {{ … }}`) yields a
  typed value; **quoted** (`"k": "… {{ … }} …"`) interpolates into a string.
- `input.` is the render-time input, `this.` reads sibling output keys, bare
  names are `let` variables.
- `?.` tolerates an absent field (→ `null`); `??` supplies a fallback for `null`.
- `#` starts a comment.

## What is *not* supported

- **Unbounded loops & recursion.** Temple has bounded `map` / `filter` / `fold`
  over input collections, but no `while` and no recursion — nothing that can
  fail to terminate.
- **Side effects.** Evaluation is pure and deterministic.

> **Design stage:** Temple is not yet implemented — these samples illustrate the
> intended language. Syntax is settled per [`../DESIGN.md`](../DESIGN.md) but unbuilt.
