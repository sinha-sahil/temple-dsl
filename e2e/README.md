# End-to-end tests (separate from the library)

The core `temple-dsl` crate intentionally has **no Postgres / database dependency** —
those belong in a standalone end-to-end harness, not in the library's dependency tree.

[`postgres_e2e.rs`](postgres_e2e.rs) is that harness's seed: it drives the full
lifecycle against a real Postgres — compile → `to_bytes` → store in a `BYTEA`
column → reload → `from_bytes` → render — exercising the whole language plus the
corrupt-blob / version-mismatch recovery paths.

It is **not** part of the `temple-dsl` build (this directory is not a Cargo target).
To run it, lift it into its own crate that depends on `temple-dsl` and `postgres`:

```toml
# e2e/Cargo.toml  (a separate package, not a temple-dsl example)
[package]
name = "temple-e2e"
version = "0.0.0"
publish = false
edition = "2021"

[dependencies]
temple-dsl = { path = ".." }
postgres = "0.19"
rust_decimal = { version = "1", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
```

Then `src/main.rs` = `postgres_e2e.rs`, and:

```bash
DATABASE_URL=postgres://user@localhost:5432/db cargo run
```
