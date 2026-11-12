# Project coding standards

These rules apply to the entire workspace, including tests and new pull requests.

## Naming and language

- Use English identifiers. Functions and test functions must use `snake_case`.
- Name tests after the behavior or guarantee they verify.
- Use specific and unique names; avoid generic names such as `data`, `handler`,
  and `Manager`.
- Prefer names that return fewer than five `grep` hits in the codebase.
- Keep types explicit; do not use `any`, `Dict`, or untyped functions.
- Write technical comments and Rust documentation in English.
- Keep user-facing CLI help, errors, and product documentation in Portuguese.
- For clap-derived commands, use explicit `about` and `help` attributes for
  Portuguese user-facing text, independent of English Rust doc comments.

## Comments and contracts

- Explain decisions, invariants, platform constraints, and failure behavior.
- Avoid comments that merely repeat an obvious statement.
- Keep existing comments during refactors; they carry intent and provenance.
- Public function documentation must state intent and include one usage example.
- Reference issue numbers or commit SHAs when a line exists because of a bug or
  upstream constraint.
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

## Code structure

- Keep functions between 4 and 20 lines; split longer functions by
  responsibility.
- Keep files under 500 lines and split modules by responsibility.
- Apply single responsibility: one thing per function and one responsibility
  per module.
- Avoid duplication; extract shared behavior into a function or module.
- Prefer early returns and no more than two levels of indentation.
- Exception messages must include the offending value and expected shape.

## Tests

- Tests run with one documented project-specific command.
- Put crate tests in the crate's `tests/` directory; do not add new tests
  inside production modules under `src/`.
- Every new function requires a test; every bug fix requires a regression test.
- Mock external I/O with named fake types rather than inline stubs.
- Keep tests F.I.R.S.T.: fast, independent, repeatable, self-validating, and
  timely.

## Dependencies and boundaries

- Inject dependencies through constructors or parameters, not globals/imports.
- Wrap third-party libraries behind thin interfaces owned by this project.
- Follow framework conventions and prefer small focused modules over god files.
- Use predictable paths and the language's default formatter.

## Logging

- Use structured JSON for debugging and observability logs.
- Use plain text only for user-facing CLI output.
