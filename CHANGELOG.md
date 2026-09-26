# Changelog

All notable changes to temple-dsl are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the crate follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- The embedding API, for host languages that put temple expressions inside
  their own syntax: `ExprUnit`, `TemplateUnit`, `ParsedExpr`,
  `ParsedTemplate`, `Names`, `Env`, `Functions`, `Budget`, `EvalError` and
  `InputRead`. A unit is parsed in place at a byte offset, checked against
  the names the host will bind, and evaluated later under a shared work
  budget.
- `Template::render_ref` and `Template::render_value_ref`, which render
  without taking the input, for many renders against one shared value.
- `Value::into_arr` and `Value::into_obj`.
- `is_reserved_name` and `is_builtin_function`, so a host can refuse names
  temple would reject.
- `RenderError::span` and `RenderError::report`, matching `CompileError`.
- `rust-version = "1.80"` in `Cargo.toml`.
- A compatibility gate (`tests/compat.rs`) with goldens recorded from 0.3.0.
- CI on every push and pull request, `LICENSE-MIT` and `LICENSE-APACHE`,
  `ARCHITECTURE.md` and `CONTRIBUTING.md`.

### Changed

- A template whose syntax tree is deeper than 200 levels is now refused at
  compile time with `CompileError::TooDeep` instead of risking a stack
  overflow. 0.3.0 had no such limit: it compiled and rendered, for example, a
  chain of more than 200 `+` or `?:` in one expression until the stack ran
  out (about 5,000 on a 2 MB stack), though such a template never loaded back
  from a blob. Brackets, calls and `let … in` still nest at most 64 levels
  deep, as in 0.3.0. Every template in the compatibility goldens compiles,
  formats and renders exactly as before.
- Did-you-mean hints are deterministic: when two names are equally close, the
  one that sorts first is suggested. One check hints at most 100 unknown
  names.
- Error text puts code in backticks (``unknown identifier `x` ``), says
  ``takes 2 arguments, got 3`` for every arity error, and `CompileError`'s
  `Display` no longer starts with `syntax error at`.
- A built-in function called with the wrong number of arguments is a compile
  error, as a host function already was; before, it failed at render time.
- `Template::from_bytes` refuses a blob whose tree nests deeper than the
  compile-time limit, instead of accepting a tree the evaluator can't walk.
- Compiling is about 10% faster: the parser climbs operator precedence in one
  loop instead of descending a six-level ladder for every operand. Rendering
  is unchanged.
- The source is reorganized by stage: `syntax/` (parser, tree, formatter,
  blob format), `check.rs`, `eval/`, `embed/`, and `common/` (errors,
  values, limits). No public item moved.

## [0.3.0]

The last version published before this changelog was started. See the git
history for earlier changes.
