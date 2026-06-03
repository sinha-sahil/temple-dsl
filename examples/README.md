# Temple — examples

Runnable demonstrations of the `temple-dsl` public API. Each file is a Rust
binary you can invoke with `cargo run --example <name>`.

```bash
cargo run --example hello
cargo run --example typed_output
cargo run --example nested_data
cargo run --example many_renders
cargo run --example error_handling
```

## What each shows

| Example | What it demonstrates |
| --- | --- |
| [`hello.rs`](hello.rs) | Smallest end-to-end use — compile, render, deserialize into a struct. |
| [`typed_output.rs`](typed_output.rs) | Multi-field typed output — `String`, `i64`, `Decimal`, `bool`. |
| [`nested_data.rs`](nested_data.rs) | Walking deep paths into structured input. |
| [`many_renders.rs`](many_renders.rs) | Compile once, render many — the canonical reuse pattern. |
| [`error_handling.rs`](error_handling.rs) | `Result` from both `compile` and `render`; Temple never panics. |
| [`payment_request.rs`](payment_request.rs) | Big realistic payload — ~30 fields, four levels of nesting, fixed-shape items array. |
| [`normalize_response.rs`](normalize_response.rs) | Reshape a verbose external API response into a clean canonical struct. |
| [`compiled_blob.rs`](compiled_blob.rs) | The persistence lifecycle — `compile` → `to_bytes` → reload with `from_bytes` (no reparse) → render. |

For executing a `.temple` file directly against a JSON input file, use the
optional `temple` binary (feature-gated, so the library stays lean for
downstream consumers):

```bash
# From inside the repo
cargo run --features cli -- examples/data/customer.temple examples/data/customer.json

# Install once, then call from anywhere as `temple`
cargo install --path . --features cli
temple hello.temple input.json
```

The binary lands in `~/.cargo/bin/temple`. Uninstall with
`cargo uninstall temple-dsl`.

## End-to-end (database) tests

The library has no database dependency by design. The full Postgres
round-trip harness lives outside the crate in [`../e2e/`](../e2e/) — lift it
into a standalone package to run it. See [`../e2e/README.md`](../e2e/README.md).
