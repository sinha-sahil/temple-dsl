# 🏛️ Temple

> A small, fast Rust DSL for shaping data — input in, strongly-typed Rust value out.

> [!NOTE]
> **Design stage** — Temple is not yet implemented. This document reflects the
> settled design. Depth: [`DESIGN.md`](DESIGN.md) · Examples: [`samples/`](samples/)

## Overview

Temple takes an **input value** and a **template**, and returns a value
deserialized straight into your Rust types. A template describes the *shape* of
the output, with `{{ … }}` holes where values are computed.

It is a small, hand-rolled, dynamically-typed interpreter — built for one job,
reshaping data, and built to be fast: a template compiles once and renders many
times, on a path that never parses.

**Built for** — templating, rule engines, and reshaping data on the fly.

## Example

```
# price a cart, with a loyalty discount
let tier = when {
  input.customer.spend >= 1000: 'gold'
  input.customer.spend >= 250:  'silver'
  else:                         'standard'
}

{
  "customer": {{ input.customer.name }},
  "tier":     {{ tier }},
  "discount": {{ input.cart.subtotal * (tier == 'gold' ? 0.15 : 0.0) }},
  "total":    {{ input.cart.subtotal - this.discount }},
  "note":     "Thanks, {{ input.customer.name }}!"
}
```

`let` declares a variable · `input.` is the data you pass in · `this.` reads a
sibling key · `when` picks the first matching branch · `{{ … }}` holes compute
values · arithmetic is decimal-exact. More patterns in [`samples/`](samples/).

## Features

| Capability | What it gives you |
| --- | --- |
| **Decimal-correct math** | `rust_decimal` throughout — no `f64`, no rounding drift |
| **Typed output** | Results deserialize straight into your Rust structs, via serde |
| **Compile once, render many** | Templates pre-compile to a serializable blob; the render path never parses |
| **Variables & self-reference** | `let` bindings and `this.<key>`, wired by a compile-time dependency graph |
| **Conditionals** | `when` guard table and ternary `?:`, with full operator precedence |
| **Bounded iteration** | `map` / `filter` / `fold` over input collections — terminating, never a runaway loop |
| **Safe field access** | `?.` optional access and `??` null-coalescing for fields that may be absent |
| **Custom functions** | Register your own alongside a small built-in set |
| **Errors are values** | Every fallible call returns `Result` — Temple never panics |

## Quick start

```rust
use temple_dsl::Template;

// Write time — compile once; store the source and the compiled blob.
let compiled = Template::compile(source)?;
db.store(id, source, compiled.to_bytes());

// Render time — load the blob (no parsing), then render.
let template = Template::from_bytes(&blob)?;

match template.render::<Receipt>(input) {
    Ok(receipt) => { /* a typed Receipt */ }
    Err(err)    => { /* missing field, type mismatch, … — you decide */ }
}
```

`Receipt` is any `#[derive(Deserialize)]` struct. Every fallible call returns a
`Result` — Temple never panics, and never decides error handling for you.

## How it works

A template is compiled **once**, when it is saved — parsing, validation, and
dependency analysis all happen there. The render path only loads a pre-compiled
blob and evaluates it; it never parses. This is what keeps render latency
bounded (targets: sub-1 ms typical, sub-100 µs simple).

```mermaid
sequenceDiagram
    participant App as Application
    participant Temple
    participant DB as Database

    Note over App,DB: Write time — once, when a template is saved
    App->>Temple: compile(source)
    Temple-->>App: Template
    App->>DB: store source + compiled blob

    Note over App,DB: Render time — on every request
    App->>DB: fetch compiled blob
    DB-->>App: blob
    App->>Temple: from_bytes(blob)
    Temple-->>App: Template (no parsing)
    App->>Temple: render(input)
    Temple-->>App: Ok(value) or Err(RenderError)
```

## How Temple compares

Temple needs four things at once. Existing crates each miss at least one:

| Crate | Sub-ms render | Exact decimals | Typed Rust output | Focused surface |
| --- | :---: | :---: | :---: | :---: |
| `minijinja` | ✓ | ✗ | ✗ | ✗ |
| `jaq` | ✓ | ✗ | ✗ | ✗ |
| `jsonata-core` | ~ | ✗ | ✗ | ✗ |
| `liquid_json` | ~ | ✗ | ✗ | ~ |
| **Temple** | **✓** | **✓** | **✓** | **✓** |

*✓ yes · ~ partial · ✗ no. Our evaluation for Temple's specific needs — not a general verdict on these crates.*

## Project status

Design stage — the design is settled, implementation has not started.

- **[`DESIGN.md`](DESIGN.md)** — format, evaluation model, architecture, decisions, open questions, roadmap.
- **[`samples/`](samples/)** — a worked template for each supported shape.

Not yet published to crates.io. Feedback on the design is welcome — open an issue.

## License

Planned: dual-licensed **MIT OR Apache-2.0**.
