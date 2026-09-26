# Architecture

Where the code lives and the rules that hold it together. [`DESIGN.md`](DESIGN.md)
says what the language is.

## Pipeline

```text
source ── syntax/parser/ ──▶ tree (syntax/ast.rs) ── check.rs ──▶ Template (template.rs)
                                                                      │  eval/
                                                                      ▼
                                                                    Value (common/value.rs)
```

`embed/` runs the same parser, checker and evaluator on one expression or
value at a time, for a host language that puts temple inside its own syntax.

## Code map

| Path | Holds |
| --- | --- |
| `lib.rs` | Public API: re-exports, crate docs |
| `main.rs` | `temple` CLI (`cli` feature) |
| `check.rs` | Checker: names, `this` order, duplicate keys, arity, hints |
| `template.rs` | `Template`: compile, validate, format, render, save, load |
| `common/error.rs` | `Span`, every error type, snippet reports |
| `common/limits.rs` | Every limit |
| `common/value.rs` | `Value`, conversions, deserializing into your types |
| `syntax/ast.rs` | Syntax tree, operator table, child walker |
| `syntax/blob.rs` | Blob format: signature, version, CBOR, load checks |
| `syntax/format.rs` | Canonical formatter |
| `syntax/parser/` | `mod.rs` entry points and state; `scan.rs` tokens; `output.rs` output values; `expr.rs` operators and `let`; `primary.rs` literals, paths, calls, lambdas, `when` |
| `eval/mod.rs` | Entry points, `Context`, expression dispatch |
| `eval/budget.rs` | `Meter`, `Unmetered`, `Budget` |
| `eval/scope.rs` | `Vars`, `Scope` |
| `eval/ops.rs`, `eval/path.rs` | Operators; paths like `input.a.b[i]` |
| `eval/builtins.rs` | Built-in functions: one table of name, arity, function |
| `eval/methods/` | Methods: `mod.rs` `Call` and charged copies; `array.rs`, `object.rs`, `string.rs` one table each of name, cost, method |
| `embed/` | `mod.rs` unit types; `names.rs`, `env.rs`, `functions.rs`, `queries.rs` |

## Layout rules

- `src/` holds `lib.rs`, `main.rs`, one entry per stage, and `common/`.
- A stage is a file until it has parts, then a folder. A part with parts of
  its own gets a folder inside.
- `mod.rs` holds entry points and shared state only.
- File names stay short; the folder gives the context.
- Layers point down: `syntax` → `check` → `eval`. `embed` sits on top, and
  nothing imports from it.

## Invariants

- **No input panics or overflows the stack.** Every limit is in
  `common/limits.rs` and ends in a typed error. Arithmetic is checked,
  `Value` drops without recursion, and blobs pass a depth check before
  anything walks them.
- **Backward compatibility.** `tests/compat.rs` compares compile, format,
  render and blob output for `examples/data/` against goldens recorded from
  0.3.0. Changing them is a breaking change; regenerate only on purpose:
  `TEMPLE_BLESS=1 cargo test --all-features --test compat`.
- **Plain renders don't pay for the embedding API.** `Meter` has a `LIMITED`
  constant, and `Unmetered` compiles to nothing.

## Tests

`cargo test --all-features` runs unit tests, `tests/`, doctests and the
compat gate. `tests/robustness.rs` is the no-panic sweep; `tests/embed/`
covers the embedding API and budget accounting.
