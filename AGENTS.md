# Project coding standards

These rules apply to the entire workspace, including tests and new pull requests.

## Naming and language

- Use English identifiers. Functions and test functions must use `snake_case`.
- Name tests after the behavior or guarantee they verify.
- Write technical comments and Rust documentation in English.
- Keep user-facing CLI help, errors, and product documentation in Portuguese.
- For clap-derived commands, use explicit `about` and `help` attributes for
  Portuguese user-facing text, independent of English Rust doc comments.

## Comments and contracts

- Explain decisions, invariants, platform constraints, and failure behavior.
- Avoid comments that merely repeat an obvious statement.
- Use `///` for public API contracts and `//!` for module documentation.
- Describe guarantees precisely: distinguish per-file atomic replacement from
  a transaction across files, and approximate budgets from exact token counts.
- Keep comments current when changing the corresponding implementation.

## Validation

- Run `cargo fmt --all -- --check` and `cargo test --workspace --locked` after
  changes that affect the workspace.
- Check release compilation when changing workspace configuration or CLI code.
- Validate platform-specific behavior on the corresponding platform; report
  limitations when only other platforms are available.
- Keep changes scoped. An open pull request is not authorization to merge it.
