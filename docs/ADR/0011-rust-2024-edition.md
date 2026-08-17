# ADR 0011 — Rust 2024 edition, including its style edition

Date: 2026-08-17

## Status

Accepted

## Context

The crate was written against the 2021 edition and declares `rust-version = "1.85"`, the
release in which the 2024 edition was stabilised. The declared minimum is therefore
already high enough for 2024: moving does not raise the bar for anyone who compiles
`hunkpick`, neither the CLI nor the library.

Two properties of the 2024 edition matter for this project specifically.

**The dependency resolver becomes rust-version aware.** A 2024-edition package gets
resolver v3 with `incompatible-rust-version = "fallback"`, which refuses to select a
dependency version that requires a newer compiler than `package.rust-version`. Until now
the declared MSRV was only enforced after the fact: `cargo update` was free to pull in a
crate that needs a compiler above 1.85, and the `msrv` job in CI reported it afterwards.
With v3 the situation does not arise.

**`std::env::set_var` becomes unsafe.** `tests/git_config_isolation.rs` sets
`GIT_CONFIG_GLOBAL` for the whole process to prove that an ambient git configuration
cannot reach the test repositories. That file already holds exactly one test for this
reason, so the safety condition (no other thread reads the environment concurrently) is
structural, not incidental.

The migration was measured rather than assumed. `cargo clippy --all-targets --all-features
-- -W rust-2024-compatibility` reports exactly one item across the whole tree — the
`set_var` call above. None of the silent behavioural changes of the edition apply: no
`tail_expr_drop_order`, no `if_let_rescope`, no `static_mut_refs`, no
`impl Trait` lifetime-capture warnings. `cargo fix --edition` produced a single fix.

The edition also selects the style edition rustfmt formats with. The 2024 style edition
adds version-sorted `use` declarations (`x8`, `x16`, `x32`, `x64`, `x128` in that order),
a Unicode-aware "non-lowercase before lowercase" ordering in place of the ASCII one, and
assorted rustfmt bug fixes. On this code base it changes exactly one call: a `roundtrip`
argument that is a single string literal 109 columns wide moves into block form.

## Decision

- **The crate is a 2024-edition crate.** `edition = "2024"` in `Cargo.toml`;
  `rust-version` stays at `1.85`.
- **The style edition follows the edition.** `rustfmt.toml` declares `edition = "2024"`
  and does not pin `style_edition` separately. Pinning the style to 2021 while the code
  is 2024 would create a discrepancy that every later contributor has to be told about,
  in exchange for not reformatting one call.
- **The one unsafe operation carries its safety argument in the source.** The
  `set_var` call is wrapped in an `unsafe` block with a `SAFETY:` comment that names the
  invariant the file already relies on: this binary holds a single test.

## Consequences

- `cargo update` can no longer introduce a dependency that needs a compiler newer than
  the declared MSRV; the `msrv` job in CI turns from the mechanism that catches such a
  change into a second line of defence.
- Contributors need Rust 1.85 or newer to build — the same requirement as before, now
  enforced by the edition as well as by `rust-version`.
- Formatting is reproducible from the manifest alone: `cargo fmt` picks the style edition
  from `edition`, so a checkout formats identically without extra configuration.
- Future edition-gated lints (`unsafe_op_in_unsafe_fn` and the rest of the 2024 defaults)
  apply to new code as it is written, rather than accumulating until a later migration.
