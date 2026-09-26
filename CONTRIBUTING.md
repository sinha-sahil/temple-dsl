# Contributing

Thanks for helping with temple-dsl. This page is the short version of how the
project is built and checked; [`ARCHITECTURE.md`](ARCHITECTURE.md) explains
where things live.

## Setup

You need Rust 1.80 or newer (`rust-version` in `Cargo.toml`). Clone the
repository and run the tests:

```bash
cargo test --all-features
```

The first build installs a pre-commit hook (through `cargo-husky`) that runs
`cargo fmt --check` and `cargo clippy -D warnings`.

## Before opening a pull request

CI runs these; running them first saves a round trip.

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps
cargo build --target wasm32-wasip1 --features json
```

Public items need a doc comment (`#![warn(missing_docs)]`), and the crate
forbids `unsafe`.

## Code rules

- **Layout:** see [`ARCHITECTURE.md`](ARCHITECTURE.md).
- **Comments:** `///` on public items, one line unless there is a contract to
  state. `//` says why, never what.
- **One fact, one place:** operators, built-ins, limits, escapes, error text.
- **Errors:** all in `common/error.rs`; code in messages goes in backticks;
  one wording per kind of error.
- **Names:** spelled out, no prefix that repeats the module.
- **Budget:** a method's up-front cost goes in its table row; it clones
  receiver data only through `copy`, `copy_all` or `copy_object`, which
  charge first.
- **Fast paths:** a counter, cache, fast path or `#[inline]` needs a caller
  or a benchmark that shows it matters.

## Backward compatibility

Templates people have already written must keep compiling, formatting and
rendering the same. Two gates enforce that:

- `tests/compat.rs` compares compile, format, render and blob output for the
  templates in `examples/data/` against goldens in `tests/compat/`. If a change
  is meant to alter them, regenerate with `TEMPLE_BLESS=1 cargo test
  --all-features --test compat` and say so in the pull request.
- `cargo install cargo-semver-checks` and then `cargo semver-checks
  check-release --baseline-rev <last release tag> --all-features` reports any
  breaking change to the public API.

## Running the tests on WebAssembly

```bash
rustup target add wasm32-wasip1
cargo test --target wasm32-wasip1 --features json --no-run
```

Run each printed `.wasm` test binary with a WASI runtime such as `wasmtime`
(`wasmtime --dir . <binary>`) or Node's `node:wasi` module.

## Commits and releases

Commit messages follow conventional commits (`feat:`, `fix:`, `feat!:` for a
breaking change). The release pipeline in `.github/workflows/release.yml`
reads the latest commit on the `release` branch to pick the version bump,
tags it and publishes to crates.io. Add a line to [`CHANGELOG.md`](CHANGELOG.md)
under *Unreleased* with every user-visible change.
